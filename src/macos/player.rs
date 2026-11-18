//! macOS player: the same controls as the Linux mpv-over-IPC player, but talking
//! to libmpv in-process. The video core is made by embed.rs (it needs our GL context).

use crate::mpv::{self, Mpv};
use std::sync::{Arc, Mutex};

const EVEN_VOLUME: &str = "lavfi=[dynaudnorm=f=150:g=11:p=0.9:m=20:r=0.18]";
const HISS: &str = "av://lavfi:anoisesrc=color=white:amplitude=0.5,lowpass=f=7000";
const HISS_LEVEL: f64 = 0.35;

#[derive(Default, Clone, Debug)]
pub struct PlayerState {
    pub ended: bool,
    pub playing: bool,
    pub error: Option<String>,
    shutdown: bool,
}

pub struct Player {
    mpv: Mpv,
    state: Arc<Mutex<PlayerState>>,
}

fn events(mpv: Mpv, st: Arc<Mutex<PlayerState>>) {
    let Ok(api) = mpv::api() else { return };
    std::thread::spawn(move || {
        // Capture the whole (Send) wrapper, not just its raw pointer field.
        let mpv = mpv;
        loop {
            // SAFETY: the core is never destroyed; one thread waits on it.
            let ev = unsafe { &*(api.wait_event)(mpv.0, -1.0) };
            match ev.event_id {
                mpv::EVENT_SHUTDOWN => {
                    st.lock().unwrap().shutdown = true;
                    break;
                }
                mpv::EVENT_START_FILE => {
                    let mut s = st.lock().unwrap();
                    let shut = s.shutdown;
                    *s = PlayerState { shutdown: shut, ..Default::default() };
                }
                mpv::EVENT_PLAYBACK_RESTART => st.lock().unwrap().playing = true,
                mpv::EVENT_END_FILE if !ev.data.is_null() => {
                    // SAFETY: END_FILE events carry an mpv_event_end_file.
                    let end = unsafe { &*(ev.data as *const mpv::EventEndFile) };
                    if end.reason == mpv::END_FILE_EOF || end.reason == mpv::END_FILE_ERROR {
                        let mut s = st.lock().unwrap();
                        s.ended = true;
                        if end.reason == mpv::END_FILE_ERROR {
                            s.error = Some(mpv::error_string(end.error));
                        }
                    }
                }
                _ => {}
            }
        }
    });
}

impl Player {
    pub fn spawn(mpv: Mpv, volume: f64, even_volume: bool) -> Result<Self, String> {
        let state = Arc::new(Mutex::new(PlayerState::default()));
        events(mpv, state.clone());
        let mut p = Self { mpv, state };
        p.set_volume(volume);
        p.set_even_volume(even_volume);
        Ok(p)
    }

    pub fn spawn_hiss(volume: f64) -> Result<Self, String> {
        let opts: Vec<String> = [
            "vo=null",
            "vid=no",
            "idle=yes",
            "config=no",
            "load-scripts=no",
            "ytdl=no",
            "terminal=no",
            "pause=yes",
            "audio-client-name=RetroRabbit",
        ]
        .iter()
        .map(|s| s.to_string())
        .chain([format!("volume={}", volume * HISS_LEVEL)])
        .collect();
        let mpv = Mpv::new(&opts)?;
        let state = Arc::new(Mutex::new(PlayerState::default()));
        events(mpv, state.clone());
        mpv.command(&["loadfile", HISS, "replace"]);
        Ok(Self { mpv, state })
    }

    pub fn state(&self) -> PlayerState {
        self.state.lock().unwrap().clone()
    }

    pub fn load(&mut self, source: &str, offset: f64) {
        {
            let mut s = self.state.lock().unwrap();
            let shut = s.shutdown;
            *s = PlayerState { shutdown: shut, ..Default::default() };
        }
        self.mpv.set("start", &format!("{:.2}", offset.max(0.0)));
        self.mpv.command(&["loadfile", source, "replace"]);
    }

    pub fn stop(&mut self) {
        self.mpv.command(&["stop"]);
    }

    pub fn osd(&mut self, text: &str, ms: u32) {
        self.mpv.command(&["show-text", text, &ms.to_string()]);
    }

    pub fn set_volume(&mut self, v: f64) {
        self.mpv.set("volume", &v.to_string());
    }

    pub fn set_even_volume(&mut self, on: bool) {
        self.mpv.set("af", if on { EVEN_VOLUME } else { "" });
    }

    pub fn set_hiss(&mut self, on: bool) {
        self.mpv.set("pause", if on { "no" } else { "yes" });
    }

    pub fn set_hiss_volume(&mut self, v: f64) {
        self.set_volume(v * HISS_LEVEL);
    }

    pub fn set_mute(&mut self, m: bool) {
        self.mpv.set("mute", if m { "yes" } else { "no" });
    }

    pub fn alive(&mut self) -> bool {
        !self.state.lock().unwrap().shutdown
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.mpv.command(&["quit"]);
    }
}
