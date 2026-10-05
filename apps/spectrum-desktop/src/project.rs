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
            .icon(IconName::Plus)
            .label(if importing > 0 {
                format!("Importing {importing}…")
            } else {
                "Import assets".into()
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
        let chips = self.kind_chips(cx).into_any_element();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(import)
            .child(
                Input::new(&self.search)
                    .prefix(Icon::new(IconName::Search).small().text_color(rgb(MUTED)))
                    .cleanable(true),
            )
            .child(chips)
            .child(
                Button::new("new-canvas")
                    .icon(IconName::Plus)
                    .label("New canvas")
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| this.new_canvas(window, cx))),
            )
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
