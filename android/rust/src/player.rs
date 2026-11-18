//! Android player: the same controls app.rs uses on desktop, sent to TvPlayer.kt
//! (ExoPlayer). Static hiss is white noise made here and played with AAudio.

use crate::android;
use crate::embed::Shared;
use jni::objects::{JString, JValue};
use ndk::audio::{AudioCallbackResult, AudioDirection, AudioFormat, AudioStream, AudioStreamBuilder};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Static is quieter than programs, like on a real set.
const HISS_LEVEL: f64 = 0.35;

#[derive(Default, Clone, Debug)]
pub struct PlayerState {
    pub ended: bool,
    pub playing: bool,
    pub error: Option<String>,
}

struct Hiss {
    on: AtomicBool,
    muted: AtomicBool,
    /// f32 bits: linear gain.
    gain: AtomicU32,
}

enum Kind {
    Video(Arc<Shared>),
    Hiss(Arc<Hiss>, #[allow(dead_code)] AudioStream),
}

pub struct Player {
    kind: Kind,
}

/// mpv's volume (0-130, 100 = as recorded) as a gain. Android can't go above 1.
fn gain(v: f64) -> f32 {
    (v / 100.0).clamp(0.0, 1.0) as f32
}

impl Player {
    pub fn spawn(shared: Arc<Shared>, volume: f64, even_volume: bool) -> Result<Self, String> {
        let mut p = Self { kind: Kind::Video(shared) };
        p.set_volume(volume);
        p.set_even_volume(even_volume);
        Ok(p)
    }

    pub fn spawn_hiss(volume: f64) -> Result<Self, String> {
        let h = Arc::new(Hiss {
            on: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            gain: AtomicU32::new((gain(volume) * HISS_LEVEL as f32).to_bits()),
        });
        let st = h.clone();
        let mut seed: u32 = 0x9E37_79B9;
        let mut low = 0f32;
        let stream = AudioStreamBuilder::new()
            .map_err(|e| e.to_string())?
            .direction(AudioDirection::Output)
            .format(AudioFormat::PCM_Float)
            .channel_count(1)
            .sample_rate(48_000)
            .data_callback(Box::new(move |_s: &AudioStream, data, frames| {
                // SAFETY: AAudio hands us `frames` mono f32 samples to fill.
                let out = unsafe { std::slice::from_raw_parts_mut(data as *mut f32, frames.max(0) as usize) };
                let g = if st.on.load(Ordering::Relaxed) && !st.muted.load(Ordering::Relaxed) {
                    f32::from_bits(st.gain.load(Ordering::Relaxed))
                } else {
                    0.0
                };
                for s in out.iter_mut() {
                    // xorshift white noise, then a gentle low-pass (~7 kHz at 48 kHz).
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let white = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
                    low += 0.6 * (white - low);
                    *s = low * 0.5 * g;
                }
                AudioCallbackResult::Continue
            }))
            .open_stream()
            .map_err(|e| e.to_string())?;
        stream.request_start().map_err(|e| e.to_string())?;
        Ok(Self { kind: Kind::Hiss(h, stream) })
    }

    fn call(&self, name: &str, sig: &str, args: &[JValue]) {
        if let Kind::Video(s) = &self.kind {
            android::with(|env, _| env.call_method(s.player.as_obj(), name, sig, args).map(|_| ()));
        }
    }

    pub fn state(&self) -> PlayerState {
        let Kind::Video(s) = &self.kind else { return PlayerState::default() };
        android::with(|env, _| {
            let bits = env.call_method(s.player.as_obj(), "state", "()I", &[])?.i()?;
            let error = if bits & 4 != 0 {
                let e = env.call_method(s.player.as_obj(), "error", "()Ljava/lang/String;", &[])?.l()?;
                Some(if e.is_null() { "playback error".to_string() } else { env.get_string(&JString::from(e))?.into() })
            } else {
                None
            };
            Ok(PlayerState { ended: bits & 1 != 0, playing: bits & 2 != 0, error })
        })
        .unwrap_or_default()
    }

    /// Play `source` starting `offset` seconds in: joining the show in progress.
    pub fn load(&mut self, source: &str, offset: f64) {
        if let Kind::Video(s) = &self.kind {
            android::with(|env, _| {
                let src = env.new_string(source)?;
                env.call_method(
                    s.player.as_obj(),
                    "load",
                    "(Ljava/lang/String;J)V",
                    &[JValue::Object(&src), JValue::Long((offset.max(0.0) * 1000.0) as i64)],
                )
                .map(|_| ())
            });
        }
    }

    pub fn stop(&mut self) {
        self.call("stop", "()V", &[]);
    }

    pub fn osd(&mut self, text: &str, ms: u32) {
        if let Kind::Video(s) = &self.kind {
            *s.osd.lock().unwrap() =
                if text.is_empty() { None } else { Some((text.to_string(), Instant::now() + Duration::from_millis(ms as u64))) };
            android::repaint();
        }
    }

    pub fn set_volume(&mut self, v: f64) {
        match &self.kind {
            Kind::Video(_) => self.call("setVolume", "(F)V", &[JValue::Float(gain(v))]),
            Kind::Hiss(h, _) => h.gain.store(gain(v).to_bits(), Ordering::Relaxed),
        }
    }

    pub fn set_even_volume(&mut self, on: bool) {
        self.call("setEvenVolume", "(Z)V", &[JValue::Bool(on as u8)]);
    }

    pub fn set_hiss(&mut self, on: bool) {
        if let Kind::Hiss(h, _) = &self.kind {
            h.on.store(on, Ordering::Relaxed);
        }
    }

    pub fn set_hiss_volume(&mut self, v: f64) {
        self.set_volume(v * HISS_LEVEL);
    }

    pub fn set_mute(&mut self, m: bool) {
        match &self.kind {
            Kind::Video(_) => self.call("setMute", "(Z)V", &[JValue::Bool(m as u8)]),
            Kind::Hiss(h, _) => h.muted.store(m, Ordering::Relaxed),
        }
    }

    pub fn alive(&mut self) -> bool {
        true
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.call("release", "()V", &[]);
    }
}
