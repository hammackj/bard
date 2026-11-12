//! The app's windows: the main window (closed to the tray and rebuilt from
//! its parked model), the tray preview, quitting, and routing each event to
//! whichever model is live.

use super::backend::AppBackend;
use super::reduce::reduce;
use super::{AppEvent, Control};
use crate::model::{Backend, Ui};
use crate::views::popup::{self, Popup};
use crate::views::root::BardWindow;
use bard_core::CoreEvent;
use bard_core::types::Config;
use gpui::*;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// The app's windows and what outlives them.
pub(super) struct Shell {
    pub(super) backend: Rc<AppBackend>,
    pub(super) main: Option<WindowHandle<BardWindow>>,
    /// The window's root view. Reached through the entity rather than the
    /// handle: GPUI takes a window out of its registry while that window's
    /// callbacks run, so the handle cannot be used from inside on-close.
    pub(super) view: Option<WeakEntity<BardWindow>>,
    /// The window's model while the window is closed to the tray.
    pub(super) parked: Option<Ui>,
    pub(super) popup: Option<WindowHandle<Popup>>,
    /// Bumped on every show, so an older auto-hide timer does nothing.
    pub(super) popup_generation: u64,
    /// The last snapshot, for a popup opened between updates.
    pub(super) now: Option<bard_core::types::NowPlaying>,
    pub(super) artwork_dir: PathBuf,
    pub(super) quitting: bool,
}

pub(super) type ShellRef = Rc<RefCell<Shell>>;

/// Opens the main window from the parked model, or raises it if it is open.
///
/// `token` is the xdg-activation token the request came with. KWin will not
/// raise a window on a request it cannot tie to a user action -- a tray click
/// happens in the panel, not here -- so without it the window could open or
/// stay behind others.
pub(super) fn show_main(shell: &ShellRef, token: Option<String>, cx: &mut App) {
    let main = shell.borrow().main;
    if let Some(handle) = main {
        let _ = handle.update(cx, |_, window, _| {
            if !token.as_deref().is_some_and(|t| crate::activation::activate(window, t)) {
                window.activate_window();
            }
        });
        return;
    }
    let Some(ui) = shell.borrow_mut().parked.take() else { return };
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(1000.), px(640.))),
            titlebar: Some(TitlebarOptions { title: Some("Bard".into()), ..Default::default() }),
            app_id: Some("bard".into()),
            window_background: WindowBackgroundAppearance::Opaque,
            ..Default::default()
        },
        |window, cx| cx.new(|cx| BardWindow::new(ui, window, cx)),
    );
    let handle = match opened {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[bard] could not open the window: {e:#}");
            return;
        }
    };
    let s = shell.clone();
    let view = handle.update(cx, move |_, window, cx| {
        if let Some(t) = token.as_deref() {
            crate::activation::activate(window, t);
        }
        window.on_window_should_close(cx, move |_, cx| on_close(&s, cx));
        cx.entity().downgrade()
    });
    let mut s = shell.borrow_mut();
    s.main = Some(handle);
    s.view = view.ok();
}

/// The window's close button. Closing hides to the tray by default -- a
/// streamer will hit the X by accident mid-stream, and quitting the music is
/// not a recoverable mistake -- but only when there is a tray to come back
/// from; without one, close quits.
fn on_close(shell: &ShellRef, cx: &mut App) -> bool {
    let (view, quitting) = {
        let s = shell.borrow();
        (s.view.clone(), s.quitting)
    };
    let Some(view) = view.and_then(|v| v.upgrade()) else { return true };
    let to_tray = !quitting && view.read(cx).ui.config.window.close_to_tray && tray_running(shell);
    if !to_tray {
        quit(shell, cx);
        return true;
    }
    let backend = shell.borrow().backend.clone();
    let parked = view.update(cx, |w, _| {
        let blank = Ui::new(backend.clone() as Rc<dyn Backend>, PathBuf::new());
        let mut ui = std::mem::replace(&mut w.ui, blank);
        // Transient UI does not survive the window, as it did not in Electron's
        // reloaded panels.
        ui.menu_open = false;
        ui.sleep_open = false;
        ui.open_select = None;
        ui
    });
    let mut s = shell.borrow_mut();
    s.parked = Some(parked);
    s.main = None;
    s.view = None;
    true
}

fn tray_running(shell: &ShellRef) -> bool {
    let backend = shell.borrow().backend.clone();
    let guard = backend.services.lock().expect("services lock");
    guard.as_ref().is_some_and(|s| s.tray_running())
}

/// Stops the services, flushes the core and exits.
fn quit(shell: &ShellRef, cx: &mut App) {
    let backend = {
        let mut s = shell.borrow_mut();
        if s.quitting {
            return;
        }
        s.quitting = true;
        s.backend.clone()
    };
    backend.player.stop();
    if let Some(services) = backend.services.lock().expect("services lock").take() {
        services.shutdown();
    }
    // Stats, flags and loved tracks, and a stopped nowplaying.json -- or the
    // overlay keeps showing a track that stopped with us.
    backend.with(|b| b.shutdown());
    cx.quit();
}

/// The tray preview: shown in the configured corner, hidden again after the
/// configured time. Toggles.
fn toggle_popup(shell: &ShellRef, cx: &mut App) {
    let open = shell.borrow_mut().popup.take();
    // A handle to a popup that was closed some other way fails here, and the
    // click opens a new one instead of doing nothing.
    if let Some(handle) = open
        && handle.update(cx, |_, window, _| window.remove_window()).is_ok()
    {
        return;
    }
    let (config, now, track, artwork_dir) = {
        let s = shell.borrow();
        let ui = current_ui(&s, cx);
        (
            ui.as_ref().map(|u| u.0.clone()).unwrap_or_default(),
            s.now.clone(),
            ui.and_then(|u| u.1),
            s.artwork_dir.clone(),
        )
    };
    let build = || {
        let mut p = Popup::new(artwork_dir.clone());
        p.now = now.clone();
        p.track_id = track.clone();
        p
    };
    let debug = std::env::var_os("BARD_POPUP_DEBUG").is_some();
    let layer = std::env::var_os("BARD_POPUP_PLAIN").is_none();
    let opened = cx.open_window(popup_options(&config, layer), |_, cx| cx.new(|_| build())).or_else(|e| {
        if debug {
            eprintln!("[bard] preview: layer-shell refused ({e:#}), using a plain window");
        }
        // X11, or a compositor without layer-shell: a plain window, placed
        // wherever the window manager decides, as Electron's was on Wayland.
        cx.open_window(popup_options(&config, false), |_, cx| cx.new(|_| build()))
    });
    if debug {
        eprintln!("[bard] preview: opened={} has_snapshot={}", opened.is_ok(), now.is_some());
    }
    let Ok(handle) = opened else { return };
    let generation = {
        let mut s = shell.borrow_mut();
        s.popup = Some(handle);
        s.popup_generation += 1;
        s.popup_generation
    };
    let seconds = config.window.tray_popup_seconds;
    if seconds > 0 {
        let shell = shell.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(std::time::Duration::from_secs(seconds as u64)).await;
            cx.update(|cx| {
                let mut s = shell.borrow_mut();
                if s.popup_generation == generation
                    && let Some(h) = s.popup.take()
                {
                    drop(s);
                    let _ = h.update(cx, |_, window, _| window.remove_window());
                }
            });
        })
        .detach();
    }
}

fn popup_options(config: &Config, layer: bool) -> WindowOptions {
    use bard_core::types::TrayPopupCorner as C;
    use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
    let size = size(px(popup::WIDTH), px(popup::HEIGHT));
    let kind = if layer {
        // The compositor places a layer surface by anchor and margin, so on
        // Wayland the corner setting finally works. "Near the cursor" has no
        // equivalent there and uses the default corner.
        let anchor = match config.window.tray_popup_corner {
            C::BottomLeft => Anchor::BOTTOM | Anchor::LEFT,
            C::TopRight => Anchor::TOP | Anchor::RIGHT,
            C::TopLeft => Anchor::TOP | Anchor::LEFT,
            C::BottomRight | C::Cursor => Anchor::BOTTOM | Anchor::RIGHT,
        };
        let m = px(16.);
        WindowKind::LayerShell(LayerShellOptions {
            namespace: "bard-popup".into(),
            layer: Layer::Overlay,
            anchor,
            exclusive_zone: None,
            exclusive_edge: None,
            margin: Some((m, m, m, m)),
            keyboard_interactivity: KeyboardInteractivity::None,
        })
    } else {
        WindowKind::PopUp
    };
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds { origin: point(px(0.), px(0.)), size })),
        titlebar: None,
        focus: false,
        show: true,
        kind,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        app_id: Some("bard".into()),
        window_background: WindowBackgroundAppearance::Transparent,
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

/// The config and the playing track id, from wherever the model is.
fn current_ui(s: &Shell, cx: &App) -> Option<(Config, Option<String>)> {
    let pick = |u: &Ui| (u.config.clone(), u.track.as_ref().map(|t| t.id.clone()));
    match (s.view.as_ref().and_then(|v| v.upgrade()), &s.parked) {
        (Some(v), _) => Some(pick(&v.read(cx).ui)),
        (None, Some(u)) => Some(pick(u)),
        (None, None) => None,
    }
}

/// Routes one event: window-level ones here, model updates to `reduce` on the
/// open window or the parked model.
pub(super) fn dispatch(shell: &ShellRef, ev: AppEvent, cx: &mut App) {
    match ev {
        AppEvent::Control(Control::ShowWindow(token)) => return show_main(shell, token, cx),
        AppEvent::Control(Control::ShowPopup) => return toggle_popup(shell, cx),
        AppEvent::Control(Control::Quit) => return quit(shell, cx),
        AppEvent::ChooseFolder => return choose_folder(shell, cx),
        AppEvent::Core(CoreEvent::Overlay(ref np)) => {
            let popup = {
                let mut s = shell.borrow_mut();
                s.now = Some(np.clone());
                s.popup
            };
            if let Some(h) = popup {
                let np = np.clone();
                let track = {
                    let s = shell.borrow();
                    current_ui(&s, cx).and_then(|u| u.1)
                };
                let _ = h.update(cx, |p, _, cx| {
                    p.now = Some(np);
                    p.track_id = track;
                    cx.notify();
                });
            }
            return;
        }
        _ => {}
    }
    let (view, backend) = {
        let s = shell.borrow();
        (s.view.as_ref().and_then(|v| v.upgrade()), s.backend.clone())
    };
    if let Some(view) = view {
        view.update(cx, |w, cx| {
            reduce(&mut w.ui, &*backend, ev);
            cx.notify();
        });
        return;
    }
    let mut parked = shell.borrow_mut().parked.take();
    if let Some(ui) = parked.as_mut() {
        reduce(ui, &*backend, ev);
    }
    shell.borrow_mut().parked = parked;
}

/// The folder picker. Asks GPUI, so it runs here rather than in `reduce`.
fn choose_folder(shell: &ShellRef, cx: &mut App) {
    let backend = shell.borrow().backend.clone();
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: true,
        prompt: Some("Add music folder".into()),
    });
    let bard = backend.bard.clone();
    cx.spawn(async move |_| {
        if let Ok(Ok(Some(paths))) = picked.await {
            // The Electron dialog allowed several folders at once.
            // A folder actually added changes the config, which
            // starts the scan (see ConfigChanged).
            bard.lock().expect("core lock").update_config(|c| {
                for dir in paths {
                    let dir = dir.to_string_lossy().into_owned();
                    if !c.library_paths.contains(&dir) {
                        c.library_paths.push(dir);
                    }
                }
            });
        }
    })
    .detach();
}
