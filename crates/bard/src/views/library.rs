//! `LibraryPanel.tsx`, `AlbumGrid.tsx`, `ContentIdBar.tsx` and `AvatarMenu.tsx`
//! -- everything in the left dock.

use super::root::{BardWindow, LIBRARY_W};
use super::{icon, mono, truncate};
use crate::model::{Album, LibraryView, fmt_count, fmt_time};
use crate::{fonts, theme};
use bard_core::types::{ContentIdStatus, Track};
use gpui::prelude::FluentBuilder;
use gpui::*;

/// Fixed row height, so the virtual list's estimate is exact.
pub const ROW_H: f32 = 44.;
/// `::-webkit-scrollbar { width: 9px }` -- a classic scrollbar takes layout width.
pub const SCROLLBAR_W: f32 = 9.;

/// `.pane-depth`: light from the top, a settle to darker at the foot.
pub fn pane_depth() -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .child(
            div().absolute().top_0().left_0().right_0().h(relative(0.22)).bg(linear_gradient(
                180.,
                linear_color_stop(hsla(0., 0., 1., 0.03), 0.),
                linear_color_stop(hsla(0., 0., 1., 0.), 1.),
            )),
        )
        .child(
            div().absolute().bottom_0().left_0().right_0().h(relative(0.20)).bg(linear_gradient(
                0.,
                linear_color_stop(hsla(0., 0., 0., 0.35), 0.),
                linear_color_stop(hsla(0., 0., 0., 0.), 1.),
            )),
        )
}

pub fn pane(this: &mut BardWindow, window: &mut Window, cx: &mut Context<BardWindow>) -> impl IntoElement {
    let ui = &this.ui;
    let albums_view = ui.view == LibraryView::Albums && ui.open_album.is_none();

    let tab = |id: &'static str, label: &'static str, view: LibraryView, on: Rgba| {
        let active = ui.view == view && ui.open_album.is_none();
        div()
            .id(id)
            .cursor_pointer()
            .text_color(if active { on } else { theme::muted() })
            .when(!active, |el| el.hover(|s| s.text_color(theme::text())))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ui.view = view;
                this.ui.open_album = None;
                cx.notify();
            }))
    };

    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .px_4()
        .pt(px(13.))
        .pb(px(11.))
        .font_family(fonts::TRACK13)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(theme::S1)
        .text_color(theme::muted())
        .child(match &ui.open_album {
            Some(name) => truncate(
                div()
                    .id("back-to-albums")
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme::text()))
                    .child(format!("‹ {name}"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.open_album = None;
                        this.ui.selected = 0;
                        cx.notify();
                    })),
            )
            .into_any_element(),
            None => div().child("LIBRARY").into_any_element(),
        })
        .child(
            div()
                .flex()
                .flex_none()
                .gap_2()
                .child(tab("view-albums", "Albums", LibraryView::Albums, theme::cyan()))
                .child(tab("view-tracks", "Tracks", LibraryView::Tracks, theme::cyan()))
                .child(tab("view-loved", "Loved", LibraryView::Loved, theme::rose())),
        );

    let filter = div().px_3().pb(px(10.)).child(
        div()
            .w_full()
            .rounded(px(2.))
            .border_1()
            .border_color(theme::edge())
            .bg(theme::panel())
            .px(px(10.))
            .py(px(7.))
            .text_size(theme::S2)
            .child(this.filter.clone()),
    );

    let body = if albums_view {
        album_grid(this, window, cx).into_any_element()
    } else {
        track_list(this, cx).into_any_element()
    };

    let ui = &this.ui;
    let footer_left = if let (true, Some(p)) = (ui.scanning, &ui.scan_progress) {
        format!("Scanning {}/{}", p.scanned, p.total)
    } else if albums_view {
        let mut names: Vec<&str> = ui.tracks.iter().map(|t| t.album.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        format!("{} albums", names.len())
    } else if ui.view == LibraryView::Loved && ui.open_album.is_none() {
        format!("{} loved", fmt_count(ui.filtered_tracks().len()))
    } else {
        format!("{} tracks", fmt_count(ui.filtered_tracks().len()))
    };

    div()
        .relative()
        .w(px(LIBRARY_W))
        .flex_none()
        .flex()
        .flex_col()
        .min_h_0()
        .overflow_hidden()
        .border_r_1()
        .border_color(theme::edge())
        .child(pane_depth())
        .child(header)
        .children(this.ui.open_album.clone().map(|a| content_id_bar(this, &a, window, cx)))
        .child(filter)
        .child(body)
        .child(
            div()
                .flex()
                .justify_between()
                .border_t_1()
                .border_color(theme::edge())
                .px_4()
                .py(px(9.))
                .text_size(theme::S1)
                .text_color(theme::muted())
                .child(footer_left)
                .child("Enter to queue"),
        )
        .child(avatar_menu(this, cx))
}

fn album_grid(this: &mut BardWindow, _window: &mut Window, cx: &mut Context<BardWindow>) -> impl IntoElement {
    let albums = this.ui.filtered_albums();
    // grid-cols-2 gap-3 px-3, inside a scroller whose bar takes 9px.
    let col_w = (LIBRARY_W - SCROLLBAR_W - 24. - 12.) / 2.;
    let mut rows = Vec::new();
    for pair in albums.chunks(2) {
        let mut row = div().flex().gap_3();
        for album in pair {
            row = row.child(album_card(this, album, col_w, cx));
        }
        rows.push(row);
    }
    let empty = albums.is_empty();
    let handle = this.album_scroll.clone();
    div().relative().flex_1().min_h_0().flex().flex_col().child(div()
        .id("album-grid")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&handle)
        .child(
            div()
                .w(px(LIBRARY_W - SCROLLBAR_W))
                .flex()
                .flex_col()
                .gap_3()
                .px_3()
                .pb_3()
                .children(rows)
                .when(empty, |el| {
                    el.child(
                        div()
                            .px_1()
                            .py_3()
                            .text_size(theme::S2)
                            .text_color(theme::muted())
                            .child("No albums match that."),
                    )
                }),
        ))
        .child(super::scrollbar(handle))
}

fn album_card(this: &mut BardWindow, album: &Album, w: f32, cx: &mut Context<BardWindow>) -> impl IntoElement {
    let status = this.ui.album_status(&album.name);
    let art = album.art_track_id.as_deref().and_then(|id| this.ui.covers.cover(id));
    let name = album.name.clone();
    let group = SharedString::from(format!("album-{}", album.name));
    let ids: Vec<String> = album.tracks.iter().map(|t| t.id.clone()).collect();
    let ids_play = ids.clone();

    let cover = match art {
        Some(img) => gpui::img(ImageSource::Render(img))
            .size(px(w))
            .object_fit(ObjectFit::Cover)
            .when(status == ContentIdStatus::Claimed, |el| el.opacity(0.45))
            .into_any_element(),
        None => div().size(px(w)).bg(theme::panel()).into_any_element(),
    };

    let badge = match status {
        ContentIdStatus::Untested => None,
        ContentIdStatus::Safe => Some(("✓ safe", theme::green())),
        ContentIdStatus::Claimed => Some(("⚠ claimed", theme::rose())),
    };

    div()
        .w(px(w))
        .flex_none()
        .group(group.clone())
        .child(
            div()
                .id(SharedString::from(format!("album-open-{}", album.name)))
                .relative()
                .size(px(w))
                .border_1()
                .border_color(theme::edge())
                .overflow_hidden()
                .cursor_pointer()
                .child(cover)
                .children(badge.map(|(label, color)| {
                    div()
                        .absolute()
                        .top(px(6.))
                        .left(px(6.))
                        .bg(theme::a(theme::ink(), 0.85))
                        .px(px(6.))
                        .py(px(2.))
                        .text_size(theme::S1)
                        .text_color(color)
                        .child(label)
                }))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_end()
                        .justify_end()
                        .gap_1()
                        .p(px(6.))
                        .invisible()
                        .group_hover(group, |s| s.visible())
                        .child(
                            div()
                                .id(SharedString::from(format!("album-queue-{}", album.name)))
                                .bg(theme::a(theme::ink(), 0.85))
                                .px(px(6.))
                                .py(px(2.))
                                .text_size(theme::S1)
                                .hover(|s| s.text_color(theme::cyan()))
                                .child("+ queue")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.ui.backend.enqueue_many(&ids, false);
                                })),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("album-play-{}", album.name)))
                                .bg(theme::cyan_deep())
                                .px(px(6.))
                                .py(px(2.))
                                .text_size(theme::S1)
                                .child("play")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.ui.backend.enqueue_many(&ids_play, true);
                                })),
                        ),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ui.open_album = Some(name.clone());
                    this.ui.selected = 0;
                    cx.notify();
                })),
        )
        .child(truncate(div().mt(px(6.)).text_size(theme::S2).child(album.name.clone())))
        .child(truncate(
            mono(theme::S1)
                .text_color(theme::muted())
                .child(format!("{} · {}", album.tracks.len(), fmt_time(album.duration))),
        ))
}

fn track_list(this: &mut BardWindow, cx: &mut Context<BardWindow>) -> impl IntoElement {
    let rows = this.ui.filtered_tracks();
    let count = rows.len();
    let selected = this.ui.selected;
    let stats = this.ui.stats.clone();
    let loved = this.ui.loved.clone();
    let handle = this.tracks_scroll.clone();
    // Keep the selection in view as it moves, and when the rows change.
    if count > 0 && this.scrolled_to != Some((selected, count)) {
        handle.scroll_to_item(selected.min(count - 1), ScrollStrategy::Nearest);
    }
    this.scrolled_to = Some((selected, count));
    let base = handle.0.borrow().base_handle.clone();
    div()
        .id("library-list")
        .track_focus(&this.list_focus)
        .relative()
        .flex_1()
        .min_h_0()
        .flex()
        .child(super::scrollbar(base))
        .child(
            uniform_list("library-tracks", count, cx.processor(move |_this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|i| track_row(&rows[i], i, i == selected, &stats, &loved, cx))
                    .collect::<Vec<_>>()
            }))
            .track_scroll(&handle)
            .flex_1()
            .px(px(6.))
            .pr(px(6. + SCROLLBAR_W)),
        )
}

fn track_row(
    t: &Track,
    index: usize,
    selected: bool,
    stats: &bard_core::types::StatsMap,
    loved: &[String],
    cx: &mut Context<BardWindow>,
) -> Stateful<Div> {
    let plays = stats.get(&t.id).map(|s| s.plays).unwrap_or(0);
    let is_loved = loved.contains(&t.id);
    let id = t.id.clone();
    let love_id = t.id.clone();
    div()
        .id(SharedString::from(format!("track-{}", t.id)))
        .h(px(ROW_H))
        .w_full()
        .relative()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .rounded(px(2.))
        .cursor_pointer()
        .hover(|s| s.bg(theme::panel()))
        .when(selected, |el| {
            el.bg(theme::panel())
                .child(div().absolute().left_0().top_0().bottom_0().w(px(2.)).bg(theme::cyan()))
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(truncate(div().text_size(theme::S2).child(t.title.clone())))
                .child(truncate(
                    div().mt(px(1.)).text_size(theme::S1).text_color(theme::muted()).child(t.artist.clone()),
                )),
        )
        .child(
            mono(theme::S1)
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .text_color(theme::muted())
                .child(love_button(&love_id, is_loved, 13., cx))
                .when(plays > 0, |el| el.child(div().text_color(theme::cyan()).child(format!("{plays}×"))))
                .child(fmt_time(t.duration_sec)),
        )
        .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
            this.ui.selected = index;
            if ev.click_count() >= 2 {
                this.ui.backend.enqueue(&id);
            }
            cx.notify();
        }))
}

/// `LoveButton.tsx`.
pub fn love_button(track_id: &str, on: bool, size: f32, cx: &mut Context<BardWindow>) -> Stateful<Div> {
    let id = track_id.to_string();
    div()
        .id(SharedString::from(format!("love-{track_id}-{size}")))
        .flex_none()
        .cursor_pointer()
        .child(icon(
            if on { "heart-filled" } else { "heart" },
            size,
            if on { theme::rose() } else { theme::muted() },
        ))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.ui.backend.toggle_loved(&id);
        }))
}

/// `ContentIdBar.tsx`.
fn content_id_bar(this: &mut BardWindow, album: &str, window: &Window, cx: &mut Context<BardWindow>) -> Div {
    let flag = this.ui.album_flags.get(album).cloned();
    let status = flag.as_ref().map(|f| f.status).unwrap_or_default();
    let choices = [
        (ContentIdStatus::Untested, "Untested"),
        (ContentIdStatus::Safe, "Safe"),
        (ContentIdStatus::Claimed, "Claimed"),
    ];
    let mut row = div().flex().gap_1();
    for (value, label) in choices {
        let active = status == value;
        let album = album.to_string();
        let note = flag.as_ref().and_then(|f| f.note.clone());
        let (border, bg, fg) = match (active, value) {
            (true, ContentIdStatus::Claimed) => (theme::rose().into(), Some(theme::a(theme::rose(), 0.2)), theme::rose()),
            (true, ContentIdStatus::Safe) => (theme::green().into(), Some(theme::a(theme::green(), 0.2)), theme::green()),
            (true, _) => (Hsla::from(theme::edge()), Some(theme::panel().into()), theme::text()),
            (false, _) => (Hsla::from(theme::edge()), None, theme::muted()),
        };
        row = row.child(
            div()
                .id(SharedString::from(format!("cid-{label}")))
                .flex_1()
                .flex()
                .justify_center()
                .border_1()
                .border_color(border)
                .when_some(bg, |el, bg| el.bg(bg))
                .px_2()
                .py_1()
                .text_size(theme::S1)
                .text_color(fg)
                .cursor_pointer()
                .child(label)
                .on_click(cx.listener(move |this, _, _, _| {
                    this.ui.backend.set_album_flag(&album, value, note.clone());
                })),
        );
    }
    // The field is shared between albums; load the note when the album changes.
    if this.note_album.as_deref() != Some(album) {
        let note = flag.as_ref().and_then(|f| f.note.clone()).unwrap_or_default();
        this.note.update(cx, |i, cx| i.set_text(&note, cx));
        this.note_album = Some(album.to_string());
    }
    let focused = this.note.focus_handle(cx).is_focused(window);
    div()
        .border_b_1()
        .border_color(theme::edge())
        .px_3()
        .py_2()
        .child(
            div()
                .mb(px(6.))
                .font_family(fonts::TRACK13)
                .text_size(theme::S1)
                .text_color(theme::muted())
                .child("CONTENT ID"),
        )
        .child(row)
        .child(
            div()
                .mt(px(6.))
                .w_full()
                .border_1()
                .border_color(theme::edge())
                .bg(theme::panel())
                .px_2()
                .py_1()
                .when(focused, |el| el.border_color(theme::cyan()))
                .text_size(theme::S1)
                .child(this.note.clone()),
        )
        .children(flag.and_then(|f| f.checked_at).map(|at| {
            mono(theme::S1)
                .mt_1()
                .text_color(theme::muted())
                .child(format!("checked {}", local_date(at)))
        }))
}

/// `new Date(ms).toLocaleDateString()` in en-US: 8/9/2026.
fn local_date(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    // Civil-from-days (Howard Hinnant), UTC. Close enough for a "checked" date.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{}/{}/{}", m, d, if m <= 2 { y + 1 } else { y })
}

/// `AvatarMenu.tsx`: the account-style menu pinned to the foot of the dock.
fn avatar_menu(this: &mut BardWindow, cx: &mut Context<BardWindow>) -> impl IntoElement {
    let ui = &this.ui;
    let open = ui.menu_open;
    let count = fmt_count(ui.tracks.len());

    let item = |id: &'static str, label: &'static str, hint: String, cx: &mut Context<BardWindow>, f: fn(&mut BardWindow, &mut Context<BardWindow>)| {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .py(px(6.))
            .text_size(theme::S2)
            .cursor_pointer()
            .hover(|s| s.bg(theme::edge()))
            .child(label)
            .child(mono(theme::S1).text_color(theme::muted()).child(hint))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ui.menu_open = false;
                f(this, cx);
                cx.notify();
            }))
    };

    let menu = open.then(|| {
        div()
            .absolute()
            .bottom_full()
            .left_2()
            .mb_2()
            .w(px(240.))
            .border_1()
            .border_color(theme::edge())
            .bg(theme::panel())
            .py_1()
            .shadow_2xl()
            .occlude()
            .child(super::record_bounds(cx.entity(), |this, b| this.menu_bounds = Some(b)))
            .child(item("menu-settings", "Settings", "Ctrl ,".into(), cx, |w, cx| w.open_settings(cx)))
            .child(item("menu-rescan", "Rescan library", format!("{count} tracks"), cx, |w, _| w.ui.backend.rescan()))
            .child(item("menu-shortcuts", "Keyboard shortcuts", "?".into(), cx, |w, _| w.ui.shortcuts_open = true))
            .child(
                div()
                    .mt_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .border_t_1()
                    .border_color(theme::edge())
                    .px_3()
                    .pt(px(6.))
                    .pb(px(2.))
                    .child(div().text_size(theme::S1).text_color(theme::muted()).child("Version"))
                    .child(
                        mono(theme::S1)
                            .flex()
                            .text_color(theme::muted())
                            .child(format!("v{}", this.ui.version))
                            .child(div().ml(px(6.)).opacity(0.7).child(this.ui.commit.clone())),
                    ),
            )
    });

    div()
        .id("avatar-menu")
        .relative()
        .border_t_1()
        .border_color(theme::edge())
        // A click anywhere else closes the menu, as the document listener did.
        .when(open, |el| {
            el.on_mouse_down_out(cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                if !this.menu_bounds.is_some_and(|b| b.contains(&ev.position)) {
                    this.ui.menu_open = false;
                    cx.notify();
                }
            }))
        })
        .children(menu)
        .child(
            div()
                .id("avatar")
                .flex()
                .w_full()
                .items_center()
                .gap(px(10.))
                .px_3()
                .py(px(10.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::panel()))
                .when(open, |el| el.bg(theme::panel()))
                .child(
                    div()
                        .flex_none()
                        .size(px(28.))
                        .rounded_full()
                        .border_1()
                        .border_color(theme::edge())
                        .overflow_hidden()
                        .child(svg_image("icons/avatar.svg", 28.)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(truncate(div().text_size(theme::S2).child("Bard")))
                        .child(truncate(
                            div().text_size(theme::S1).text_color(theme::muted()).child(format!("{count} tracks")),
                        )),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(theme::S1)
                        .text_color(theme::muted())
                        .child(if open { "▾" } else { "▴" }),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ui.menu_open = !this.ui.menu_open;
                    cx.notify();
                })),
        )
}

/// A full-colour SVG, drawn as an image rather than a tinted mask.
pub fn svg_image(path: &'static str, size: f32) -> Img {
    let bytes = crate::assets::bytes(path).unwrap_or_default().to_vec();
    gpui::img(ImageSource::Image(std::sync::Arc::new(gpui::Image::from_bytes(ImageFormat::Svg, bytes))))
        .size(px(size))
}
