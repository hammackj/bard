//! The tray's now-playing preview (`trayPopup.ts`).
//!
//! The Electron build loaded the OBS overlay page into a small frameless
//! window, so the preview and the on-stream card could not drift apart. Here
//! the card is drawn natively from the same `NowPlaying` snapshot the overlay
//! server streams, following `resources/overlay/overlay.css`. That page names
//! fonts it never ships ("Public Sans", "Fraunces", "JetBrains Mono"), so
//! Chromium fell through to its defaults: system-ui (Noto Sans), its default
//! serif "Times New Roman" (Liberation Serif, the metric clone -- Chromium
//! will not take fontconfig's non-metric Georgia substitute) and monospace
//! (Noto Sans Mono). The same families are named here.

use crate::model::{Covers, fmt_time};
use bard_core::types::NowPlaying;
use gpui::prelude::FluentBuilder;
use gpui::*;

pub const WIDTH: f32 = 470.;
pub const HEIGHT: f32 = 208.;
/// `.card { width: 460px }`.
const CARD_W: f32 = 460.;

const SANS: &str = "Noto Sans";
const SERIF: &str = "Liberation Serif";
const MONO: &str = "Noto Sans Mono";

// overlay.css's own palette (slate), not the app's.
fn panel() -> Hsla {
    rgba(0x0f172aeb).into() // rgba(15, 23, 42, 0.92)
}
fn edge() -> Rgba {
    rgb(0x1e293b)
}
fn text() -> Rgba {
    rgb(0xf1f5f9)
}
fn muted() -> Rgba {
    rgb(0x94a3b8)
}
fn cyan() -> Rgba {
    rgb(0x22d3ee)
}
fn cyan_deep() -> Rgba {
    rgb(0x0891b2)
}

pub struct Popup {
    pub now: Option<NowPlaying>,
    /// The track on air, for its cover.
    pub track_id: Option<String>,
    covers: Covers,
}

impl Popup {
    pub fn new(artwork_dir: std::path::PathBuf) -> Self {
        Self { now: None, track_id: None, covers: Covers::new(artwork_dir) }
    }
}

/// `letter-spacing: .16em` on the eyebrow. GPUI has no tracking, so each
/// letter is its own box with the spacing as the gap. Each box is laid out to
/// whole pixels, which adds about half a pixel per letter, so that comes off
/// the gap (measured against Chromium's render of the overlay page).
fn tracked(text: &str, size: f32, em: f32) -> Div {
    div()
        .flex()
        .gap(px((size * em - 0.5).max(0.)))
        .children(text.chars().map(|c| div().child(if c == ' ' { "\u{a0}".to_string() } else { c.to_string() })))
}

impl Render for Popup {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let root = div().size_full().font_family(SANS).text_color(text());
        // Hidden until there is something to show, as the overlay is.
        let Some(np) = self.now.clone().filter(|n| !n.title.is_empty()) else {
            return root;
        };
        let cover = self.track_id.clone().and_then(|id| self.covers.cover(&id));
        let pct = if np.duration_sec > 0. { (np.position_sec / np.duration_sec).clamp(0., 1.) as f32 } else { 0. };
        let album = [np.album.clone(), if np.queue_length > 0 { format!("{} queued", np.queue_length) } else { String::new() }]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let inner_w = CARD_W - 18. * 2. - 1. - 2.;


        // `border-left: 2px solid cyan-deep` over a 1px edge elsewhere. GPUI
        // borders take one colour, so the left edge is its own strip; it runs
        // the full height, as the CSS left border covers the corners.
        root.child(
            div().flex().w(px(CARD_W)).child(div().w(px(2.)).flex_none().bg(cyan_deep())).child(
            div()
                .flex_1()
                .pt(px(16.))
                .px(px(18.))
                .pb(px(20.))
                .bg(panel())
                .border_t_1()
                .border_r_1()
                .border_b_1()
                .border_color(edge())
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .gap(px(16.))
                        .child(match cover {
                            Some(img) => gpui::img(ImageSource::Render(img))
                                .size(px(84.))
                                .flex_none()
                                .object_fit(ObjectFit::Cover)
                                .border_1()
                                .border_color(edge())
                                .bg(rgb(0x020617))
                                .into_any_element(),
                            None => div().into_any_element(),
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    tracked("NOW PLAYING", 10., 0.16)
                                        .mb(px(6.))
                                        .text_size(px(10.))
                                        .line_height(relative(1.362))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(cyan()),
                                )
                                .child(
                                    div()
                                        .font_family(SERIF)
                                        .text_size(px(22.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .line_height(relative(1.12))
                                        .line_clamp(2)
                                        .overflow_hidden()
                                        .child(np.title.clone()),
                                )
                                .child(div().mt(px(4.)).text_size(px(14.)).line_height(relative(1.362)).child(np.artist.clone()))
                                .when(!album.is_empty(), |el| {
                                    el.child(div().mt(px(3.)).text_size(px(11.)).line_height(relative(1.362)).text_color(muted()).child(album))
                                }),
                        ),
                )
                .child(
                    // .bar, with the fill's glow drawn as a soft wider strip.
                    div()
                        .mt(px(14.))
                        .w(px(inner_w))
                        .h(px(2.))
                        .bg(edge())
                        .relative()
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .h_full()
                                .w(px(inner_w * pct))
                                .bg(if np.is_playing { cyan() } else { muted() })
                                .when(np.is_playing, |el| {
                                    el.shadow(vec![BoxShadow {
                                        color: hsla(188. / 360., 0.86, 0.53, 0.55),
                                        offset: point(px(0.), px(0.)),
                                        blur_radius: px(6.),
                                        spread_radius: px(0.),
                                        inset: false,
                                    }])
                                }),
                        ),
                )
                .child(
                    div()
                        .mt(px(6.))
                        .flex()
                        .justify_between()
                        .font_family(MONO)
                        .text_size(px(10.))
                        // Chromium's line box for this row measures 2px taller
                        // than Noto Sans Mono's own metrics give.
                        .line_height(px(15.6))
                        .text_color(muted())
                        .child(fmt_time(np.position_sec))
                        .child(fmt_time(np.duration_sec)),
                ),
            ),
        )
    }
}
