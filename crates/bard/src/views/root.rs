//! `App.tsx`: the title bar, the streamer banner, the three docks and the
//! layers over and under them.

use super::{icon, library, now_playing, overlays, queue, truncate};
use crate::backdrop;
use crate::model::Ui;
use crate::{fonts, theme};
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::sync::Arc;

/// Library dock width, `280px`.
pub const LIBRARY_W: f32 = 280.;
/// Queue dock width, `320px`.
pub const QUEUE_W: f32 = 320.;
/// Title bar height, `34px`.
pub const TITLEBAR_H: f32 = 34.;

pub struct BardWindow {
    pub ui: Ui,
    pub string_bounds: Option<Bounds<Pixels>>,
    pub volume_bounds: Option<Bounds<Pixels>>,
    pub album_scroll: ScrollHandle,
    pub tracks_scroll: UniformListScrollHandle,
    pub queue_scroll: ScrollHandle,
    /// Backdrop bitmaps, rebuilt when the window's device size changes.
    layers: Option<((u32, u32), Layers)>,
}

struct Layers {
    glow: Arc<RenderImage>,
    vignette: Arc<RenderImage>,
    grain: Arc<RenderImage>,
}

impl BardWindow {
    pub fn new(ui: Ui) -> Self {
        Self { ui, layers: None, string_bounds: None, volume_bounds: None, album_scroll: ScrollHandle::new(), tracks_scroll: UniformListScrollHandle::new(), queue_scroll: ScrollHandle::new() }
    }

    fn layers(&mut self, window: &Window) -> &Layers {
        let size = window.viewport_size();
        let scale = window.scale_factor();
        let key = (
            (f32::from(size.width) * scale).round() as u32,
            (f32::from(size.height) * scale).round() as u32,
        );
        if self.layers.as_ref().map(|(k, _)| *k) != Some(key) {
            let layers = Layers {
                glow: backdrop::glow(key.0, key.1),
                vignette: backdrop::vignette(key.0, key.1),
                grain: backdrop::grain(key.0, key.1),
            };
            self.layers = Some((key, layers));
        }
        &self.layers.as_ref().unwrap().1
    }
}

fn full(img: Arc<RenderImage>) -> Img {
    gpui::img(ImageSource::Render(img))
        .absolute()
        .inset_0()
        .size_full()
        .object_fit(ObjectFit::Fill)
}

impl Render for BardWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (glow, vignette, grain) = {
            let l = self.layers(window);
            (l.glow.clone(), l.vignette.clone(), l.grain.clone())
        };
        let viewport = window.viewport_size();
        let center_w = f32::from(viewport.width)
            - if self.ui.library_open { LIBRARY_W } else { 0. }
            - if self.ui.queue_open { QUEUE_W } else { 0. };

        div()
            .id("bard")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::ink())
            .text_color(theme::text())
            .font_family(fonts::UI)
            .text_size(theme::S3)
            .line_height(relative(1.5))
            // body::before -- the warm falloff from the centre-left.
            .child(full(glow))
            .child(self.titlebar(cx))
            .when(self.ui.armed(), |el| el.child(self.banner(cx)))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(self.ui.library_open, |el| {
                        el.child(library::pane(self, window, cx))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(now_playing::pane(self, center_w, window, cx)),
                    )
                    .when(self.ui.queue_open, |el| el.child(queue::pane(self, window, cx))),
            )
            // .depth-overlay -- above the panes, below Settings and the notice.
            .child(full(vignette))
            .child(full(grain))
            .children(overlays::notice(self, cx))
            .children(overlays::shortcuts(self, cx))
    }
}

impl BardWindow {
    fn titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = &self.ui;
        div()
            .relative()
            .h(px(TITLEBAR_H))
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .border_b_1()
            .border_color(theme::edge())
            .child(dock_toggle("dock-left", ui.library_open, cx.listener(|this, _, _, cx| {
                this.ui.library_open = !this.ui.library_open;
                cx.notify();
            })))
            .child(
                div()
                    .font_family(fonts::TRACK18)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(theme::S2)
                    .text_color(theme::muted())
                    .child("BARD"),
            )
            .child(div().ml_auto().child(dock_toggle(
                "dock-right",
                ui.queue_open,
                cx.listener(|this, _, _, cx| {
                    this.ui.queue_open = !this.ui.queue_open;
                    cx.notify();
                }),
            )))
    }

    /// `StreamerBanner.tsx`.
    fn banner(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let what = match self.ui.streamer_source {
            None => "Streamer mode",
            Some(bard_core::types::StreamerSource::Stream) => "Streaming",
            Some(bard_core::types::StreamerSource::Record) => "Recording",
            Some(bard_core::types::StreamerSource::Replay) => "Replay buffer running",
            Some(bard_core::types::StreamerSource::Virtualcam) => "Virtual camera on",
        };
        div()
            .relative()
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py(px(6.))
            .border_b_1()
            .border_color(theme::a(theme::green(), 0.4))
            .bg(theme::a(theme::green(), 0.15))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(theme::green())
                    .child(icon("shield", 14., theme::green()))
                    .child(super::section_label(what)),
            )
            .child(truncate(
                div()
                    .flex_1()
                    .text_size(theme::S1)
                    .text_color(theme::muted())
                    .child(
                        "Only albums marked safe for Content ID will play — untested and claimed \
                         albums are blocked, including ones you pick yourself.",
                    ),
            ))
            .child(
                div()
                    .id("streamer-off")
                    .flex_none()
                    .border_1()
                    .border_color(theme::a(theme::green(), 0.5))
                    .px_2()
                    .py(px(2.))
                    .text_size(theme::S1)
                    .text_color(theme::green())
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::a(theme::green(), 0.2)))
                    .child("Turn off")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.backend.update_config(Box::new(|c| c.streamer.enabled = false));
                        cx.notify();
                    })),
            )
    }
}

/// Chevron that folds the dock on its side of the window away.
fn dock_toggle(
    side: &'static str,
    open: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let name = format!("{side}-{}", if open { "open" } else { "closed" });
    div()
        .id(side)
        .rounded(px(2.))
        .px_1()
        .py(px(2.))
        .text_color(theme::muted())
        .cursor_pointer()
        .hover(|s| s.bg(theme::panel()).text_color(theme::text()))
        .child(icon(&name, 15., theme::muted()))
        .on_click(on_click)
}
