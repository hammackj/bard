//! `QueuePanel.tsx` and the overlay URL strip -- the right dock.

use super::library::{SCROLLBAR_W, pane_depth};
use super::root::{BardWindow, QUEUE_W};
use super::{mono, truncate};
use crate::model::fmt_time;
use crate::{fonts, theme};
use gpui::prelude::FluentBuilder;
use gpui::*;

pub fn pane(this: &mut BardWindow, window: &mut Window, cx: &mut Context<BardWindow>) -> impl IntoElement {
    let ui = &this.ui;
    let n = ui.queue.len();
    let max_h = f32::from(window.viewport_size().height) * 0.38;

    let action = |id: &'static str, label: &'static str, enabled: bool, danger: bool| {
        div()
            .id(id)
            .font_family(fonts::UI)
            .when(!enabled, |el| el.opacity(0.4))
            .when(enabled, |el| {
                el.cursor_pointer().hover(move |s| s.text_color(if danger { theme::rose() } else { theme::text() }))
            })
            .child(label)
    };

    let header = div()
        .flex()
        .items_center()
        .gap_3()
        .px_4()
        .pt(px(13.))
        .pb(px(11.))
        .font_family(fonts::TRACK13)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(theme::S1)
        .text_color(theme::muted())
        .child("QUEUE")
        .child(mono(theme::S1).text_color(theme::text()).child(n.to_string()))
        .child(
            div()
                .ml_auto()
                .flex()
                .items_center()
                .gap_3()
                .child(action("q-shuffle", "Shuffle", n >= 2, false).on_click(cx.listener(|this, _, _, _| {
                    this.ui.backend.shuffle_queue()
                })))
                .child(action("q-fill", "Fill", true, false).on_click(cx.listener(|this, _, _, _| {
                    this.ui.backend.fill_queue()
                })))
                .child(action("q-clear", "Clear", n > 0, true).on_click(cx.listener(|this, _, _, _| {
                    this.ui.backend.clear_queue()
                }))),
        );

    let rows = ui.queue.iter().enumerate().map(|(i, item)| {
        let group = SharedString::from(format!("q-{}", item.id));
        let id = item.id.clone();
        div()
            .id(SharedString::from(format!("qrow-{}", item.id)))
            .group(group.clone())
            .flex()
            .items_center()
            .gap_3()
            .rounded(px(2.))
            .px_2()
            .py(px(7.))
            .hover(|s| s.bg(theme::panel()))
            .child(mono(theme::S1).w(px(22.)).flex_none().text_color(theme::muted()).cursor_grab().child((i + 1).to_string()))
            .child(truncate(div().flex_1().text_size(theme::S2).child(item.track.title.clone())))
            .child(mono(theme::S1).flex_none().text_color(theme::muted()).child(fmt_time(item.track.duration_sec)))
            .child(
                div()
                    .id(SharedString::from(format!("qremove-{}", item.id)))
                    .flex_none()
                    .invisible()
                    .group_hover(group, |s| s.visible())
                    .text_size(theme::S1)
                    .text_color(theme::muted())
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme::rose()))
                    .child("✕")
                    .on_click(cx.listener(move |this, _, _, _| this.ui.backend.remove_from_queue(&id))),
            )
    });

    let handle = this.queue_scroll.clone();
    let list = div()
        .id("queue-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&handle)
        .px_2()
        .pr(px(8. + SCROLLBAR_W))
        .pb_2()
        .map(|el| {
            if n == 0 {
                el.child(
                    div()
                        .px_2()
                        .py_3()
                        .text_size(theme::S2)
                        .text_color(theme::muted())
                        .child("Nothing queued. Pick a track, or hit Fill."),
                )
            } else {
                el.children(rows)
            }
        });

    let overlay = this
        .ui
        .overlay_url
        .as_deref()
        .map(|u| u.trim_start_matches("http://").to_string())
        .unwrap_or_else(|| "off".into());

    div()
        .relative()
        .w(px(QUEUE_W))
        .flex_none()
        .flex()
        .flex_col()
        .min_h_0()
        .overflow_hidden()
        .border_l_1()
        .border_color(theme::edge())
        .child(pane_depth())
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .max_h(px(max_h))
                .border_t_1()
                .border_color(theme::edge())
                .child(header)
                .child(div().relative().flex_1().min_h_0().flex().flex_col().child(list).child(super::scrollbar(handle))),
        )
        .child(
            div()
                .relative()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .border_t_1()
                .border_color(theme::edge())
                .px_4()
                .py_2()
                .text_size(theme::S1)
                .child(div().text_color(theme::muted()).child("Overlay"))
                .child(truncate(mono(theme::S1).child(overlay))),
        )
}
