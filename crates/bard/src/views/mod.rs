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
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// Text that truncates with an ellipsis, like Tailwind's `truncate`.
pub fn truncate<E: Styled>(el: E) -> E {
    el.min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis()
}

/// An uppercase, tracked label: `text-s1 font-semibold tracking-[.13em] uppercase`.
pub fn section_label(text: &str) -> Div {
    div()
        .font_family(fonts::TRACK13)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(theme::S1)
        .child(text.to_uppercase())
}

/// `.mono` at a given size.
pub fn mono(size: Pixels) -> Div {
    div().font_family(fonts::MONO).text_size(size)
}

/// An invisible layer that records its parent's bounds at prepaint, for
/// controls that turn a pointer position into a value (the string, the volume
/// slider). Stored without a notify: it is read on the next input event, not
/// rendered.
pub fn record_bounds<T: 'static>(
    entity: Entity<T>,
    store: fn(&mut T, Bounds<Pixels>),
) -> impl IntoElement {
    canvas(
        move |bounds, _, cx| {
            entity.update(cx, |this, _| store(this, bounds));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

/// `::-webkit-scrollbar`: a 9px gutter whose thumb is edge-grey inside a 3px
/// ink border, with a transparent track. Painted over the gutter the scroller
/// reserves, from the scroll handle's state in the same frame.
pub fn scrollbar(handle: ScrollHandle) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let view = handle.bounds();
            let max = f32::from(handle.max_offset().y);
            if max <= 0.5 {
                return;
            }
            let h = f32::from(view.size.height);
            let thumb = (h * h / (h + max)).max(18.);
            let y = (-f32::from(handle.offset().y) / max).clamp(0., 1.) * (h - thumb);
            let x = view.origin.x + view.size.width - px(library::SCROLLBAR_W);
            let outer = Bounds::new(point(x, view.origin.y + px(y)), size(px(library::SCROLLBAR_W), px(thumb)));
            window.paint_quad(fill(outer, theme::ink()));
            let inner = Bounds::new(outer.origin + point(px(3.), px(3.)), size(px(3.), px(thumb - 6.)));
            window.paint_quad(fill(inner, theme::edge()));
        },
    )
    .absolute()
    .inset_0()
}
