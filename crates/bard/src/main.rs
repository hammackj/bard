//! Bard: a local music player for a curated library.

use gpui::*;
use gpui_platform::application;

fn main() {
    application().with_assets(bard::assets::Assets).run(|cx: &mut App| {
        bard::fonts::load(cx).expect("embedded fonts");
        bard::views::text_input::bind_keys(cx);
        if let Err(e) = bard::app::open(cx, bard::app::Launch::from_env()) {
            eprintln!("[bard] failed to start: {e:#}");
            cx.quit();
        }
        cx.activate(true);
    });
}
