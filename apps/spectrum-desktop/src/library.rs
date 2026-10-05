use crate::{
    controls::chip,
    store::{Store, importable},
    theme::*,
    workspace::{LibraryView, Place, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    input::Input,
    menu::DropdownMenu,
    notification::Notification,
};
use spectrum_assets::Service;
use spectrum_library::AssetId;
use std::path::PathBuf;

pub const SORTS: [&str; 3] = ["Recently added", "Name", "Kind"];

/// Files being imported, shown as placeholder cards until the batch lands.
pub struct PendingImport {
    pub token: u64,
    pub project: Option<spectrum_library::ProjectId>,
    pub names: Vec<SharedString>,
}

impl Workspace {
    pub fn view_name(&self) -> SharedString {
        match self.view {
            LibraryView::All => "All assets".into(),
            LibraryView::Unassigned => "Unassigned".into(),
            LibraryView::Trash => "Trash".into(),
            LibraryView::Project(id) => self
                .store
                .as_ref()
                .ok()
                .and_then(|s| s.project_name(id))
                .unwrap_or("Project")
                .to_string()
                .into(),
        }
    }

    pub fn asset_name(&self, id: AssetId) -> SharedString {
        self.store
            .as_ref()
            .ok()
            .and_then(|s| s.entries.iter().find(|e| e.asset.id == id))
            .map(|e| e.asset.name.clone())
            .unwrap_or_default()
            .into()
    }

    /// Reloads the current view from the library.
    pub fn reload(&mut self) -> anyhow::Result<()> {
        if let Ok(store) = &mut self.store {
            self.view = store.load(self.view)?;
            let live: Vec<AssetId> = store.entries.iter().map(|e| e.asset.id).collect();
            self.selection.retain(|id| live.contains(id));
        }
        Ok(())
    }

    pub fn show(&mut self, view: LibraryView, window: &mut Window, cx: &mut Context<Self>) {
        if self.view != view {
            self.selection.clear();
            self.anchor = None;
        }
        self.view = view;
        if let Err(error) = self.reload() {
            self.notify_error(error, window, cx);
        }
        cx.notify();
    }

    pub fn notify_error(
        &mut self,
        error: impl std::fmt::Display,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.push_notification(Notification::error(format!("{error:#}")), cx);
    }

    /// Runs a library change, then reloads the view or reports the error.
    pub fn change(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Store) -> anyhow::Result<()>,
    ) {
        let result = match &mut self.store {
            Ok(store) => f(store),
            Err(_) => Ok(()),
        }
        .and_then(|_| self.reload());
        if let Err(error) = result {
            self.notify_error(error, window, cx);
        }
        cx.notify();
    }

    pub fn import_button(&self, cx: &mut Context<Self>) -> Button {
        let importing = self.importing;
        Button::new("import")
            .primary()
            .icon(IconName::Plus)
            .label(if importing > 0 {
                format!("Importing {importing}…")
            } else {
                "Import assets".into()
            })
            .loading(importing > 0)
            .w_full()
            .on_click(cx.listener(|this, _, window, cx| this.choose_import(window, cx)))
    }

    pub fn new_project_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new("new-project")
            .primary()
            .icon(IconName::Plus)
            .label("New project")
            .w_full()
            .on_click(cx.listener(|this, _, window, cx| this.open_new_project(window, cx)))
    }

    /// Home's Assets mode: the whole library.
    pub fn library_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let buttons = self.import_button(cx).into_any_element();
        let chips = self.kind_chips(cx).into_any_element();
        let place = |id: &'static str, icon, label: &'static str, view: LibraryView| {
            let selected = self.view == view;
            div()
                .id(id)
                .h(px(34.))
                .px_2p5()
                .flex()
                .items_center()
                .gap_2p5()
                .rounded_md()
                .text_sm()
                .when(selected, |el| el.bg(rgb(SELECTED)))
                .when(!selected, |el| el.hover(|el| el.bg(rgb(HOVER))))
                .child(Icon::new(icon).small().text_color(rgb(MUTED)))
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| this.show(view, window, cx)))
        };
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(buttons)
            .child(
                Input::new(&self.search)
                    .prefix(Icon::new(IconName::Search).small().text_color(rgb(MUTED)))
                    .cleanable(true),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(place(
                        "all",
                        IconName::LayoutDashboard,
                        "All assets",
                        LibraryView::All,
                    ))
                    .child(place(
                        "unassigned",
                        IconName::Inbox,
                        "Unassigned",
                        LibraryView::Unassigned,
                    ))
                    .child(place(
                        "trash",
                        IconName::Delete,
                        "Trash",
                        LibraryView::Trash,
                    )),
            )
            .child(chips)
    }

    pub fn kind_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = |id: &'static str, label: &'static str, on: bool| {
            chip(id, label, on).on_click(cx.listener(move |this, _, _, cx| {
                if id == "images" {
                    this.show_images = !this.show_images;
                } else {
                    this.show_canvases = !this.show_canvases;
                }
                cx.notify();
            }))
        };
        div()
            .flex()
            .gap_2()
            .child(kind("images", "Images", self.show_images))
            .child(kind("canvases", "Canvases", self.show_canvases))
    }

    /// Sort order, at the right of the title row above a grid.
    pub fn grid_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        Button::new("sort")
            .ghost()
            .small()
            .label(SORTS[self.sort])
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, cx| {
                let current = view.read(cx).sort;
                SORTS.iter().enumerate().fold(menu, |menu, (index, name)| {
                    let view = view.clone();
                    menu.item(
                        gpui_component::menu::PopupMenuItem::new(*name)
                            .checked(index == current)
                            .on_click(move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.sort = index;
                                    cx.notify();
                                })
                            }),
                    )
                })
            })
    }

    pub fn choose_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Import".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| this.import_paths(paths, window, cx))
                .ok();
        })
        .detach();
    }

    /// Imports files and folders as one batch, into the open project if any.
    pub fn import_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(store) = &self.store else {
            return;
        };
        let files = importable(paths);
        if files.is_empty() {
            self.notify_error("No supported images to import.", window, cx);
            return;
        }
        let project = match self.place {
            Place::Project(id) => Some(id),
            Place::Home => None,
        };
        let root = store.root.clone();
        let count = files.len();
        self.importing += count;
        // Show the incoming files at once; real cards replace them when the
        // engine finishes, and their thumbnails fade in as they render.
        self.import_token += 1;
        let token = self.import_token;
        let names = files
            .iter()
            .map(|f| {
                f.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
                    .into()
            })
            .collect();
        self.pending_imports.push(PendingImport {
            token,
            project,
            names,
        });
        cx.notify();
        let import = cx
            .background_executor()
            .spawn(async move { Service::open(&root)?.import_into(files, project) });
        cx.spawn_in(window, async move |this, cx| {
            let result = import.await;
            this.update_in(cx, |this, window, cx| {
                this.importing -= count;
                this.pending_imports.retain(|p| p.token != token);
                if this.view == LibraryView::Trash && result.is_ok() {
                    this.view = LibraryView::Unassigned;
                }
                if let Err(error) = result.map(|_| ()).and_then(|_| this.reload()) {
                    this.notify_error(error, window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn open_new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_add.clear();
        let input = self.new_project_name.clone();
        input.update(cx, |state, cx| state.set_value("", window, cx));
        let view = cx.entity();
        window.open_dialog(cx, {
            let input = input.clone();
            move |dialog, _, _| {
                let view = view.clone();
                dialog
                    .title("New project")
                    .w(px(400.))
                    .child(Input::new(&input))
                    .confirm()
                    .button_props(DialogButtonProps::default().ok_text("Create"))
                    .on_ok(move |_, window, cx| {
                        view.update(cx, |this, cx| this.create_project(window, cx))
                    })
            }
        });
        cx.defer_in(window, move |_, window, cx| {
            input.update(cx, |state, cx| state.focus(window, cx));
        });
    }

    /// Asks for a new name, then applies `apply` to it.
    pub fn open_rename(
        &mut self,
        title: &'static str,
        current: String,
        apply: impl Fn(&mut Store, &str) -> anyhow::Result<()> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.rename_input.clone();
        input.update(cx, |state, cx| state.set_value(current, window, cx));
        let view = cx.entity();
        let apply = std::rc::Rc::new(apply);
        window.open_dialog(cx, {
            let input = input.clone();
            move |dialog, _, _| {
                let view = view.clone();
                let input = input.clone();
                let apply = apply.clone();
                dialog
                    .title(title)
                    .w(px(400.))
                    .child(Input::new(&input))
                    .confirm()
                    .button_props(DialogButtonProps::default().ok_text("Rename"))
                    .on_ok(move |_, window, cx| {
                        let name = input.read(cx).value().to_string();
                        let apply = apply.clone();
                        view.update(cx, |this, cx| {
                            this.change(window, cx, |store| apply(store, &name))
                        });
                        true
                    })
            }
        });
        cx.defer_in(window, move |_, window, cx| {
            input.update(cx, |state, cx| state.focus(window, cx));
        });
    }
}

/// A menu item label with an explanation below it.
pub fn described(label: impl Into<SharedString>, detail: &'static str, color: u32) -> Div {
    div()
        .py_0p5()
        .flex()
        .flex_col()
        .gap_0p5()
        .child(div().text_sm().text_color(rgb(color)).child(label.into()))
        .child(
            div()
                .text_xs()
                .text_color(rgb(if color == DANGER { 0xb07a7c } else { MUTED }))
                .whitespace_normal()
                .child(detail),
        )
}

pub fn empty(title: &'static str, detail: SharedString) -> Div {
    div()
        .size_full()
        .pb(px(52.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .child(div().font_weight(FontWeight::MEDIUM).child(title))
        .child(div().pb_3().text_sm().text_color(rgb(MUTED)).child(detail))
}
