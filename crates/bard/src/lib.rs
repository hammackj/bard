//! The GPUI front end of Bard.

#[cfg(target_os = "linux")]
pub mod activation;
/// xdg-activation is a Wayland protocol; elsewhere GPUI's own window
/// activation is all there is, and all that is needed.
#[cfg(not(target_os = "linux"))]
pub mod activation {
    pub fn activate(_window: &gpui::Window, _token: &str) -> bool {
        false
    }
}
pub mod app;
pub mod assets;
pub mod backdrop;
pub mod fonts;
pub mod model;
pub mod theme;
pub mod views;
