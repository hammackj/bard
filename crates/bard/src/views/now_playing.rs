//! `NowPlaying.tsx` and `SleepTimer.tsx` -- the centre of the window.

use super::library::{love_button, svg_image};
use super::root::BardWindow;
use super::{icon, mono, string_bar, truncate};
use crate::model::fmt_time;
use crate::{fonts, theme};
use bard_core::progress::Sample;
use bard_core::types::{AutoFillMode, RepeatMode};
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::time::{Duration, Instant};

/// `@2xl` -- the container query that widens the padding and the volume slider.
const WIDE: f32 = 672.;
const SLEEP_PRESETS: [u64; 5] = [15, 30, 45, 60, 90];

pub fn pane(this: &mut BardWindow, width: f32, window: &mut Window, cx: &mut Context<BardWindow>) -> impl IntoElement {
    if this.ui.config.library_paths.is_empty() {
        return super::overlays::first_run(cx).into_any_element();
    }
    let wide = width >= WIDE;
    let pad_x = if wide { 40. } else { 20. };
    let viewport_h = f32::from(window.viewport_size().height);
    // w-[min(336px,58%,45vh)], where 58% is of the row inside the padding.
    let cover_w = 336f32.min((width - pad_x * 2.) * 0.58).min(viewport_h * 0.45);

    let ui = &mut this.ui;
    let track = ui.track.clone();
    let cover = track
        .as_ref()
        .filter(|t| t.has_artwork)
        .and_then(|t| ui.covers.cover(&t.id));
    let ambient = cover.as_ref().and(track.as_ref()).and_then(|t| ui.covers.ambient(&t.id));

    let sample = Sample {
        position: ui.position,
        duration: ui.duration,
        playing: ui.playing,
        at: ui.position_at.duration_since(ui.epoch).as_secs_f64() * 1000.,
    };
    let (frame, animating) = ui.string.step(sample, ui.epoch);
    if animating {
        window.request_animation_frame();
    }

    let status = if ui.playing {
        "NOW PLAYING"
    } else if track.is_some() {
        "PAUSED"
    } else {
        "NOTHING PLAYING"
    };

    let art = match cover {
        Some(img) => div()
            .flex_none()
            .size(px(cover_w))
            .border_1()
            .border_color(theme::edge())
            .overflow_hidden()
            .child(gpui::img(ImageSource::Render(img)).size_full().object_fit(ObjectFit::Cover))
            .into_any_element(),
        None => div()
            .flex_none()
            .size(px(cover_w))
            .border_1()
            .border_color(theme::edge())
            .child(svg_image("icons/placeholder.svg", cover_w - 2.))
            .into_any_element(),
    };

    let loved = track.as_ref().is_some_and(|t| ui.is_loved(&t.id));
    let heading = div()
        .min_w_0()
        .pb_1()
        .child(
            div()
                .mb(px(9.))
                .font_family(fonts::TRACK15)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(theme::S1)
                .text_color(theme::cyan())
                .child(status),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .min_w_0()
                .child(truncate(
                    div()
                        .font_family(fonts::TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(theme::S5)
                        .line_height(relative(1.06))
                        .child(track.as_ref().map(|t| t.title.clone()).unwrap_or("—".into())),
                ))
                .children(track.as_ref().map(|t| love_button(&t.id, loved, 22., cx))),
        )
        .child(truncate(
            div().mt(px(7.)).text_size(theme::S4).child(track.as_ref().map(|t| t.artist.clone()).unwrap_or_default()),
        ))
        .child(truncate(
            div()
                .mt_1()
                .text_size(theme::S2)
                .text_color(theme::muted())
                .child(track.as_ref().map(|t| t.album.clone()).unwrap_or_default()),
        ));

    let seek_listener = cx.listener(|this, ev: &MouseDownEvent, _, cx| {
        this.seek_from(ev.position.x);
        cx.notify();
    });
    let drag_listener = cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
        if ev.dragging() {
            this.seek_from(ev.position.x);
            cx.notify();
        }
    });
    let string = div()
        .child(
            div()
                .id("string")
                .py(px(2.))
                .cursor_pointer()
                .child(string_bar::element(frame))
                .on_mouse_down(MouseButton::Left, seek_listener)
                .on_mouse_move(drag_listener)
                .child(super::record_bounds(cx.entity(), |this, b| this.string_bounds = Some(b))),
        )
        .child(
            div()
                .mt(px(-2.))
                .flex()
                .justify_between()
                .text_color(theme::muted())
                .child(mono(theme::S1).child(fmt_time(this.ui.position)))
                .child(mono(theme::S1).child(fmt_time(this.ui.duration))),
        );

    let ambient_layer = ambient.map(|img| {
        div()
            .absolute()
            .inset_0()
            .overflow_hidden()
            .child(
                // scale-110 so the blur never shows a soft border, at 22% opacity.
                div()
                    .absolute()
                    .top(relative(-0.05))
                    .left(relative(-0.05))
                    .w(relative(1.1))
                    .h(relative(1.1))
                    .opacity(0.22)
                    .child(gpui::img(ImageSource::Render(img)).size_full().object_fit(ObjectFit::Cover)),
            )
            // bg-gradient-to-b from-ink/75 via-ink/60 to-ink/90, as two halves.
            .child(div().absolute().top_0().left_0().right_0().h(relative(0.5)).bg(linear_gradient(
                180.,
                linear_color_stop(theme::a(theme::ink(), 0.75), 0.),
                linear_color_stop(theme::a(theme::ink(), 0.60), 1.),
            )))
            .child(div().absolute().bottom_0().left_0().right_0().h(relative(0.5)).bg(linear_gradient(
                180.,
                linear_color_stop(theme::a(theme::ink(), 0.60), 0.),
                linear_color_stop(theme::a(theme::ink(), 0.90), 1.),
            )))
    });

    div()
        .id("now-playing")
        .relative()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(26.))
        .overflow_hidden()
        .px(px(pad_x))
        .pt(px(34.))
        .pb(px(22.))
        .children(ambient_layer)
        .child(div().relative().flex().min_w_0().items_end().gap(px(26.)).child(art).child(heading))
        .child(div().relative().child(string))
        .child(div().relative().child(transport(this, wide, cx)))
        .into_any_element()
}

impl BardWindow {
    /// Click or drag anywhere on the string seeks there.
    fn seek_from(&mut self, x: Pixels) {
        let Some(bounds) = self.string_bounds else { return };
        if self.ui.duration <= 0. {
            return;
        }
        let ratio = ((x - bounds.origin.x) / bounds.size.width).clamp(0., 1.) as f64;
        let target = ratio * self.ui.duration;
        self.ui.position = target;
        self.ui.position_at = Instant::now();
        self.ui.backend.seek(target);
    }
}

fn transport(this: &mut BardWindow, wide: bool, cx: &mut Context<BardWindow>) -> impl IntoElement {
    let ui = &this.ui;
    let has_track = ui.track.is_some();

    let button = |id: &'static str, name: &'static str, size: f32| {
        div()
            .id(id)
            .rounded(px(2.))
            .p_1()
            .cursor_pointer()
            .child(icon(name, size, theme::muted()))
    };

    let buttons = div()
        .flex()
        .items_center()
        .gap(px(18.))
        .child(button("prev", "prev", 18.).on_click(cx.listener(|this, _, _, _| this.ui.backend.previous())))
        .child(
            button("play", if ui.playing { "pause" } else { "play" }, 26.)
                .when(!has_track, |el| el.opacity(0.4))
                .on_click(cx.listener(|this, _, _, _| this.ui.backend.toggle_play())),
        )
        .child(button("skip", "skip", 18.).on_click(cx.listener(|this, _, _, _| this.ui.backend.skip())));

    let repeat = ui.config.playback.repeat;
    let repeat_btn = div()
        .id("repeat")
        .relative()
        .ml_4()
        .rounded(px(2.))
        .p_1()
        .cursor_pointer()
        .child(icon("repeat", 17., if repeat == RepeatMode::Off { theme::muted() } else { theme::cyan() }))
        .when(repeat == RepeatMode::One, |el| {
            el.child(
                mono(px(9.))
                    .absolute()
                    .right(px(-2.))
                    .bottom(px(-2.))
                    .line_height(relative(1.))
                    .text_color(theme::cyan())
                    .child("1"),
            )
        })
        .on_click(cx.listener(move |this, _, _, _| {
            let next = match repeat {
                RepeatMode::Off => RepeatMode::All,
                RepeatMode::All => RepeatMode::One,
                RepeatMode::One => RepeatMode::Off,
            };
            this.ui.backend.update_config(Box::new(move |c| c.playback.repeat = next));
        }));

    let endless = ui.config.playback.endless;
    let endless_btn = div()
        .id("endless")
        .flex()
        .items_center()
        .gap(px(6.))
        .border_1()
        .px_2()
        .py(px(2.))
        .text_size(theme::S1)
        .cursor_pointer()
        .map(|el| {
            if endless {
                el.border_color(theme::cyan_deep()).bg(theme::cyan_deep()).text_color(theme::text())
            } else {
                el.border_color(theme::edge()).text_color(theme::muted())
            }
        })
        .child(icon("endless", 13., if endless { theme::text() } else { theme::muted() }))
        .child("Endless")
        .on_click(cx.listener(move |this, _, _, _| {
            this.ui.backend.update_config(Box::new(move |c| c.playback.endless = !endless));
        }));

    let mode = ui.config.playback.auto_fill;
    let mut segmented = div().flex().items_center().gap_1().border_1().border_color(theme::edge()).p(px(2.));
    for (value, label) in [(AutoFillMode::Off, "Off"), (AutoFillMode::Shuffle, "Shuffle"), (AutoFillMode::Album, "Album")] {
        let active = mode == value;
        segmented = segmented.child(
            div()
                .id(label)
                .px_2()
                .py(px(2.))
                .text_size(theme::S1)
                .cursor_pointer()
                .map(|el| if active { el.bg(theme::cyan_deep()).text_color(theme::text()) } else { el.text_color(theme::muted()) })
                .child(label)
                .on_click(cx.listener(move |this, _, _, _| {
                    this.ui.backend.update_config(Box::new(move |c| c.playback.auto_fill = value));
                })),
        );
    }

    let volume = ui.volume;
    let slider_w = if wide { 96. } else { 64. };

    div()
        .flex()
        .flex_wrap()
        .min_w_0()
        .items_center()
        .gap_x(px(22.))
        .gap_y_3()
        .child(buttons)
        .child(repeat_btn)
        .child(div().ml_6().flex().items_center().gap_3().child(endless_btn).child(segmented))
        .child(div().ml_auto().child(sleep_timer(this, cx)))
        .child(
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap(px(9.))
                .text_color(theme::muted())
                .child(icon("volume", 15., theme::muted()))
                .child(volume_slider(volume, slider_w, cx))
                .child(mono(theme::S1).child(format!("{}", (volume * 100.).round() as i64))),
        )
}

/// `<input type="range" class="h-px bg-edge accent-cyan">`: a hairline track
/// and Chromium's round thumb, in the accent colour.
fn volume_slider(volume: f64, w: f32, cx: &mut Context<BardWindow>) -> impl IntoElement {
    const THUMB: f32 = 16.;
    let set = move |this: &mut BardWindow, x: Pixels| {
        if let Some(b) = this.volume_bounds {
            let v = (((x - b.origin.x) - px(THUMB / 2.)) / (b.size.width - px(THUMB))).clamp(0., 1.) as f64;
            this.ui.volume = (v * 100.).round() / 100.;
            this.ui.backend.set_volume(this.ui.volume);
        }
    };
    div()
        .id("volume")
        .relative()
        .w(px(w))
        .h(px(THUMB))
        .flex_none()
        .cursor_pointer()
        .child(div().absolute().left_0().right_0().top(px(THUMB / 2.)).h(px(1.)).bg(theme::edge()))
        .child(
            div()
                .absolute()
                .top_0()
                .left(px((w - THUMB) * volume as f32))
                .size(px(THUMB))
                .rounded_full()
                .bg(theme::cyan()),
        )
        .child(super::record_bounds(cx.entity(), |this, b| this.volume_bounds = Some(b)))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
            set(this, ev.position.x);
            cx.notify();
        }))
        .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
            if ev.dragging() {
                set(this, ev.position.x);
                cx.notify();
            }
        }))
}

/// `SleepTimer.tsx`. Pauses rather than quitting: waking to a closed app and a
/// lost queue is worse than waking to a paused one.
fn sleep_timer(this: &mut BardWindow, cx: &mut Context<BardWindow>) -> impl IntoElement {
    let ui = &this.ui;
    let remaining = ui
        .sleep_ends_at
        .map(|end| end.saturating_duration_since(Instant::now()).as_secs().div_ceil(60));
    let open = ui.sleep_open;

    let popover = open.then(|| {
        let mut presets = div().flex().gap_1();
        for m in SLEEP_PRESETS {
            presets = presets.child(
                mono(theme::S1)
                    .id(SharedString::from(format!("sleep-{m}")))
                    .border_1()
                    .border_color(theme::edge())
                    .px_2()
                    .py_1()
                    .text_color(theme::muted())
                    .cursor_pointer()
                    .hover(|s| s.border_color(theme::cyan()).text_color(theme::text()))
                    .child(m.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ui.sleep_ends_at = Some(Instant::now() + Duration::from_secs(m * 60));
                        this.ui.sleep_open = false;
                        cx.notify();
                    })),
            );
        }
        // left-1/2 -translate-x-1/2: GPUI has no translate, so the popover is
        // centred inside a span wider than itself, centred on the button.
        div()
            .absolute()
            .bottom_full()
            .mb_2()
            .left(px(-200.))
            .right(px(-200.))
            .flex()
            .justify_center()
            .child(div()
            .border_1()
            .border_color(theme::edge())
            .bg(theme::panel())
            .p_2()
            .shadow_2xl()
            .occlude()
            .child(div().mb(px(6.)).whitespace_nowrap().text_size(theme::S1).text_color(theme::muted()).child("Pause playback in…"))
            .child(presets)
            .children(remaining.map(|r| {
                div()
                    .id("sleep-cancel")
                    .mt(px(6.))
                    .w_full()
                    .flex()
                    .justify_center()
                    .border_1()
                    .border_color(theme::edge())
                    .px_2()
                    .py_1()
                    .text_size(theme::S1)
                    .text_color(theme::rose())
                    .cursor_pointer()
                    .hover(|s| s.border_color(theme::rose()))
                    .child(format!("Cancel ({r} min left)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.sleep_ends_at = None;
                        this.ui.sleep_open = false;
                        cx.notify();
                    }))
            })))
    });

    div()
        .relative()
        .children(popover)
        .child(
            div()
                .id("sleep")
                .flex()
                .items_center()
                .gap_1()
                .rounded(px(2.))
                .p_1()
                .cursor_pointer()
                .child(icon("moon", 16., if remaining.is_some() { theme::cyan() } else { theme::muted() }))
                .children(remaining.map(|r| mono(theme::S1).text_color(theme::cyan()).child(r.to_string())))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ui.sleep_open = !this.ui.sleep_open;
                    cx.notify();
                })),
        )
}
