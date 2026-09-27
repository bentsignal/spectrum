mod adjust;
mod canvas;
mod controls;
mod library;
mod samples;
mod theme;
mod workspace;

use gpui::{
    App, Application, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, prelude::*,
    px, size,
};
use gpui_component::Root;
use gpui_component_assets::Assets;
use workspace::{Mode, Workspace};

gpui::actions!(spectrum_demo, [Quit, ShowLibrary, ShowAdjust, ShowCanvas]);

fn main() {
    if std::env::args().any(|arg| arg == "--version") {
        println!("Spectrum controls demo {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("secondary-q", Quit, None),
            KeyBinding::new("secondary-1", ShowLibrary, None),
            KeyBinding::new("secondary-2", ShowAdjust, None),
            KeyBinding::new("secondary-3", ShowCanvas, None),
        ]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        gpui_component::init(cx);
        theme::apply(cx);
        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(860.), px(560.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Spectrum".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(gpui::point(px(18.), px(19.))),
                }),
                ..Default::default()
            },
            |window, cx| {
                let workspace = cx.new(|cx| Workspace::new(window, cx));
                for (mode, bind) in [(Mode::Library, 0), (Mode::Adjust, 1), (Mode::Canvas, 2)] {
                    let workspace = workspace.downgrade();
                    let show = move |cx: &mut App| {
                        workspace
                            .update(cx, |this, cx| this.set_mode(mode, cx))
                            .ok();
                    };
                    match bind {
                        0 => cx.on_action(move |_: &ShowLibrary, cx| show(cx)),
                        1 => cx.on_action(move |_: &ShowAdjust, cx| show(cx)),
                        _ => cx.on_action(move |_: &ShowCanvas, cx| show(cx)),
                    }
                }
                cx.new(|cx| Root::new(workspace, window, cx))
            },
        )
        .expect("could not open Spectrum controls demo");
        cx.activate(true);
    });
}
