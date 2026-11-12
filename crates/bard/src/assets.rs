//! Embedded SVG icons, served to GPUI's `svg()` and `img()` elements.
//!
//! The paths are copied from the React components so the icons are the same
//! drawings, not lookalikes from an icon set.

use gpui::{AssetSource, SharedString};
use std::borrow::Cow;

pub struct Assets;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((concat!("icons/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")))),*]
    };
}

const FILES: &[(&str, &[u8])] = icons![
    "dock-left-open", "dock-left-closed", "dock-right-open", "dock-right-closed",
    "prev", "play", "pause", "skip", "volume", "repeat", "endless", "moon",
    "heart", "heart-filled", "shield", "avatar", "placeholder", "chevron", "check",
];

pub fn bytes(path: &str) -> Option<&'static [u8]> {
    FILES.iter().find(|(p, _)| *p == path).map(|(_, b)| *b)
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(bytes(path).map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(FILES
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}
