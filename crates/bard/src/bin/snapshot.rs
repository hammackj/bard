//! Renders a view headlessly to PNG: `bard-snapshot <scene> <out.png> [WxH]`.
//!
//! The live Wayland window cannot be read back, and screenshotting the desktop
//! captures whatever has focus. This draws through the same wgpu renderer and
//! real text shaping, with no window on screen, so the result can be compared
//! pixel for pixel against a capture of the Electron build.

use bard::model::{LibraryView, NullBackend, Ui};
use bard::views::root::BardWindow;
use bard::{fonts, theme};
use bard_core::types::{AlbumFlags, Config, QueueItem, StatsMap, Track};
use gpui::*;
use std::rc::Rc;
use std::sync::Arc;

/// State dumped from the Electron build alongside its reference capture, so
/// both renders show the same library, queue and track.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    tracks: Vec<Track>,
    config: Config,
    stats: StatsMap,
    loved: Vec<String>,
    album_flags: AlbumFlags,
    #[serde(default)]
    now: Option<String>,
    #[serde(default)]
    queue: Vec<String>,
    #[serde(default)]
    position: f64,
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    open_album: Option<String>,
    #[serde(default)]
    menu_open: bool,
    #[serde(default)]
    sleep_open: bool,
    #[serde(default)]
    shortcuts_open: bool,
    #[serde(default)]
    playing: Option<bool>,
    #[serde(default)]
    library_open: Option<bool>,
    #[serde(default)]
    queue_open: Option<bool>,
    #[serde(default)]
    settings_open: bool,
    /// Pixels scrolled down the settings body.
    #[serde(default)]
    settings_scroll: f32,
    #[serde(default)]
    outputs: Vec<(String, String)>,
    #[serde(default)]
    paths: Option<bard_core::types::AppPaths>,
    /// Keystrokes to send before capturing, in GPUI's syntax ("ctrl-,", "/", "down").
    #[serde(default)]
    keys: Vec<String>,
}

fn fixture_ui(path: &str) -> anyhow::Result<(Ui, f32, Vec<String>)> {
    let f: Fixture = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let home = std::env::var("HOME")?;
    let mut ui = Ui::new(Rc::new(NullBackend), format!("{home}/.config/bard/artwork").into());
    let by_title = |t: &str| f.tracks.iter().find(|x| x.title == t).cloned();
    ui.track = f.now.as_deref().and_then(by_title);
    ui.duration = ui.track.as_ref().map(|t| t.duration_sec).unwrap_or(0.);
    ui.position = f.position;
    ui.playing = f.playing.unwrap_or(ui.track.is_some());
    ui.volume = f.config.audio.volume;
    ui.queue = f
        .queue
        .iter()
        .enumerate()
        .filter_map(|(i, t)| by_title(t).map(|track| QueueItem { id: format!("q{i}"), track, added_at: 0 }))
        .collect();
    ui.tracks = f.tracks;
    ui.config = f.config;
    ui.stats = f.stats;
    ui.loved = f.loved;
    ui.album_flags = f.album_flags;
    ui.overlay_url = Some("http://127.0.0.1:4747".into());
    ui.version = "0.1.3".into();
    ui.commit = "74ef9a8".into();
    ui.view = match f.view.as_deref() {
        Some("tracks") => LibraryView::Tracks,
        Some("loved") => LibraryView::Loved,
        _ => LibraryView::Albums,
    };
    ui.open_album = f.open_album;
    ui.menu_open = f.menu_open;
    ui.sleep_open = f.sleep_open;
    ui.shortcuts_open = f.shortcuts_open;
    ui.library_open = f.library_open.unwrap_or(true);
    ui.queue_open = f.queue_open.unwrap_or(true);
    ui.settings_open = f.settings_open;
    ui.outputs = f.outputs;
    ui.paths = f.paths;
    Ok((ui, f.settings_scroll, f.keys))
}

struct Fonts;
impl Render for Fonts {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let row = |w: FontWeight, label: &str| {
            div().font_weight(w).child(format!("{label} — Radiant Vibes 1:27 / 3:34"))
        };
        div()
            .size_full().bg(theme::ink()).text_color(theme::text())
            .font_family(fonts::UI).text_size(px(20.)).p_4().flex().flex_col().gap_2()
            .child(row(FontWeight::NORMAL, "400"))
            .child(row(FontWeight::MEDIUM, "500"))
            .child(row(FontWeight::SEMIBOLD, "600"))
            .child(row(FontWeight::BOLD, "700"))
            .child(div().font_family(fonts::MONO).child("Fira Code 0123456789 -> !="))
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let scene = args.get(1).map(String::as_str).unwrap_or("fonts");
    let out = args.get(2).cloned().unwrap_or_else(|| format!("{scene}.png"));
    let (w, h) = args
        .get(3)
        .and_then(|s| s.split_once('x'))
        .map(|(w, h)| (w.parse().unwrap(), h.parse().unwrap()))
        .unwrap_or((600.0f32, 260.0f32));

    let text = Arc::new(gpui_wgpu::CosmicTextSystem::new("Inter"));
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(bard::assets::Assets), || {
        gpui_platform::current_headless_renderer()
    });
    cx.update(|cx| fonts::load(cx))?;
    let mut keys = Vec::new();
    let window: AnyWindowHandle = match scene {
        "fonts" => cx.open_window(size(px(w), px(h)), |_, cx| cx.new(|_| Fonts))?.into(),
        fixture if fixture.ends_with(".json") => {
            let (ui, scroll, k) = fixture_ui(fixture)?;
            keys = k;
            cx.open_window(size(px(w), px(h)), move |window, cx| {
                cx.new(|cx| {
                    let w = BardWindow::new(ui, window, cx);
                    w.settings_scroll.set_offset(point(px(0.), px(-scroll)));
                    w
                })
            })?
            .into()
        }
        other => anyhow::bail!("unknown scene {other}"),
    };
    // Two passes: images decoded during the first frame are drawn in the second.
    cx.run_until_parked();
    for k in &keys {
        // parse() takes a 'static str; this tool runs once and exits.
        let stroke = Keystroke::parse(k.clone().leak()).map_err(|e| anyhow::anyhow!("{k}: {e:?}"))?;
        cx.update_window(window, |_, window, cx| {
            window.dispatch_keystroke(stroke, cx);
        })?;
        cx.run_until_parked();
    }
    cx.update_window(window, |_, window, _| window.refresh())?;
    cx.run_until_parked();
    let img = cx.capture_screenshot(window)?;
    img.save(&out)?;
    println!("{out} {}x{}", img.width(), img.height());
    Ok(())
}
