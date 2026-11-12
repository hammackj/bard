use gpui::*;
use gpui_platform::application;

fn main() {
    application().run(|cx: &mut App| {
        bard::fonts::load(cx).expect("embedded fonts");
        gpui_component::init(cx);
        cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Empty)).unwrap();
        cx.activate(true);
    });
}

struct Empty;
impl Render for Empty {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(bard::theme::ink())
    }
}
