//! Internet Archive channels. Search results and metadata are kept in RAM only;
//! videos are streamed, never downloaded.

use crate::library::Media;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How many items each channel airs: the most popular ones, plus a fixed
/// random sample from deeper in the collection for variety.
const TOP_ITEMS: usize = 40;
const SAMPLED_ITEMS: usize = 80;
/// How deep into the collection (by popularity) the sample reaches.
const SEARCH_DEPTH: usize = 1000;
const MAX_FILES_PER_ITEM: usize = 8;
/// First pass: this many items per channel, so every channel is on the air
/// quickly. The rest of the pool is filled in by a second pass.
const STARTER_ITEMS: usize = 20;
/// Connections per channel, and channels loading at once (12 × 2 = 24 requests in flight).
const FETCH_THREADS: usize = 12;
const WORKERS: usize = 2;

#[derive(Clone, Debug)]
pub enum State {
    /// Waiting for its turn (channels load one at a time).
    Queued,
    Loading,
    Ready,
    Failed(String),
}

struct Entry {
    state: State,
    items: Vec<Arc<Media>>,
    /// (done, total) while loading.
    progress: Arc<(AtomicUsize, AtomicUsize)>,
    /// Items still to fetch in the second pass.
    rest_ids: Vec<String>,
}

pub struct Archive {
    entries: Mutex<HashMap<String, Entry>>,
    queue: Mutex<VecDeque<String>>,
    /// Second-pass work, done once every channel has its starter set.
    later: Mutex<VecDeque<String>>,
    workers: Mutex<usize>,
    generation: AtomicU64,
}

impl Archive {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(HashMap::new()),
            queue: Mutex::new(VecDeque::new()),
            later: Mutex::new(VecDeque::new()),
            workers: Mutex::new(0),
            generation: AtomicU64::new(0),
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn items(&self, query: &str) -> Vec<Arc<Media>> {
        self.entries.lock().unwrap().get(query.trim()).map(|e| e.items.clone()).unwrap_or_default()
    }

    pub fn state(&self, query: &str) -> Option<State> {
        self.entries.lock().unwrap().get(query.trim()).map(|e| e.state.clone())
    }

    /// (videos checked, videos to check) for a search that's loading.
    pub fn progress(&self, query: &str) -> Option<(usize, usize)> {
        let map = self.entries.lock().unwrap();
        let p = &map.get(query.trim())?.progress;
        let total = p.1.load(Ordering::Relaxed);
        (total > 0).then(|| (p.0.load(Ordering::Relaxed), total))
    }

    /// Queue a search for loading unless it's already loaded or queued.
    /// Searches load one at a time so we don't flood archive.org.
    pub fn request(self: &Arc<Self>, query: &str, repaint: Arc<dyn Fn() + Send + Sync>) {
        let query = query.trim().to_string();
        if query.is_empty() {
            return;
        }
        {
            let mut map = self.entries.lock().unwrap();
            if map.contains_key(&query) {
                return;
            }
            map.insert(query.clone(), Entry { state: State::Queued, items: vec![], progress: Default::default(), rest_ids: vec![] });
        }
        self.queue.lock().unwrap().push_back(query);
        self.ensure_worker(repaint);
    }

    /// Move a queued search to the front (the channel you're watching).
    pub fn prioritize(&self, query: &str) {
        for queue in [&self.queue, &self.later] {
            let mut q = queue.lock().unwrap();
            if let Some(pos) = q.iter().position(|x| x == query.trim()) {
                if pos > 0 {
                    let item = q.remove(pos).unwrap();
                    q.push_front(item);
                }
            }
        }
    }

    fn ensure_worker(self: &Arc<Self>, repaint: Arc<dyn Fn() + Send + Sync>) {
        let mut running = self.workers.lock().unwrap();
        while *running < WORKERS {
            *running += 1;
            let me = Arc::clone(self);
            let repaint = repaint.clone();
            std::thread::spawn(move || me.work(repaint));
        }
    }

    fn work(self: Arc<Self>, repaint: Arc<dyn Fn() + Send + Sync>) {
        let tick = {
            let repaint = repaint.clone();
            move || repaint()
        };
        loop {
            // Starters first, then second passes. Popped under the workers lock so a
            // new request can't slip past a worker that's about to exit.
            let next = {
                let mut running = self.workers.lock().unwrap();
                let next = match self.queue.lock().unwrap().pop_front() {
                    Some(q) => Some((q, true)),
                    None => self.later.lock().unwrap().pop_front().map(|q| (q, false)),
                };
                if next.is_none() {
                    *running -= 1;
                }
                next
            };
            let Some((query, starter)) = next else { break };
            if starter {
                self.load_starter(&query, &tick);
            } else {
                self.load_rest(&query);
            }
            self.generation.fetch_add(1, Ordering::Relaxed);
            repaint();
        }
    }

    fn load_starter(&self, query: &str, tick: &(dyn Fn() + Sync)) {
        let progress = match self.entries.lock().unwrap().get_mut(query) {
            Some(e) => {
                e.state = State::Loading;
                e.progress.clone()
            }
            None => return,
        };
        tick();
        let result = search(query).map(|ids| {
            let mut ids = pick_ids(query, ids);
            let rest = ids.split_off(ids.len().min(STARTER_ITEMS));
            (fetch_items(query, ids, &progress, tick), rest)
        });
        let mut map = self.entries.lock().unwrap();
        let Some(e) = map.get_mut(query) else { return };
        match result {
            Err(err) => e.state = State::Failed(err),
            Ok((items, rest)) => {
                e.items = items;
                let more = !rest.is_empty();
                e.rest_ids = rest;
                e.state = if !e.items.is_empty() {
                    State::Ready
                } else if more {
                    State::Loading
                } else {
                    State::Failed("nothing playable found".into())
                };
                if more {
                    let mut later = self.later.lock().unwrap();
                    // Nothing playable yet: don't make it wait behind the others.
                    if e.items.is_empty() { later.push_front(query.to_string()) } else { later.push_back(query.to_string()) }
                }
            }
        }
    }

    fn load_rest(&self, query: &str) {
        let ids = match self.entries.lock().unwrap().get_mut(query) {
            Some(e) => std::mem::take(&mut e.rest_ids),
            None => return,
        };
        let items = fetch_items(query, ids, &Default::default(), &|| {});
        if let Some(e) = self.entries.lock().unwrap().get_mut(query) {
            e.items.extend(items);
            e.state = if e.items.is_empty() { State::Failed("nothing playable found".into()) } else { State::Ready };
        }
    }

    /// Forget a failed search so the next `request` tries again.
    pub fn retry(&self, query: &str) {
        let mut map = self.entries.lock().unwrap();
        if matches!(map.get(query.trim()).map(|e| &e.state), Some(State::Failed(_))) {
            map.remove(query.trim());
        }
    }
}

/// The most popular items, plus a sample from further down that's the same on
/// every run (seeded by the query), so the schedule stays put across restarts.
fn pick_ids(query: &str, ids: Vec<String>) -> Vec<String> {
    let mut picked: Vec<String> = ids.iter().take(TOP_ITEMS).cloned().collect();
    let mut rest: Vec<String> = ids.into_iter().skip(TOP_ITEMS).collect();
    let seed = query.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, c| (h ^ c as u64).wrapping_mul(0x100_0000_01b3));
    let mut rng = crate::schedule::SplitMix(seed);
    for i in (1..rest.len()).rev() {
        let j = (rng.next() % (i as u64 + 1)) as usize;
        rest.swap(i, j);
    }
    picked.extend(rest.into_iter().take(SAMPLED_ITEMS));
    picked
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("RetroRabbit/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Splits RetroRabbit's own options off a search. `max:30` skips anything
/// longer than 30 minutes (archive.org's search can't filter by length).
pub fn split_options(query: &str) -> (String, Option<f64>) {
    let mut max = None;
    let rest: Vec<&str> = query
        .split_whitespace()
        .filter(|w| match w.strip_prefix("max:").and_then(|m| m.trim_end_matches('m').parse::<f64>().ok()) {
            Some(mins) => {
                max = Some(mins * 60.0);
                false
            }
            None => true,
        })
        .collect();
    (rest.join(" "), max)
}

/// Item identifiers for a search, most downloaded first.
fn search(query: &str) -> Result<Vec<String>, String> {
    let (query, _) = split_options(query);
    let q = format!("({query}) AND mediatype:movies");
    let url = format!(
        "https://archive.org/advancedsearch.php?q={}&fl[]=identifier&sort[]=downloads+desc&rows={}&output=json",
        encode(&q),
        SEARCH_DEPTH
    );
    let json: serde_json::Value =
        agent().get(&url).call().map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string())?;
    Ok(json["response"]["docs"]
        .as_array()
        .ok_or("unexpected search response")?
        .iter()
        .filter_map(|d| d["identifier"].as_str().map(str::to_string))
        .collect())
}

/// Fetch each item's file list and durations, several at a time.
fn fetch_items(
    query: &str,
    ids: Vec<String>,
    progress: &Arc<(AtomicUsize, AtomicUsize)>,
    tick: &(dyn Fn() + Sync),
) -> Vec<Arc<Media>> {
    let agent = agent();
    progress.0.store(0, Ordering::Relaxed);
    progress.1.store(ids.len(), Ordering::Relaxed);
    let queue = Mutex::new(ids);
    let found = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..FETCH_THREADS {
            let (queue, found, agent) = (&queue, &found, &agent);
            scope.spawn(move || loop {
                let Some(id) = queue.lock().unwrap().pop() else { break };
                if let Ok(items) = item_media(agent, &id, query) {
                    found.lock().unwrap().extend(items);
                }
                progress.0.fetch_add(1, Ordering::Relaxed);
                tick();
            });
        }
    });
    found.into_inner().unwrap().into_iter().map(Arc::new).collect()
}

fn item_media(agent: &ureq::Agent, id: &str, query: &str) -> Result<Vec<Media>, String> {
    let url = format!("https://archive.org/metadata/{}", encode(id));
    let json: serde_json::Value =
        agent.get(&url).call().map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string())?;
    let meta = &json["metadata"];
    let title = text(&meta["title"]).unwrap_or_else(|| id.to_string());

    let mut tags = vec![("archive".to_string(), query.to_lowercase())];
    for v in list(&meta["collection"]) {
        tags.push(("collection".into(), v.to_lowercase()));
    }
    for v in list(&meta["subject"]) {
        for s in v.split(';') {
            let s = s.trim().to_lowercase();
            if !s.is_empty() {
                tags.push(("subject".into(), s));
            }
        }
    }
    if let Some(y) = text(&meta["year"]).or_else(|| text(&meta["date"])).and_then(|d| d.get(..4).map(str::to_string)) {
        if y.chars().all(|c| c.is_ascii_digit()) {
            tags.push(("decade".into(), format!("{}0s", &y[..3])));
            tags.push(("year".into(), y));
        }
    }

    let files = json["files"].as_array().cloned().unwrap_or_default();
    // Prefer the archive's own web-friendly encodes.
    let pick = |fmt: &str| -> Vec<&serde_json::Value> {
        files.iter().filter(|f| f["format"].as_str() == Some(fmt) && length(f).is_some()).collect()
    };
    let chosen = ["h.264", "512Kb MPEG4", "MPEG4", "h.264 IA", "Ogg Video"]
        .iter()
        .map(|f| pick(f))
        .find(|v| !v.is_empty())
        .unwrap_or_default();

    let multi = chosen.len() > 1;
    Ok(chosen
        .into_iter()
        .take(MAX_FILES_PER_ITEM)
        .filter_map(|f| {
            let name = f["name"].as_str()?;
            let duration = length(f)?;
            let max = split_options(query).1.unwrap_or(4.0 * 3600.0);
            if !(10.0..=max).contains(&duration) {
                return None;
            }
            let title = if multi {
                format!("{title} – {}", crate::library::clean_name(name.rsplit_once('.').map_or(name, |p| p.0)))
            } else {
                title.clone()
            };
            Some(Media {
                title,
                source: format!("https://archive.org/download/{}/{}", encode(id), encode(name)),
                duration,
                tags: tags.clone(),
            })
        })
        .collect())
}

fn text(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(a) => a.first().and_then(|x| x.as_str()).map(str::to_string),
        _ => None,
    }
}

fn list(v: &serde_json::Value) -> Vec<String> {
    match v {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        _ => vec![],
    }
}

/// Archive lengths are either "363.16" or "01:02:03".
fn length(f: &serde_json::Value) -> Option<f64> {
    let s = f["length"].as_str()?;
    if s.contains(':') {
        s.split(':').try_fold(0.0, |acc, p| p.parse::<f64>().ok().map(|n| acc * 60.0 + n))
    } else {
        s.parse().ok()
    }
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loads every preset channel from archive.org and reports timing.
    /// Network test: `cargo test -- --ignored --nocapture load_all_presets`
    #[test]
    fn options() {
        assert_eq!(split_options("collection:x max:30"), ("collection:x".to_string(), Some(1800.0)));
        assert_eq!(split_options("max:5m collection:x"), ("collection:x".to_string(), Some(300.0)));
        assert_eq!(split_options("collection:x"), ("collection:x".to_string(), None));
    }

    #[test]
    #[ignore]
    fn load_all_presets() {
        let a = Archive::new();
        let t0 = std::time::Instant::now();
        let presets = crate::schedule::ARCHIVE_PRESETS;
        for (_, q, _) in presets {
            a.request(q, Arc::new(|| {}));
        }
        let ready = || presets.iter().filter(|(_, q, _)| matches!(a.state(q), Some(State::Ready | State::Failed(_)))).count();
        while ready() < presets.len() {
            std::thread::sleep(Duration::from_millis(250));
        }
        let on_air = t0.elapsed();
        while *a.workers.lock().unwrap() > 0 {
            std::thread::sleep(Duration::from_millis(250));
        }
        for (name, q, _) in presets {
            eprintln!("{name:22} {:?} {} videos", a.state(q).unwrap(), a.items(q).len());
        }
        eprintln!("all {} channels on the air in {on_air:.0?}, fully loaded in {:.0?}", presets.len(), t0.elapsed());
    }
}

#[cfg(test)]
mod one_channel {
    use super::*;

    /// `RR_QUERY='…' cargo test -- --ignored --nocapture load_one`
    #[test]
    #[ignore]
    fn load_one() {
        let q = std::env::var("RR_QUERY").expect("set RR_QUERY");
        let a = Archive::new();
        a.request(&q, Arc::new(|| {}));
        while *a.workers.lock().unwrap() > 0 || matches!(a.state(&q), Some(State::Queued)) {
            std::thread::sleep(Duration::from_millis(250));
        }
        let items = a.items(&q);
        let longest = items.iter().map(|m| m.duration).fold(0.0, f64::max);
        let total: f64 = items.iter().map(|m| m.duration).sum();
        eprintln!("{:?}: {} videos, longest {:.1} min, {:.1} h total", a.state(&q).unwrap(), items.len(), longest / 60.0, total / 3600.0);
    }
}
