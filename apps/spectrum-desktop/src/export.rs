//! Export: a dialog for format, quality, and resolution (a share of full
//! size, or a custom percent or width, with the height following), then
//! the save panel. The choices and last folder carry over to the next export.
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
    input::{Input, InputEvent, InputState},
    notification::Notification,
    slider::SliderState,
};
use spectrum_assets::{ExportOptions, Service};
use spectrum_library::AssetId;
use std::path::PathBuf;

/// Name and extension; canvases offer the first two.
const FORMATS: [(&str, &str); 4] = [
    ("JPEG", "jpg"),
    ("PNG", "png"),
    ("TIFF", "tiff"),
    ("WebP", "webp"),
];

/// The resolutions offered, as shares of full size; the last is custom.
const SHARES: [(&str, f32); 4] = [("100%", 1.), ("75%", 0.75), ("50%", 0.5), ("25%", 0.25)];

pub struct ExportSettings {
    format: usize,
    quality: Entity<SliderState>,
    /// An index into `SHARES`, or its length for a custom size.
    share: usize,
    /// The custom size is a width in pixels rather than a percent.
    in_pixels: bool,
    custom: Entity<InputState>,
    /// The asset's full size.
    full: (u32, u32),
    folder: Option<PathBuf>,
    _changed: Subscription,
}

impl ExportSettings {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Entity<Self> {
        let quality = slider(cx, 1., 100., 1., 92.);
        let custom = cx.new(|cx| InputState::new(window, cx).default_value("50"));
        cx.new(|cx| Self {
            format: 0,
            quality,
            share: 0,
            in_pixels: false,
            _changed: cx.subscribe(&custom, |_, _, _: &InputEvent, cx| cx.notify()),
            custom,
            full: (0, 0),
            folder: None,
        })
    }

    /// The share of full size to export at, at most all of it.
    fn scale(&self, cx: &App) -> f32 {
        if let Some((_, share)) = SHARES.get(self.share) {
            return *share;
        }
        let value = self
            .custom
            .read(cx)
            .value()
            .trim()
            .parse::<f32>()
            .unwrap_or(100.);
        let scale = if self.in_pixels {
            value / self.full.0.max(1) as f32
        } else {
            value / 100.
        };
        if scale.is_finite() {
            scale.clamp(0.001, 1.)
        } else {
            1.
        }
    }

    /// The width and height the export will have.
    fn size(&self, cx: &App) -> (u32, u32) {
        let scale = self.scale(cx);
        let side = |full: u32| ((full as f32 * scale).round() as u32).max(1);
        (side(self.full.0), side(self.full.1))
    }

    fn options(&self, cx: &App) -> ExportOptions {
        let (width, height) = self.size(cx);
        ExportOptions {
            quality: self.quality.read(cx).value().start().round() as u8,
            max_size: (self.scale(cx) < 1.).then_some(width.max(height)),
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
        let canvas = asset.kind == spectrum_library::AssetKind::Canvas;
        let full = self
            .store
            .as_ref()
            .ok()
            .and_then(|s| s.service.export_size(id).ok())
            .unwrap_or((0, 0));
        let settings = self.export.clone();
        settings.update(cx, |settings, _| {
            if canvas && settings.format > 1 {
                settings.format = 0;
            }
            settings.full = full;
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
            let pick_format = settings.clone();
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
            let (pick_share, pick_unit) = (settings.clone(), settings.clone());
            let shares = SHARES
                .iter()
                .map(|(name, _)| (None, *name))
                .chain([(None, "Custom")]);
            let (width, height) = current.size(cx);
            let size = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label("Resolution"))
                .child(segmented(
                    "export-share",
                    shares,
                    current.share,
                    move |index, _, cx| {
                        pick_share.update(cx, |settings, cx| {
                            settings.share = index;
                            cx.notify();
                        })
                    },
                ))
                .when(current.share == SHARES.len(), |el| {
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().w(px(120.)).child(Input::new(&current.custom)))
                            .child(div().w(px(170.)).child(segmented(
                                "export-unit",
                                [(None, "Percent"), (None, "Width px")],
                                current.in_pixels as usize,
                                move |index, window, cx| {
                                    pick_unit.update(cx, |settings, cx| {
                                        let pixels = index == 1;
                                        if settings.in_pixels != pixels {
                                            // Keep the size, in the new unit.
                                            let (width, _) = settings.size(cx);
                                            let percent =
                                                width as f32 * 100. / settings.full.0.max(1) as f32;
                                            let value = if pixels {
                                                width.to_string()
                                            } else {
                                                format!("{percent:.0}")
                                            };
                                            settings.in_pixels = pixels;
                                            settings.custom.update(cx, |input, cx| {
                                                input.set_value(value, window, cx)
                                            });
                                        }
                                        cx.notify();
                                    })
                                },
                            ))),
                    )
                })
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child(format!("{width} × {height} px")),
                );
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
