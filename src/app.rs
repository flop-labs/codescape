use makepad_widgets::*;

app_main!(App);

script_mod! {
    use mod.prelude.widgets.*

    startup() do #(App::script_component(vm)){
        ui: Root{
            main_window := Window{
                window.title: "FLOP codescape"
                window.inner_size: vec2(1920, 1080)
                // 2.0 draws a title strip on macOS by default; 1.0 ran the
                // scene full-height under the traffic lights.
                show_caption_bar: false
                // A clear, not a drawn background: a drawn one writes depth
                // in front of the scene, which pins itself to the far plane.
                pass +: { clear_color: #x0A1128 }
                body +: {
                    scape := mod.widgets.CodeScape{
                        width: Fill
                        height: Fill
                    }
                }
            }
        }
    }
}

#[derive(Script, ScriptHook)]
pub struct App {
    #[live]
    ui: WidgetRef,
}

impl MatchEvent for App {}

impl AppMain for App {
    fn script_mod(vm: &mut ScriptVm) -> ScriptValue {
        makepad_widgets::script_mod(vm);
        crate::scape::script_mod(vm);
        self::script_mod(vm)
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
