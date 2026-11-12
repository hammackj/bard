//! `StringBar.tsx`: the progress bar is a plucked string.
//!
//! At rest it is a straight cyan hairline; during playback its points are
//! displaced by the live analyser amplitude, so it vibrates in time with the
//! music. On pause the displacement damps to flat rather than snapping.
//!
//! The geometry is computed in render (it needs the analyser and the clock)
//! and handed to a canvas to paint. A frame is requested only while there is
//! motion to draw -- playing, or still damping out -- so a paused player costs
//! nothing, which matters for an app left open for an eight-hour stream.

use crate::theme;
use bard_core::progress::{Sample, TAU_MS, ease, predict};
use gpui::*;
use std::time::Instant;

const H: f32 = 44.;
const MID: f32 = H / 2.;
const SEGMENTS: usize = 90;
/// Displacement in pixels at full deflection.
const AMPLITUDE: f32 = 8.5;

/// Reads the analyser's time-domain bytes (128 = silence). None in the
/// snapshot tool, where nothing is playing.
pub type Analyser = std::rc::Rc<dyn Fn(&mut [u8])>;

pub struct StringState {
    amp: f32,
    t: f32,
    /// The drawn position, which trails the reported one smoothly.
    shown: f64,
    last: Instant,
    buf: Vec<u8>,
    pub analyser: Option<Analyser>,
}

impl Default for StringState {
    fn default() -> Self {
        Self { amp: 0., t: 0., shown: 0., last: Instant::now(), buf: vec![128; 2048], analyser: None }
    }
}

pub struct Frame {
    points: Vec<Point<f32>>,
    /// 0..1 along the string.
    progress: f32,
}

impl StringState {
    /// Advances one frame. Returns the geometry, and whether another frame is wanted.
    pub fn step(&mut self, sample: Sample, epoch: Instant) -> (Frame, bool) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64() * 1000.;
        self.last = now;
        let now_ms = now.duration_since(epoch).as_secs_f64() * 1000.;
        self.shown = ease(self.shown, predict(&sample, now_ms), dt, TAU_MS);

        let mut target = 0.;
        if sample.playing
            && let Some(read) = &self.analyser
        {
            read(&mut self.buf);
            // Peak deviation from the 128 midpoint, normalised to 0..1.
            let peak = self.buf.iter().step_by(4).map(|b| (*b as i32 - 128).abs()).max().unwrap_or(0);
            target = (peak as f32 / 90.).min(1.) * AMPLITUDE;
        }
        // Damps toward the target instead of snapping. The rates are per
        // frame in the original, which ran at the display's 60Hz.
        let frames = (dt / (1000. / 60.)).clamp(0., 4.) as f32;
        self.amp += (target - self.amp) * (1. - (1f32 - 0.045).powf(frames));
        self.t += 0.06 * frames;

        let (amp, t) = (self.amp, self.t);
        let points = (0..=SEGMENTS)
            .map(|i| {
                let x = i as f32 / SEGMENTS as f32;
                // Pinned at both ends, like a real string.
                let env = (std::f32::consts::PI * x).sin();
                let y = MID
                    - env
                        * amp
                        * ((x * 13. + t * 3.1).sin() * 0.62
                            + (x * 29. - t * 4.7).sin() * 0.26
                            + (x * 47. + t * 2.3).sin() * 0.12)
                        * (1. + 0.35 * (t * 1.7).sin());
                point(x, y)
            })
            .collect();
        let progress = if sample.duration > 0. { (self.shown / sample.duration).clamp(0., 1.) as f32 } else { 0. };
        let animating =
            sample.playing || self.amp.abs() > 0.01 || (self.shown - predict(&sample, now_ms)).abs() > 0.001;
        (Frame { points, progress }, animating)
    }
}

fn stroke(points: &[Point<Pixels>], width: f32) -> Option<Path<Pixels>> {
    let mut b = PathBuilder::stroke(px(width));
    b.move_to(points[0]);
    for p in &points[1..] {
        b.line_to(*p);
    }
    b.build().ok()
}

/// An ellipse, near enough: a quad rounded by its narrower half-axis.
fn dot(window: &mut Window, c: Point<Pixels>, rx: f32, ry: f32, color: Hsla) {
    window.paint_quad(
        fill(Bounds::new(point(c.x - px(rx), c.y - px(ry)), size(px(rx * 2.), px(ry * 2.))), color)
            .corner_radii(px(rx.min(ry))),
    );
}

/// The string itself, `h-11` (44px) by the full width.
pub fn element(frame: Frame) -> impl IntoElement {
    canvas(
        move |_, _, _| frame,
        |bounds, frame, window, _| {
            let w = f32::from(bounds.size.width);
            let pts: Vec<Point<Pixels>> =
                frame.points.iter().map(|p| point(bounds.origin.x + px(p.x * w), bounds.origin.y + px(p.y))).collect();

            // Ahead of the playhead: edge-coloured, 1.25px.
            if let Some(path) = stroke(&pts, 1.25) {
                window.paint_path(path, theme::edge());
            }

            // Played: cyan, 1.5px, with drop-shadow(0 0 4px rgba(34,211,238,.45)).
            let played = Bounds::new(
                bounds.origin - point(px(0.), px(8.)),
                size(px(frame.progress * w), bounds.size.height + px(16.)),
            );
            window.with_content_mask(Some(ContentMask { bounds: played }), |window| {
                for (width, alpha) in [(6.0, 0.035), (3.5, 0.07)] {
                    if let Some(path) = stroke(&pts, width) {
                        window.paint_path(path, theme::a(theme::cyan(), alpha));
                    }
                }
                if let Some(path) = stroke(&pts, 1.5) {
                    window.paint_path(path, theme::cyan());
                }
            });

            // The playhead: r=3.5 with drop-shadow(0 0 5px rgba(34,211,238,.6)).
            //
            // The SVG is a 1000-unit viewBox stretched with
            // preserveAspectRatio="none", so the circle is squeezed by the
            // bar's width / 1000 horizontally while staying 3.5 tall -- on a
            // 600px bar it is an upright oval. Strokes are non-scaling, so the
            // string itself is unaffected.
            let i = ((frame.progress * SEGMENTS as f32).round() as usize).min(SEGMENTS);
            let head = pts[i];
            let rx = 3.5 * w / 1000.;
            for (grow, alpha) in [(3.0, 0.07), (1.5, 0.14)] {
                dot(window, head, rx + grow, 3.5 + grow, theme::a(theme::cyan(), alpha));
            }
            dot(window, head, rx, 3.5, theme::cyan().into());
        },
    )
    .w_full()
    .h(px(H))
}
