//! Inter and Fira Code, embedded so the app needs nothing installed.
//!
//! These are the faces the Electron build self-hosted, instanced from its
//! variable WOFF2 files at the weights the UI uses (400 and 600). GPUI's text
//! stack reads TrueType rather than WOFF2, and sees one face per variable file
//! at its default instance -- so loading the variable fonts directly rendered
//! every weight as regular and named Fira Code "Fira Code Light".
//!
//! GPUI has no letter-spacing. CSS `letter-spacing` adds a fixed amount after
//! every glyph, which is exactly what widening each glyph's advance does, so
//! each tracking value the stylesheet uses is baked into its own family. Kerning
//! survives, and so does truncation with an ellipsis, which per-character layout
//! would have lost.

use gpui::App;
use std::borrow::Cow;

/// Body text. `font-family: Inter`, no tracking.
pub const UI: &str = "Inter";
/// `.mono` -- Fira Code at -0.02em.
pub const MONO: &str = "Fira Code Tight";
/// Uppercase section labels, `tracking-[.13em]`.
pub const TRACK13: &str = "Inter Track13";
/// "Now playing", `tracking-[.15em]`.
pub const TRACK15: &str = "Inter Track15";
/// The "Bard" wordmark in the title bar, `tracking-[.18em]`.
pub const TRACK18: &str = "Inter Track18";
/// The now-playing title, `tracking-[-.015em]`.
pub const TITLE: &str = "Inter Tight15";
/// `.display` -- Inter at -0.02em.
pub const DISPLAY: &str = "Inter Tight20";

macro_rules! faces {
    ($($f:literal),* $(,)?) => { &[$(include_bytes!(concat!("../assets/fonts/", $f, ".ttf"))),*] };
}

const FACES: &[&[u8]] = faces![
    "Inter-400",
    "Inter-600",
    "InterTrack13-400",
    "InterTrack13-600",
    "InterTrack15-400",
    "InterTrack15-600",
    "InterTrack18-400",
    "InterTrack18-600",
    "InterTight15-400",
    "InterTight15-600",
    "InterTight20-400",
    "InterTight20-600",
    "FiraCode-400",
    "FiraCode-600",
    "FiraCodeTight-400",
    "FiraCodeTight-600",
];

pub fn load(cx: &mut App) -> anyhow::Result<()> {
    cx.text_system().add_fonts(FACES.iter().map(|f| Cow::Borrowed(*f)).collect())
}
