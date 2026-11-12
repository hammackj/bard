//! Raising a window with an xdg-activation token from elsewhere.
//!
//! On Wayland only the compositor decides which window comes to the front,
//! and KWin only raises one when the request carries a token tied to a user
//! action. A tray click is a user action in the panel, which passes its token
//! to us over D-Bus (`ProvideXdgActivationToken`). GPUI only takes a token
//! from the environment at startup, so this hands it to the compositor
//! directly, on GPUI's own Wayland connection, for the window's surface.

use gpui::Window;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, backend::Backend};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;

/// Activates `window` with `token`. False when the window is not a Wayland
/// one or the compositor has no xdg-activation, so the caller can fall back
/// to GPUI's own request.
pub fn activate(window: &Window, token: &str) -> bool {
    match try_activate(window, token) {
        Ok(()) => true,
        Err(e) => {
            if !matches!(e, Skip::NotWayland) {
                eprintln!("[bard] window activation: {e:?}");
            }
            false
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
enum Skip {
    NotWayland,
    Failed(String),
}

fn failed(e: impl std::fmt::Display) -> Skip {
    Skip::Failed(e.to_string())
}

fn try_activate(window: &Window, token: &str) -> Result<(), Skip> {
    let RawDisplayHandle::Wayland(display) = window.display_handle().map_err(|_| Skip::NotWayland)?.as_raw() else {
        return Err(Skip::NotWayland);
    };
    let RawWindowHandle::Wayland(surface) =
        HasWindowHandle::window_handle(window).map_err(|_| Skip::NotWayland)?.as_raw()
    else {
        return Err(Skip::NotWayland);
    };

    // SAFETY: both pointers come from GPUI's live libwayland connection and
    // the window's live surface, on the thread that owns them. A foreign
    // backend does not close the display when dropped, and this uses its own
    // event queue, so GPUI's dispatching is undisturbed.
    let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
    let conn = Connection::from_backend(backend);
    let surface_id =
        unsafe { wayland_client::backend::ObjectId::from_ptr(WlSurface::interface(), surface.surface.as_ptr().cast()) }
            .map_err(failed)?;
    let surface = WlSurface::from_id(&conn, surface_id).map_err(failed)?;

    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(failed)?;
    let qh = queue.handle();
    let activation: XdgActivationV1 = globals.bind(&qh, 1..=1, ()).map_err(failed)?;
    activation.activate(token.to_string(), &surface);
    activation.destroy();
    queue.roundtrip(&mut State).map_err(failed)?;
    Ok(())
}

struct State;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgActivationV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &XdgActivationV1,
        _: <XdgActivationV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
