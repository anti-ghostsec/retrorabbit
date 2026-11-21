//! The TV wrapper around the desktop app: remote-control keys, the on-screen
//! keyboard for the channel editor's text boxes, and the storage-permission rescan.
//!
//! MainActivity.kt turns remote buttons into the desktop's keys (Channel +/- →
//! Page Up/Down, Guide/Menu → G, Info → I, colour buttons → L/S/O/C, Back → Esc).

use crate::android;
use crate::app::RetroRabbit;
use eframe::egui::{self, Event, Key, Modifiers};
use winit::platform::android::activity::AndroidApp;
use winit::platform::android::activity::input::{TextInputState, TextSpan};

pub struct TvApp {
    inner: RetroRabbit,
    app: AndroidApp,
    /// The focused text box's text as last given to (or taken from) the keyboard.
    ime: Option<String>,
    /// The text box `ime` belongs to.
    ime_for: Option<egui::Id>,
    next_permission_check: f64,
}

impl TvApp {
    pub fn new(inner: RetroRabbit, app: AndroidApp) -> Self {
        Self { inner, app, ime: None, ime_for: None, next_permission_check: 0.0 }
    }
}

impl eframe::App for TvApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.inner.ui(ui, frame);
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        if now >= self.next_permission_check {
            self.next_permission_check = now + 1.0;
            if android::take_permission_granted() {
                self.inner.tv_rescan();
            }
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Keep the toolbar and windows clear of notches (egui only knows them on iOS).
        let [l, t, r, b] = android::safe_insets();
        let ppp = ctx.pixels_per_point();
        raw.safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 {
            left: l as f32 / ppp,
            top: t as f32 / ppp,
            right: r as f32 / ppp,
            bottom: b as f32 / ppp,
        }));

        let typing = ctx.egui_wants_keyboard_input();
        // A drop-down is open: Back closes just that (egui does it on Esc).
        let popup = egui::Popup::is_any_open(ctx);
        // Remotes have no Tab: in a menu with nothing focused, the first arrow
        // press focuses the first button, then the arrows move between them.
        let nothing_focused = ctx.memory(|m| m.focused().is_none());
        let menus = self.inner.tv_menus_open();
        let mut keep = Vec::with_capacity(raw.events.len());
        for e in raw.events.drain(..) {
            match &e {
                Event::Key { key: Key::Escape, .. } if popup => {}
                Event::Key { key: Key::Escape, pressed, .. } => {
                    if *pressed {
                        self.inner.tv_back();
                    }
                    continue;
                }
                Event::Key { key: Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight, pressed, .. }
                    if menus && nothing_focused && !typing =>
                {
                    keep.push(Event::Key {
                        key: Key::Tab,
                        physical_key: None,
                        pressed: *pressed,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    });
                    continue;
                }
                Event::Key { key: Key::Enter, pressed: true, .. } if !typing && self.inner.tv_ok() => continue,
                _ => {}
            }
            keep.push(e);
        }
        raw.events = keep;

        // On-screen keyboard: a newly focused text box starts the keyboard empty;
        // after that, whatever the keyboard changes is sent on as typing (new
        // characters, and backspaces for what it took back), at egui's cursor.
        // (egui's own per-widget events can't be relied on here: inside the
        // guide and editor windows they never reach the root output.)
        if typing {
            let focused = ctx.memory(|m| m.focused());
            if focused != self.ime_for || self.ime.is_none() {
                self.ime_for = focused;
                self.ime = Some(String::new());
                self.app.set_text_input_state(TextInputState {
                    text: String::new(),
                    selection: TextSpan { start: 0, end: 0 },
                    compose_region: None,
                });
                log::info!("keyboard for text box {focused:?}");
            } else if let Some(last) = self.ime.clone() {
                let now = self.app.text_input_state().text;
                if now != last {
                    let common = last.chars().zip(now.chars()).take_while(|(a, b)| a == b).count();
                    for _ in common..last.chars().count() {
                        raw.events.push(Event::Key {
                            key: Key::Backspace,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Modifiers::NONE,
                        });
                    }
                    let added: String = now.chars().skip(common).collect();
                    if !added.is_empty() {
                        raw.events.push(Event::Text(added));
                    }
                    self.ime = Some(now);
                }
            }
        } else {
            self.ime = None;
            self.ime_for = None;
        }
    }
}
