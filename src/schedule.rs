//! The "always live" schedule.
//!
//! Every channel has been airing since EPOCH. What's on right now is pure math:
//! walk the channel's playlist from the epoch until we reach `now`. Nothing is
//! stored, so switching away and coming back finds the show further along, the
//! same way real TV does, and the schedule survives restarts.

use crate::archive::Archive;
use crate::library::Media;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 2000-01-01 00:00 UTC: the day RetroRabbit went on the air.
pub const EPOCH: f64 = 946_684_800.0;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Rule {
    /// Every local video.
    Everything,
    /// Local videos under this folder.
    Folder(String),
    /// Videos with this tag. Key "*" matches any key.
    Tag { key: String, value: String },
    /// A show, matched on the `show` tag.
    Show(String),
    /// An Internet Archive search, e.g. `collection:classic_cartoons`.
    Archive(String),
    /// Everything you've pressed Like on.
    Liked,
}

impl Rule {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Rule::Everything => "Everything",
            Rule::Folder(_) => "Folder",
            Rule::Tag { .. } => "Tag",
            Rule::Show(_) => "Show",
            Rule::Archive(_) => "Internet Archive",
            Rule::Liked => "Liked",
        }
    }

    pub fn matches(&self, m: &Media) -> bool {
        match self {
            Rule::Everything => !m.source.starts_with("http"),
            Rule::Folder(dir) => {
                let dir = dir.trim_end_matches('/');
                !dir.is_empty() && m.source.starts_with(&format!("{dir}/"))
            }
            Rule::Tag { key, value } => m.has_tag(key, value),
            Rule::Show(s) => m.has_tag("show", s),
            Rule::Archive(_) | Rule::Liked => false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Order {
    /// A fresh shuffle every time the channel runs through its whole playlist.
    Shuffle,
    /// Episodes air in order, then start over.
    InOrder,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Channel {
    pub number: u32,
    pub name: String,
    pub order: Order,
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub favorite: bool,
}

/// Something you liked. A full copy, so the Favorites channel works even when
/// the folder or Internet Archive search it came from isn't loaded.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Liked {
    pub title: String,
    pub source: String,
    pub duration: f64,
    pub tags: Vec<(String, String)>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Lineup {
    pub media_roots: Vec<String>,
    pub channels: Vec<Channel>,
    /// While true, channels are regenerated from the library automatically.
    pub auto: bool,
    pub volume: f64,
    #[serde(default)]
    pub liked: Vec<Liked>,
    /// Ch+/Ch- only stops on favorite channels.
    #[serde(default)]
    pub favorites_only: bool,
    /// Level out loudness between programs, like a TV station does.
    #[serde(default = "yes")]
    pub even_volume: bool,
}

fn yes() -> bool {
    true
}

impl Default for Lineup {
    fn default() -> Self {
        let media_roots = dirs::video_dir()
            .filter(|d| d.is_dir())
            .map(|d| vec![d.to_string_lossy().to_string()])
            .unwrap_or_default();
        Self { media_roots, channels: Vec::new(), auto: true, volume: 80.0, liked: Vec::new(), favorites_only: false, even_volume: true }
    }
}

pub fn build_playlist(ch: &Channel, local: &[Arc<Media>], archive: &Archive, liked: &[Liked]) -> Vec<Arc<Media>> {
    let mut out: Vec<Arc<Media>> = local
        .iter()
        .filter(|m| ch.rules.iter().any(|r| r.matches(m)))
        .cloned()
        .collect();
    for r in &ch.rules {
        match r {
            Rule::Archive(q) => out.extend(archive.items(q)),
            Rule::Liked => out.extend(liked.iter().map(|l| {
                Arc::new(Media { title: l.title.clone(), source: l.source.clone(), duration: l.duration, tags: l.tags.clone() })
            })),
            _ => {}
        }
    }
    out.retain(|m| m.duration > 0.0);
    out.sort_by_key(|m| m.sort_key());
    out.dedup_by(|a, b| a.source == b.source);
    out
}

#[derive(Clone, Debug)]
pub struct Slot {
    pub media: Arc<Media>,
    pub start: f64,
    pub end: f64,
}

/// Everything airing between `from` and `until` (unix seconds).
pub fn slots(ch: &Channel, list: &[Arc<Media>], from: f64, until: f64) -> Vec<Slot> {
    let total: f64 = list.iter().map(|m| m.duration).sum();
    if list.is_empty() || total <= 0.0 {
        return Vec::new();
    }
    let elapsed = (from - EPOCH).max(0.0);
    let mut cycle = (elapsed / total).floor() as u64;
    let mut t = EPOCH + cycle as f64 * total;
    let mut out = Vec::new();
    'outer: loop {
        for m in ordering(ch, list, cycle) {
            let end = t + m.duration;
            if end > from && t <= until {
                out.push(Slot { media: m.clone(), start: t, end });
            }
            t = end;
            if t > until || out.len() >= 500 {
                break 'outer;
            }
        }
        cycle += 1;
    }
    out
}

pub fn now_playing(ch: &Channel, list: &[Arc<Media>], now: f64) -> Option<Slot> {
    slots(ch, list, now, now).into_iter().next()
}

fn ordering(ch: &Channel, list: &[Arc<Media>], cycle: u64) -> Vec<Arc<Media>> {
    let mut v = list.to_vec();
    if ch.order == Order::Shuffle {
        let mut rng = SplitMix(fnv1a(ch.name.as_bytes()) ^ cycle.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        for i in (1..v.len()).rev() {
            let j = (rng.next() % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
    }
    v
}

fn fnv1a(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325, |h, &c| (h ^ c as u64).wrapping_mul(0x100_0000_01b3))
}

pub struct SplitMix(pub u64);
impl SplitMix {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Free public-domain channels from the Internet Archive.
pub const ARCHIVE_PRESETS: &[(&str, &str, Order)] = &[
    ("Toon Town", "collection:classic_cartoons OR collection:vintage_cartoons", Order::Shuffle),
    ("Classic TV", "collection:classic_tv", Order::Shuffle),
    ("Creature Feature", "collection:SciFi_Horror", Order::Shuffle),
    ("Comedy Matinee", "collection:Comedy_Films", Order::Shuffle),
    ("Educational Films", "collection:prelinger", Order::Shuffle),
    ("Commercial Break", "collection:classic_tv_commercials", Order::Shuffle),
    ("Midnight Movies", "(collection:SciFi_Horror AND year:[1950 TO 1969]) OR creator:(ed wood)", Order::Shuffle),
    ("Film Noir", "collection:Film_Noir", Order::Shuffle),
    ("Silent Era", "collection:silent_films", Order::Shuffle),
    ("Saddle Up Westerns", "(collection:feature_films OR collection:classic_tv) AND subject:(western)", Order::Shuffle),
    ("Whodunit", "collection:feature_films AND subject:(mystery)", Order::Shuffle),
    ("War Stories", "collection:feature_films AND subject:(war)", Order::Shuffle),
    ("Song & Dance", "collection:feature_films AND subject:(musical)", Order::Shuffle),
    ("Fifties TV", "collection:classic_tv_1950s", Order::Shuffle),
    ("Sixties TV", "collection:classic_tv_1960s", Order::Shuffle),
    ("Space Age TV", "collection:classic_tv AND subject:(sci-fi OR science fiction)", Order::Shuffle),
    ("Game Show Network", "collection:classic_tv AND subject:(game show)", Order::Shuffle),
    ("Coming Attractions", "collection:movie_trailers AND year:[1930 TO 1979]", Order::Shuffle),
    ("Time Capsule", "collection:ephemera", Order::Shuffle),
    ("Computer Chronicles", "collection:computerchronicles", Order::InOrder),
    ("Saturday Mornings", "subject:(saturday morning cartoons)", Order::Shuffle),
    ("80s Toons", "subject:(\"80s cartoons\" OR \"1980s cartoons\" OR \"80s cartoon\")", Order::Shuffle),
    ("90s Toons", "subject:(\"90s cartoons\" OR \"1990s cartoons\" OR \"90s cartoon\")", Order::Shuffle),
    ("Toon Tube", "collection:animationandcartoons AND year:[2000 TO 2015]", Order::Shuffle),
    ("Claymation Station", "collection:animationandcartoons AND subject:(stop motion OR claymation)", Order::Shuffle),
    ("Animation Station", "collection:animationandcartoons", Order::Shuffle),
];

/// Builds a sensible starting lineup: Internet Archive channels first (so their
/// numbers don't move), then one channel per media folder and per genre.
pub fn auto_lineup(local: &[Arc<Media>], roots: &[String]) -> Vec<Channel> {
    let mut chans = vec![Channel {
        number: 1,
        name: "Favorites".into(),
        order: Order::Shuffle,
        rules: vec![Rule::Liked],
        favorite: true,
    }];
    let mut n = 2;
    let mut add = |chans: &mut Vec<Channel>, name: String, order, rules| {
        chans.push(Channel { number: n, name, order, rules, favorite: false });
        n += 1;
    };
    for (name, q, order) in ARCHIVE_PRESETS {
        add(&mut chans, name.to_string(), *order, vec![Rule::Archive(q.to_string())]);
    }
    if local.is_empty() {
        return chans;
    }
    add(&mut chans, "Channel Surf".into(), Order::Shuffle, vec![Rule::Everything]);

    for root in roots {
        let Ok(rd) = std::fs::read_dir(root) else { continue };
        let mut dirs: Vec<_> = rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.path()).collect();
        dirs.sort();
        for d in dirs {
            let rule = Rule::Folder(d.to_string_lossy().to_string());
            let items: Vec<_> = local.iter().filter(|m| rule.matches(m)).collect();
            if items.is_empty() {
                continue;
            }
            let episodic = items.iter().filter(|m| m.tag("episode").is_some()).count() * 2 > items.len();
            let name = d.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            add(&mut chans, name, if episodic { Order::InOrder } else { Order::Shuffle }, vec![rule]);
        }
    }

    let mut genres: Vec<(String, usize)> = Vec::new();
    for m in local {
        for (k, v) in &m.tags {
            if k == "genre" {
                match genres.iter_mut().find(|(g, _)| g == v) {
                    Some((_, c)) => *c += 1,
                    None => genres.push((v.clone(), 1)),
                }
            }
        }
    }
    genres.sort();
    for (g, count) in genres {
        if count >= 3 {
            let name = title_case(&g);
            add(&mut chans, name, Order::Shuffle, vec![Rule::Tag { key: "genre".into(), value: g }]);
        }
    }
    chans
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(n: usize, dur: f64) -> Vec<Arc<Media>> {
        (0..n)
            .map(|i| {
                Arc::new(Media { title: format!("m{i}"), source: format!("/v/{i}"), duration: dur, tags: vec![] })
            })
            .collect()
    }

    #[test]
    fn schedule_is_continuous_and_deterministic() {
        let ch = Channel { number: 2, name: "x".into(), order: Order::Shuffle, rules: vec![], favorite: false };
        let list = media(5, 100.0);
        let now = 1_800_000_000.0;
        let s = slots(&ch, &list, now, now + 3600.0);
        assert!(s[0].start <= now && s[0].end > now);
        for w in s.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
        // Same answer every time: it's all math, no state.
        let again = now_playing(&ch, &list, now).unwrap();
        assert_eq!(again.media.source, s[0].media.source);
        assert_eq!(again.start, s[0].start);
    }

    #[test]
    fn in_order_cycles() {
        let ch = Channel { number: 2, name: "x".into(), order: Order::InOrder, rules: vec![], favorite: false };
        let list = media(3, 10.0);
        let s = slots(&ch, &list, EPOCH, EPOCH + 59.0);
        let order: Vec<_> = s.iter().map(|s| s.media.title.as_str()).collect();
        assert_eq!(order, ["m0", "m1", "m2", "m0", "m1", "m2"]);
    }
}
