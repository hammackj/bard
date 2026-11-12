use gpui::*;
use gpui_platform::application;

struct Hello;
impl Render for Hello {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(0x0b0d12)).text_color(rgb(0xe5e7eb)).child("Bard")
    }
}

fn main() {
    application().run(|cx: &mut App| {
        gpui_component::init(cx);
        cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Hello)).unwrap();
    });
}
