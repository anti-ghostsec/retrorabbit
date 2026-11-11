//! Local media library. Everything lives in RAM: folders are probed with
//! `ffprobe` at startup and the results are never written anywhere.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

#[derive(Debug, Clone)]
pub struct Media {
    pub title: String,
    /// Local path or URL handed to the player.
    pub source: String,
    /// Seconds.
    pub duration: f64,
    /// Lowercased (key, value) pairs, e.g. ("genre", "horror").
    pub tags: Vec<(String, String)>,
}

impl Media {
    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// `key` of "" or "*" matches any key.
    pub fn has_tag(&self, key: &str, value: &str) -> bool {
        let key = key.trim().to_lowercase();
        let value = value.trim().to_lowercase();
        self.tags
            .iter()
            .any(|(k, v)| (key.is_empty() || key == "*" || *k == key) && *v == value)
    }

    /// Show, season, episode, source: gives a natural "in order" sort.
    pub fn sort_key(&self) -> (String, u32, u32, String) {
        let num = |k| self.tag(k).and_then(|v| v.parse().ok()).unwrap_or(0);
        (
            self.tag("show").unwrap_or("").to_string(),
            num("season"),
            num("episode"),
            self.source.clone(),
        )
    }
}

pub const VIDEO_EXTS: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "wmv", "mpg", "mpeg", "ts", "m2ts", "webm", "ogv", "flv",
    "vob", "3gp", "divx", "asf", "rm", "rmvb",
];

#[derive(Default, Clone)]
pub struct ScanStatus {
    pub running: bool,
    pub done: usize,
    pub total: usize,
    pub message: String,
}

pub struct Library {
    items: RwLock<Vec<Arc<Media>>>,
    generation: AtomicU64,
    status: Mutex<ScanStatus>,
}

impl Library {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            items: RwLock::new(Vec::new()),
            generation: AtomicU64::new(0),
            status: Mutex::new(ScanStatus::default()),
        })
    }

    pub fn items(&self) -> Vec<Arc<Media>> {
        self.items.read().unwrap().clone()
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn status(&self) -> ScanStatus {
        self.status.lock().unwrap().clone()
    }

    /// Rescan all roots in the background, replacing the library when done.
    pub fn scan(self: &Arc<Self>, roots: Vec<String>, repaint: impl Fn() + Send + Sync + 'static) {
        {
            let mut st = self.status.lock().unwrap();
            if st.running {
                return;
            }
            *st = ScanStatus { running: true, message: "Looking for videos…".into(), ..Default::default() };
        }
        let lib = Arc::clone(self);
        let repaint = Arc::new(repaint);
        std::thread::spawn(move || {
            let mut files = Vec::new();
            for root in &roots {
                let root = PathBuf::from(root);
                walk(&root, &root, &mut files);
            }
            lib.status.lock().unwrap().total = files.len();
            repaint();

            let queue = Arc::new(Mutex::new(files));
            let found = Arc::new(Mutex::new(Vec::new()));
            let workers = std::thread::available_parallelism().map(|n| n.get().min(4)).unwrap_or(2);
            let handles: Vec<_> = (0..workers)
                .map(|_| {
                    let (queue, found, lib, repaint) =
                        (queue.clone(), found.clone(), lib.clone(), repaint.clone());
                    std::thread::spawn(move || loop {
                        let Some((root, path)) = queue.lock().unwrap().pop() else { break };
                        if let Some(m) = probe(&root, &path) {
                            found.lock().unwrap().push(Arc::new(m));
                        }
                        lib.status.lock().unwrap().done += 1;
                        repaint();
                    })
                })
                .collect();
            for h in handles {
                let _ = h.join();
            }

            let mut items = std::mem::take(&mut *found.lock().unwrap());
            items.sort_by_key(|m| m.sort_key());
            let n = items.len();
            *lib.items.write().unwrap() = items;
            lib.generation.fetch_add(1, Ordering::Relaxed);
            {
                let mut st = lib.status.lock().unwrap();
                st.running = false;
                st.message = format!("{n} videos found");
            }
            repaint();
        });
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        // Don't follow symlinked directories (avoids loops).
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.is_dir() {
            walk(root, &path, out);
        } else if is_video(&path) {
            out.push((root.to_path_buf(), path));
        }
    }
}

fn is_video(p: &Path) -> bool {
    p.extension()
        .map(|e| VIDEO_EXTS.contains(&e.to_string_lossy().to_lowercase().as_str()))
        .unwrap_or(false)
}

fn probe(root: &Path, path: &Path) -> Option<Media> {
    let out = Command::new("ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_format", "--"])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let format = json.get("format")?;
    let duration: f64 = format.get("duration")?.as_str()?.parse().ok()?;
    if duration < 1.0 {
        return None;
    }

    let mut tags: Vec<(String, String)> = Vec::new();
    let mut push = |k: &str, v: &str| {
        let v = v.trim().to_lowercase();
        if !v.is_empty() && !tags.iter().any(|(a, b)| a == k && *b == v) {
            tags.push((k.to_string(), v));
        }
    };

    // Folder names become tags: Videos/Cartoons/Looney → folder=cartoons, folder=looney
    if let Some(name) = root.file_name() {
        push("folder", &name.to_string_lossy());
    }
    if let Ok(rel) = path.parent().unwrap_or(root).strip_prefix(root) {
        for c in rel.components() {
            push("folder", &c.as_os_str().to_string_lossy());
        }
    }

    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut file_tags = std::collections::BTreeMap::<String, String>::new();
    if let Some(t) = format.get("tags").and_then(|t| t.as_object()) {
        for (k, v) in t {
            if let Some(v) = v.as_str() {
                file_tags.insert(k.to_lowercase(), v.to_string());
            }
        }
    }

    let ep = parse_episode(&stem);
    let show = file_tags.get("show").cloned().or_else(|| {
        ep.as_ref().map(|(s, _, _)| s.clone()).filter(|s| !s.is_empty()).or_else(|| {
            // "Show/Season 2/S02E05.mkv" — use the folder above the season folder.
            ep.as_ref()?;
            path.ancestors()
                .skip(1)
                .map(|a| a.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
                .find(|n| !n.to_lowercase().starts_with("season") && !n.is_empty())
        })
    });
    if let Some(show) = &show {
        push("show", show);
    }
    if let Some((_, s, e)) = ep {
        push("season", &s.to_string());
        push("episode", &e.to_string());
    }

    if let Some(g) = file_tags.get("genre") {
        for part in g.split([',', ';', '/']) {
            push("genre", part);
        }
    }
    for k in ["artist", "album", "album_artist", "network", "studio"] {
        if let Some(v) = file_tags.get(k) {
            push(k, v);
        }
    }
    let year = file_tags
        .get("date")
        .or(file_tags.get("year"))
        .and_then(|d| d.get(..4))
        .filter(|y| y.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_string)
        .or_else(|| find_year(&stem).map(|y| y.to_string()));
    if let Some(y) = &year {
        push("year", y);
        if let Ok(n) = y.parse::<u32>() {
            push("decade", &format!("{}s", n / 10 * 10));
        }
    }

    let file_title = file_tags.get("title").filter(|t| !t.trim().is_empty()).cloned();
    let title = match (&show, ep) {
        (Some(show), Some((_, s, e))) => match &file_title {
            Some(t) => format!("{show} S{s:02}E{e:02} – {t}"),
            None => format!("{show} S{s:02}E{e:02}"),
        },
        _ => file_title.unwrap_or_else(|| clean_name(&stem)),
    };

    Some(Media { title, source: path.to_string_lossy().to_string(), duration, tags })
}

/// Finds `S01E02` / `1x02` patterns. Returns (show name before it, season, episode).
pub fn parse_episode(stem: &str) -> Option<(String, u32, u32)> {
    let b = stem.as_bytes();
    let digits = |i: usize, max: usize| -> (u32, usize) {
        let mut n = 0u32;
        let mut j = i;
        while j < b.len() && j - i < max && b[j].is_ascii_digit() {
            n = n * 10 + (b[j] - b'0') as u32;
            j += 1;
        }
        (n, j - i)
    };
    for i in 0..b.len() {
        // S01E02
        if (b[i] == b's' || b[i] == b'S') && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
            let (s, sl) = digits(i + 1, 2);
            let e_at = i + 1 + sl;
            if sl > 0 && e_at < b.len() && (b[e_at] == b'e' || b[e_at] == b'E') {
                let (e, el) = digits(e_at + 1, 3);
                if el > 0 {
                    return Some((clean_name(&stem[..i]), s, e));
                }
            }
        }
        // 1x02
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
            let (s, sl) = digits(i, 2);
            let x_at = i + sl;
            if x_at < b.len() && (b[x_at] == b'x' || b[x_at] == b'X') {
                let (e, el) = digits(x_at + 1, 3);
                let end = x_at + 1 + el;
                if el == 2 && (end == b.len() || !b[end].is_ascii_digit()) {
                    return Some((clean_name(&stem[..i]), s, e));
                }
            }
        }
    }
    None
}

fn find_year(s: &str) -> Option<u32> {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(3)).find_map(|i| {
        let w = &b[i..i + 4];
        let ok = w.iter().all(u8::is_ascii_digit)
            && (w.starts_with(b"19") || w.starts_with(b"20"))
            && (i == 0 || !b[i - 1].is_ascii_digit())
            && (i + 4 == b.len() || !b[i + 4].is_ascii_digit());
        ok.then(|| s[i..i + 4].parse().ok()).flatten()
    })
}

/// "The.Thing.1982.1080p.BluRay" → "The Thing"
pub fn clean_name(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth = (depth - 1).max(0),
            '.' | '_' if depth == 0 => out.push(' '),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    if let Some(y) = find_year(&out) {
        if let Some(pos) = out.find(&y.to_string()) {
            if pos > 0 {
                out.truncate(pos);
            }
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    out.trim_matches(|c: char| c == '-' || c.is_whitespace()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episodes() {
        assert_eq!(parse_episode("Show.Name.S02E05.720p"), Some(("Show Name".into(), 2, 5)));
        assert_eq!(parse_episode("Show - 1x07 - Pilot"), Some(("Show".into(), 1, 7)));
        assert_eq!(parse_episode("S01E02"), Some(("".into(), 1, 2)));
        assert_eq!(parse_episode("Movie 1984"), None);
    }

    #[test]
    fn names() {
        assert_eq!(clean_name("The.Thing.1982.1080p.BluRay"), "The Thing");
        assert_eq!(clean_name("[Group] Cool_Show"), "Cool Show");
        assert_eq!(find_year("Night.of.the.Living.Dead.1968"), Some(1968));
    }
}
