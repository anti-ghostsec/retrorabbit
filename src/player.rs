//! Video playback by the system `mpv`, drawn inside our window (`--wid`) and
//! driven over its JSON IPC socket. The socket lives in $XDG_RUNTIME_DIR (tmpfs),
//! mpv's cache is RAM-only, and resume/watch-later files are switched off.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Dynamic loudness leveling: quiet old films get lifted, loud ads get pulled
/// down. About 0.8 s of look-ahead, so channel changes stay quick.
const EVEN_VOLUME: &str = "lavfi=[dynaudnorm=f=150:g=11:p=0.9:m=20:r=0.18]";

#[derive(Default, Clone, Debug)]
pub struct PlayerState {
    /// The current file stopped on its own (finished early or failed).
    pub ended: bool,
    /// The new file's first frames are on screen (set once mpv finishes seeking in).
    pub playing: bool,
    pub error: Option<String>,
}

pub struct Player {
    child: Child,
    sock: UnixStream,
    path: PathBuf,
    state: Arc<Mutex<PlayerState>>,
}

/// TV static: white noise with the harshest top end rolled off.
const HISS: &str = "av://lavfi:anoisesrc=color=white:amplitude=0.5,lowpass=f=7000";

/// Static is quieter than programs, like on a real set.
const HISS_LEVEL: f64 = 0.35;

fn socket_path(tag: &str) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| PathBuf::from("/dev/shm"));
    dir.join(format!("retrorabbit-{}{tag}.sock", std::process::id()))
}

/// mpv settings shared by the video player and the static hiss.
const COMMON: &[&str] = &[
    "--idle=yes",
    "--keep-open=no",
    // Our own settings only: no user config, scripts or youtube-dl.
    "--config=no",
    "--load-scripts=no",
    "--ytdl=no",
    // Keys and mouse belong to RetroRabbit, not mpv.
    "--input-default-bindings=no",
    "--input-vo-keyboard=no",
    "--input-cursor=no",
    // RAM only: no disk cache, no resume files.
    "--cache-on-disk=no",
    "--resume-playback=no",
    "--save-position-on-quit=no",
    "--audio-client-name=RetroRabbit",
    "--term-osd=no",
    "--really-quiet",
];

/// Remove sockets left behind by RetroRabbit instances that crashed.
fn clean_stale_sockets(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let pid = name
            .strip_prefix("retrorabbit-")
            .filter(|r| r.ends_with(".sock"))
            .map(|r| r.chars().take_while(char::is_ascii_digit).collect::<String>());
        if let Some(pid) = pid.and_then(|p| p.parse::<u32>().ok()) {
            if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

impl Player {
    pub fn spawn(wid: u32, volume: f64, even_volume: bool) -> Result<Self, String> {
        Self::launch(
            "",
            &[
                format!("--wid={wid}"),
                format!("--volume={volume}"),
                format!("--af={}", if even_volume { EVEN_VOLUME } else { "" }),
            ],
            &[
                "--force-window=yes",
                "--cursor-autohide=no",
                "--cache=yes",
                "--demuxer-max-bytes=64MiB",
                "--demuxer-max-back-bytes=16MiB",
                "--network-timeout=20",
                "--hwdec=auto-safe",
                // Start at the nearest keyframe instead of decoding up to the exact second:
                // much quicker to tune in, and nobody notices a second or two.
                "--hr-seek=no",
                // Old-school green channel numbers in the corner.
                "--osd-level=1",
                "--osd-font=monospace",
                "--osd-font-size=44",
                "--osd-color=#7CFC00",
                "--osd-border-color=#000000",
                "--osd-border-size=3",
                "--osd-align-x=right",
                "--osd-align-y=top",
                "--osd-margin-x=40",
                "--osd-margin-y=30",
            ],
        )
    }

    /// A second, audio-only mpv that plays static hiss while the screen shows static.
    /// It sits paused on an endless noise source so switching it on is instant.
    pub fn spawn_hiss(volume: f64) -> Result<Self, String> {
        let mut p = Self::launch(
            "-static",
            &[format!("--volume={}", volume * HISS_LEVEL)],
            &["--vid=no", "--force-window=no", "--pause=yes"],
        )?;
        p.send(json!(["loadfile", HISS, "replace"]));
        Ok(p)
    }

    fn launch(tag: &str, args: &[String], fixed: &[&str]) -> Result<Self, String> {
        let path = socket_path(tag);
        if let Some(dir) = path.parent() {
            clean_stale_sockets(dir);
        }
        let _ = std::fs::remove_file(&path);
        let child = Command::new("mpv")
            .arg(format!("--input-ipc-server={}", path.display()))
            .args(args)
            .args(COMMON)
            .args(fixed)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("couldn't start mpv: {e}"))?;

        let deadline = Instant::now() + Duration::from_secs(5);
        let sock = loop {
            match UnixStream::connect(&path) {
                Ok(s) => break s,
                Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => return Err(format!("mpv didn't open its control socket: {e}")),
            }
        };

        let state = Arc::new(Mutex::new(PlayerState::default()));
        let reader = sock.try_clone().map_err(|e| e.to_string())?;
        let st = state.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(reader).lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                match v["event"].as_str() {
                    Some("start-file") => *st.lock().unwrap() = PlayerState::default(),
                    Some("playback-restart") => st.lock().unwrap().playing = true,
                    Some("end-file") => {
                        let reason = v["reason"].as_str().unwrap_or("");
                        if reason == "eof" || reason == "error" {
                            let mut s = st.lock().unwrap();
                            s.ended = true;
                            if reason == "error" {
                                s.error = Some(v["file_error"].as_str().unwrap_or("playback error").to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        });

        Ok(Self { child, sock, path, state })
    }

    fn send(&mut self, cmd: Value) {
        let mut s = json!({ "command": cmd }).to_string();
        s.push('\n');
        let _ = self.sock.write_all(s.as_bytes());
    }

    pub fn state(&self) -> PlayerState {
        self.state.lock().unwrap().clone()
    }

    /// Play `source` starting `offset` seconds in: joining the show in progress.
    pub fn load(&mut self, source: &str, offset: f64) {
        *self.state.lock().unwrap() = PlayerState::default();
        self.send(json!(["set_property", "start", format!("{:.2}", offset.max(0.0))]));
        self.send(json!(["loadfile", source, "replace"]));
    }

    pub fn stop(&mut self) {
        self.send(json!(["stop"]));
    }

    pub fn osd(&mut self, text: &str, ms: u32) {
        self.send(json!(["show-text", text, ms]));
    }

    pub fn set_volume(&mut self, v: f64) {
        self.send(json!(["set_property", "volume", v]));
    }

    pub fn set_even_volume(&mut self, on: bool) {
        self.send(json!(["set_property", "af", if on { EVEN_VOLUME } else { "" }]));
    }

    /// Hiss only: start or stop the static sound.
    pub fn set_hiss(&mut self, on: bool) {
        self.send(json!(["set_property", "pause", !on]));
    }

    pub fn set_hiss_volume(&mut self, v: f64) {
        self.set_volume(v * HISS_LEVEL);
    }

    pub fn set_mute(&mut self, m: bool) {
        self.send(json!(["set_property", "mute", m]));
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.send(json!(["quit"]));
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.path);
    }
}
