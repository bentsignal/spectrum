mod adjust;
mod canvas;
mod color;
mod controls;
mod grid;
mod home;
mod library;
mod palette;
mod project;
mod samples;
mod store;
mod theme;
mod trash;
mod workspace;

use gpui::{
    App, Application, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, prelude::*,
    px, size,
};
use gpui_component::Root;
use gpui_component_assets::Assets;
use workspace::Workspace;

gpui::actions!(
    spectrum_demo,
    [
        Quit,
        Mode1,
        Mode2,
        Mode3,
        Mode4,
        Mode5,
        OpenPalette,
        SelectAll,
        ClearSelection
    ]
);

fn main() {
    if std::env::args().any(|arg| arg == "--version") {
        println!("Spectrum preview {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        // Command+S stays unbound: people press it by habit, and Spectrum saves as it goes.
        cx.bind_keys([
            KeyBinding::new("secondary-q", Quit, None),
            KeyBinding::new("secondary-1", Mode1, None),
            KeyBinding::new("secondary-2", Mode2, None),
            KeyBinding::new("secondary-3", Mode3, None),
            KeyBinding::new("secondary-4", Mode4, None),
            KeyBinding::new("secondary-5", Mode5, None),
            KeyBinding::new("secondary-k", OpenPalette, None),
            KeyBinding::new("secondary-a", SelectAll, None),
            KeyBinding::new("escape", ClearSelection, None),
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
                workspace.read(cx).focus_handle.focus(window);
                cx.new(|cx| Root::new(workspace, window, cx))
            },
        )
        .expect("could not open Spectrum");
        cx.activate(true);
    });
}
