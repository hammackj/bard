//! The layers over the window: the notice strip, the shortcut list, and the
//! first-run invitation that replaces the player when there is no library.

use super::mono;
use super::root::BardWindow;
use crate::{fonts, theme};
use gpui::*;

const KEYS: &[(&str, &str)] = &[
    ("Space", "Play / pause"),
    ("← / →", "Seek 5 seconds"),
    ("↑ / ↓", "Volume"),
    ("/", "Focus the library filter"),
    ("Enter", "Queue the selected track"),
    ("Shift + Enter", "Play the selected track now"),
    ("Ctrl + ,", "Settings"),
    ("Esc", "Close / clear filter"),
    ("?", "This list"),
];

/// Transient, dismissible strip for things worth saying once.
pub fn notice(this: &BardWindow, cx: &mut Context<BardWindow>) -> Option<impl IntoElement> {
    let text = this.ui.notice.clone()?;
    Some(
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .border_t_1()
            .border_color(theme::cyan_deep())
            .bg(theme::panel())
            .px_4()
            .py_2()
            .text_size(theme::S2)
            .occlude()
            .child(super::truncate(div().child(text)))
            .child(
                div()
                    .id("dismiss-notice")
                    .flex_none()
                    .text_size(theme::S1)
                    .text_color(theme::muted())
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme::text()))
                    .child("Dismiss")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.notice = None;
                        cx.notify();
                    })),
            ),
    )
}

/// `ShortcutsOverlay.tsx`.
pub fn shortcuts(this: &BardWindow, cx: &mut Context<BardWindow>) -> Option<impl IntoElement> {
    if !this.ui.shortcuts_open {
        return None;
    }
    let mut keys = div().flex().flex_col();
    let mut what = div().flex().flex_col();
    for (k, w) in KEYS {
        keys = keys.child(mono(theme::S2).child(*k));
        what = what.child(div().text_size(theme::S2).text_color(theme::muted()).child(*w));
    }
    Some(
        div()
            .id("shortcuts")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::a(theme::ink(), 0.85))
            .occlude()
            .on_click(cx.listener(|this, _, _, cx| {
                this.ui.shortcuts_open = false;
                cx.notify();
            }))
            .child(
                div()
                    .id("shortcuts-card")
                    .border_1()
                    .border_color(theme::edge())
                    .bg(theme::panel())
                    .px_8()
                    .py(px(28.))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .mb_4()
                            .font_family(fonts::TRACK13)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(theme::S1)
                            .text_color(theme::cyan())
                            .child("SHORTCUTS"),
                    )
                    .child(div().flex().gap_8().child(keys.gap_2()).child(what.gap_2())),
            ),
    )
}

/// `FirstRun.tsx`: an invitation, not an error.
pub fn first_run(cx: &mut Context<BardWindow>) -> impl IntoElement {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_6()
        .px_10()
        .child(
            div()
                .font_family(fonts::DISPLAY)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(theme::S5)
                .line_height(relative(1.25))
                .child("Nothing to play yet"),
        )
        .child(div().max_w(px(448.)).text_center().text_size(theme::S2).text_color(theme::muted()).child(
            "Bard only plays files you point it at on this machine. Nothing is \
                     downloaded, streamed, or sent anywhere.",
        ))
        .child(
            div()
                .id("choose-folder")
                .bg(theme::cyan_deep())
                .px_4()
                .py_2()
                .text_size(theme::S2)
                .cursor_pointer()
                .child("Choose a music folder")
                .on_click(cx.listener(|this, _, _, _| this.ui.backend.choose_library_folder())),
        )
}
