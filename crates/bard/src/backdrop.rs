//! The window-wide light and texture: the cyan glow under everything, and the
//! vignette and film grain over it.
//!
//! GPUI draws linear gradients but not radial ones, so these are rendered to
//! bitmaps with the stylesheet's own geometry. The radial terms are smooth, so
//! they are computed at a fraction of the window's resolution and scaled up by
//! the GPU; the grain is per-pixel and computed at full device resolution.

use gpui::RenderImage;
use image::{Frame, Rgba, RgbaImage};
use std::sync::Arc;

/// Device pixels per computed pixel for the smooth layers.
const SMOOTH_STEP: u32 = 4;

/// Hands an RGBA buffer to GPUI, which wants BGRA.
pub fn render_image(mut img: RgbaImage) -> Arc<RenderImage> {
    for p in img.pixels_mut() {
        p.0.swap(0, 2);
    }
    Arc::new(RenderImage::new(vec![Frame::new(img)]))
}

/// Normalised distance from an ellipse centre, as CSS sizes a
/// `radial-gradient(<rx>% <ry>% at <cx>% <cy>%, ...)`.
fn radial_t(x: f32, y: f32, w: f32, h: f32, (rx, ry, cx, cy): (f32, f32, f32, f32)) -> f32 {
    let dx = (x - cx * w) / (rx * w);
    let dy = (y - cy * h) / (ry * h);
    (dx * dx + dy * dy).sqrt()
}

fn smooth(w: u32, h: u32, f: impl Fn(f32, f32, f32, f32) -> [f32; 4]) -> Arc<RenderImage> {
    let (sw, sh) = ((w / SMOOTH_STEP).max(2), (h / SMOOTH_STEP).max(2));
    let img = RgbaImage::from_fn(sw, sh, |x, y| {
        let [r, g, b, a] = f(x as f32 + 0.5, y as f32 + 0.5, sw as f32, sh as f32);
        Rgba([(r * 255.) as u8, (g * 255.) as u8, (b * 255.) as u8, (a * 255.).round() as u8])
    });
    render_image(img)
}

/// `body::before`: radial-gradient(120% 90% at 38% 42%, rgba(8,145,178,.09), transparent 62%).
pub fn glow(w: u32, h: u32) -> Arc<RenderImage> {
    smooth(w, h, |x, y, w, h| {
        let t = radial_t(x, y, w, h, (1.2, 0.9, 0.38, 0.42));
        let a = 0.09 * (1.0 - (t / 0.62).min(1.0));
        [8. / 255., 145. / 255., 178. / 255., a]
    })
}

/// `.depth-overlay`: radial-gradient(128% 88% at 50% 32%, transparent 58%, rgb(0 0 0 / .5) 100%).
pub fn vignette(w: u32, h: u32) -> Arc<RenderImage> {
    smooth(w, h, |x, y, w, h| {
        let t = radial_t(x, y, w, h, (1.28, 0.88, 0.5, 0.32));
        let a = 0.5 * ((t - 0.58) / 0.42).clamp(0.0, 1.0);
        [0., 0., 0., a]
    })
}

/// `.depth-overlay::after`: fractal noise at 4% opacity. At that strength the
/// exact spectrum of feTurbulence is not distinguishable from hashed noise, and
/// a hash needs no allocation beyond the image itself.
pub fn grain(w: u32, h: u32) -> Arc<RenderImage> {
    let img = RgbaImage::from_fn(w.max(1), h.max(1), |x, y| {
        let mut n = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77);
        n ^= n >> 15;
        n = n.wrapping_mul(0xC2B2_AE3D);
        n ^= n >> 13;
        let v = (n & 0xff) as u8;
        Rgba([v, ((n >> 8) & 0xff) as u8, ((n >> 16) & 0xff) as u8, 10])
    });
    render_image(img)
}

/// The now-playing pane's wash: the cover blurred out to its own colours.
///
/// CSS blurs at 64px over a ~680px pane; downsampling to a few pixels and
/// letting the GPU's bilinear scaling stretch it back gives the same field of
/// colour for a fraction of the work, and it is done once per track.
pub fn ambient(cover: &image::DynamicImage) -> Arc<RenderImage> {
    let small = cover.resize_exact(24, 24, image::imageops::FilterType::Triangle);
    let blurred = image::imageops::blur(&small.to_rgba8(), 2.4);
    render_image(blurred)
}
