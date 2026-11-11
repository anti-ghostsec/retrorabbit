use crate::archive::{Archive, State as ArchiveState};
use crate::embed::VideoWindow;
use crate::library::{Library, Media};
use crate::player::Player;
use crate::schedule::{self, Channel, Liked, Lineup, Order, Rule, Slot, SplitMix};
use crate::vault;
use crate::win95::{self, BLACK, FACE, GUIDE_BG, GUIDE_CELL, NAVY, SHADOW, TV_GREEN, WHITE, YELLOW};
use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, Key, Rect, Sense, TextureHandle, TextureOptions, Ui,
    ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::HashMap;
use std::sync::Arc;
use zeroize::Zeroizing;

const STATIC_SECS: f64 = 0.45;
/// Guide zoom levels: how much time the grid shows at once.
const GUIDE_SPANS: &[(f64, &str)] = &[(1800.0, "30 min"), (3600.0, "1 hr"), (7200.0, "2 hr"), (4.0 * 3600.0, "4 hr")];

type Repaint = Arc<dyn Fn() + Send + Sync>;

pub struct RetroRabbit {
    lib: Arc<Library>,
    archive: Arc<Archive>,
    lineup: Lineup,
    playlists: HashMap<usize, (u64, Arc<Vec<Arc<Media>>>)>,
    repaint: Repaint,
    last_lib_gen: u64,
    liked_gen: u64,

    current: usize,
    previous: usize,
    loaded: Option<(String, f64)>,
    /// When the program that's playing ends (unix seconds).
    loaded_end: f64,
    player: Option<Player>,
    video: Option<VideoWindow>,
    /// Second, audio-only mpv for the static hiss.
    hiss: Option<Player>,
    hiss_on: bool,
    static_this_frame: bool,
    player_error: Option<String>,

    time: f64,
    static_until: f64,
    banner_pending: bool,
    /// Just changed channel: keep the static up until the new picture plays.
    tuning: bool,
    tuning_since: f64,
    digits: String,
    /// Partial mouse-wheel notches (touchpads scroll in small steps).
    wheel: f32,
    digits_deadline: f64,
    fullscreen: bool,
    muted: bool,
    noise: Option<TextureHandle>,
    rng: SplitMix,

    show_guide: bool,
    guide_span: f64,
    /// Pages forward/back in time, in units of the current span.
    guide_page: i32,
    show_editor: bool,
    /// Channel asked for on the command line; tuned once it exists.
    want_channel: Option<u32>,
    ed_selected: usize,
    ed_new_root: String,
    ed_pass: Zeroizing<String>,
    ed_status: String,
    saved_to_disk: bool,
}

impl RetroRabbit {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        win95::apply(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let repaint: Repaint = Arc::new(move || ctx.request_repaint());

        // Embed mpv in our window (X11 only for now).
        let parent = cc.window_handle().ok().and_then(|h| match h.as_raw() {
            RawWindowHandle::Xlib(x) => Some(x.window as u32),
            RawWindowHandle::Xcb(x) => Some(x.window.get()),
            _ => None,
        });
        let mut lineup = Lineup::default();
        let args = crate::Args::parse();
        if !args.folders.is_empty() {
            lineup.media_roots = args.folders.clone();
        }
        let (video, player, player_error) = match parent {
            None => (None, None, Some("Video needs an X11 session (or XWayland).".to_string())),
            Some(parent) => match VideoWindow::new(parent) {
                Err(e) => (None, None, Some(format!("Couldn't make the video window: {e}"))),
                Ok(v) => match Player::spawn(v.id, lineup.volume, lineup.even_volume) {
                    Ok(p) => (Some(v), Some(p), None),
                    Err(e) => (Some(v), None, Some(e)),
                },
            },
        };

        let hiss = if player.is_some() { Player::spawn_hiss(lineup.volume).ok() } else { None };
        let lib = Library::new();
        let mut app = Self {
            lib,
            archive: Archive::new(),
            lineup,
            playlists: HashMap::new(),
            repaint,
            last_lib_gen: 0,
            liked_gen: 0,
            current: 0,
            previous: 0,
            loaded: None,
            loaded_end: 0.0,
            player,
            video,
            hiss,
            hiss_on: false,
            static_this_frame: false,
            player_error,
            time: 0.0,
            static_until: STATIC_SECS,
            banner_pending: true,
            tuning: true,
            tuning_since: 0.0,
            digits: String::new(),
            wheel: 0.0,
            digits_deadline: 0.0,
            fullscreen: false,
            muted: false,
            noise: None,
            rng: SplitMix(unix_now().to_bits()),
            show_guide: args.guide,
            guide_span: 3600.0,
            guide_page: 0,
            show_editor: args.channels,
            want_channel: args.channel,
            ed_selected: 0,
            ed_new_root: String::new(),
            ed_pass: Zeroizing::new(String::new()),
            ed_status: String::new(),
            saved_to_disk: false,
        };
        app.lineup.channels = schedule::auto_lineup(&[], &app.lineup.media_roots);
        app.rescan();
        app
    }

    fn rescan(&mut self) {
        let r = self.repaint.clone();
        self.lib.scan(self.lineup.media_roots.clone(), move || r());
    }

    // ---- data ------------------------------------------------------------

    fn sync_sources(&mut self) {
        let lib_gen = self.lib.generation();
        if lib_gen != self.last_lib_gen {
            self.last_lib_gen = lib_gen;
            if self.lineup.auto {
                self.rebuild_auto_lineup();
            }
        }
        if let Some(n) = self.want_channel {
            if let Some(i) = self.lineup.channels.iter().position(|c| c.number == n) {
                self.want_channel = None;
                self.tune(i);
            } else if lib_gen > 0 && !self.lib.status().running {
                self.want_channel = None;
            }
        }
        for ch in &self.lineup.channels {
            for r in &ch.rules {
                if let Rule::Archive(q) = r {
                    self.archive.request(q, self.repaint.clone());
                }
            }
        }
        // The channel you're on loads first.
        if let Some(ch) = self.lineup.channels.get(self.current) {
            for r in ch.rules.iter().rev() {
                if let Rule::Archive(q) = r {
                    self.archive.prioritize(q);
                }
            }
        }
    }

    fn rebuild_auto_lineup(&mut self) {
        let keep = self.lineup.channels.get(self.current).map(|c| c.number);
        self.lineup.channels = schedule::auto_lineup(&self.lib.items(), &self.lineup.media_roots);
        self.current = keep
            .and_then(|n| self.lineup.channels.iter().position(|c| c.number == n))
            .unwrap_or(0);
        self.previous = self.current;
        self.playlists.clear();
    }

    fn playlist(&mut self, idx: usize) -> Arc<Vec<Arc<Media>>> {
        if idx >= self.lineup.channels.len() {
            return Arc::new(Vec::new());
        }
        // Wait until every Internet Archive search on the channel is in, so the
        // schedule doesn't jump when the second one arrives.
        if matches!(self.channel_status(idx), Some(ArchiveState::Loading | ArchiveState::Queued)) {
            return Arc::new(Vec::new());
        }
        let Some(ch) = self.lineup.channels.get(idx) else { return Arc::new(Vec::new()) };
        let sig = {
            let s = format!("{:?}|{}|{}|{}", ch, self.lib.generation(), self.archive.generation(), self.liked_gen);
            s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, c| (h ^ c as u64).wrapping_mul(0x100_0000_01b3))
        };
        if let Some((s, list)) = self.playlists.get(&idx) {
            if *s == sig {
                return list.clone();
            }
            // More programs arrived for the channel you're watching: switch to the
            // new schedule when this program ends, not in the middle of it.
            if idx == self.current && self.loaded.is_some() && unix_now() < self.loaded_end && !list.is_empty() {
                return list.clone();
            }
        }
        let list = Arc::new(schedule::build_playlist(ch, &self.lib.items(), &self.archive, &self.lineup.liked));
        self.playlists.insert(idx, (sig, list.clone()));
        list
    }

    fn channel_status(&self, idx: usize) -> Option<ArchiveState> {
        let ch = self.lineup.channels.get(idx)?;
        let (mut queued, mut failed) = (false, None);
        for r in &ch.rules {
            if let Rule::Archive(q) = r {
                match self.archive.state(q) {
                    Some(ArchiveState::Loading) => return Some(ArchiveState::Loading),
                    Some(ArchiveState::Queued) | None => queued = true,
                    Some(ArchiveState::Failed(e)) => failed = Some(ArchiveState::Failed(e)),
                    Some(ArchiveState::Ready) => {}
                }
            }
        }
        if queued { Some(ArchiveState::Queued) } else { failed }
    }

    /// Combined loading progress of a channel's Internet Archive searches.
    fn channel_progress(&self, idx: usize) -> Option<(usize, usize)> {
        let ch = self.lineup.channels.get(idx)?;
        let (mut d, mut t) = (0, 0);
        for r in &ch.rules {
            if let Rule::Archive(q) = r {
                if let Some((a, b)) = self.archive.progress(q) {
                    d += a;
                    t += b;
                }
            }
        }
        (t > 0).then_some((d, t))
    }

    /// Channel indices sorted by channel number (what Ch+/Ch- walks through).
    fn dial(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.lineup.channels.len()).collect();
        v.sort_by_key(|&i| self.lineup.channels[i].number);
        v
    }

    /// The channels Ch+/Ch- stop on: favorites only, if that's switched on.
    fn surf_dial(&self) -> Vec<usize> {
        let all = self.dial();
        if !self.lineup.favorites_only {
            return all;
        }
        let favs: Vec<usize> = all.iter().copied().filter(|&i| self.lineup.channels[i].favorite).collect();
        if favs.is_empty() { all } else { favs }
    }

    fn toggle_favorite_channel(&mut self) {
        let Some(ch) = self.lineup.channels.get_mut(self.current) else { return };
        ch.favorite = !ch.favorite;
        let msg = if ch.favorite { format!("★ {} added to favorites", ch.name) } else { format!("{} removed from favorites", ch.name) };
        self.lineup.auto = false;
        if let Some(p) = &mut self.player {
            p.osd(&msg, 2000);
        }
    }

    fn toggle_favorites_only(&mut self) {
        self.lineup.favorites_only = !self.lineup.favorites_only;
        let msg = if self.lineup.favorites_only { "FAVORITES ONLY" } else { "ALL CHANNELS" };
        if let Some(p) = &mut self.player {
            p.osd(msg, 1500);
        }
    }

    fn current_slot(&mut self, wall: f64) -> Option<Slot> {
        let ch = self.lineup.channels.get(self.current)?.clone();
        schedule::now_playing(&ch, &self.playlist(self.current), wall)
    }

    fn is_liked(&self, source: &str) -> bool {
        self.lineup.liked.iter().any(|l| l.source == source)
    }

    /// Like (or un-like) whatever is airing right now.
    fn toggle_like(&mut self, wall: f64) {
        let Some(s) = self.current_slot(wall) else { return };
        let m = &s.media;
        let msg = if let Some(i) = self.lineup.liked.iter().position(|l| l.source == m.source) {
            self.lineup.liked.remove(i);
            format!("Unliked: {}", m.title)
        } else {
            self.lineup.liked.push(Liked {
                title: m.title.clone(),
                source: m.source.clone(),
                duration: m.duration,
                tags: m.tags.clone(),
            });
            format!("♥ Liked: {}\nIt'll air on Favorites", m.title)
        };
        self.liked_gen += 1;
        if let Some(p) = &mut self.player {
            p.osd(&msg, 2500);
        }
    }

    // ---- tuning ------------------------------------------------------------

    fn tune(&mut self, idx: usize) {
        if idx >= self.lineup.channels.len() {
            return;
        }
        if idx != self.current {
            self.previous = self.current;
        }
        self.current = idx;
        self.static_until = self.time + STATIC_SECS;
        self.banner_pending = true;
        self.tuning = true;
        self.tuning_since = self.time;
        self.loaded = None;
        // Cut the old channel immediately, even if the new one is still loading.
        if let Some(p) = &mut self.player {
            p.stop();
        }
    }

    fn step(&mut self, d: i32) {
        let dial = self.surf_dial();
        if dial.is_empty() {
            return;
        }
        let pos = dial.iter().position(|&i| i == self.current).unwrap_or(0) as i32;
        let next = (pos + d).rem_euclid(dial.len() as i32) as usize;
        self.tune(dial[next]);
    }

    fn tune_number(&mut self, n: u32) {
        if let Some(i) = self.lineup.channels.iter().position(|c| c.number == n) {
            self.tune(i);
        }
    }

    fn set_volume(&mut self, v: f64) {
        self.lineup.volume = v.clamp(0.0, 130.0);
        if let Some(h) = &mut self.hiss {
            h.set_hiss_volume(self.lineup.volume);
        }
        if let Some(p) = &mut self.player {
            p.set_volume(self.lineup.volume);
            p.osd(&format!("VOL {:.0}", self.lineup.volume), 1200);
        }
    }

    fn toggle_even_volume(&mut self) {
        self.lineup.even_volume = !self.lineup.even_volume;
        let on = self.lineup.even_volume;
        if let Some(p) = &mut self.player {
            p.set_even_volume(on);
            p.osd(if on { "EVEN VOLUME ON" } else { "EVEN VOLUME OFF" }, 1500);
        }
    }

    fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        if let Some(h) = &mut self.hiss {
            h.set_mute(self.muted);
        }
        if let Some(p) = &mut self.player {
            p.set_mute(self.muted);
            p.osd(if self.muted { "MUTE" } else { "" }, if self.muted { 2000 } else { 1 });
        }
    }

    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(self.fullscreen));
    }

    fn show_banner(&mut self, wall: f64) {
        let Some(ch) = self.lineup.channels.get(self.current).cloned() else { return };
        let list = self.playlist(self.current);
        let now = schedule::now_playing(&ch, &list, wall).map(|s| s.media.title.clone()).unwrap_or_default();
        if let Some(p) = &mut self.player {
            p.osd(&format!("{:02}\n{}\n{}", ch.number, ch.name, now), 3500);
        }
    }

    /// Keep the player in step with the schedule.
    fn tick(&mut self, wall: f64) {
        if let Some(p) = &mut self.player {
            if !p.alive() {
                self.player = None;
                self.player_error = Some("mpv stopped unexpectedly.".into());
            }
        }
        let slot = match self.lineup.channels.get(self.current).cloned() {
            Some(ch) => schedule::now_playing(&ch, &self.playlist(self.current), wall),
            None => None,
        };
        let Some(p) = &mut self.player else { return };
        match slot {
            None => {
                if self.loaded.take().is_some() {
                    p.stop();
                }
            }
            Some(s) => {
                let key = (s.media.source.clone(), s.start);
                if self.loaded.as_ref() != Some(&key) {
                    p.load(&s.media.source, wall - s.start);
                    self.loaded = Some(key);
                    self.loaded_end = s.end;
                }
            }
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Mouse wheel = volume, middle click = mute, like a remote's volume rocker.
        let (wheel, middle) = ctx.input(|i| {
            let notches: f32 = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                        egui::MouseWheelUnit::Point => delta.y / 40.0,
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Page => delta.y * 3.0,
                    }),
                    _ => None,
                })
                .sum();
            (notches, i.pointer.button_pressed(egui::PointerButton::Middle))
        });
        self.wheel += wheel;
        while self.wheel.abs() >= 1.0 {
            let up = self.wheel > 0.0;
            self.wheel -= if up { 1.0 } else { -1.0 };
            self.set_volume(self.lineup.volume + if up { 5.0 } else { -5.0 });
        }
        if middle {
            self.toggle_mute();
        }

        let (events, keys) = ctx.input(|i| {
            let texts: Vec<String> = i
                .events
                .iter()
                .filter_map(|e| if let egui::Event::Text(t) = e { Some(t.clone()) } else { None })
                .collect();
            let pressed = |k| i.key_pressed(k);
            let keys = [
                pressed(Key::ArrowUp) || pressed(Key::PageUp),
                pressed(Key::ArrowDown) || pressed(Key::PageDown),
                pressed(Key::Backspace),
                pressed(Key::G),
                pressed(Key::C),
                pressed(Key::F) || pressed(Key::F11),
                pressed(Key::Escape),
                pressed(Key::M),
                pressed(Key::I),
                pressed(Key::Plus) || pressed(Key::Equals),
                pressed(Key::Minus),
                pressed(Key::S),
                pressed(Key::L),
                pressed(Key::O),
                pressed(Key::N),
            ];
            (texts, keys)
        });
        let [up, down, recall, guide, chans, full, esc, mute, info, vol_up, vol_down, fav, like, only, even] = keys;
        if even {
            self.toggle_even_volume();
        }
        if fav {
            self.toggle_favorite_channel();
        }
        if like {
            self.toggle_like(unix_now());
        }
        if only {
            self.toggle_favorites_only();
        }
        if up {
            self.step(1);
        }
        if down {
            self.step(-1);
        }
        if recall {
            self.tune(self.previous);
        }
        if guide {
            self.show_guide = !self.show_guide;
        }
        if chans {
            self.show_editor = !self.show_editor;
        }
        if full || (esc && self.fullscreen) {
            self.toggle_fullscreen(ctx);
        }
        if mute {
            self.toggle_mute();
        }
        if info {
            self.show_banner(unix_now());
        }
        if vol_up {
            self.set_volume(self.lineup.volume + 5.0);
        }
        if vol_down {
            self.set_volume(self.lineup.volume - 5.0);
        }
        // Number keys: type "1", "2" → channel 12, like a real remote.
        for t in events {
            for c in t.chars().filter(char::is_ascii_digit) {
                self.digits.push(c);
                self.digits_deadline = self.time + 1.5;
            }
        }
        if !self.digits.is_empty() && (self.time >= self.digits_deadline || self.digits.len() >= 3) {
            if let Ok(n) = self.digits.parse() {
                self.tune_number(n);
            }
            self.digits.clear();
        }
    }

    // ---- main window -------------------------------------------------------

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("RetroRabbit").strong().color(NAVY).size(16.0));
            ui.separator();
            if win95::button_ex(ui, "Ch +", false, 54.0).clicked() {
                self.step(1);
            }
            if win95::button_ex(ui, "Ch -", false, 54.0).clicked() {
                self.step(-1);
            }
            if win95::button(ui, "Recall").on_hover_text("Back to the last channel (Backspace)").clicked() {
                self.tune(self.previous);
            }
            ui.separator();
            if win95::button_ex(ui, "TV Guide", self.show_guide, 0.0).on_hover_text("G").clicked() {
                self.show_guide = !self.show_guide;
            }
            if win95::button_ex(ui, "Channels", self.show_editor, 0.0).on_hover_text("C").clicked() {
                self.show_editor = !self.show_editor;
            }
            ui.separator();
            let fav = self.lineup.channels.get(self.current).is_some_and(|c| c.favorite);
            if win95::button_ex(ui, "Fav Channel", fav, 0.0).on_hover_text("Add this channel to your favorites (S)").clicked() {
                self.toggle_favorite_channel();
            }
            if win95::button_ex(ui, "Favorites Only", self.lineup.favorites_only, 0.0)
                .on_hover_text("Ch +/- skips channels that aren't favorites (O)")
                .clicked()
            {
                self.toggle_favorites_only();
            }
            let wall = unix_now();
            let liked = self.current_slot(wall).is_some_and(|s| self.is_liked(&s.media.source));
            if win95::button_ex(ui, "Like", liked, 0.0).on_hover_text("Like what's on: it'll air on the Favorites channel (L)").clicked() {
                self.toggle_like(wall);
            }
            ui.separator();
            if win95::button(ui, "Vol -").clicked() {
                self.set_volume(self.lineup.volume - 5.0);
            }
            if win95::button(ui, "Vol +").clicked() {
                self.set_volume(self.lineup.volume + 5.0);
            }
            if win95::button_ex(ui, "Even Volume", self.lineup.even_volume, 0.0)
                .on_hover_text("Level out loud ads and quiet old films, like a TV station (N)")
                .clicked()
            {
                self.toggle_even_volume();
            }
            if win95::button_ex(ui, "Mute", self.muted, 0.0).on_hover_text("M").clicked() {
                self.toggle_mute();
            }
            if win95::button(ui, "Full Screen").on_hover_text("F / double-click").clicked() {
                let ctx = ui.ctx().clone();
                self.toggle_fullscreen(&ctx);
            }
        });
    }

    fn status_bar(&mut self, ui: &mut Ui, wall: f64) {
        let (chan, now, next) = match self.lineup.channels.get(self.current).cloned() {
            None => ("No channels".to_string(), String::new(), String::new()),
            Some(ch) => {
                let list = self.playlist(self.current);
                let s = schedule::slots(&ch, &list, wall, wall + 1.0);
                let after = schedule::slots(&ch, &list, wall, wall + 6.0 * 3600.0);
                let now = s.first().map(|s| {
                    format!("Now: {}  ({} / {})", s.media.title, clock(wall - s.start), clock(s.end - s.start))
                });
                let next = after.get(1).map(|n| format!("Next: {} at {}", n.media.title, local_hm(n.start)));
                (format!("CH {:02}  {}", ch.number, ch.name), now.unwrap_or_default(), next.unwrap_or_default())
            }
        };
        let privacy = if self.saved_to_disk { "Lineup: encrypted" } else { "RAM only" };
        ui.horizontal(|ui| {
            let field = |ui: &mut Ui, text: &str, w: f32| {
                let (r, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
                win95::shallow(ui.painter(), r);
                let p = ui.painter().with_clip_rect(r.shrink(2.0));
                p.text(r.left_center() + vec2(5.0, 0.0), Align2::LEFT_CENTER, text, FontId::proportional(13.0), BLACK);
            };
            // Squeeze to fit: drop the privacy and "next" boxes when narrow.
            let gap = ui.spacing().item_spacing.x;
            let rest = ui.available_width();
            let chan_w = 190.0f32.min(rest * 0.35);
            let priv_w = if rest > 700.0 { 120.0 } else { 0.0 };
            let boxes = if rest > 520.0 { 4.0 } else { 3.0 };
            let left = (rest - chan_w - priv_w - gap * boxes).max(0.0);
            let (now_w, next_w) = if rest > 520.0 { (left * 0.55, left * 0.45) } else { (left, 0.0) };
            for (text, w) in [(chan.as_str(), chan_w), (now.as_str(), now_w), (next.as_str(), next_w), (privacy, priv_w)] {
                if w >= 24.0 {
                    field(ui, text, w);
                }
            }
        });
    }

    fn tv(&mut self, ui: &mut Ui, wall: f64) {
        let full = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(full, Sense::click());
        if resp.double_clicked() {
            let ctx = ui.ctx().clone();
            self.toggle_fullscreen(&ctx);
        }
        let screen = if self.fullscreen {
            full
        } else {
            win95::bevel(ui.painter(), full, false);
            full.shrink(2.0)
        };
        let p = ui.painter().clone();
        p.rect_filled(screen, 0.0, BLACK);

        let in_static = self.time < self.static_until;
        let ch = self.lineup.channels.get(self.current).cloned();
        let has_slot = ch
            .as_ref()
            .map(|c| schedule::now_playing(c, &self.playlist(self.current), wall).is_some())
            .unwrap_or(false);
        let pstate = self.player.as_ref().map(|p| p.state());

        let mut show_video = false;
        self.static_this_frame = false;
        if in_static {
            self.draw_static(ui, screen);
            if let Some(c) = &ch {
                p.text(
                    screen.right_top() + vec2(-40.0, 30.0),
                    Align2::RIGHT_TOP,
                    format!("{:02}", c.number),
                    FontId::monospace(48.0),
                    TV_GREEN,
                );
            }
        } else if let Some(err) = &self.player_error {
            win95::color_bars(&p, screen, "NO PICTURE", err);
        } else if ch.is_none() {
            win95::color_bars(&p, screen, "NO CHANNELS", "Press C to set up channels.");
        } else if !has_slot {
            match self.channel_status(self.current) {
                Some(ArchiveState::Loading | ArchiveState::Queued) => {
                    self.draw_static(ui, screen);
                    let r = Rect::from_center_size(screen.center(), vec2(360.0, 72.0));
                    p.rect_filled(r, 0.0, BLACK);
                    p.text(r.center() - vec2(0.0, 10.0), Align2::CENTER_CENTER, "TUNING IN…", FontId::monospace(24.0), WHITE);
                    let sub = match self.channel_progress(self.current) {
                        Some((d, t)) => format!("Gathering programs from the Internet Archive: {d}/{t}"),
                        None => "Waiting for other channels to finish loading…".to_string(),
                    };
                    p.text(r.center() + vec2(0.0, 20.0), Align2::CENTER_CENTER, sub, FontId::proportional(13.0), Color32::LIGHT_GRAY);
                }
                Some(ArchiveState::Failed(e)) => win95::color_bars(&p, screen, "NO SIGNAL", &e),
                _ if ch.as_ref().is_some_and(|c| c.rules.contains(&Rule::Liked)) => {
                    win95::color_bars(&p, screen, "NO SIGNAL", "Press Like (L) on anything you enjoy and it airs here.")
                }
                _ => win95::color_bars(&p, screen, "NO SIGNAL", "Nothing to air yet. Press C to add media."),
            }
        } else if pstate.as_ref().is_some_and(|s| s.ended) {
            self.tuning = false;
            let sub = pstate.and_then(|s| s.error).unwrap_or_else(|| "Back after this program ends.".into());
            win95::color_bars(&p, screen, "PLEASE STAND BY", &sub);
        } else if !pstate.as_ref().is_some_and(|s| s.playing) {
            // Loading. After a channel change: static until the picture arrives.
            // Between two programs on the same channel: just black, like real TV.
            if self.tuning {
                self.draw_static(ui, screen);
                if let Some(c) = &ch {
                    p.text(screen.right_top() + vec2(-40.0, 30.0), Align2::RIGHT_TOP, format!("{:02}", c.number), FontId::monospace(48.0), TV_GREEN);
                }
                if self.time - self.tuning_since > 3.0 {
                    let r = Rect::from_center_size(screen.center(), vec2(300.0, 56.0));
                    p.rect_filled(r, 0.0, BLACK);
                    p.text(r.center(), Align2::CENTER_CENTER, "TUNING IN…", FontId::monospace(24.0), WHITE);
                }
            }
        } else {
            self.tuning = false;
            show_video = true;
        }

        if let Some(v) = &mut self.video {
            let ppp = ui.ctx().pixels_per_point();
            v.set_geometry(
                (screen.min.x * ppp).round() as i32,
                (screen.min.y * ppp).round() as i32,
                (screen.width() * ppp).round() as u32,
                (screen.height() * ppp).round() as u32,
            );
            v.set_visible(show_video);
        }
        // Static on screen ⇒ static in the speakers.
        if self.static_this_frame != self.hiss_on {
            self.hiss_on = self.static_this_frame;
            if let Some(h) = &mut self.hiss {
                h.set_hiss(self.hiss_on);
            }
        }
        if show_video && self.banner_pending {
            self.banner_pending = false;
            self.show_banner(wall);
        }
        if !self.digits.is_empty() {
            p.text(
                screen.left_top() + vec2(30.0, 30.0),
                Align2::LEFT_TOP,
                format!("{}-", self.digits),
                FontId::monospace(48.0),
                TV_GREEN,
            );
        }
    }

    fn draw_static(&mut self, ui: &mut Ui, r: Rect) {
        self.static_this_frame = true;
        let (w, h) = (160, 120);
        let mut px = vec![0u8; w * h];
        for chunk in px.chunks_mut(8) {
            let v = self.rng.next().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
        // Rolling bar, like a badly tuned set.
        let roll = ((self.time * 60.0) as usize) % h;
        for y in 0..h {
            let dim = (y + h - roll) % h < 14;
            for x in 0..w {
                let v = &mut px[y * w + x];
                *v = if dim { *v / 3 } else { (*v / 2) + 60 };
            }
        }
        let img = ColorImage::from_gray([w, h], &px);
        match &mut self.noise {
            Some(t) => t.set(img, TextureOptions::NEAREST),
            None => self.noise = Some(ui.ctx().load_texture("static", img, TextureOptions::NEAREST)),
        }
        let tex = self.noise.as_ref().unwrap();
        ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }

    // ---- TV guide ------------------------------------------------------------

    fn guide_window(&mut self, ctx: &egui::Context, wall: f64) {
        let builder = ViewportBuilder::default()
            .with_title("TV Guide – RetroRabbit")
            .with_inner_size([900.0, 560.0]);
        ctx.show_viewport_immediate(ViewportId::from_hash_of("rr-guide"), builder, |ui, _| {
            if ui.ctx().input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape) || i.key_pressed(Key::G)) {
                self.show_guide = false;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(FACE).inner_margin(6))
                .show(ui, |ui| {
                    win95::title_bar(ui, "TV Guide");
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        if win95::button(ui, "< Earlier").clicked() {
                            self.guide_page -= 1;
                        }
                        if win95::button_ex(ui, "Now", self.guide_page == 0, 0.0).clicked() {
                            self.guide_page = 0;
                        }
                        if win95::button(ui, "Later >").clicked() {
                            self.guide_page += 1;
                        }
                        ui.separator();
                        ui.label("Show:");
                        for &(span, name) in GUIDE_SPANS {
                            if win95::button_ex(ui, name, self.guide_span == span, 0.0).clicked() {
                                self.guide_span = span;
                                self.guide_page = 0;
                            }
                        }
                    });
                    ui.add_space(4.0);
                    self.guide_ui(ui, wall);
                });
        });
    }

    fn guide_ui(&mut self, ui: &mut Ui, wall: f64) {
        let span = self.guide_span;
        let start = (wall / 1800.0).floor() * 1800.0 + self.guide_page as f64 * span;
        let end = start + span;
        let name_w = 180.0;
        let row_h = 38.0;

        let area = ui.available_rect_before_wrap();
        win95::bevel(ui.painter(), area, false);
        let area = area.shrink(2.0);
        ui.painter().rect_filled(area, 0.0, GUIDE_BG);
        let grid_w = area.width() - name_w;
        let x_of = |t: f64| area.min.x + name_w + ((t - start) / span) as f32 * grid_w;

        // Time header.
        let header = Rect::from_min_size(area.min, vec2(area.width(), 24.0));
        let p = ui.painter().with_clip_rect(area);
        p.rect_filled(header, 0.0, NAVY);
        p.text(header.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, local_hm(wall), FontId::monospace(14.0), YELLOW);
        // A time mark every 15/30/60 minutes depending on zoom.
        let step = (span / 4.0).clamp(900.0, 3600.0);
        let mut t = start;
        while t < end {
            if x_of(t) > area.min.x + name_w - 1.0 {
                p.text(pos2(x_of(t) + 6.0, header.center().y), Align2::LEFT_CENTER, local_hm(t), FontId::proportional(14.0), WHITE);
            }
            t += step;
        }

        let body = Rect::from_min_max(pos2(area.min.x, header.max.y), area.max);
        let mut clicked = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for idx in self.dial() {
                    let ch = self.lineup.channels[idx].clone();
                    let list = self.playlist(idx);
                    let (row, resp) = ui.allocate_exact_size(vec2(area.width().max(0.0), row_h), Sense::click());
                    if resp.clicked() {
                        clicked = Some(idx);
                    }
                    let p = ui.painter().with_clip_rect(row.intersect(body));
                    let name_r = Rect::from_min_size(row.min, vec2(name_w, row_h)).shrink(1.0);
                    let here = idx == self.current;
                    p.rect_filled(name_r, 0.0, if here { YELLOW } else { FACE });
                    win95::bevel(&p, name_r, true);
                    p.text(name_r.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, format!("{:02}", ch.number), FontId::monospace(18.0), BLACK);
                    p.text(name_r.left_center() + vec2(42.0, 0.0), Align2::LEFT_CENTER, &ch.name, FontId::proportional(14.0), BLACK);
                    if ch.favorite {
                        win95::star(&p, name_r.right_center() - vec2(14.0, 0.0), 8.0, Color32::from_rgb(200, 140, 0));
                    }

                    let slots: Vec<Slot> = schedule::slots(&ch, &list, start, end);
                    if slots.is_empty() {
                        let msg = match self.channel_status(idx) {
                            Some(ArchiveState::Loading) => match self.channel_progress(idx) {
                                Some((d, t)) => format!("Tuning in… {d}/{t}"),
                                None => "Tuning in…".to_string(),
                            },
                            Some(ArchiveState::Queued) => "Waiting to tune in…".to_string(),
                            Some(ArchiveState::Failed(e)) => format!("Off the air ({e})"),
                            _ => "Off the air".to_string(),
                        };
                        let r = Rect::from_min_max(pos2(x_of(start), row.min.y), row.max).shrink(1.0);
                        p.rect_filled(r, 0.0, GUIDE_CELL);
                        p.text(r.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, msg, FontId::proportional(14.0), SHADOW);
                    }
                    // Items too narrow to read are merged into one "First + N more" cell.
                    struct Cell {
                        start: f64,
                        end: f64,
                        titles: Vec<(f64, String)>,
                        short: bool,
                        liked: bool,
                    }
                    let px = |a: f64, b: f64| x_of(b.min(end)) - x_of(a.max(start));
                    let mut cells: Vec<Cell> = Vec::new();
                    for s in slots {
                        let liked = self.is_liked(&s.media.source);
                        let short = px(s.start, s.end) < 26.0;
                        match cells.last_mut() {
                            // Keep merging while the merged cell is still too narrow to read.
                            Some(c) if short && c.short && px(c.start, c.end) < 90.0 => {
                                c.end = s.end;
                                c.titles.push((s.start, s.media.title.clone()));
                                c.liked |= liked;
                            }
                            _ => cells.push(Cell {
                                start: s.start,
                                end: s.end,
                                titles: vec![(s.start, s.media.title.clone())],
                                short,
                                liked,
                            }),
                        }
                    }
                    for (ci, c) in cells.into_iter().enumerate() {
                        let r = Rect::from_min_max(
                            pos2(x_of(c.start.max(start)), row.min.y),
                            pos2(x_of(c.end.min(end)), row.max.y),
                        )
                        .shrink(1.0);
                        if r.width() < 1.0 {
                            continue;
                        }
                        let airing = c.start <= wall && wall < c.end;
                        p.rect_filled(r, 0.0, if airing && here { NAVY } else { GUIDE_CELL });
                        if c.liked {
                            p.rect_filled(Rect::from_min_max(pos2(r.min.x, r.max.y - 3.0), r.max), 0.0, Color32::from_rgb(255, 105, 180));
                        }
                        let n = c.titles.len();
                        let title = if n > 1 { format!("{} + {} more", c.titles[0].1, n - 1) } else { c.titles[0].1.clone() };
                        let tip = c
                            .titles
                            .iter()
                            .take(25)
                            .map(|(t, name)| format!("{}  {}", local_hm(*t), name))
                            .collect::<Vec<_>>()
                            .join("\n")
                            + &if n > 25 { format!("\n…and {} more", n - 25) } else { String::new() };
                        ui.interact(r.intersect(body), egui::Id::unique(("rr-guide-cell", idx, ci)), Sense::hover()).on_hover_text(tip);
                        let cp = p.with_clip_rect(r.shrink(2.0).intersect(body));
                        let arrow = if c.start < start { "« " } else { "" };
                        cp.text(r.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, format!("{arrow}{title}"), FontId::proportional(14.0), if airing { YELLOW } else { WHITE });
                    }
                }
            });
        });
        // "Now" line.
        let x = x_of(wall);
        ui.painter().with_clip_rect(area).line_segment([pos2(x, area.min.y), pos2(x, area.max.y)], egui::Stroke::new(2.0, Color32::from_rgb(255, 64, 64)));
        if let Some(i) = clicked {
            self.tune(i);
        }
    }

    // ---- channel editor --------------------------------------------------------

    fn editor_window(&mut self, ctx: &egui::Context) {
        let builder = ViewportBuilder::default()
            .with_title("Channels – RetroRabbit")
            .with_inner_size([860.0, 640.0]);
        ctx.show_viewport_immediate(ViewportId::from_hash_of("rr-editor"), builder, |ui, _| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                self.show_editor = false;
            }
            egui::Panel::left("ed-list")
                .resizable(false)
                .exact_size(230.0)
                .frame(egui::Frame::new().fill(FACE).inner_margin(6))
                .show(ui, |ui| self.editor_list(ui));
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(FACE).inner_margin(6))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.editor_detail(ui));
                });
        });
    }

    fn editor_list(&mut self, ui: &mut Ui) {
        win95::title_bar(ui, "Channel Lineup");
        ui.add_space(4.0);
        let list_h = (ui.available_height() - 112.0).max(40.0);
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), list_h), Sense::hover());
        ui.painter().rect_filled(r, 0.0, WHITE);
        win95::bevel(ui.painter(), r, false);
        ui.scope_builder(egui::UiBuilder::new().max_rect(r.shrink(4.0)), |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for idx in self.dial() {
                    let ch = &self.lineup.channels[idx];
                    let label = format!("{:02}  {}{}", ch.number, ch.name, if ch.favorite { "  (fav)" } else { "" });
                    if ui.selectable_label(self.ed_selected == idx, label).clicked() {
                        self.ed_selected = idx;
                    }
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if win95::button_ex(ui, "New", false, 70.0).clicked() {
                let n = self.lineup.channels.iter().map(|c| c.number).max().unwrap_or(1) + 1;
                self.lineup.channels.push(Channel {
                    number: n,
                    name: format!("Channel {n}"),
                    order: Order::Shuffle,
                    rules: vec![],
                    favorite: false,
                });
                self.ed_selected = self.lineup.channels.len() - 1;
                self.lineup.auto = false;
            }
            if win95::button_ex(ui, "Delete", false, 70.0).clicked() && self.ed_selected < self.lineup.channels.len() {
                self.lineup.channels.remove(self.ed_selected);
                self.playlists.clear();
                self.lineup.auto = false;
                self.ed_selected = self.ed_selected.saturating_sub(1);
                if self.current >= self.lineup.channels.len() || self.current == self.ed_selected + 1 {
                    self.current = 0;
                    self.previous = 0;
                    self.tune(0);
                }
            }
            if win95::button_ex(ui, "Watch", false, 70.0).clicked() {
                self.tune(self.ed_selected);
            }
        });
        if win95::button_ex(ui, "Auto-build lineup", false, ui.available_width())
            .on_hover_text("Rebuild channels from your folders, genres and the Internet Archive. Replaces your edits.")
            .clicked()
        {
            self.lineup.auto = true;
            self.rebuild_auto_lineup();
            self.tune(self.current);
        }
        ui.label(if self.lineup.auto { "Auto lineup: updates with your library." } else { "Custom lineup." });
    }

    fn editor_detail(&mut self, ui: &mut Ui) {
        let items = self.lib.items();
        let known = known_tags(&items, &self.archive_items());
        let sel = self.ed_selected;

        if sel < self.lineup.channels.len() {
            let before = self.lineup.channels[sel].clone();
            let mut retry = Vec::new();
            {
                let ch = &mut self.lineup.channels[sel];
                win95::group(ui, "Channel", |ui| {
                    egui::Grid::new("ch-grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                        ui.label("Name:");
                        ui.add(egui::TextEdit::singleline(&mut ch.name).desired_width(280.0));
                        ui.end_row();
                        ui.label("Number:");
                        ui.add(egui::DragValue::new(&mut ch.number).range(1..=999));
                        ui.end_row();
                        ui.label("Favorite:");
                        ui.checkbox(&mut ch.favorite, "Show up when surfing favorites only");
                        ui.end_row();
                        ui.label("Order:");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut ch.order, Order::Shuffle, "Shuffle");
                            ui.radio_value(&mut ch.order, Order::InOrder, "In order (episodes air in sequence)");
                        });
                        ui.end_row();
                    });
                });

                win95::group(ui, "What airs on this channel (any of these)", |ui| {
                    let mut remove = None;
                    for (i, rule) in ch.rules.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt(("kind", i))
                                .selected_text(rule.kind_name())
                                .width(130.0)
                                .show_ui(ui, |ui| {
                                    let opts = [
                                        Rule::Everything,
                                        Rule::Folder(String::new()),
                                        Rule::Tag { key: "genre".into(), value: String::new() },
                                        Rule::Show(String::new()),
                                        Rule::Archive(String::new()),
                                        Rule::Liked,
                                    ];
                                    for o in opts {
                                        let name = o.kind_name();
                                        if ui.selectable_label(rule.kind_name() == name, name).clicked() && rule.kind_name() != name {
                                            *rule = o;
                                        }
                                    }
                                });
                            match rule {
                                Rule::Everything => {
                                    ui.label("All your local videos");
                                }
                                Rule::Liked => {
                                    ui.label(format!("Everything you've liked ({})", self.lineup.liked.len()));
                                }
                                Rule::Folder(path) => {
                                    ui.add(egui::TextEdit::singleline(path).hint_text("/path/to/folder").desired_width(330.0));
                                }
                                Rule::Tag { key, value } => {
                                    pick_text(ui, ("tk", i), key, known.keys().cloned().collect(), 100.0);
                                    ui.label("=");
                                    let vals = known.get(key.as_str()).cloned().unwrap_or_default();
                                    pick_text(ui, ("tv", i), value, vals, 200.0);
                                }
                                Rule::Show(name) => {
                                    let shows = known.get("show").cloned().unwrap_or_default();
                                    pick_text(ui, ("sh", i), name, shows, 330.0);
                                }
                                Rule::Archive(q) => {
                                    let presets = schedule::ARCHIVE_PRESETS.iter().map(|p| p.1.to_string()).collect();
                                    pick_text(ui, ("ia", i), q, presets, 330.0);
                                    match self.archive.state(q) {
                                        Some(ArchiveState::Queued) => {
                                            ui.label("waiting…");
                                        }
                                        Some(ArchiveState::Loading) => {
                                            ui.label("loading…");
                                        }
                                        Some(ArchiveState::Failed(e)) => {
                                            ui.colored_label(Color32::DARK_RED, "failed").on_hover_text(e);
                                            if win95::button(ui, "Retry").clicked() {
                                                retry.push(q.clone());
                                            }
                                        }
                                        Some(ArchiveState::Ready) => {
                                            ui.label(format!("{} videos", self.archive.items(q).len()));
                                        }
                                        None => {}
                                    }
                                }
                            }
                            if win95::button(ui, "X").on_hover_text("Remove rule").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        ch.rules.remove(i);
                    }
                    if win95::button(ui, "Add rule").clicked() {
                        ch.rules.push(Rule::Tag { key: "genre".into(), value: String::new() });
                    }
                });
            }
            for q in retry {
                self.archive.retry(&q);
            }
            if self.lineup.channels[sel] != before {
                self.lineup.auto = false;
                if sel == self.current {
                    self.loaded = None;
                }
            }
            let ch = self.lineup.channels[sel].clone();
            let list = self.playlist(sel);
            let total: f64 = list.iter().map(|m| m.duration).sum();
            ui.label(format!("{} videos · {} of programming before it repeats", list.len(), long_duration(total)));
            if let Some(s) = schedule::now_playing(&ch, &list, unix_now()) {
                ui.label(format!("On now: {}", s.media.title));
            }
        } else {
            ui.label("Select a channel, or press New.");
        }

        win95::group(ui, "Media folders (scanned into RAM at startup, never indexed to disk)", |ui| {
            let mut remove = None;
            for (i, root) in self.lineup.media_roots.iter().enumerate() {
                ui.horizontal(|ui| {
                    if win95::button(ui, "X").clicked() {
                        remove = Some(i);
                    }
                    ui.label(root);
                });
            }
            let mut changed = false;
            if let Some(i) = remove {
                self.lineup.media_roots.remove(i);
                changed = true;
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.ed_new_root).hint_text("/path/to/videos").desired_width(330.0));
                if win95::button(ui, "Add folder").clicked() {
                    let p = self.ed_new_root.trim().trim_end_matches('/').to_string();
                    if std::path::Path::new(&p).is_dir() && !self.lineup.media_roots.contains(&p) {
                        self.lineup.media_roots.push(p);
                        self.ed_new_root.clear();
                        changed = true;
                    } else {
                        self.ed_status = "That folder doesn't exist (or is already added).".into();
                    }
                }
            });
            let st = self.lib.status();
            ui.horizontal(|ui| {
                if win95::button(ui, "Rescan").clicked() {
                    changed = true;
                }
                if st.running {
                    ui.label(format!("Scanning… {}/{}", st.done, st.total));
                } else {
                    ui.label(&st.message);
                }
            });
            if changed {
                self.rescan();
            }
        });

        win95::group(ui, "Save lineup (encrypted)", |ui| {
            ui.label("Nothing is written to disk unless you save here. The file is encrypted with your passphrase.");
            ui.label(egui::RichText::new(vault::default_path().display().to_string()).monospace().size(12.0));
            ui.horizontal(|ui| {
                ui.label("Passphrase:");
                ui.add(egui::TextEdit::singleline(&mut *self.ed_pass).password(true).desired_width(220.0));
                if win95::button(ui, "Save").clicked() {
                    match vault::save(&self.lineup, &self.ed_pass, &vault::default_path()) {
                        Ok(()) => {
                            self.saved_to_disk = true;
                            self.ed_status = "Lineup saved (encrypted).".into();
                        }
                        Err(e) => self.ed_status = format!("Save failed: {e}"),
                    }
                }
                if win95::button(ui, "Load").clicked() {
                    match vault::load(&self.ed_pass, &vault::default_path()) {
                        Ok(l) => {
                            self.lineup = l;
                            self.playlists.clear();
                            self.saved_to_disk = true;
                            self.ed_selected = 0;
                            self.ed_status = "Lineup loaded.".into();
                            self.rescan();
                            self.tune(0);
                            let (v, even) = (self.lineup.volume, self.lineup.even_volume);
                            if let Some(p) = &mut self.player {
                                p.set_volume(v);
                                p.set_even_volume(even);
                            }
                            if let Some(h) = &mut self.hiss {
                                h.set_hiss_volume(v);
                            }
                        }
                        Err(e) => self.ed_status = format!("Load failed: {e}"),
                    }
                }
            });
        });
        if !self.ed_status.is_empty() {
            ui.label(&self.ed_status);
        }
    }

    fn archive_items(&self) -> Vec<Arc<Media>> {
        let mut v = Vec::new();
        for ch in &self.lineup.channels {
            for r in &ch.rules {
                if let Rule::Archive(q) = r {
                    v.extend(self.archive.items(q));
                }
            }
        }
        v
    }
}

impl eframe::App for RetroRabbit {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.time = ctx.input(|i| i.time);
        let wall = unix_now();

        self.sync_sources();
        self.handle_keys(&ctx);
        self.tick(wall);

        if !self.fullscreen {
            egui::Panel::top("toolbar")
                .frame(egui::Frame::new().fill(FACE).inner_margin(egui::Margin::symmetric(6, 5)))
                .show(ui, |ui| self.toolbar(ui));
            egui::Panel::bottom("status")
                .frame(egui::Frame::new().fill(FACE).inner_margin(egui::Margin::symmetric(4, 3)))
                .show(ui, |ui| self.status_bar(ui, wall));
        }
        let margin = if self.fullscreen { 0 } else { 6 };
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(if self.fullscreen { BLACK } else { FACE }).inner_margin(margin))
            .show(ui, |ui| self.tv(ui, wall));

        if self.show_guide {
            self.guide_window(&ctx, wall);
        }
        if self.show_editor {
            self.editor_window(&ctx);
        }

        let busy = self.time < self.static_until || self.tuning || self.lib.status().running || !self.digits.is_empty();
        if busy || self.channel_status(self.current).is_some_and(|s| matches!(s, ArchiveState::Loading | ArchiveState::Queued)) {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}

/// Text box with a drop-down of suggestions (tag values found in your media).
fn pick_text(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: &mut String, options: Vec<String>, width: f32) {
    ui.add(egui::TextEdit::singleline(value).desired_width(width));
    if options.is_empty() {
        return;
    }
    egui::ComboBox::from_id_salt(id).selected_text("").width(18.0).show_ui(ui, |ui| {
        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            for o in options {
                if ui.selectable_label(*value == o, &o).clicked() {
                    *value = o;
                }
            }
        });
    });
}

/// Tag keys → their values, most common first.
fn known_tags(local: &[Arc<Media>], archive: &[Arc<Media>]) -> HashMap<String, Vec<String>> {
    let mut counts: HashMap<String, HashMap<String, usize>> = HashMap::new();
    for m in local.iter().chain(archive) {
        for (k, v) in &m.tags {
            if k == "episode" || k == "archive" {
                continue;
            }
            *counts.entry(k.clone()).or_default().entry(v.clone()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .map(|(k, vals)| {
            let mut v: Vec<_> = vals.into_iter().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            (k, v.into_iter().take(200).map(|x| x.0).collect())
        })
        .collect()
}

pub fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn local_hm(t: f64) -> String {
    let tt = t as libc::time_t;
    // SAFETY: localtime_r only writes into the tm we pass it.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&tt, &mut tm);
        tm
    };
    let h = tm.tm_hour % 12;
    format!("{}:{:02} {}", if h == 0 { 12 } else { h }, tm.tm_min, if tm.tm_hour < 12 { "AM" } else { "PM" })
}

fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn long_duration(secs: f64) -> String {
    let m = (secs / 60.0).round() as u64;
    if m >= 60 { format!("{}h {:02}m", m / 60, m % 60) } else { format!("{m}m") }
}
