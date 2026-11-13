//! Bard: a local music player for a curated library.

// A release build on Windows is a GUI app: no console window behind it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use gpui::*;
use gpui_platform::application;

/// This process's claim on its data directory, held until it exits.
static INSTANCE: std::sync::OnceLock<bard_core::DataDirLock> = std::sync::OnceLock::new();

fn main() {
    let launch = bard::app::Launch::from_env();
    match launch.start() {
        Ok(bard::app::Start::Run(lock)) => {
            let _ = INSTANCE.set(lock);
        }
        Ok(bard::app::Start::AlreadyRunning { raised }) => {
            if raised {
                eprintln!("[bard] already running; raised the existing window");
            } else {
                eprintln!("[bard] already running (its window could not be raised); exiting");
            }
            return;
        }
        Err(e) => {
            eprintln!("[bard] cannot lock {}: {e}", launch.data_dir.display());
            std::process::exit(1);
        }
    }
    application().with_assets(bard::assets::Assets).run(|cx: &mut App| {
        bard::fonts::load(cx).expect("embedded fonts");
        bard::views::text_input::bind_keys(cx);
        if let Err(e) = bard::app::open(cx, launch) {
            eprintln!("[bard] failed to start: {e:#}");
            cx.quit();
        }
        cx.activate(true);
    });
}
