//! macOS: mpv runs inside RetroRabbit and renders each frame into a texture that
//! egui draws where the TV screen is (see mpv.rs for why).

use crate::mpv::{self, Mpv, OpenGlFbo, OpenGlInitParams, RenderParam};
use crate::gltex::VideoTexture;
use eframe::egui::{Rect, Ui};
use eframe::glow::{self, HasContext};
use std::ffi::{CStr, c_char, c_void};

/// mpv settings for the video core (the Linux build passes the same on mpv's command line).
pub const VIDEO_OPTIONS: &[&str] = &[
    "vo=libmpv",
    "idle=yes",
    "keep-open=no",
    "config=no",
    "load-scripts=no",
    "ytdl=no",
    "input-default-bindings=no",
    "input-vo-keyboard=no",
    "cache-on-disk=no",
    "resume-playback=no",
    "save-position-on-quit=no",
    "audio-client-name=RetroRabbit",
    "terminal=no",
    "cache=yes",
    "demuxer-max-bytes=64MiB",
    "demuxer-max-back-bytes=16MiB",
    "network-timeout=20",
    // Hardware decoding, copied back: VideoToolbox's zero-copy hand-off to
    // OpenGL showed black/garbled frames inside our texture.
    "hwdec=auto-copy-safe",
    "hr-seek=no",
    "osd-level=1",
    "osd-font=Menlo",
    "osd-font-size=44",
    "osd-color=#7CFC00",
    "osd-border-color=#000000",
    "osd-border-size=3",
    "osd-align-x=right",
    "osd-align-y=top",
    "osd-margin-x=40",
    "osd-margin-y=30",
];

pub struct Parent {
    mpv: Mpv,
    render: mpv::Handle,
    ctx: eframe::egui::Context,
}

struct ProcCtx<'a>(&'a dyn Fn(&CStr) -> *const c_void);

unsafe extern "C" fn get_proc(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: ctx is the ProcCtx passed below, alive for mpv_render_context_create.
    unsafe {
        let f = &*(ctx as *const ProcCtx);
        (f.0)(CStr::from_ptr(name)) as *mut c_void
    }
}

unsafe extern "C" fn on_update(ctx: *mut c_void) {
    // SAFETY: ctx is the leaked egui::Context below.
    unsafe { (*(ctx as *const eframe::egui::Context)).request_repaint() }
}

/// Start the video mpv and hook its renderer to our OpenGL context.
pub fn parent(cc: &eframe::CreationContext<'_>) -> Result<Parent, String> {
    let get = cc.get_proc_address.clone().ok_or("No OpenGL context.")?;
    let mut opts: Vec<String> = VIDEO_OPTIONS.iter().map(|s| s.to_string()).collect();
    // RETRORABBIT_DEBUG=1: mpv logs to stderr (never to a file).
    if std::env::var_os("RETRORABBIT_DEBUG").is_some() {
        opts.extend(["terminal=yes".to_string(), "msg-level=all=v".to_string()]);
    }
    let mpv = Mpv::new(&opts)?;
    let api = mpv::api()?;
    let mut pc = ProcCtx(&*get);
    let mut init = OpenGlInitParams { get_proc_address: get_proc, get_proc_address_ctx: &mut pc as *mut ProcCtx as *mut c_void };
    let api_type = c"opengl";
    let mut params = [
        RenderParam { kind: mpv::RENDER_PARAM_API_TYPE, data: api_type.as_ptr() as *mut c_void },
        RenderParam { kind: mpv::RENDER_PARAM_OPENGL_INIT_PARAMS, data: &mut init as *mut _ as *mut c_void },
        RenderParam { kind: mpv::RENDER_PARAM_INVALID, data: std::ptr::null_mut() },
    ];
    let mut render: mpv::Handle = std::ptr::null_mut();
    // SAFETY: params are valid and outlive the call; GL context is current in eframe's creator.
    let r = unsafe { (api.render_context_create)(&mut render, mpv.0, params.as_mut_ptr()) };
    if r < 0 {
        return Err(format!("mpv renderer: {}", mpv::error_string(r)));
    }
    let ctx = Box::leak(Box::new(cc.egui_ctx.clone()));
    // SAFETY: render is valid; ctx lives forever.
    unsafe { (api.render_context_set_update_callback)(render, Some(on_update), ctx as *mut _ as *mut c_void) };
    Ok(Parent { mpv, render, ctx: cc.egui_ctx.clone() })
}

pub struct VideoWindow {
    mpv: Mpv,
    render: mpv::Handle,
    tex: VideoTexture,
    ctx: eframe::egui::Context,
    /// mpv has a frame for us that we haven't drawn yet. mpv's video output
    /// waits for it, so it must not get lost (e.g. before the texture exists).
    pending: bool,
}

impl VideoWindow {
    pub fn new(p: Parent) -> Result<Self, String> {
        Ok(Self { mpv: p.mpv, render: p.render, tex: VideoTexture::flipped(), ctx: p.ctx, pending: false })
    }

    pub fn handle(&self) -> Mpv {
        self.mpv
    }

    /// Draw mpv's latest frame into our texture (on the GL thread, before egui paints).
    pub fn prepare(&mut self, frame: &mut eframe::Frame) {
        let Ok(api) = mpv::api() else { return };
        // SAFETY: render context is valid; called on the GL thread.
        let flags = unsafe { (api.render_context_update)(self.render) };
        if flags & mpv::RENDER_UPDATE_FRAME != 0 {
            self.pending = true;
        }
        // egui only repaints when asked; mpv must be drawn promptly, every frame
        // it makes, so keep the UI ticking while mpv runs.
        self.ctx.request_repaint();
        let Some((fb, w, h, fresh)) = self.tex.target(frame) else { return };
        if !self.pending && !fresh {
            return;
        }
        self.pending = false;
        let mut fbo = OpenGlFbo { fbo: fb.0.get() as i32, w, h, internal_format: 0 };
        // mpv draws bottom-up for flip_y=1 (clean on every Mac we tried; 0 showed
        // block corruption), so the texture is shown flipped (VideoTexture::flipped).
        let mut flip: i32 = 1;
        let mut params = [
            RenderParam { kind: mpv::RENDER_PARAM_OPENGL_FBO, data: &mut fbo as *mut _ as *mut c_void },
            RenderParam { kind: mpv::RENDER_PARAM_FLIP_Y, data: &mut flip as *mut _ as *mut c_void },
            RenderParam { kind: mpv::RENDER_PARAM_INVALID, data: std::ptr::null_mut() },
        ];
        // SAFETY: params outlive the call; eframe's GL context is current.
        unsafe {
            // egui leaves its last clip rectangle as the scissor: mpv would only
            // draw inside it (the rest of the texture keeps stale garbage).
            // Also hand mpv default upload/binding state, as libmpv expects.
            if let Some(gl) = frame.gl() {
                gl.disable(glow::SCISSOR_TEST);
                gl.disable(glow::BLEND);
                gl.disable(glow::CULL_FACE);
                gl.disable(glow::DEPTH_TEST);
                gl.color_mask(true, true, true, true);
                gl.bind_vertex_array(None);
                gl.use_program(None);
                gl.bind_buffer(glow::ARRAY_BUFFER, None);
                gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
                gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, None);
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
                gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
                gl.pixel_store_i32(glow::UNPACK_SKIP_ROWS, 0);
                gl.pixel_store_i32(glow::UNPACK_SKIP_PIXELS, 0);
            }
            (api.render_context_render)(self.render, params.as_mut_ptr());
            if let Some(gl) = frame.gl() {
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            }
        }
    }

    pub fn show(&mut self, ui: &Ui, screen: Rect, visible: bool) {
        self.tex.show(ui, screen, visible);
    }
}
