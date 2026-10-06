//! Info mode: an image's capture details and where it is used.
use crate::{controls::group, theme::*, workspace::Workspace};
use gpui::{prelude::*, *};

fn row(label: &'static str, value: String) -> Div {
    div()
        .flex()
        .justify_between()
        .gap_4()
        .text_sm()
        .child(div().text_color(rgb(MUTED)).child(label))
        .child(div().truncate().child(value))
}

impl Workspace {
    pub fn info_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(photo) = &self.image.photo_info else {
            return div();
        };
        let meta = &photo.metadata;
        let dash = || "—".to_string();
        let camera = match (&meta.camera_make, &meta.camera_model) {
            (Some(make), Some(model)) if model.starts_with(make.as_str()) => model.clone(),
            (Some(make), Some(model)) => format!("{make} {model}"),
            (make, model) => make.clone().or(model.clone()).unwrap_or_else(dash),
        };
        let shutter = meta.shutter_seconds.map_or_else(dash, |s| {
            if s < 1. {
                format!("1/{:.0} s", 1. / s)
            } else {
                format!("{s:.1} s")
            }
        });
        let usage = self.color_target().and_then(|target| match target {
            crate::color::Target::Image(id) => self.store.as_ref().ok()?.service.usage(id).ok(),
            _ => None,
        });
        let projects: Vec<String> = usage
            .as_ref()
            .map(|u| u.projects.iter().map(|p| p.name.clone()).collect())
            .unwrap_or_default();
        let canvases: Vec<String> = usage
            .as_ref()
            .map(|u| u.dependents.iter().map(|a| a.name.clone()).collect())
            .unwrap_or_default();
        let list = |items: Vec<String>, none: &'static str| {
            div().flex().flex_col().gap_1().text_sm().map(|el| {
                if items.is_empty() {
                    el.child(div().text_color(rgb(FAINT)).child(none))
                } else {
                    el.children(items)
                }
            })
        };
        let _ = cx;
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(
                group("Image", None)
                    .gap_2()
                    .child(row("Size", format!("{} × {}", photo.width, photo.height)))
                    .child(row(
                        "Format",
                        if photo.format.is_empty() {
                            dash()
                        } else {
                            photo.format.to_uppercase()
                        },
                    )),
            )
            .child(
                group("Capture", None)
                    .gap_2()
                    .child(row("Camera", camera))
                    .child(row("Lens", meta.lens.clone().unwrap_or_else(dash)))
                    .child(row(
                        "Focal length",
                        meta.focal_length_mm
                            .map_or_else(dash, |f| format!("{f:.0} mm")),
                    ))
                    .child(row(
                        "Aperture",
                        meta.aperture.map_or_else(dash, |a| format!("f/{a:.1}")),
                    ))
                    .child(row("Shutter", shutter))
                    .child(row("ISO", meta.iso.map_or_else(dash, |i| i.to_string())))
                    .child(row(
                        "Captured",
                        meta.captured_at.clone().unwrap_or_else(dash),
                    )),
            )
            .child(group("Projects", None).child(list(projects, "In no project")))
            .child(group("Used in", None).child(list(canvases, "No canvases use it")))
    }
}
