mod gallery;
mod theme;
mod views;

use gpui::{
    App, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size,
};
use gpui_component::Root;
use gpui_component_assets::Assets;

gpui::actions!(spectrum_demo, [Quit]);

fn main() {
    if std::env::args().any(|arg| arg == "--version") {
        println!("Spectrum controls demo {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            gpui::KeyBinding::new("cmd-q", Quit, None),
            gpui::KeyBinding::new("ctrl-q", Quit, None),
        ]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        gpui_component::init(cx);
        theme::apply(cx);
        let bounds = Bounds::centered(None, size(px(1180.), px(850.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(760.), px(620.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Spectrum · Controls".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let gallery = cx.new(|cx| gallery::Gallery::new(window, cx));
                cx.new(|cx| Root::new(gallery, window, cx))
            },
        )
        .expect("could not open Spectrum controls demo");
        cx.activate(true);
    });
}
