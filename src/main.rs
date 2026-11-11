mod app;
mod archive;
mod embed;
mod library;
mod player;
mod schedule;
mod vault;
mod win95;

const USAGE: &str = "\
RetroRabbit: your own little cable TV.

Usage: retrorabbit [OPTIONS] [FOLDER...]

  FOLDER          Media folders to scan (default: ~/Videos)
  --channel N     Start on channel N
  --guide         Open the TV guide
  --channels      Open the channel editor
  -h, --help      Show this help

Keys: Up/Down change channel, 0-9 type a channel, Backspace recall,
      G guide, C channels, I info, M mute, +/- volume, F full screen,
      S favorite channel, L like, O favorites only, N even volume.
Mouse: wheel = volume, middle click = mute, double-click = full screen";

#[derive(Default)]
pub struct Args {
    pub folders: Vec<String>,
    pub channel: Option<u32>,
    pub guide: bool,
    pub channels: bool,
}

impl Args {
    pub fn parse() -> Self {
        let mut a = Args::default();
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "-h" | "--help" => {
                    println!("{USAGE}");
                    std::process::exit(0);
                }
                "--channel" => a.channel = it.next().and_then(|n| n.parse().ok()),
                "--guide" => a.guide = true,
                "--channels" => a.channels = true,
                _ if std::path::Path::new(&arg).is_dir() => {
                    let p = std::fs::canonicalize(&arg).map(|p| p.to_string_lossy().to_string()).unwrap_or(arg);
                    a.folders.push(p);
                }
                _ => {
                    eprintln!("retrorabbit: not a folder or option: {arg}\n\n{USAGE}");
                    std::process::exit(2);
                }
            }
        }
        a
    }
}

fn main() -> eframe::Result {
    Args::parse(); // handle --help / bad arguments before opening a window
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("RetroRabbit")
            .with_app_id("retrorabbit")
            .with_inner_size([1024.0, 720.0])
            .with_min_inner_size([320.0, 240.0]),
        ..Default::default()
    };
    eframe::run_native("RetroRabbit", options, Box::new(|cc| Ok(Box::new(app::RetroRabbit::new(cc)))))
}
