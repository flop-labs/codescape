use makepad_widgets::*;

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;
    use crate::scape::CodeScape;

    App = {{App}} {
        ui: <Root> {
            main_window = <Window> {
                window: { title: "FLOP codescape", inner_size: vec2(1920, 1080) }
                show_bg: true
                draw_bg: { color: #0A1128 }
                body = <View> {
                    width: Fill, height: Fill
                    scape = <CodeScape> {}
                }
            }
        }
    }
}

app_main!(App);

#[derive(Live, LiveHook)]
pub struct App {
    #[live]
    ui: WidgetRef,
}

impl LiveRegister for App {
    fn live_register(cx: &mut Cx) {
        makepad_widgets::live_design(cx);
        crate::scape::live_design(cx);
    }
}

impl AppMain for App {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
