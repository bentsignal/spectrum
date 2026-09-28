//! Export: a dialog for format, quality, and size, then the save panel. The
//! choices and last folder carry over to the next export.
use crate::{
    controls::{segmented, slider_row},
    theme::*,
    workspace::{Workspace, slider},
};
use gpui::{prelude::*, *};
use gpui_component::{
    WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    input::{Input, InputState},
    notification::Notification,
    slider::SliderState,
};
use spectrum::library::{ExportOptions, Service};
use spectrum_library::AssetId;
use std::path::PathBuf;

/// Name and extension; canvases offer the first two.
const FORMATS: [(&str, &str); 4] = [
    ("JPEG", "jpg"),
    ("PNG", "png"),
    ("TIFF", "tiff"),
    ("WebP", "webp"),
];

pub struct ExportSettings {
    format: usize,
    quality: Entity<SliderState>,
    /// Limit the long edge to `edge` pixels instead of exporting at full size.
    resize: bool,
    edge: Entity<InputState>,
    folder: Option<PathBuf>,
}

impl ExportSettings {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Entity<Self> {
        let quality = slider(cx, 1., 100., 1., 92.);
        let edge = cx.new(|cx| InputState::new(window, cx).default_value("2048"));
        cx.new(|_| Self {
            format: 0,
            quality,
            resize: false,
            edge,
            folder: None,
        })
    }

    fn options(&self, cx: &App) -> ExportOptions {
        let edge = self.edge.read(cx).value().trim().parse::<u32>().ok();
        ExportOptions {
            quality: self.quality.read(cx).value().start().round() as u8,
            max_size: edge.filter(|edge| self.resize && *edge > 0),
        }
    }
}

fn label(text: &'static str) -> Div {
    div().text_xs().text_color(rgb(MUTED)).child(text)
}

impl Workspace {
    /// Opens the export dialog for an image or canvas.
    pub fn export_asset(&mut self, id: AssetId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(asset) = self
            .store
            .as_ref()
            .ok()
            .and_then(|s| s.service.library.get(id).ok())
        else {
            return;
        };
        let canvas = asset.kind == "canvas";
        let settings = self.export.clone();
        settings.update(cx, |settings, _| {
            if canvas && settings.format > 1 {
                settings.format = 0;
            }
        });
        let view = cx.entity();
        let title: SharedString = format!("Export {}", asset.name).into();
        // The dialog layer draws inside the workspace, so the dialog reads its
        // own settings entity rather than the workspace.
        window.open_dialog(cx, move |dialog, _, cx| {
            let current = settings.read(cx);
            let formats = FORMATS
                .iter()
                .take(if canvas { 2 } else { FORMATS.len() })
                .map(|(name, _)| (None, *name));
            let (pick_format, pick_size) = (settings.clone(), settings.clone());
            let quality = current.quality.read(cx).value().start();
            let format = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label("Format"))
                .child(segmented(
                    "export-format",
                    formats,
                    current.format,
                    move |index, _, cx| {
                        pick_format.update(cx, |settings, cx| {
                            settings.format = index;
                            cx.notify();
                        })
                    },
                ));
            let size = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label("Size"))
                .child(segmented(
                    "export-size",
                    [(None, "Full size"), (None, "Long edge")],
                    current.resize as usize,
                    move |index, _, cx| {
                        pick_size.update(cx, |settings, cx| {
                            settings.resize = index == 1;
                            cx.notify();
                        })
                    },
                ))
                .when(current.resize, |el| {
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().w(px(120.)).child(Input::new(&current.edge)))
                            .child(div().text_sm().text_color(rgb(MUTED)).child("pixels")),
                    )
                });
            let body = div()
                .flex()
                .flex_col()
                .gap_5()
                .child(format)
                .when(current.format == 0, |el| {
                    el.child(slider_row(
                        "Quality",
                        format!("{quality:.0}"),
                        &current.quality,
                    ))
                })
                .child(size);
            let save = view.clone();
            dialog
                .title(title.clone())
                .w(px(420.))
                .child(body)
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Export…"))
                .on_ok(move |_, window, cx| {
                    save.update(cx, |this, cx| this.choose_export_path(id, window, cx));
                    true
                })
        });
    }

    /// Asks where to save, then exports off the main thread.
    fn choose_export_path(&mut self, id: AssetId, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(store) = &self.store else {
            return;
        };
        let Ok(asset) = store.service.library.get(id) else {
            return;
        };
        let root = store.root.clone();
        let settings = self.export.read(cx);
        let options = settings.options(cx);
        let extension = FORMATS[settings.format].1;
        let directory = settings.folder.clone().unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            let pictures = home.join("Pictures");
            if pictures.is_dir() { pictures } else { home }
        });
        let stem = std::path::Path::new(&asset.name)
            .file_stem()
            .map_or(asset.name.clone(), |s| s.to_string_lossy().to_string());
        let destination = cx.prompt_for_new_path(&directory, Some(&format!("{stem}.{extension}")));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = destination.await else {
                return;
            };
            let target = path.clone();
            let export = cx
                .background_executor()
                .spawn(async move { Service::open(&root)?.export_with(id, &target, options) });
            let result = export.await;
            this.update_in(cx, |this, window, cx| {
                this.export.update(cx, |settings, _| {
                    settings.folder = path.parent().map(PathBuf::from);
                });
                match result {
                    Ok(()) => {
                        let file = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        window.push_notification(
                            Notification::success(format!("Exported {file}.")).action(
                                move |_, _, _| {
                                    let path = path.clone();
                                    Button::new("reveal-export")
                                        .ghost()
                                        .label("Show")
                                        .on_click(move |_, _, cx| cx.reveal_path(&path))
                                },
                            ),
                            cx,
                        );
                    }
                    Err(error) => this.notify_error(error, window, cx),
                }
            })
            .ok();
        })
        .detach();
    }
}
