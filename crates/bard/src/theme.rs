//! The palette and type scale from `src/renderer/src/styles/index.css`.
//!
//! Palette taken from hammackj.com: Tailwind's `gray` ramp for the neutrals
//! and two cyans doing different jobs -- cyan-400 for hairlines and small text
//! on near-black, cyan-600 (the site's theme-color) for solid fills.

use gpui::{Hsla, Pixels, Rgba, px, rgb};

pub fn ink() -> Rgba {
    rgb(0x030712)
}
pub fn panel() -> Rgba {
    rgb(0x111827)
}
pub fn edge() -> Rgba {
    rgb(0x1f2937)
}
pub fn text() -> Rgba {
    rgb(0xf3f4f6)
}
pub fn muted() -> Rgba {
    rgb(0x9ca3af)
}
pub fn cyan() -> Rgba {
    rgb(0x22d3ee)
}
pub fn cyan_deep() -> Rgba {
    rgb(0x0891b2)
}
pub fn green() -> Rgba {
    rgb(0x34d399)
}
pub fn rose() -> Rgba {
    rgb(0xfb7185)
}

/// A palette colour at reduced opacity, like Tailwind's `bg-ink/85`.
pub fn a(color: Rgba, alpha: f32) -> Hsla {
    let mut c: Hsla = color.into();
    c.a = alpha;
    c
}

/// Five sizes, no improvising.
pub const S1: Pixels = px(11.);
pub const S2: Pixels = px(13.);
pub const S3: Pixels = px(15.);
pub const S4: Pixels = px(20.);
pub const S5: Pixels = px(34.);
