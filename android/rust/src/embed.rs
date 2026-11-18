//! Android: ExoPlayer (TvPlayer.kt) decodes into a SurfaceTexture bound to an
//! external OpenGL texture. Each new frame is copied, letterboxed, into the
//! texture egui shows where the TV screen is. The channel-number OSD that mpv
//! draws on desktop is drawn here by egui.

use crate::android;
use crate::gltex::VideoTexture;
use eframe::egui::{self, Align2, Color32, FontId, Rect, Ui, vec2};
use eframe::glow::{self, HasContext};
use jni::objects::{GlobalRef, JFloatArray, JValue};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const TEXTURE_EXTERNAL_OES: u32 = 0x8D65;

/// What the video window and the player share: the Java player and the OSD text.
pub struct Shared {
    pub player: GlobalRef,
    pub osd: Mutex<Option<(String, Instant)>>,
}

pub struct Parent {
    gl: Arc<glow::Context>,
}

pub fn parent(cc: &eframe::CreationContext<'_>) -> Result<Parent, String> {
    android::set_egui(&cc.egui_ctx);
    let gl = cc.gl.clone().ok_or("No OpenGL context.")?;
    Ok(Parent { gl })
}

struct Blit {
    program: glow::Program,
    buf: glow::Buffer,
    vao: Option<glow::VertexArray>,
    u_tex: Option<glow::UniformLocation>,
    u_m: Option<glow::UniformLocation>,
}

const VERT: &str = "#version 100
attribute vec2 pos;
uniform mat4 m;
varying vec2 uv;
void main() {
    // egui reads texture row 0 as the top of the picture, GL as the bottom: flip.
    vec2 t = vec2(pos.x * 0.5 + 0.5, 0.5 - pos.y * 0.5);
    uv = (m * vec4(t, 0.0, 1.0)).xy;
    gl_Position = vec4(pos, 0.0, 1.0);
}";

const FRAG: &str = "#version 100
#extension GL_OES_EGL_image_external : require
precision mediump float;
uniform samplerExternalOES tex;
varying vec2 uv;
void main() { gl_FragColor = vec4(texture2D(tex, uv).rgb, 1.0); }";

impl Blit {
    unsafe fn new(gl: &glow::Context) -> Result<Self, String> {
        // SAFETY (whole fn): GL thread, eframe's context current.
        unsafe {
            let program = gl.create_program()?;
            for (kind, src) in [(glow::VERTEX_SHADER, VERT), (glow::FRAGMENT_SHADER, FRAG)] {
                let s = gl.create_shader(kind)?;
                gl.shader_source(s, src);
                gl.compile_shader(s);
                if !gl.get_shader_compile_status(s) {
                    return Err(gl.get_shader_info_log(s));
                }
                gl.attach_shader(program, s);
            }
            gl.bind_attrib_location(program, 0, "pos");
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(gl.get_program_info_log(program));
            }
            let buf = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buf));
            let quad: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
            let bytes: Vec<u8> = quad.iter().flat_map(|f| f.to_ne_bytes()).collect();
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            let vao = gl.create_vertex_array().ok();
            let u_tex = gl.get_uniform_location(program, "tex");
            let u_m = gl.get_uniform_location(program, "m");
            Ok(Self { program, buf, vao, u_tex, u_m })
        }
    }

    /// Draw the external texture into `fb` (w×h), letterboxed to `aspect`.
    unsafe fn draw(&self, gl: &glow::Context, oes: glow::Texture, fb: glow::Framebuffer, w: i32, h: i32, m: &[f32; 16], aspect: f32) {
        // SAFETY (whole fn): GL thread, eframe's context current.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.viewport(0, 0, w, h);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            let (vw, vh) = if aspect > 0.0 {
                let box_aspect = w as f32 / h as f32;
                if aspect > box_aspect { (w, (w as f32 / aspect).round() as i32) } else { ((h as f32 * aspect).round() as i32, h) }
            } else {
                (w, h)
            };
            gl.viewport((w - vw) / 2, (h - vh) / 2, vw, vh);
            gl.use_program(Some(self.program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(TEXTURE_EXTERNAL_OES, Some(oes));
            gl.uniform_1_i32(self.u_tex.as_ref(), 0);
            gl.uniform_matrix_4_f32_slice(self.u_m.as_ref(), false, m);
            if let Some(v) = self.vao {
                gl.bind_vertex_array(Some(v));
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buf));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.disable_vertex_attrib_array(0);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            if self.vao.is_some() {
                gl.bind_vertex_array(None);
            }
            gl.bind_texture(TEXTURE_EXTERNAL_OES, None);
            gl.use_program(None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
    }
}

pub struct VideoWindow {
    shared: Arc<Shared>,
    oes: glow::Texture,
    blit: Blit,
    /// 16 floats of SurfaceTexture transform, then video width and height.
    out: GlobalRef,
    tex: VideoTexture,
    visible: bool,
}

impl VideoWindow {
    pub fn new(p: Parent) -> Result<Self, String> {
        let gl = &p.gl;
        // SAFETY: GL thread, eframe's context current during app creation.
        let (oes, blit) = unsafe {
            let t = gl.create_texture()?;
            gl.bind_texture(TEXTURE_EXTERNAL_OES, Some(t));
            gl.tex_parameter_i32(TEXTURE_EXTERNAL_OES, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(TEXTURE_EXTERNAL_OES, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(TEXTURE_EXTERNAL_OES, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(TEXTURE_EXTERNAL_OES, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            gl.bind_texture(TEXTURE_EXTERNAL_OES, None);
            (t, Blit::new(gl)?)
        };
        let id = oes.0.get() as i32;
        let (player, out) = android::with(|env, act| {
            let p = env
                .call_method(act, "makePlayer", "(I)Lorg/retrorabbit/tv/TvPlayer;", &[JValue::Int(id)])?
                .l()?;
            let arr = env.new_float_array(18)?;
            Ok((env.new_global_ref(p)?, env.new_global_ref(arr)?))
        })
        .ok_or("Couldn't start the video player.")?;
        Ok(Self {
            shared: Arc::new(Shared { player, osd: Mutex::new(None) }),
            oes,
            blit,
            out,
            tex: VideoTexture::new(),
            visible: false,
        })
    }

    pub fn handle(&self) -> Arc<Shared> {
        self.shared.clone()
    }

    pub fn prepare(&mut self, frame: &mut eframe::Frame) {
        let Some(gl) = frame.gl().cloned() else { return };
        let player = self.shared.player.clone();
        let out = self.out.clone();
        // TvPlayer.update() calls SurfaceTexture.updateTexImage (needs our GL thread).
        let got = android::with(|env, _| {
            let arr: &JFloatArray = out.as_obj().into();
            let fresh = env.call_method(player.as_obj(), "update", "([F)Z", &[JValue::Object(arr)])?.z()?;
            let mut v = [0f32; 18];
            env.get_float_array_region(arr, 0, &mut v)?;
            Ok((fresh, v))
        });
        let Some((fresh_frame, v)) = got else { return };
        let Some((fb, w, h, fresh_tex)) = self.tex.target(frame) else { return };
        if !fresh_frame && !fresh_tex {
            return;
        }
        let mut m = [0f32; 16];
        m.copy_from_slice(&v[..16]);
        let aspect = if v[17] > 0.0 { v[16] / v[17] } else { 0.0 };
        // SAFETY: GL thread, eframe's context current.
        unsafe { self.blit.draw(&gl, self.oes, fb, w, h, &m, aspect) };
    }

    pub fn show(&mut self, ui: &Ui, screen: Rect, visible: bool) {
        self.tex.show(ui, screen, visible);
        if visible != self.visible {
            self.visible = visible;
            ui.ctx().request_repaint();
        }
        if !visible {
            return;
        }
        let mut osd = self.shared.osd.lock().unwrap();
        let Some((text, until)) = osd.as_ref() else { return };
        let now = Instant::now();
        if now >= *until {
            *osd = None;
            return;
        }
        ui.ctx().request_repaint_after(*until - now);
        // Like mpv's OSD on desktop: green monospace, black border, top right.
        let p = ui.painter().with_clip_rect(screen);
        let font = FontId::monospace(30.0);
        let mut y = screen.min.y + 30.0;
        for line in text.lines() {
            let pos = egui::pos2(screen.max.x - 40.0, y);
            for (dx, dy) in [(-2.0, 0.0), (2.0, 0.0), (0.0, -2.0), (0.0, 2.0), (-2.0, -2.0), (2.0, 2.0), (-2.0, 2.0), (2.0, -2.0)] {
                p.text(pos + vec2(dx, dy), Align2::RIGHT_TOP, line, font.clone(), Color32::BLACK);
            }
            p.text(pos, Align2::RIGHT_TOP, line, font.clone(), crate::win95::TV_GREEN);
            y += 36.0;
        }
    }
}
