//! `App.tsx`: the title bar, the streamer banner, the three docks and the
//! layers over and under them.

use super::settings::Fields;
use super::text_input::{InputEvent, TextInput};
use super::{icon, library, now_playing, overlays, queue, settings, truncate};
use crate::backdrop;
use crate::model::{Covers, Ui};
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
    pub album_scroll: UniformListScrollHandle,
    pub tracks_scroll: UniformListScrollHandle,
    pub queue_scroll: ScrollHandle,
    pub settings_scroll: ScrollHandle,
    pub crossfade_bounds: Option<Bounds<Pixels>>,
    /// A once-a-second repaint for the sleep countdown is pending.
    pub sleep_tick: bool,
    pub filter: Entity<TextInput>,
    /// The Content ID note for the open album, and which album it was loaded for.
    pub note: Entity<TextInput>,
    pub note_album: Option<String>,
    pub fields: Fields,
    /// Focus for the window itself, so shortcuts arrive with no field focused.
    pub focus: FocusHandle,
    /// The track list, focused by clicking a row, takes ↑/↓, Enter and Esc
    /// as the Electron list's buttons did.
    pub list_focus: FocusHandle,
    /// The (selection, row count) the track list last scrolled for.
    pub scrolled_to: Option<(usize, usize)>,
    /// Where the avatar menu and the sleep popover were drawn, so a click
    /// inside them is not taken as a click away.
    pub menu_bounds: Option<Bounds<Pixels>>,
    pub sleep_bounds: Option<Bounds<Pixels>>,
    /// A press on a slider or scrollbar that the pointer is still holding.
    pub drag: Option<Drag>,
    /// Backdrop bitmaps, rebuilt when the window's device size changes.
    layers: Option<((u32, u32), Layers)>,
}

/// What a held left button is dragging. Followed window-wide until release,
/// as Chromium's pointer capture did, so leaving the control does not drop it.
pub enum Drag {
    String,
    Volume,
    Crossfade,
    /// A scrollbar thumb, grabbed `grab` pixels below its top.
    Scroll {
        handle: ScrollHandle,
        grab: f32,
    },
}

struct Layers {
    glow: Arc<RenderImage>,
    vignette: Arc<RenderImage>,
    grain: Arc<RenderImage>,
}

impl BardWindow {
    pub fn new(mut ui: Ui, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Decodes a window closed before they finished will never deliver.
        ui.covers.forget_requests();
        let c = ui.config.clone();
        let input = |window: &mut Window, cx: &mut Context<Self>, placeholder: &str, value: &str| {
            cx.new(|cx| TextInput::new(window, cx, placeholder, value))
        };
        let filter = input(window, cx, "Filter tracks…", &ui.filter);
        cx.subscribe_in(&filter, window, |this, input, ev, window, cx| match ev {
            InputEvent::Changed => {
                this.ui.filter = input.read(cx).text().to_string();
                this.ui.selected = 0;
                cx.notify();
            }
            // Escape clears the filter and gives focus back to the window.
            InputEvent::Cancel => {
                input.update(cx, |i, cx| i.set_text("", cx));
                this.ui.filter.clear();
                window.focus(&this.focus, cx);
                cx.notify();
            }
            // Enter queues the selected track; Shift+Enter plays it now.
            InputEvent::Submit | InputEvent::SubmitShift => {
                if let Some(t) = this.ui.filtered_tracks().get(this.ui.selected) {
                    if *ev == InputEvent::Submit {
                        this.ui.backend.enqueue(&t.id);
                    } else {
                        this.ui.backend.play_now(&t.id);
                    }
                }
            }
            _ => {}
        })
        .detach();

        let note = input(window, cx, "who claimed it, or a note", "");
        // Saved on blur, as the Electron field was. Enter blurs, so it saves too.
        cx.subscribe_in(&note, window, |this, input, ev, window, cx| match ev {
            InputEvent::Blur => {
                if let Some(album) = this.note_album.clone() {
                    let text = input.read(cx).text().to_string();
                    let status = this.ui.album_status(&album);
                    let saved = this.ui.album_flags.get(&album).and_then(|f| f.note.clone()).unwrap_or_default();
                    if text != saved {
                        this.ui.backend.set_album_flag(&album, status, Some(text));
                    }
                }
            }
            InputEvent::Submit | InputEvent::Cancel => window.focus(&this.focus, cx),
            _ => {}
        })
        .detach();

        let fields = Fields {
            avoid_repeat: input(window, cx, "", &c.playback.avoid_repeat.to_string()),
            fill_count: input(window, cx, "", &c.playback.fill_count.to_string()),
            popup_seconds: input(window, cx, "", &c.window.tray_popup_seconds.to_string()),
            obs_url: input(window, cx, "", &c.streamer.obs_url),
            obs_password: input(window, cx, if c.streamer.has_obs_password { "••••••••" } else { "none" }, ""),
        };
        fields.obs_password.update(cx, |i, _| i.masked = true);
        for f in [&fields.avoid_repeat, &fields.fill_count, &fields.popup_seconds] {
            f.update(cx, |i, _| i.numeric = true);
        }

        // Escape in a settings field closes Settings, as it did in Electron,
        // and hands focus back to the window so shortcuts work again.
        for field in
            [&fields.avoid_repeat, &fields.fill_count, &fields.popup_seconds, &fields.obs_url, &fields.obs_password]
        {
            cx.subscribe_in(field, window, |this, _, ev, window, cx| {
                if *ev == InputEvent::Cancel {
                    this.ui.settings_open = false;
                    this.ui.open_select = None;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }
            })
            .detach();
        }

        // Number fields commit on blur, as `NumberRow` did: a value that is
        // not a whole number >= 0 is dropped rather than saved.
        let number =
            |cx: &mut Context<Self>, field: &Entity<TextInput>, set: fn(&mut bard_core::types::Config, u32)| {
                cx.subscribe(field, move |this, input, ev, cx| {
                    if matches!(ev, InputEvent::Blur | InputEvent::Submit)
                        && let Ok(v) = input.read(cx).text().trim().parse::<f64>()
                        && v.is_finite()
                        && v >= 0.
                    {
                        let v = v.floor() as u32;
                        this.ui.backend.update_config(Box::new(move |c| set(c, v)));
                    }
                })
                .detach();
            };
        number(cx, &fields.avoid_repeat, |c, v| c.playback.avoid_repeat = v);
        number(cx, &fields.fill_count, |c, v| c.playback.fill_count = v);
        number(cx, &fields.popup_seconds, |c, v| c.window.tray_popup_seconds = v);
        cx.subscribe(&fields.obs_url, |this, input, ev, cx| {
            if matches!(ev, InputEvent::Blur | InputEvent::Submit) {
                let url = input.read(cx).text().trim().to_string();
                this.ui.backend.update_config(Box::new(move |c| c.streamer.obs_url = url));
            }
        })
        .detach();
        // Enter saves as well as blur: saving only on blur discarded a password
        // typed and confirmed with Enter, and the field looks the same either way.
        cx.subscribe(&fields.obs_password, |this, input, ev, cx| {
            if matches!(ev, InputEvent::Blur | InputEvent::Submit) {
                // Trimmed: a pasted trailing space fails authentication with
                // nothing on screen to suggest why.
                let value = input.read(cx).text().trim().to_string();
                if value.is_empty() {
                    return;
                }
                input.update(cx, |i, cx| i.set_text("", cx));
                this.ui.backend.set_obs_password(&value);
                this.ui.obs_message = None;
                cx.notify();
            }
        })
        .detach();

        Self {
            ui,
            layers: None,
            list_focus: cx.focus_handle(),
            scrolled_to: None,
            menu_bounds: None,
            sleep_bounds: None,
            drag: None,
            string_bounds: None,
            volume_bounds: None,
            album_scroll: UniformListScrollHandle::new(),
            tracks_scroll: UniformListScrollHandle::new(),
            queue_scroll: ScrollHandle::new(),
            settings_scroll: ScrollHandle::new(),
            crossfade_bounds: None,
            sleep_tick: false,
            filter,
            note,
            note_album: None,
            fields,
            focus: {
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                focus
            },
        }
    }

    /// Opens Settings, refreshing what the panel reads from the system and
    /// resetting its fields to the saved values, as the remounted React panel did.
    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.ui.settings_open = true;
        self.ui.outputs = self.ui.backend.outputs();
        self.ui.paths = self.ui.backend.paths();
        self.ui.device_error = None;
        self.ui.obs_message = None;
        let c = self.ui.config.clone();
        let f = &self.fields;
        for (field, value) in [
            (&f.avoid_repeat, c.playback.avoid_repeat.to_string()),
            (&f.fill_count, c.playback.fill_count.to_string()),
            (&f.popup_seconds, c.window.tray_popup_seconds.to_string()),
            (&f.obs_url, c.streamer.obs_url.clone()),
            (&f.obs_password, String::new()),
        ] {
            field.update(cx, |i, cx| i.set_text(&value, cx));
        }
        let placeholder = if c.streamer.has_obs_password { "••••••••" } else { "none" };
        f.obs_password.update(cx, |i, _| i.placeholder = placeholder.into());
        cx.notify();
    }

    fn typing(&self, window: &Window, cx: &App) -> bool {
        let f = &self.fields;
        [&self.filter, &self.note, &f.avoid_repeat, &f.fill_count, &f.popup_seconds, &f.obs_url, &f.obs_password]
            .iter()
            .any(|i| i.focus_handle(cx).is_focused(window))
    }

    /// `App.tsx`'s global shortcuts, and the library's list keys. Arrow keys
    /// and Space are left to a field being typed in, except that Up and Down
    /// in the filter move the list selection, as they did in Electron.
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let typing = self.typing(window, cx);
        let in_filter = self.filter.focus_handle(cx).is_focused(window);
        let in_list = self.list_focus.is_focused(window);
        let ctrl = k.modifiers.control || k.modifiers.platform;
        let key = k.key.as_str();
        let handled = match key {
            "/" if !typing => {
                self.ui.library_open = true;
                self.filter.focus_handle(cx).focus(window, cx);
                true
            }
            "space" if !typing => {
                self.ui.backend.toggle_play();
                true
            }
            "right" if !typing => {
                let to = (self.position_now() + 5.).min(self.ui.duration);
                self.ui.backend.seek(to);
                true
            }
            "left" if !typing => {
                self.ui.backend.seek((self.position_now() - 5.).max(0.));
                true
            }
            "up" | "down" if in_filter || in_list => {
                let n = self.ui.filtered_tracks().len();
                if n > 0 {
                    self.ui.selected = if key == "down" {
                        (self.ui.selected + 1).min(n - 1)
                    } else {
                        self.ui.selected.saturating_sub(1)
                    };
                    self.tracks_scroll.scroll_to_item(self.ui.selected, ScrollStrategy::Nearest);
                }
                true
            }
            "up" | "down" if !typing => {
                let step = if key == "up" { 0.05 } else { -0.05 };
                let v = (self.ui.volume + step).clamp(0., 1.);
                self.ui.volume = v;
                self.ui.backend.set_volume(v);
                true
            }
            "enter" if in_list => {
                if let Some(t) = self.ui.filtered_tracks().get(self.ui.selected) {
                    if k.modifiers.shift {
                        self.ui.backend.play_now(&t.id);
                    } else {
                        self.ui.backend.enqueue(&t.id);
                    }
                }
                true
            }
            "?" if !typing => {
                self.ui.shortcuts_open = !self.ui.shortcuts_open;
                true
            }
            "," if ctrl => {
                if self.ui.settings_open {
                    self.ui.settings_open = false;
                } else {
                    self.open_settings(cx);
                }
                true
            }
            "escape" => {
                // In the list, Esc also clears the filter, as in Electron.
                if in_list {
                    self.filter.update(cx, |i, cx| i.set_text("", cx));
                    self.ui.filter.clear();
                }
                self.ui.settings_open = false;
                self.ui.shortcuts_open = false;
                self.ui.open_select = None;
                self.ui.menu_open = false;
                self.ui.sleep_open = false;
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// Where playback is now, between the engine's progress reports.
    fn position_now(&self) -> f64 {
        if self.ui.playing { self.ui.position + self.ui.position_at.elapsed().as_secs_f64() } else { self.ui.position }
    }

    fn layers(&mut self, window: &Window) -> &Layers {
        let size = window.viewport_size();
        let scale = window.scale_factor();
        let key = ((f32::from(size.width) * scale).round() as u32, (f32::from(size.height) * scale).round() as u32);
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

/// While a drag is held, every pointer move and the release reach the window,
/// wherever the pointer is.
fn drag_tracker(view: Entity<BardWindow>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let v = view.clone();
            window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    v.update(cx, |this, cx| {
                        // A release outside the window can go unreported.
                        if ev.pressed_button != Some(MouseButton::Left) {
                            this.drag = None;
                        } else {
                            this.drag_to(ev.position);
                        }
                        cx.notify();
                    });
                }
            });
            let v = view.clone();
            window.on_mouse_event(move |ev: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture && ev.button == MouseButton::Left {
                    v.update(cx, |this, cx| {
                        this.drag = None;
                        cx.notify();
                    });
                }
            });
        },
    )
    .absolute()
    .size_0()
}

impl BardWindow {
    pub fn drag_to(&mut self, pos: Point<Pixels>) {
        match &self.drag {
            Some(Drag::String) => self.seek_from(pos.x),
            Some(Drag::Volume) => self.volume_from(pos.x),
            Some(Drag::Crossfade) => self.crossfade_from(pos.x),
            Some(Drag::Scroll { handle, grab }) => super::scroll_thumb_to(handle, pos.y, *grab),
            None => {}
        }
    }
}

fn full(img: Arc<RenderImage>) -> Img {
    gpui::img(ImageSource::Render(img)).absolute().inset_0().size_full().object_fit(ObjectFit::Fill)
}

impl BardWindow {
    /// Decodes the covers this frame asked for on the background executor,
    /// and redraws as each arrives; releases evicted ones from the GPU.
    pub fn load_covers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for img in self.ui.covers.take_evicted() {
            cx.drop_image(img, Some(window));
        }
        let dir = self.ui.covers.dir.clone();
        for (kind, id) in self.ui.covers.take_wanted() {
            let dir = dir.clone();
            cx.spawn(async move |this, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn({
                        let id = id.clone();
                        async move { Covers::load(&dir, kind, &id) }
                    })
                    .await;
                let _ = this.update(cx, |w, cx| {
                    w.ui.covers.insert((kind, id), decoded);
                    cx.notify();
                });
            })
            .detach();
        }
    }
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

        let root = div()
            .id("bard")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
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
                    .when(self.ui.library_open, |el| el.child(library::pane(self, window, cx)))
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
            .children(overlays::shortcuts(self, cx))
            .children(settings::panel(self, cx))
            // Last, so a notice shows above Settings and Shortcuts.
            .children(overlays::notice(self, cx))
            .when(self.drag.is_some(), |el| el.child(drag_tracker(cx.entity())));
        // Covers asked for while building this frame.
        self.load_covers(window, cx);
        root
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
            .child(dock_toggle(
                "dock-left",
                ui.library_open,
                cx.listener(|this, _, _, cx| {
                    this.ui.library_open = !this.ui.library_open;
                    cx.notify();
                }),
            ))
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
            .child(truncate(div().flex_1().text_size(theme::S1).text_color(theme::muted()).child(
                "Only albums marked safe for Content ID will play — untested and claimed \
                         albums are blocked, including ones you pick yourself.",
            )))
            .child(
                div()
                    .id("streamer-off")
                    .tooltip(super::tip("Stop restricting playback. Nothing re-arms it until OBS goes live again."))
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
        .tooltip(super::tip(format!(
            "{} {} panel",
            if open { "Hide" } else { "Show" },
            if side == "dock-left" { "library" } else { "queue" }
        )))
        .rounded(px(2.))
        .px_1()
        .py(px(2.))
        .text_color(theme::muted())
        .cursor_pointer()
        .group(side)
        .hover(|s| s.bg(theme::panel()).text_color(theme::text()))
        .child(super::hover_icon(side, &name, 15., theme::muted(), theme::text()))
        .on_click(on_click)
}
