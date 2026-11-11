//! A Windows 95 look for egui: grey 3D bevels, navy title bars, teal desktop.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Painter, Rect, Response, Sense, Stroke, Ui, Vec2, pos2, vec2,
};

pub const FACE: Color32 = Color32::from_rgb(192, 192, 192);
pub const LIGHT: Color32 = Color32::from_rgb(223, 223, 223);
pub const SHADOW: Color32 = Color32::from_rgb(128, 128, 128);
pub const WHITE: Color32 = Color32::WHITE;
pub const BLACK: Color32 = Color32::BLACK;
pub const NAVY: Color32 = Color32::from_rgb(0, 0, 128);
pub const TITLE_END: Color32 = Color32::from_rgb(16, 132, 208);
pub const GUIDE_BG: Color32 = Color32::from_rgb(20, 20, 110);
pub const GUIDE_CELL: Color32 = Color32::from_rgb(40, 40, 170);
pub const YELLOW: Color32 = Color32::from_rgb(255, 255, 85);
pub const TV_GREEN: Color32 = Color32::from_rgb(124, 252, 0);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Light);
    ctx.style_mut_of(egui::Theme::Light, |s| {
        let v = &mut s.visuals;
        v.panel_fill = FACE;
        v.window_fill = FACE;
        v.extreme_bg_color = WHITE;
        v.faint_bg_color = LIGHT;
        v.window_corner_radius = CornerRadius::ZERO;
        v.menu_corner_radius = CornerRadius::ZERO;
        v.window_shadow = egui::Shadow::NONE;
        v.popup_shadow = egui::Shadow::NONE;
        v.window_stroke = Stroke::new(1.0, BLACK);
        v.selection.bg_fill = NAVY;
        v.selection.stroke = Stroke::new(1.0, WHITE);
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = CornerRadius::ZERO;
            w.bg_fill = FACE;
            w.weak_bg_fill = FACE;
            w.expansion = 0.0;
            w.fg_stroke = Stroke::new(1.0, BLACK);
            w.bg_stroke = Stroke::new(1.0, SHADOW);
        }
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, BLACK);
        v.widgets.active.bg_stroke = Stroke::new(1.0, BLACK);
        v.widgets.inactive.bg_fill = WHITE; // checkbox/radio wells
        s.spacing.item_spacing = vec2(6.0, 6.0);
        s.spacing.button_padding = vec2(8.0, 3.0);
    });
}

fn edges(p: &Painter, r: Rect, tl: Color32, br: Color32) {
    let z = CornerRadius::ZERO;
    p.rect_filled(Rect::from_min_max(r.min, pos2(r.max.x, r.min.y + 1.0)), z, tl);
    p.rect_filled(Rect::from_min_max(r.min, pos2(r.min.x + 1.0, r.max.y)), z, tl);
    p.rect_filled(Rect::from_min_max(pos2(r.min.x, r.max.y - 1.0), r.max), z, br);
    p.rect_filled(Rect::from_min_max(pos2(r.max.x - 1.0, r.min.y), r.max), z, br);
}

/// The classic two-pixel 3D border.
pub fn bevel(p: &Painter, r: Rect, raised: bool) {
    if raised {
        edges(p, r, WHITE, BLACK);
        edges(p, r.shrink(1.0), LIGHT, SHADOW);
    } else {
        edges(p, r, SHADOW, WHITE);
        edges(p, r.shrink(1.0), BLACK, LIGHT);
    }
}

/// A shallow one-pixel sunken border (status bar fields).
pub fn shallow(p: &Painter, r: Rect) {
    edges(p, r, SHADOW, WHITE);
}

pub fn button(ui: &mut Ui, text: &str) -> Response {
    button_ex(ui, text, false, 0.0)
}

/// `on`: draw it pushed in (toggle buttons like Mute).
pub fn button_ex(ui: &mut Ui, text: &str, on: bool, min_width: f32) -> Response {
    let font = FontId::proportional(14.0);
    let w = ui.painter().layout_no_wrap(text.to_string(), font.clone(), BLACK).size().x;
    let (rect, resp) = ui.allocate_exact_size(vec2((w + 22.0).max(min_width), 26.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let down = on || resp.is_pointer_button_down_on();
        p.rect_filled(rect, CornerRadius::ZERO, if on { LIGHT } else { FACE });
        bevel(p, rect, !down);
        let off = if down { vec2(1.0, 1.0) } else { Vec2::ZERO };
        p.text(rect.center() + off, Align2::CENTER_CENTER, text, font, BLACK);
        if resp.has_focus() {
            p.rect_stroke(rect.shrink(4.0), CornerRadius::ZERO, Stroke::new(1.0, SHADOW), egui::StrokeKind::Inside);
        }
    }
    resp
}

/// Navy-to-blue gradient title strip.
pub fn title_bar(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(0.0), 22.0), Sense::hover());
    let p = ui.painter();
    let steps = 24;
    for i in 0..steps {
        let t = i as f32 / steps as f32;
        let c = lerp(NAVY, TITLE_END, t);
        let x0 = rect.min.x + rect.width() * t;
        let x1 = rect.min.x + rect.width() * (i + 1) as f32 / steps as f32 + 1.0;
        p.rect_filled(Rect::from_min_max(pos2(x0, rect.min.y), pos2(x1.min(rect.max.x), rect.max.y)), 0.0, c);
    }
    p.text(
        rect.left_center() + vec2(6.0, 0.0),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(14.0),
        WHITE,
    );
}

/// A captioned, etched group box like Win95 property sheets.
pub fn group<R>(ui: &mut Ui, caption: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.add_space(8.0);
    let inner = egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 14, bottom: 10 }).show(ui, add);
    let r = inner.response.rect;
    let p = ui.painter();
    // Etched line: shadow then highlight, offset by one pixel.
    let z = CornerRadius::ZERO;
    p.rect_stroke(r.translate(vec2(1.0, 1.0)), z, Stroke::new(1.0, WHITE), egui::StrokeKind::Inside);
    p.rect_stroke(r, z, Stroke::new(1.0, SHADOW), egui::StrokeKind::Inside);
    let font = FontId::proportional(14.0);
    let tw = p.layout_no_wrap(caption.to_string(), font.clone(), BLACK).size().x;
    let label = Rect::from_min_size(r.min + vec2(8.0, -8.0), vec2(tw + 8.0, 16.0));
    p.rect_filled(label, z, FACE);
    p.text(label.center(), Align2::CENTER_CENTER, caption, font, BLACK);
    inner.inner
}

pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Seven-bar test pattern with a caption box, for channels with no signal.
pub fn color_bars(p: &Painter, r: Rect, caption: &str, sub: &str) {
    let bars = [
        Color32::from_rgb(191, 191, 191),
        Color32::from_rgb(191, 191, 0),
        Color32::from_rgb(0, 191, 191),
        Color32::from_rgb(0, 191, 0),
        Color32::from_rgb(191, 0, 191),
        Color32::from_rgb(191, 0, 0),
        Color32::from_rgb(0, 0, 191),
    ];
    let w = r.width() / bars.len() as f32;
    for (i, c) in bars.iter().enumerate() {
        let x = r.min.x + w * i as f32;
        p.rect_filled(Rect::from_min_max(pos2(x, r.min.y), pos2(x + w + 1.0, r.max.y)), 0.0, *c);
    }
    let bx = Rect::from_center_size(r.center(), vec2((r.width() * 0.8).min(520.0), 86.0));
    p.rect_filled(bx, 0.0, BLACK);
    p.text(bx.center() - vec2(0.0, 14.0), Align2::CENTER_CENTER, caption, FontId::monospace(24.0), WHITE);
    p.text(bx.center() + vec2(0.0, 20.0), Align2::CENTER_CENTER, sub, FontId::proportional(14.0), LIGHT);
}

/// A filled five-point star (drawn, since not every font has ★).
pub fn star(p: &Painter, c: egui::Pos2, r: f32, color: Color32) {
    let pt = |i: usize, rad: f32| {
        let a = (-90.0 + 36.0 * i as f32).to_radians();
        c + vec2(a.cos(), a.sin()) * rad
    };
    let inner: Vec<_> = (0..5).map(|k| pt(2 * k + 1, r * 0.42)).collect();
    p.add(egui::Shape::convex_polygon(inner.clone(), color, Stroke::NONE));
    for k in 0..5 {
        let tri = vec![inner[(k + 4) % 5], pt(2 * k, r), inner[k]];
        p.add(egui::Shape::convex_polygon(tri, color, Stroke::NONE));
    }
}
