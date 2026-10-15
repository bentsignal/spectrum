//! The project overview's sidebar: import into the project, search, filters.
use crate::{
    theme::*,
    workspace::{Open, Place, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
    menu::{DropdownMenu, PopupMenuItem},
};

impl Workspace {
    pub fn project_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let importing = self.importing;
        let import = Button::new("import")
            .primary()
            .label(if importing > 0 {
                format!("Importing {importing}…")
            } else {
                "Import".into()
            })
            .loading(importing > 0)
            .dropdown_caret(true)
            .w_full()
            .dropdown_menu(move |menu, _, _| {
                let (files, library) = (view.clone(), view.clone());
                menu.min_w(px(256.))
                    .item(PopupMenuItem::new("From your computer…").on_click(
                        move |_, window, cx| {
                            files.update(cx, |this, cx| this.choose_import(window, cx))
                        },
                    ))
                    .item(PopupMenuItem::new("From your library…").on_click(
                        move |_, window, cx| {
                            library.update(cx, |this, cx| this.open_picker(window, cx))
                        },
                    ))
            });
        // Something new for the project: a canvas or a video.
        let made = cx.entity();
        let new = Button::new("new")
            .icon(IconName::Plus)
            .label("New")
            .dropdown_caret(true)
            .w_full()
            .dropdown_menu(move |menu, _, _| {
                let (canvas, video) = (made.clone(), made.clone());
                menu.min_w(px(200.))
                    .item(PopupMenuItem::new("Canvas").on_click(move |_, window, cx| {
                        canvas.update(cx, |this, cx| this.new_canvas(window, cx))
                    }))
                    .item(PopupMenuItem::new("Video").on_click(move |_, window, cx| {
                        video.update(cx, |this, cx| this.new_video(window, cx))
                    }))
            });
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(import))
                    .child(div().flex_1().min_w_0().child(new)),
            )
            .child(
                Input::new(&self.search)
                    .prefix(Icon::new(IconName::Search).small().text_color(rgb(MUTED)))
                    .cleanable(true),
            )
            .child(self.kind_list(cx))
    }

    /// Creates a video in the open project, shaped like its newest footage
    /// (or 1080p at 30 fps), and opens it.
    fn new_video(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Place::Project(project), Ok(store)) = (self.place, &mut self.store) else {
            return;
        };
        let service = &mut store.service;
        let shape = service
            .library
            .project_assets(project)
            .unwrap_or_default()
            .into_iter()
            .filter(|asset| asset.kind == spectrum_library::AssetKind::Footage)
            .find_map(|asset| service.media(asset.id).ok()?.0.info.video)
            .map(|video| (video.width & !1, video.height & !1, video.rate))
            .unwrap_or((1920, 1080, spectrum_media::FrameRate::new(30, 1)));
        let created = service
            .create_video("Untitled video", shape.0, shape.1, shape.2)
            .and_then(|video| {
                service.library.add_to_project(project, &[video.id])?;
                Ok(video.id)
            });
        match created {
            Ok(id) => {
                if let Err(error) = self.reload() {
                    self.notify_error(error, window, cx);
                }
                self.open_item(Open::Video(id), window, cx);
            }
            Err(error) => self.notify_error(error, window, cx),
        }
    }

    /// Creates a canvas in the open project and opens it.
    fn new_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Place::Project(project), Ok(store)) = (self.place, &mut self.store) else {
            return;
        };
        let created = store
            .service
            .create_canvas("Untitled canvas".into(), 1920, 1080)
            .and_then(|canvas| {
                store
                    .service
                    .library
                    .add_to_project(project, &[canvas.id])?;
                Ok(canvas.id)
            });
        match created {
            Ok(id) => {
                if let Err(error) = self.reload() {
                    self.notify_error(error, window, cx);
                }
                self.open_item(Open::Canvas(id), window, cx);
            }
            Err(error) => self.notify_error(error, window, cx),
        }
    }
}
