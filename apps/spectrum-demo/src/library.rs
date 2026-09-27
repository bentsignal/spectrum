use crate::{
    controls::{Field, chip, group, picker},
    store::{Entry, Store, Thumb, importable},
    theme::*,
    workspace::{LibraryView, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    input::Input,
    menu::{ContextMenuExt, PopupMenu, PopupMenuItem},
    notification::Notification,
    slider::Slider,
};
use spectrum::library::Service;
use spectrum_library::AssetId;
use std::path::PathBuf;

const SORTS: [&str; 3] = ["Recently added", "Name", "Kind"];
/// Long edge of library thumbnails, enough for the largest grid size on Retina.
const THUMBNAIL: u32 = 640;

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

    /// Reloads the current view from the library.
    pub fn reload(&mut self) -> anyhow::Result<()> {
        if let Ok(store) = &mut self.store {
            self.view = store.load(self.view)?;
        }
        Ok(())
    }

    pub fn show(&mut self, view: LibraryView, window: &mut Window, cx: &mut Context<Self>) {
        self.view = view;
        self.library_selected = None;
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

    pub fn library_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let sort = Field::new("sort", SORTS[self.sort]).options(
            SORTS.map(SharedString::from).to_vec(),
            {
                let view = view.clone();
                move |cx| view.read(cx).sort
            },
            move |index, _, cx| {
                view.update(cx, |this, cx| {
                    this.sort = index;
                    cx.notify();
                })
            },
        );
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
            .flex_col()
            .gap_7()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        Button::new("import")
                            .primary()
                            .icon(IconName::Plus)
                            .label(if self.importing > 0 {
                                format!("Importing {}…", self.importing)
                            } else {
                                "Import".into()
                            })
                            .loading(self.importing > 0)
                            .w_full()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.choose_import(window, cx)),
                            ),
                    )
                    .child(
                        Input::new(&self.search)
                            .prefix(Icon::new(IconName::Search).small().text_color(rgb(MUTED)))
                            .cleanable(true),
                    )
                    .child(picker("view", self.view_name()).on_click(
                        cx.listener(|this, _, window, cx| this.open_palette(window, cx)),
                    )),
            )
            .child(
                group("Sort by", None).child(sort).child(
                    div()
                        .flex()
                        .gap_2()
                        .child(kind("images", "Images", self.show_images))
                        .child(kind("canvases", "Canvases", self.show_canvases)),
                ),
            )
            .child(group("Thumbnail size", None).child(Slider::new(&self.thumbnail)))
    }

    fn visible(&self, cx: &App) -> Vec<&Entry> {
        let Ok(store) = &self.store else {
            return Vec::new();
        };
        let query = self.search.read(cx).value().to_lowercase();
        let mut items: Vec<&Entry> = store
            .entries
            .iter()
            .filter(|e| {
                let kind = if e.asset.kind == "canvas" {
                    self.show_canvases
                } else {
                    self.show_images
                };
                kind && e.asset.name.to_lowercase().contains(&query)
            })
            .collect();
        match self.sort {
            1 => items.sort_by_key(|e| e.asset.name.to_lowercase()),
            2 => items.sort_by_key(|e| (e.asset.kind.clone(), e.asset.name.to_lowercase())),
            _ => items.sort_by_key(|e| (std::cmp::Reverse(e.added), e.asset.name.to_lowercase())),
        }
        items
    }

    pub fn library(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let store = match &self.store {
            Ok(store) => store,
            Err(error) => {
                return empty("Spectrum could not open your library", error.clone())
                    .into_any_element();
            }
        };
        if store.entries.is_empty() {
            let detail = match self.view {
                LibraryView::Unassigned => "Every asset belongs to a project.",
                LibraryView::Trash => "Deleted assets stay here for 30 days.",
                LibraryView::All => "Import images, or drop them here.",
                LibraryView::Project(_) => "Import images into this project, or drop them here.",
            };
            return empty("No assets", detail.into())
                .when(
                    matches!(self.view, LibraryView::All | LibraryView::Project(_)),
                    |el| {
                        el.child(
                            Button::new("empty-import")
                                .icon(IconName::Plus)
                                .label("Import")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.choose_import(window, cx)
                                })),
                        )
                    },
                )
                .into_any_element();
        }
        let items = self.visible(cx);
        if items.is_empty() {
            return empty("No matches", "Try another search or filter.".into()).into_any_element();
        }
        let width = self.thumbnail.read(cx).value().start();
        let cards: Vec<AnyElement> = items
            .into_iter()
            .map(|entry| {
                let id = entry.asset.id;
                let menu = self.asset_menu(id, cx);
                let selected = self.library_selected == Some(id);
                div()
                    .id(SharedString::from(format!("card-{id}")))
                    .child(
                        card(entry, store.thumbs.get(&id), width, selected, cx).context_menu(menu),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .id("library-grid")
            .size_full()
            .overflow_y_scroll()
            .px_6()
            .pt_2()
            .pb_8()
            .child(div().flex().flex_wrap().gap_x_4().gap_y_6().children(cards))
            .into_any_element()
    }

    /// Starts background renders for thumbnails the grid has not requested yet.
    pub fn request_thumbnails(&mut self, cx: &mut Context<Self>) {
        let Ok(store) = &mut self.store else {
            return;
        };
        let missing: Vec<AssetId> = store
            .entries
            .iter()
            .map(|e| e.asset.id)
            .filter(|id| !store.thumbs.contains_key(id))
            .collect();
        for id in missing {
            store.thumbs.insert(id, Thumb::Loading);
            let root = store.root.clone();
            let render = cx
                .background_executor()
                .spawn(async move { Service::open(&root)?.thumbnail(id, THUMBNAIL) });
            cx.spawn(async move |this, cx| {
                let thumb = match render.await {
                    Ok(path) => Thumb::Ready(path),
                    Err(_) => Thumb::Failed,
                };
                this.update(cx, |this, cx| {
                    if let Ok(store) = &mut this.store {
                        store.thumbs.insert(id, thumb);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    fn choose_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Imports files and folders as one batch, into the project being viewed.
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
        let project = match self.view {
            LibraryView::Project(id) => Some(id),
            _ => None,
        };
        let root = store.root.clone();
        let count = files.len();
        self.importing += count;
        cx.notify();
        let import = cx
            .background_executor()
            .spawn(async move { Service::open(&root)?.import_into(files, project) });
        cx.spawn_in(window, async move |this, cx| {
            let result = import.await;
            this.update_in(cx, |this, window, cx| {
                this.importing -= count;
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

    /// Right-click menu for an asset. Built when opened, from current state.
    fn asset_menu(
        &self,
        id: AssetId,
        cx: &mut Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let view = cx.entity();
        move |menu, window, cx| {
            let menu = menu.min_w(px(280.));
            let (current, projects, member_of) = {
                let this = view.read(cx);
                let Ok(store) = &this.store else {
                    return menu;
                };
                let member_of: Vec<_> = store
                    .service
                    .library
                    .asset_projects(id)
                    .map(|p| p.into_iter().map(|p| p.id).collect())
                    .unwrap_or_default();
                let projects: Vec<_> = store
                    .projects
                    .iter()
                    .map(|p| (p.id, SharedString::from(p.name.clone())))
                    .collect();
                (this.view, projects, member_of)
            };
            if current == LibraryView::Trash {
                let view = view.clone();
                return menu.item(
                    PopupMenuItem::element(|_, _| {
                        described("Restore", "Returns it to its projects and canvases.", TEXT)
                    })
                    .on_click(move |_, window, cx| {
                        view.update(cx, |this, cx| {
                            this.change(window, cx, |store| store.service.restore(id).map(|_| ()))
                        })
                    }),
                );
            }
            let others: Vec<_> = projects
                .into_iter()
                .filter(|(project, _)| !member_of.contains(project))
                .collect();
            let menu = if others.is_empty() {
                menu
            } else {
                let view = view.clone();
                menu.submenu("Add to project", window, cx, move |menu, _, _| {
                    others.iter().fold(menu, |menu, (project, name)| {
                        let view = view.clone();
                        let project = *project;
                        menu.item(PopupMenuItem::new(name.clone()).on_click(
                            move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.change(window, cx, |store| {
                                        store.service.library.add_to_project(project, &[id])
                                    })
                                })
                            },
                        ))
                    })
                })
            };
            let menu = match current {
                LibraryView::Project(project) => {
                    let view = view.clone();
                    menu.item(
                        PopupMenuItem::element(|_, _| {
                            described(
                                "Remove from project",
                                "Removes it from this project. It stays in your library.",
                                TEXT,
                            )
                        })
                        .on_click(move |_, window, cx| {
                            view.update(cx, |this, cx| {
                                this.change(window, cx, |store| {
                                    store.service.library.remove_from_project(project, &[id])
                                })
                            })
                        }),
                    )
                }
                _ => menu,
            };
            let view = view.clone();
            menu.separator().item(
                PopupMenuItem::element(|_, _| {
                    described(
                        "Delete asset",
                        "Moves it to the trash. Spectrum deletes it permanently after 30 days.",
                        DANGER,
                    )
                })
                .on_click(move |_, window, cx| {
                    view.update(cx, |this, cx| this.confirm_delete(id, window, cx))
                }),
            )
        }
    }

    pub fn open_new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
}

/// A menu item label with an explanation below it.
pub fn described(label: &'static str, detail: &'static str, color: u32) -> Div {
    div()
        .py_0p5()
        .flex()
        .flex_col()
        .gap_0p5()
        .child(div().text_sm().text_color(rgb(color)).child(label))
        .child(
            div()
                .text_xs()
                .text_color(rgb(if color == DANGER { 0xb07a7c } else { MUTED }))
                .whitespace_normal()
                .child(detail),
        )
}

fn empty(title: &'static str, detail: SharedString) -> Div {
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

fn card(
    entry: &Entry,
    thumb: Option<&Thumb>,
    width: f32,
    selected: bool,
    cx: &mut Context<Workspace>,
) -> Stateful<Div> {
    let id = entry.asset.id;
    let detail: SharedString = match entry.purge_after {
        Some(purge_after) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            let days = ((purge_after - now) as f32 / 86_400.).ceil().max(0.) as i64;
            format!("{days} day{} left", if days == 1 { "" } else { "s" }).into()
        }
        None if entry.asset.kind == "canvas" => "Canvas".into(),
        None => "Image".into(),
    };
    let frame = div()
        .w(px(width))
        .h(px(width * 0.75))
        .rounded(px(7.))
        .overflow_hidden()
        .bg(rgb(SURFACE));
    let frame = match thumb {
        Some(Thumb::Ready(path)) => {
            frame.child(img(path.clone()).size_full().object_fit(ObjectFit::Cover))
        }
        Some(Thumb::Failed) => frame
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(rgb(FAINT))
            .child("No preview"),
        _ => frame,
    };
    div()
        .id(SharedString::from(format!("asset-{id}")))
        .w(px(width + 8.))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .p(px(2.))
                .rounded(px(11.))
                .border_2()
                .border_color(if selected {
                    rgb(0xd6d6d6).into()
                } else {
                    transparent_black()
                })
                .when(!selected, |el| {
                    el.hover(|el| el.border_color(rgb(0x2e2e2e)))
                })
                .child(frame),
        )
        .child(
            div()
                .px_1()
                .flex()
                .flex_col()
                .child(div().text_sm().truncate().child(entry.asset.name.clone()))
                .child(div().text_xs().text_color(rgb(FAINT)).child(detail)),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.library_selected = Some(id);
            cx.notify();
        }))
}
