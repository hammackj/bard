//! The tray icon: a StatusNotifierItem (what Plasma's system tray shows), via
//! `ksni`. A port of `src/main/tray.ts`.
//!
//! The tray *popup* (the now-playing preview, `trayPopup.ts`) is a window and
//! belongs to the app; the tray only asks for it through
//! [`AppControls::show_popup`].

use crate::AppControls;
use ksni::menu::{MenuItem, StandardItem};
use ksni::{Icon, ToolTip, TrayMethods};
use std::sync::Arc;

/// What the menu and tooltip show.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrayState {
    /// (title, artist) of the track on air.
    pub current: Option<(String, String)>,
    pub playing: bool,
    pub queue_length: usize,
}

impl TrayState {
    /// The tooltip, as the Electron build worded it.
    pub fn tooltip(&self) -> String {
        match &self.current {
            Some((title, artist)) => format!("Bard \u{2014} {title} \u{00b7} {artist}"),
            None => "Bard \u{2014} nothing playing".into(),
        }
    }

    /// The menu's first, disabled line.
    pub fn heading(&self) -> String {
        match &self.current {
            Some((title, artist)) => format!("{title} \u{2014} {artist}"),
            None => "Nothing playing".into(),
        }
    }

    /// What the menu and tooltip are built from (`updateTray`'s key).
    ///
    /// Updates arrive about four times a second, because the now-playing
    /// snapshot carries the playback position. None of that reaches the tray
    /// -- rebuilding the menu at that rate is pure waste, and on some desktops
    /// it makes the menu flicker while it is open -- so the tray is only
    /// touched when this changes.
    fn key(&self) -> (Option<(String, String)>, bool, bool) {
        (self.current.clone(), self.playing, self.queue_length > 0)
    }
}

struct BardTray {
    state: TrayState,
    controls: Arc<dyn AppControls>,
    icons: Vec<Icon>,
}

impl ksni::Tray for BardTray {
    fn id(&self) -> String {
        "bard".into()
    }
    fn title(&self) -> String {
        "Bard".into()
    }
    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }
    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icons.clone()
    }
    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: self.state.tooltip(), ..Default::default() }
    }
    // Left click restores the window -- that is what a tray icon is expected
    // to do, and routing it to the preview instead left no way back to the app.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.controls.show_window();
    }
    // Middle click: the now-playing preview.
    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.controls.show_popup();
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let item = |label: &str, enabled: bool, f: fn(&Self)| -> MenuItem<Self> {
            StandardItem {
                label: label.into(),
                enabled,
                activate: Box::new(move |t: &mut Self| f(t)),
                ..Default::default()
            }
            .into()
        };
        vec![
            item(&self.state.heading(), false, |_| {}),
            MenuItem::Separator,
            item("Show Bard", true, |t| t.controls.show_window()),
            item("Now playing preview", true, |t| t.controls.show_popup()),
            MenuItem::Separator,
            item(if self.state.playing { "Pause" } else { "Play" }, true, |t| t.controls.toggle()),
            item("Skip", self.state.queue_length > 0, |t| t.controls.next()),
            MenuItem::Separator,
            item("Quit", true, |t| t.controls.quit()),
        ]
    }
}

/// The icon at the sizes panels ask for. A full-size app icon in a tray slot
/// renders as a blurry smear; 22px is the common Linux panel height, and the
/// host picks whichever size fits best.
fn icons() -> Vec<Icon> {
    const PNGS: [&[u8]; 6] = [
        include_bytes!("../../../resources/icons/icon-16.png"),
        include_bytes!("../../../resources/icons/icon-22.png"),
        include_bytes!("../../../resources/icons/icon-24.png"),
        include_bytes!("../../../resources/icons/icon-32.png"),
        include_bytes!("../../../resources/icons/icon-48.png"),
        include_bytes!("../../../resources/icons/icon-64.png"),
    ];
    PNGS.iter().filter_map(|p| png_to_argb(p)).collect()
}

/// Decodes a PNG into the ARGB32 (network byte order) pixmap SNI wants.
pub(crate) fn png_to_argb(bytes: &[u8]) -> Option<Icon> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let rgba: &[u8] = &buf[..info.buffer_size()];
    let data = match info.color_type {
        png::ColorType::Rgba => rgba.chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect(),
        png::ColorType::GrayscaleAlpha => rgba.chunks_exact(2).flat_map(|p| [p[1], p[0], p[0], p[0]]).collect(),
        _ => return None,
    };
    Some(Icon { width: info.width as i32, height: info.height as i32, data })
}

/// A running tray icon.
pub struct Tray {
    handle: ksni::Handle<BardTray>,
    last: std::sync::Mutex<TrayState>,
    /// Updates go through one task so they land in the order they were made.
    tx: tokio::sync::mpsc::UnboundedSender<TrayState>,
}

impl Tray {
    /// Registers with the StatusNotifierWatcher. Must run on a tokio runtime.
    pub async fn start(initial: TrayState, controls: Arc<dyn AppControls>) -> Result<Tray, ksni::Error> {
        let tray = BardTray { state: initial.clone(), controls, icons: icons() };
        let handle = tray.spawn().await?;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TrayState>();
        let h = handle.clone();
        tokio::spawn(async move {
            while let Some(next) = rx.recv().await {
                h.update(|t| t.state = next).await;
            }
        });
        Ok(Tray { handle, last: std::sync::Mutex::new(initial), tx })
    }

    /// The state last shown.
    pub fn state(&self) -> TrayState {
        self.last.lock().unwrap().clone()
    }

    /// Updates the tooltip and menu when something they show changed. Cheap;
    /// call on every event.
    pub fn update(&self, next: TrayState) {
        let mut last = self.last.lock().unwrap();
        if last.key() == next.key() {
            *last = next;
            return;
        }
        *last = next.clone();
        let _ = self.tx.send(next);
    }

    /// Whether the tray service is still up.
    pub fn is_running(&self) -> bool {
        !self.handle.is_closed()
    }

    pub fn shutdown(&self) -> ksni::ShutdownAwaiter {
        self.handle.shutdown()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_decode_to_argb() {
        let icons = icons();
        assert_eq!(icons.len(), 6);
        let i22 = &icons[1];
        assert_eq!((i22.width, i22.height), (22, 22));
        assert_eq!(i22.data.len(), 22 * 22 * 4);
    }

    #[test]
    fn tooltip_and_heading_wording() {
        let mut s = TrayState::default();
        assert_eq!(s.tooltip(), "Bard \u{2014} nothing playing");
        assert_eq!(s.heading(), "Nothing playing");
        s.current = Some(("Song".into(), "Band".into()));
        assert_eq!(s.tooltip(), "Bard \u{2014} Song \u{00b7} Band");
        assert_eq!(s.heading(), "Song \u{2014} Band");
    }

    #[test]
    fn position_only_updates_do_not_rebuild() {
        let a = TrayState { current: Some(("a".into(), "b".into())), playing: true, queue_length: 2 };
        let b = TrayState { queue_length: 5, ..a.clone() };
        assert_eq!(a.key(), b.key());
        let c = TrayState { queue_length: 0, ..a.clone() };
        assert_ne!(a.key(), c.key());
    }
}
