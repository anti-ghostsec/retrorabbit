//! An OpenGL texture + framebuffer that a video decoder draws into and egui shows
//! as an image. Used where the player can't have a window of its own (macOS, Android).

use eframe::egui::{self, Rect, TextureId, Ui, pos2};
use eframe::glow::{self, HasContext};

pub struct VideoTexture {
    tex: Option<(glow::Texture, glow::Framebuffer, TextureId)>,
    /// Size of the texture now, and the size egui last asked for (pixels).
    size: (i32, i32),
    want: (i32, i32),
    /// The decoder draws bottom-up (OpenGL's way): show it flipped.
    flip: bool,
}

impl VideoTexture {
    pub fn new() -> Self {
        Self { tex: None, size: (0, 0), want: (0, 0), flip: false }
    }

    pub fn flipped() -> Self {
        Self { flip: true, ..Self::new() }
    }

    /// Make sure the texture matches the screen size. Returns the framebuffer to
    /// draw into and its size, and whether it was just (re)made.
    pub fn target(&mut self, frame: &mut eframe::Frame) -> Option<(glow::Framebuffer, i32, i32, bool)> {
        let (w, h) = self.want;
        if w < 2 || h < 2 {
            return None;
        }
        let gl = frame.gl()?.clone();
        let mut fresh = false;
        if self.tex.is_none() || self.size != (w, h) {
            // SAFETY: called on the GL thread with eframe's context current.
            unsafe {
                if let Some((t, f, _)) = self.tex.take() {
                    gl.delete_framebuffer(f);
                    gl.delete_texture(t);
                }
                let t = gl.create_texture().ok()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    w,
                    h,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
                for (p, v) in [
                    (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                    (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                    (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                    (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
                ] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, p, v as i32);
                }
                gl.bind_texture(glow::TEXTURE_2D, None);
                let f = gl.create_framebuffer().ok()?;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(f));
                gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(t), 0);
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                let id = frame.register_native_glow_texture(t);
                self.tex = Some((t, f, id));
            }
            self.size = (w, h);
            fresh = true;
        }
        self.tex.as_ref().map(|(_, f, _)| (*f, w, h, fresh))
    }

    /// Show the picture over `screen` (and remember its size for next frame).
    pub fn show(&mut self, ui: &Ui, screen: Rect, visible: bool) {
        let ppp = ui.ctx().pixels_per_point();
        self.want = ((screen.width() * ppp).round() as i32, (screen.height() * ppp).round() as i32);
        if let (true, Some((_, _, id))) = (visible, &self.tex) {
            let (top, bottom) = if self.flip { (1.0, 0.0) } else { (0.0, 1.0) };
            let uv = Rect::from_min_max(pos2(0.0, top), pos2(1.0, bottom));
            ui.painter().image(*id, screen, uv, egui::Color32::WHITE);
        }
    }
}
