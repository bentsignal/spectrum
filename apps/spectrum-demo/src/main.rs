mod canvas_layers;
mod canvas_size;
mod canvas_split;
mod canvas_state;
mod canvas_style;
mod canvas_view;
mod color;
mod color_fields;
mod color_picker;
mod color_text;
mod colors;
mod compare;
mod controls;
mod crop;
mod curves;
mod edits;
mod export;
mod grid;
mod histogram;
mod home;
mod info;
mod layer_cache;
mod library;
mod marquee;
mod palette;
mod picker;
mod preview;
mod project;
mod store;
mod theme;
mod titlebar;
mod tools;
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
        ClearSelection,
        Undo,
        Redo,
        DeleteSelection,
        NudgeLeft,
        NudgeRight,
        NudgeUp,
        NudgeDown,
        NudgeLeftFar,
        NudgeRightFar,
        NudgeUpFar,
        NudgeDownFar,
        CopyEdits,
        PasteEdits,
        Section1,
        Section2,
        Section3,
        Section4,
        Section5,
        Section6,
        EditText,
        OpenTools
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
            KeyBinding::new("alt-1", Section1, None),
            KeyBinding::new("alt-2", Section2, None),
            KeyBinding::new("alt-3", Section3, None),
            KeyBinding::new("alt-4", Section4, None),
            KeyBinding::new("alt-5", Section5, None),
            KeyBinding::new("alt-6", Section6, None),
            KeyBinding::new("secondary-2", Mode2, None),
            KeyBinding::new("secondary-3", Mode3, None),
            KeyBinding::new("secondary-4", Mode4, None),
            KeyBinding::new("secondary-5", Mode5, None),
            KeyBinding::new("secondary-k", OpenPalette, None),
            KeyBinding::new("secondary-a", SelectAll, None),
            KeyBinding::new("escape", ClearSelection, None),
            KeyBinding::new("secondary-z", Undo, None),
            KeyBinding::new("secondary-shift-z", Redo, None),
            KeyBinding::new("secondary-shift-c", CopyEdits, None),
            KeyBinding::new("secondary-e", EditText, None),
            KeyBinding::new("secondary-p", OpenTools, None),
            KeyBinding::new("secondary-shift-v", PasteEdits, None),
            // Text fields bind these in their own context, which wins while typing.
            KeyBinding::new("backspace", DeleteSelection, None),
            KeyBinding::new("delete", DeleteSelection, None),
            KeyBinding::new("left", NudgeLeft, None),
            KeyBinding::new("right", NudgeRight, None),
            KeyBinding::new("up", NudgeUp, None),
            KeyBinding::new("down", NudgeDown, None),
            KeyBinding::new("shift-left", NudgeLeftFar, None),
            KeyBinding::new("shift-right", NudgeRightFar, None),
            KeyBinding::new("shift-up", NudgeUpFar, None),
            KeyBinding::new("shift-down", NudgeDownFar, None),
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
                titlebar::install(window);
                let workspace = cx.new(|cx| Workspace::new(window, cx));
                workspace.read(cx).focus_handle.focus(window);
                cx.new(|cx| Root::new(workspace, window, cx))
            },
        )
        .expect("could not open Spectrum");
        cx.activate(true);
    });
}
