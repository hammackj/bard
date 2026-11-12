//! The window, pane by pane. Each module mirrors one React component file.

pub mod library;
pub mod now_playing;
pub mod overlays;
pub mod popup;
pub mod queue;
pub mod root;
pub mod settings;
pub mod string_bar;
pub mod text_input;

use crate::{fonts, theme};
use gpui::*;

/// A monochrome icon, tinted like `currentColor`.
pub fn icon(name: &str, size: f32, color: impl Into<Hsla>) -> Svg {
    svg().path(SharedString::from(format!("icons/{name}.svg"))).size(px(size)).flex_none().text_color(color)
}

/// An icon that takes `hover` while the pointer is over `group`, for
/// `hover:text-*` on a button wrapping an SVG drawn in currentColor.
pub fn hover_icon(group: &'static str, name: &str, size: f32, color: impl Into<Hsla>, hover: impl Into<Hsla>) -> Svg {
    let hover = hover.into();
    icon(name, size, color).group_hover(group, move |s| s.text_color(hover))
}

/// Text that truncates with an ellipsis, like Tailwind's `truncate`.
pub fn truncate<E: Styled>(el: E) -> E {
    el.min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis()
}

/// An uppercase, tracked label: `text-s1 font-semibold tracking-[.13em] uppercase`.
pub fn section_label(text: &str) -> Div {
    div().font_family(fonts::TRACK13).font_weight(FontWeight::SEMIBOLD).text_size(theme::S1).child(text.to_uppercase())
}

/// `.mono` at a given size.
pub fn mono(size: Pixels) -> Div {
    div().font_family(fonts::MONO).text_size(size)
}

/// An invisible layer that records its parent's bounds at prepaint, for
/// controls that turn a pointer position into a value (the string, the volume
/// slider). Stored without a notify: it is read on the next input event, not
/// rendered.
pub fn record_bounds<T: 'static>(entity: Entity<T>, store: fn(&mut T, Bounds<Pixels>)) -> impl IntoElement {
    canvas(
        move |bounds, _, cx| {
            entity.update(cx, |this, _| store(this, bounds));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

/// The thumb's height and top, relative to the scroller, or None when the
/// content fits.
fn thumb(handle: &ScrollHandle) -> Option<(f32, f32)> {
    let max = f32::from(handle.max_offset().y);
    if max <= 0.5 {
        return None;
    }
    let h = f32::from(handle.bounds().size.height);
    let thumb = (h * h / (h + max)).max(18.);
    let y = (-f32::from(handle.offset().y) / max).clamp(0., 1.) * (h - thumb);
    Some((thumb, y))
}

/// Scrolls so the thumb's top sits `grab` pixels above the pointer at `y`.
pub fn scroll_thumb_to(handle: &ScrollHandle, y: Pixels, grab: f32) {
    let Some((thumb, _)) = thumb(handle) else { return };
    let view = handle.bounds();
    let travel = f32::from(view.size.height) - thumb;
    if travel <= 0. {
        return;
    }
    let top = (f32::from(y - view.origin.y) - grab).clamp(0., travel);
    let max = f32::from(handle.max_offset().y);
    let offset = handle.offset();
    handle.set_offset(point(offset.x, px(-(top / travel) * max)));
}

/// `::-webkit-scrollbar`: a 9px gutter whose thumb is edge-grey inside a 3px
/// ink border, with a transparent track. Painted over the gutter the scroller
/// reserves, from the scroll handle's state in the same frame. The thumb can
/// be dragged, and a press on the track pages towards the pointer.
pub fn scrollbar(handle: ScrollHandle, view: Entity<root::BardWindow>) -> impl IntoElement {
    let paint_handle = handle.clone();
    canvas(
        move |_, window, _| {
            let v = handle.bounds();
            let gutter = Bounds::new(
                point(v.origin.x + v.size.width - px(library::SCROLLBAR_W), v.origin.y),
                size(px(library::SCROLLBAR_W), v.size.height),
            );
            window.insert_hitbox(gutter, HitboxBehavior::Normal)
        },
        move |_, hitbox, window, _| {
            let handle = paint_handle;
            let Some((thumb_h, y)) = thumb(&handle) else { return };
            let view_bounds = handle.bounds();
            let x = view_bounds.origin.x + view_bounds.size.width - px(library::SCROLLBAR_W);
            let outer =
                Bounds::new(point(x, view_bounds.origin.y + px(y)), size(px(library::SCROLLBAR_W), px(thumb_h)));
            window.paint_quad(fill(outer, theme::ink()));
            let inner = Bounds::new(outer.origin + point(px(3.), px(3.)), size(px(3.), px(thumb_h - 6.)));
            window.paint_quad(fill(inner, theme::edge()));

            window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || ev.button != MouseButton::Left || !hitbox.is_hovered(window) {
                    return;
                }
                let Some((thumb_h, top)) = thumb(&handle) else { return };
                let at = f32::from(ev.position.y - handle.bounds().origin.y);
                let grab = if at >= top && at <= top + thumb_h {
                    at - top
                } else {
                    // Chromium pages on a track press; one page, then the
                    // thumb follows the pointer while held.
                    let page = f32::from(handle.bounds().size.height);
                    let offset = handle.offset();
                    let max = f32::from(handle.max_offset().y);
                    let dir = if at < top { 1. } else { -1. };
                    let next = (f32::from(offset.y) + dir * page).clamp(-max, 0.);
                    handle.set_offset(point(offset.x, px(next)));
                    let (_, top) = thumb(&handle).unwrap_or((thumb_h, top));
                    at - top
                };
                let handle = handle.clone();
                view.update(cx, |this, cx| {
                    this.drag = Some(root::Drag::Scroll { handle, grab });
                    cx.notify();
                });
                cx.stop_propagation();
            });
        },
    )
    .absolute()
    .inset_0()
}

/// A `title=` tooltip: shown after a hover delay, near the pointer. Drawn in
/// the app's panel style rather than imitating Chromium's native bubble.
pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(320.))
            .border_1()
            .border_color(theme::edge())
            .bg(theme::panel())
            .shadow_lg()
            .px_2()
            .py_1()
            .font_family(fonts::UI)
            .text_size(theme::S1)
            .line_height(relative(1.4))
            .text_color(theme::text())
            .child(self.0.clone())
    }
}
