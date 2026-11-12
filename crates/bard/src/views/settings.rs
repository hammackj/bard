//! `SettingsPanel.tsx`.
//!
//! A full-window sheet over everything, including the title bar. The controls
//! are drawn to match how Chromium rendered the native ones on this palette:
//! boxed selects with a chevron, 13px checkboxes, hairline range sliders.

use super::root::BardWindow;
use super::text_input::TextInput;
use super::{mono, truncate};
use crate::{fonts, theme};
use bard_core::types::{AutoFillMode, Config, NormalizeMode, TrayPopupCorner};
use gpui::prelude::FluentBuilder;
use gpui::*;

/// The text fields Settings owns. Created with the window, since a field needs
/// a Window to register its blur handler.
#[derive(Clone)]
pub struct Fields {
    pub avoid_repeat: Entity<TextInput>,
    pub fill_count: Entity<TextInput>,
    pub popup_seconds: Entity<TextInput>,
    pub obs_url: Entity<TextInput>,
    pub obs_password: Entity<TextInput>,
}

#[derive(Clone, Copy)]
enum Span {
    Plain,
    Mono,
    Italic,
}

/// A muted paragraph with inline mono and italic spans. Each part has to be a
/// run in one text element: separate elements are blocks in GPUI and would
/// each start a line.
fn rich(parts: &[(&str, Span)]) -> Div {
    let text: String = parts.iter().map(|(t, _)| *t).collect();
    let runs = parts
        .iter()
        .map(|(t, span)| {
            let mut f = font(match span {
                Span::Mono => fonts::MONO,
                _ => fonts::UI,
            });
            if matches!(span, Span::Italic) {
                f.style = FontStyle::Italic;
            }
            TextRun { len: t.len(), font: f, color: theme::muted().into(), ..Default::default() }
        })
        .collect();
    div().text_size(theme::S1).text_color(theme::muted()).child(StyledText::new(text).with_runs(runs))
}

fn p(text: impl Into<SharedString>) -> Div {
    div().text_size(theme::S1).text_color(theme::muted()).child(text.into())
}

fn group(title: &str, children: Vec<AnyElement>) -> Div {
    div()
        .child(
            div()
                .mb_2()
                .font_family(fonts::TRACK13)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(theme::S1)
                .text_color(theme::cyan())
                .child(title.to_uppercase()),
        )
        .child(
            div().flex().flex_col().gap(px(6.)).border_t_1().border_color(theme::edge()).pt(px(10.)).children(children),
        )
}

fn row(label: impl Into<SharedString>, control: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .py_1()
        .child(div().text_size(theme::S2).text_color(theme::muted()).child(label.into()))
        .child(control)
        .into_any_element()
}

/// A boxed field around a TextInput: `border border-edge bg-panel px-2 py-1`.
fn field(input: &Entity<TextInput>, w: f32, size: Pixels, is_mono: bool) -> Div {
    div()
        .w(px(w))
        .border_1()
        .border_color(theme::edge())
        .bg(theme::panel())
        .px_2()
        .py_1()
        .text_size(size)
        .when(is_mono, |el| el.font_family(fonts::MONO))
        .child(input.clone())
}

/// Chromium's checkbox under `accent-cyan`: white and grey-bordered when off,
/// filled with the accent and a dark tick when on.
fn checkbox(id: &'static str, on: bool, f: fn(&mut Config, bool), cx: &mut Context<BardWindow>) -> impl IntoElement {
    div()
        .id(id)
        .size(px(13.))
        .rounded(px(2.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .map(|el| {
            if on {
                el.bg(theme::cyan()).child(super::icon("check", 11., theme::ink()))
            } else {
                el.bg(gpui::white()).border_1().border_color(rgb(0x767676))
            }
        })
        .on_click(cx.listener(move |this, _, _, _| {
            let next = !on;
            this.ui.backend.update_config(Box::new(move |c| f(c, next)));
        }))
}

/// A boxed select. The list is deferred so it paints over the rows below it.
fn select<T: Copy + PartialEq + 'static>(
    this: &BardWindow,
    id: &'static str,
    value: T,
    options: Vec<(T, String)>,
    on_pick: impl Fn(&mut BardWindow, T) + 'static,
    cx: &mut Context<BardWindow>,
) -> impl IntoElement {
    let current = options.iter().find(|(v, _)| *v == value).map(|(_, l)| l.clone()).unwrap_or_default();
    let open = this.ui.open_select == Some(id);
    // A native select is as wide as its longest option, not its current one.
    let sizer = div()
        .h_0()
        .overflow_hidden()
        .flex()
        .flex_col()
        .children(options.iter().map(|(_, l)| div().whitespace_nowrap().child(l.clone())));
    let on_pick = std::rc::Rc::new(on_pick);
    let list = open.then(|| {
        let mut list = div()
            .absolute()
            .top_full()
            .right_0()
            .min_w_full()
            .mt_1()
            .border_1()
            .border_color(theme::edge())
            .bg(theme::panel())
            .py_1()
            .shadow_2xl()
            .occlude();
        for (i, (v, label)) in options.into_iter().enumerate() {
            let pick = on_pick.clone();
            let active = v == value;
            list = list.child(
                div()
                    .id(SharedString::from(format!("{id}-{i}")))
                    .px_3()
                    .py_1()
                    .whitespace_nowrap()
                    .text_size(theme::S2)
                    .cursor_pointer()
                    .when(active, |el| el.bg(theme::cyan_deep()))
                    .when(!active, |el| el.hover(|s| s.bg(theme::edge())))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        // Handled here: the panel's click-away handler must
                        // not see the click that made the choice.
                        cx.stop_propagation();
                        this.ui.open_select = None;
                        pick(this, v);
                        cx.notify();
                    })),
            );
        }
        deferred(list).with_priority(1)
    });
    div()
        .relative()
        .child(
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(16.))
                .max_w(px(288.))
                .border_1()
                .border_color(theme::edge())
                .bg(theme::panel())
                .pl_3()
                .pr(px(6.))
                .py_1()
                .text_size(theme::S2)
                .cursor_pointer()
                .child(div().flex_1().min_w_0().flex().flex_col().child(sizer).child(truncate(div().child(current))))
                .child(super::icon("chevron", 12., theme::text()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    // Without this the click bubbles to the panel's
                    // click-away handler, which closes the list it just opened.
                    cx.stop_propagation();
                    this.ui.open_select = if this.ui.open_select == Some(id) { None } else { Some(id) };
                    cx.notify();
                })),
        )
        .children(list)
}

/// The crossfade slider: 0..12 seconds in whole steps.
fn crossfade(value: f64, cx: &mut Context<BardWindow>) -> impl IntoElement {
    const W: f32 = 128.;
    const THUMB: f32 = 16.;
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .id("crossfade")
                .relative()
                .w(px(W))
                .h(px(THUMB))
                .cursor_pointer()
                .child(div().absolute().left_0().right_0().top(px(THUMB / 2.)).h(px(1.)).bg(theme::edge()))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px((W - THUMB) * (value / 12.).clamp(0., 1.) as f32))
                        .size(px(THUMB))
                        .rounded_full()
                        .bg(theme::cyan()),
                )
                .child(super::record_bounds(cx.entity(), |this, b| this.crossfade_bounds = Some(b)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, _| {
                        this.crossfade_from(ev.position.x);
                        this.drag = Some(super::root::Drag::Crossfade);
                    }),
                ),
        )
        .child(mono(theme::S1).w(px(40.)).text_color(theme::muted()).child(if value == 0. {
            "off".to_string()
        } else {
            format!("{value}s")
        }))
}

impl BardWindow {
    pub fn crossfade_from(&mut self, x: Pixels) {
        const THUMB: f32 = 16.;
        if let Some(b) = self.crossfade_bounds {
            let v = (((x - b.origin.x) - px(THUMB / 2.)) / (b.size.width - px(THUMB))).clamp(0., 1.);
            let secs = (v * 12.).round() as f64;
            // Only on a change: each one rewrites config.json.
            if secs != self.ui.config.audio.crossfade_sec {
                self.ui.backend.update_config(Box::new(move |c| c.audio.crossfade_sec = secs));
            }
        }
    }
}

pub fn panel(this: &mut BardWindow, cx: &mut Context<BardWindow>) -> Option<impl IntoElement> {
    if !this.ui.settings_open {
        return None;
    }
    let c = this.ui.config.clone();
    let f = this.fields.clone();

    // Library
    let mut library: Vec<AnyElement> = c
        .library_paths
        .iter()
        .map(|path| {
            let path2 = path.clone();
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .py_1()
                .child(truncate(mono(theme::S2).text_color(theme::muted()).child(path.clone())))
                .child(
                    div()
                        .id(SharedString::from(format!("remove-{path}")))
                        .text_size(theme::S1)
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme::rose()))
                        .child("Remove")
                        .on_click(cx.listener(move |this, _, _, _| {
                            let p = path2.clone();
                            this.ui.backend.update_config(Box::new(move |c| c.library_paths.retain(|x| *x != p)));
                        })),
                )
                .into_any_element()
        })
        .collect();
    library.push(
        div()
            .flex()
            .child(
                div()
                    .id("add-folder")
                    .mt_2()
                    .bg(theme::cyan_deep())
                    .px_3()
                    .py(px(6.))
                    .text_size(theme::S2)
                    .cursor_pointer()
                    .child("Add music folder")
                    .on_click(cx.listener(|this, _, _, _| this.ui.backend.choose_library_folder())),
            )
            .into_any_element(),
    );

    // Audio
    let mut devices = vec![("default".to_string(), "System default".to_string())];
    devices.extend(this.ui.outputs.iter().cloned());
    let current_device = devices.iter().position(|(id, _)| *id == c.audio.output_device_id).unwrap_or(0);
    let device_options: Vec<(usize, String)> = devices.iter().enumerate().map(|(i, (_, n))| (i, n.clone())).collect();
    let device_ids: Vec<String> = devices.iter().map(|(id, _)| id.clone()).collect();
    let mut audio: Vec<AnyElement> = vec![
        row(
            "Output device",
            select(this, "output", current_device, device_options, move |this, i| {
                let id = device_ids[i].clone();
                // Persist first: applying can fail, and doing it the other way
                // round left the picker snapping back with no explanation.
                let id2 = id.clone();
                this.ui.backend.update_config(Box::new(move |c| c.audio.output_device_id = id2));
                this.ui.device_error = this.ui.backend.set_output(&id).err();
            }, cx),
        ),
        row(
            "Loudness normalisation",
            select(this, "normalize", c.audio.normalize, vec![
                (NormalizeMode::Off, "Off".into()),
                (NormalizeMode::Track, "Per track (even out shuffle)".into()),
                (NormalizeMode::Album, "Per album (keep album dynamics)".into()),
            ], |this, v| this.ui.backend.update_config(Box::new(move |c| c.audio.normalize = v)), cx),
        ),
rich(&[
            ("Uses ReplayGain tags written by ", Span::Plain),
            ("scripts/replaygain.py", Span::Mono),
            (". This library spans about 9 dB between its quietest and loudest tracks, which on shuffle is the difference between background music and reaching for the volume. Boost is capped and held back from clipping.", Span::Plain),
        ])
        .into_any_element(),
        row("Crossfade", crossfade(c.audio.crossfade_sec, cx)),
        p("Fades out, swaps, and fades back in rather than overlapping two decoders. On ambient music that is what the seam actually sounds like, and it keeps one audio path instead of two to keep in sync. Note the fade eats into the start of the next track.").into_any_element(),
    ];

    // An empty placeholder would still take a gap in the column, so the
    // message is only inserted when there is one.
    if let Some(e) = this.ui.device_error.clone() {
        audio.insert(1, p(format!("Output device: {e}")).text_color(theme::rose()).into_any_element());
    }

    // Playback
    let playback: Vec<AnyElement> = vec![
        row(
            "When the queue runs out",
            select(this, "autofill", c.playback.auto_fill, vec![
                (AutoFillMode::Off, "Stop".into()),
                (AutoFillMode::Shuffle, "Keep playing — shuffle".into()),
                (AutoFillMode::Album, "Keep playing — finish the album".into()),
            ], |this, v| this.ui.backend.update_config(Box::new(move |c| c.playback.auto_fill = v)), cx),
        ),
        row("Don't repeat the last N tracks", field(&f.avoid_repeat, 96., theme::S2, true)),
        row("Tracks added by Fill", field(&f.fill_count, 96., theme::S2, true)),
        p("Shuffle avoids anything played recently, so a small library does not loop back on itself. Album mode plays through the current record, then rolls to another.").into_any_element(),
        p("With Endless on (the ∞ button by the transport), playback never stops: the queue is kept topped up ahead of the current track, every track plays once before any repeats, and once the library is exhausted it reshuffles and keeps going. It runs until you stop it.").into_any_element(),
    ];

    // Shortcuts & window
    let window_group: Vec<AnyElement> = vec![
        row("Global media keys", checkbox("media-keys", c.shortcuts.global_media_keys, |c, v| c.shortcuts.global_media_keys = v, cx)),
        p("On KDE the media keys already reach Bard through MPRIS, like any other player. This setting is kept for parity and has no effect on Wayland.").into_any_element(),
        row(
            "Tray preview corner",
            select(this, "corner", c.window.tray_popup_corner, vec![
                (TrayPopupCorner::BottomRight, "Bottom right".into()),
                (TrayPopupCorner::BottomLeft, "Bottom left".into()),
                (TrayPopupCorner::TopRight, "Top right".into()),
                (TrayPopupCorner::TopLeft, "Top left".into()),
                (TrayPopupCorner::Cursor, "Near the cursor".into()),
            ], |this, v| this.ui.backend.update_config(Box::new(move |c| c.window.tray_popup_corner = v)), cx),
        ),
        row("Preview hides after (seconds, 0 = never)", field(&f.popup_seconds, 96., theme::S2, true)),
        p("Left-click the tray icon to restore the window; middle-click, or use the tray menu, for the preview. On Wayland the compositor places windows itself and ignores the corner setting.").into_any_element(),
        row("Close hides to tray", checkbox("close-to-tray", c.window.close_to_tray, |c, v| c.window.close_to_tray = v, cx)),
    ];

    // Files
    let files: Vec<AnyElement> = match this.ui.paths.clone() {
        Some(paths) => {
            let path_row = |label: &str, value: &str| {
                div()
                    .id(SharedString::from(format!("path-{label}")))
                    .tooltip(super::tip(value.to_string()))
                    .flex()
                    .items_baseline()
                    .justify_between()
                    .gap_4()
                    .py_1()
                    .child(div().flex_none().text_size(theme::S2).text_color(theme::muted()).child(label.to_string()))
                    .child(
                        // The start of a path is the least informative part, so it
                        // is what gets cut.
                        mono(theme::S1)
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .flex()
                            .justify_end()
                            .child(value.to_string()),
                    )
                    .into_any_element()
            };
            let dir = paths.config_dir.clone();
            vec![
                path_row("Settings", &paths.config),
                path_row("Album art cache", &paths.artwork),
                path_row("nowplaying.json", &paths.now_playing),
                div()
                    .flex()
                    .child(
                        div()
                            .id("reveal")
                            .mt_2()
                            .border_1()
                            .border_color(theme::edge())
                            .px_3()
                            .py(px(6.))
                            .text_size(theme::S2)
                            .text_color(theme::muted())
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme::text()))
                            .child("Open config folder")
                            .on_click(cx.listener(move |this, _, _, _| this.ui.backend.reveal(&dir))),
                    )
                    .into_any_element(),
            ]
        }
        None => vec![div().text_size(theme::S2).text_color(theme::muted()).child("Loading…").into_any_element()],
    };

    // Overlay
    let url = this.ui.overlay_url.clone();
    let copied = this.ui.copied;
    let overlay: Vec<AnyElement> = vec![
        row(
            "Browser source URL",
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(mono(theme::S2).child(url.clone().unwrap_or_else(|| "not running".into())))
                .children(url.map(|u| {
                    div()
                        .id("copy-url")
                        .border_b_1()
                        .border_color(theme::edge())
                        .text_size(theme::S1)
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .hover(|s| s.border_color(theme::cyan()).text_color(theme::cyan()))
                        .child(if copied { "Copied" } else { "Copy" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(u.clone()));
                            this.ui.copied = true;
                            cx.spawn(async move |this, cx| {
                                cx.background_executor().timer(std::time::Duration::from_millis(1500)).await;
                                let _ = this.update(cx, |this, cx| {
                                    this.ui.copied = false;
                                    cx.notify();
                                });
                            })
                            .detach();
                            cx.notify();
                        }))
                })),
        ),
        row(
            "Write nowplaying.json",
            checkbox("np-json", c.overlay.write_now_playing_json, |c, v| c.overlay.write_now_playing_json = v, cx),
        ),
    ];

    // Streamer mode
    let obs_msg = this.ui.obs_message.clone();
    let mut streamer: Vec<AnyElement> = vec![
        row("Only play albums marked safe", checkbox("armed", c.streamer.enabled, |c, v| c.streamer.enabled = v, cx)),
rich(&[
            ("While armed, only albums you have checked and marked ", Span::Plain),
            ("safe", Span::Mono),
            (" will play — including ones you pick deliberately. Untested albums are blocked too, on the grounds that an album nobody has checked is not evidence of anything. Blocked tracks always say so rather than failing silently.", Span::Plain),
        ])
        .into_any_element(),
        row("Arm automatically when OBS goes live", checkbox("auto-arm", c.streamer.auto_arm, |c, v| c.streamer.auto_arm = v, cx)),
        row("obs-websocket URL", field(&f.obs_url, 224., theme::S1, true)),
        row(
            if c.streamer.has_obs_password { "Password (set)" } else { "Password" },
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(field(&f.obs_password, 160., theme::S1, true))
                .when(c.streamer.has_obs_password, |el| {
                    el.child(
                        div()
                            .id("clear-password")
                            .text_size(theme::S1)
                            .text_color(theme::muted())
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme::rose()))
                            .child("Clear")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ui.backend.set_obs_password("");
                                this.ui.obs_message = None;
                                cx.notify();
                            })),
                    )
                }),
        ),
rich(&[
            ("Enable the server in OBS under ", Span::Plain),
            ("Tools → WebSocket Server Settings", Span::Italic),
            ("; it ships with OBS 28+ but is off by default. Going live arms the mode — it is never disarmed automatically, so a dropped connection cannot quietly remove the protection. Turn it off yourself when you are done. The password is stored with your OS keyring, not in ", Span::Plain),
            ("config.json", Span::Mono),
            (".", Span::Plain),
        ])
        .into_any_element(),
    ];

    if let Some(m) = obs_msg {
        let ok = m.starts_with("Password saved");
        let at = streamer.len() - 1;
        streamer.insert(at, p(m).text_color(if ok { theme::green() } else { theme::rose() }).into_any_element());
    }

    // About
    let about: Vec<AnyElement> = vec![
        row(
            "Bard",
            mono(theme::S2)
                .flex()
                .items_baseline()
                .child(this.ui.version.clone())
                .child(div().ml_2().text_size(theme::S1).text_color(theme::muted()).child(this.ui.commit.clone())),
        ),
        row(
            "Runtime",
            mono(theme::S1).text_color(theme::muted()).child(format!("GPUI · Rust · {}", std::env::consts::ARCH)),
        ),
        p("Quote these when reporting a problem.").into_any_element(),
    ];

    Some(
        div()
            .id("settings")
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            // bg-ink/95 with backdrop-blur-sm. GPUI cannot blur what is behind
            // an element, and unblurred the panes read through as text, so
            // the sheet is a little more opaque instead.
            .bg(theme::a(theme::ink(), 0.975))
            .occlude()
            // The sheet hides the window's own focus target from the pointer,
            // so a click on it that no field took hands focus back here: a
            // field being edited blurs (and commits), as in the browser.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if !window.default_prevented() {
                        window.focus(&this.focus, cx);
                    }
                }),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if this.ui.open_select.take().is_some() {
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme::edge())
                    .px_5()
                    .py_3()
                    .child(
                        div()
                            .font_family(fonts::TRACK13)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(theme::S1)
                            .text_color(theme::muted())
                            .child("SETTINGS"),
                    )
                    .child(
                        div()
                            .id("close-settings")
                            .text_size(theme::S2)
                            .text_color(theme::muted())
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme::text()))
                            .child("Close ✕")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ui.settings_open = false;
                                this.ui.open_select = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("settings-body")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&this.settings_scroll)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(28.))
                                    .px_5()
                                    .pr(px(20. + super::library::SCROLLBAR_W))
                                    .py_5()
                                    .child(group("Library", library))
                                    .child(group("Audio", audio))
                                    .child(group("Playback", playback))
                                    .child(group("Shortcuts & window", window_group))
                                    .child(group("Files", files))
                                    .child(group("Overlay", overlay))
                                    .child(group("Streamer mode", streamer))
                                    .child(group("About", about)),
                            ),
                    )
                    .child(super::scrollbar(this.settings_scroll.clone(), cx.entity())),
            ),
    )
}
