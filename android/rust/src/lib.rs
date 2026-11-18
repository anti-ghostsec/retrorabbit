//! RetroRabbit for Android TV. The app itself (schedule, Internet Archive, guide,
//! channel editor, Win95 look) is the desktop code, included as-is; only video,
//! sound, media probing and the remote control are Android's own.

#[path = "../../../src/app.rs"]
mod app;
#[path = "../../../src/archive.rs"]
mod archive;
#[path = "../../../src/gltex.rs"]
mod gltex;
#[path = "../../../src/library.rs"]
mod library;
#[path = "../../../src/schedule.rs"]
mod schedule;
#[path = "../../../src/vault.rs"]
mod vault;
#[path = "../../../src/win95.rs"]
mod win95;

mod android;
mod embed;
mod player;
mod tv;

use winit::platform::android::activity::AndroidApp;

/// Same shape as the desktop's command-line options; on a TV they come from the
/// phone/TV's own video folders.
#[derive(Default)]
pub struct Args {
    pub folders: Vec<String>,
    pub channel: Option<u32>,
    pub guide: bool,
    pub channels: bool,
}

impl Args {
    pub fn parse() -> Self {
        let folders = android::video_folders();
        Args { folders, ..Default::default() }
    }
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("RetroRabbit"));
    android::init(&app);
    let options = eframe::NativeOptions { android_app: Some(app.clone()), ..Default::default() };
    let res = eframe::run_native(
        "RetroRabbit",
        options,
        Box::new(move |cc| Ok(Box::new(tv::TvApp::new(app::RetroRabbit::new(cc), app)))),
    );
    if let Err(e) = res {
        log::error!("RetroRabbit stopped: {e}");
    }
}
