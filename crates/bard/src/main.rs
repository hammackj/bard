//! Bard: a local music player for a curated library.

use gpui::*;
use gpui_platform::application;

fn main() {
    let launch = bard::app::Launch::from_env();
    if launch.raise_running() {
        eprintln!("[bard] already running; raised the existing window");
        return;
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
