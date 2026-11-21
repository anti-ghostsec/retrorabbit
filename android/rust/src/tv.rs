//! The TV wrapper around the desktop app: remote-control keys, the on-screen
//! keyboard for the channel editor's text boxes, and the storage-permission rescan.
//!
//! MainActivity.kt turns remote buttons into the desktop's keys (Channel +/- →
//! Page Up/Down, Guide/Menu → G, Info → I, colour buttons → L/S/O/C, Back → Esc).

use crate::android;
use crate::app::RetroRabbit;
use eframe::egui::output::OutputEvent;
use eframe::egui::{self, Event, Key, Modifiers, Role};
use winit::platform::android::activity::AndroidApp;
use winit::platform::android::activity::input::{TextInputState, TextSpan};

pub struct TvApp {
    inner: RetroRabbit,
    app: AndroidApp,
    /// The focused text box's text as last given to (or taken from) the keyboard.
    ime: Option<String>,
    next_permission_check: f64,
}

impl TvApp {
    pub fn new(inner: RetroRabbit, app: AndroidApp) -> Self {
        Self { inner, app, ime: None, next_permission_check: 0.0 }
    }
}

impl eframe::App for TvApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.inner.ui(ui, frame);
        let ctx = ui.ctx().clone();

        // A text box got focus: hand its text to the keyboard so it can be edited.
        let events = ctx.output(|o| o.events.clone());
        for e in events {
            if let OutputEvent::FocusGained(info) = &e {
                // Shows in logcat (tag RetroRabbit): handy when a remote seems stuck.
                log::info!("focus: {:?} {:?}", info.role, info.label.as_deref().unwrap_or(""));
            }
            if let OutputEvent::FocusGained(info) | OutputEvent::Clicked(info) = e {
                if info.role == Role::TextInput {
                    let text = info.current_text_value.unwrap_or_default();
                    // Passphrase boxes report dots, not the text: start those empty.
                    let text = if text.chars().all(|c| c == '•') { String::new() } else { text };
                    let n = text.len();
                    self.app.set_text_input_state(TextInputState {
                        text: text.clone(),
                        selection: TextSpan { start: n, end: n },
                        compose_region: None,
                    });
                    self.ime = Some(text);
                }
            }
        }
        if !ctx.egui_wants_keyboard_input() {
            self.ime = None;
        }

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

        // The on-screen keyboard changed the text: replace the box's text with it.
        if let Some(last) = &self.ime {
            let st = self.app.text_input_state();
            if st.text != *last {
                let key = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
                raw.events.push(key(Key::A, Modifiers { ctrl: true, command: true, ..Default::default() }));
                if st.text.is_empty() {
                    raw.events.push(key(Key::Backspace, Modifiers::NONE));
                } else {
                    raw.events.push(Event::Text(st.text.clone()));
                }
                self.ime = Some(st.text);
            }
        }
    }
}
