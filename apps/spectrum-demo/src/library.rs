use crate::{
    controls::{Field, group},
    samples::{self, Asset, Look},
    theme::*,
    workspace::{LibraryView, Mode, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    dialog::DialogButtonProps,
    input::Input,
    menu::{ContextMenuExt, PopupMenu, PopupMenuItem},
    slider::Slider,
};

const SORTS: [&str; 3] = ["Recently added", "Name", "Kind"];

impl Workspace {
    pub fn view_name(&self) -> SharedString {
        match self.view {
            LibraryView::All => "All assets".into(),
            LibraryView::Unassigned => "Unassigned".into(),
            LibraryView::Trash => "Trash".into(),
            LibraryView::Project(index) => self.projects[index].name.clone(),
        }
    }

    /// Assets in the current library view, in the order they were added.
    pub fn view_assets(&self) -> Vec<usize> {
        let live = |index: &usize| !self.assets[*index].trashed;
        match self.view {
            LibraryView::All => (0..self.assets.len()).filter(live).collect(),
            LibraryView::Unassigned => (0..self.assets.len())
                .filter(live)
                .filter(|index| !self.projects.iter().any(|p| p.assets.contains(index)))
                .collect(),
            LibraryView::Trash => (0..self.assets.len())
                .filter(|index| self.assets[*index].trashed)
                .collect(),
            LibraryView::Project(project) => self.projects[project]
                .assets
                .iter()
                .copied()
                .filter(live)
                .collect(),
        }
    }

    pub fn library_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let options = ["All assets".into(), "Unassigned".into(), "Trash".into()]
            .into_iter()
            .chain(self.projects.iter().map(|p| p.name.clone()))
            .collect();
        let project = Field::new("project", self.view_name())
            .split_after(2)
            .action("New project…", {
                let view = view.clone();
                move |window, cx| view.update(cx, |this, cx| this.open_new_project(window, cx))
            })
            .options(
                options,
                {
                    let view = view.clone();
                    move |cx| match view.read(cx).view {
                        LibraryView::All => 0,
                        LibraryView::Unassigned => 1,
                        LibraryView::Trash => 2,
                        LibraryView::Project(index) => index + 3,
                    }
                },
                {
                    let view = view.clone();
                    move |index, _, cx| {
                        view.update(cx, |this, cx| {
                            this.view = match index {
                                0 => LibraryView::All,
                                1 => LibraryView::Unassigned,
                                2 => LibraryView::Trash,
                                _ => LibraryView::Project(index - 3),
                            };
                            cx.notify();
                        })
                    }
                },
            );
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
        let kind = |id: &'static str, label: &'static str, checked: bool| {
            Checkbox::new(id)
                .label(label)
                .text_sm()
                .checked(checked)
                .on_click(cx.listener(move |this, checked, _, cx| {
                    if id == "images" {
                        this.show_images = *checked;
                    } else {
                        this.show_canvases = *checked;
                    }
                    cx.notify();
                }))
        };
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(
                group("View", None).child(project).child(
                    Button::new("import")
                        .primary()
                        .icon(IconName::Plus)
                        .label("Import")
                        .w_full()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.import_sample(window, cx)),
                        ),
                ),
            )
            .child(
                group("Filter", None)
                    .child(
                        Input::new(&self.search)
                            .prefix(Icon::new(IconName::Search).small().text_color(rgb(MUTED)))
                            .cleanable(true),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_5()
                            .child(kind("images", "Images", self.show_images))
                            .child(kind("canvases", "Canvases", self.show_canvases)),
                    ),
            )
            .child(group("Sort by", None).child(sort))
            .child(group("Thumbnail size", None).child(Slider::new(&self.thumbnail)))
    }

    pub fn library(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let all = self.view_assets();
        let query = self.search.read(cx).value().to_lowercase();
        let mut items: Vec<usize> = all
            .iter()
            .copied()
            .filter(|&index| {
                let asset = &self.assets[index];
                let kind = if asset.canvas {
                    self.show_canvases
                } else {
                    self.show_images
                };
                kind && asset.name.to_lowercase().contains(&query)
            })
            .collect();
        match self.sort {
            1 => items.sort_by_key(|&index| self.assets[index].name.clone()),
            2 => items.sort_by_key(|&index| {
                (!self.assets[index].canvas, self.assets[index].name.clone())
            }),
            _ => items.reverse(),
        }
        if all.is_empty() {
            let detail = match self.view {
                LibraryView::Unassigned => "Every asset belongs to a project.",
                LibraryView::Trash => "Deleted assets stay here for 30 days.",
                LibraryView::All => "Import images to start your library.",
                LibraryView::Project(_) => "Import images into this project.",
            };
            return empty("No assets", detail)
                .when(
                    matches!(self.view, LibraryView::All | LibraryView::Project(_)),
                    |el| {
                        el.child(
                            Button::new("empty-import")
                                .icon(IconName::Plus)
                                .label("Import")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.import_sample(window, cx)
                                })),
                        )
                    },
                )
                .into_any_element();
        }
        if items.is_empty() {
            return empty("No matches", "Try another search or filter.").into_any_element();
        }
        let width = self.thumbnail.read(cx).value().start();
        let placed = &self.assets[0];
        div()
            .id("library-grid")
            .size_full()
            .overflow_y_scroll()
            .px_6()
            .pt_2()
            .pb_8()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x_4()
                    .gap_y_6()
                    .children(items.into_iter().map(|index| {
                        let menu = self.asset_menu(index, cx);
                        div().id(("card", index)).child(
                            card(
                                index,
                                &self.assets[index],
                                placed,
                                width,
                                index == self.selected,
                                cx,
                            )
                            .context_menu(menu),
                        )
                    })),
            )
            .into_any_element()
    }

    /// Right-click menu for an asset card.
    fn asset_menu(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let view = cx.entity();
        let project = match self.view {
            LibraryView::Project(project) => Some(project),
            _ => None,
        };
        let others: Vec<(usize, SharedString)> = self
            .projects
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.assets.contains(&index))
            .map(|(i, p)| (i, p.name.clone()))
            .collect();
        let trash = self.view == LibraryView::Trash;
        move |menu, window, cx| {
            let menu = menu.min_w(px(280.));
            if trash {
                let view = view.clone();
                return menu.item(
                    PopupMenuItem::element(|_, _| {
                        described("Restore", "Returns it to its projects and canvases.", TEXT)
                    })
                    .on_click(move |_, _, cx| {
                        view.update(cx, |this, cx| {
                            this.assets[index].trashed = false;
                            cx.notify();
                        })
                    }),
                );
            }
            let menu = if others.is_empty() {
                menu
            } else {
                let view = view.clone();
                let others = others.clone();
                menu.submenu("Add to project", window, cx, move |menu, _, _| {
                    others.iter().fold(menu, |menu, (project, name)| {
                        let view = view.clone();
                        let project = *project;
                        menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| {
                            view.update(cx, |this, cx| {
                                this.projects[project].assets.push(index);
                                cx.notify();
                            })
                        }))
                    })
                })
            };
            let menu = match project {
                Some(project) => {
                    let view = view.clone();
                    menu.item(
                        PopupMenuItem::element(|_, _| {
                            described(
                                "Remove from project",
                                "Removes it from this project. It stays in your library.",
                                TEXT,
                            )
                        })
                        .on_click(move |_, _, cx| {
                            view.update(cx, |this, cx| {
                                this.projects[project].assets.retain(|&a| a != index);
                                cx.notify();
                            })
                        }),
                    )
                }
                None => menu,
            };
            let view = view.clone();
            menu.separator().item(
                PopupMenuItem::element(move |_, _| {
                    described(
                        "Delete asset",
                        "Moves it to the trash. Spectrum deletes it permanently after 30 days.",
                        DANGER,
                    )
                })
                .on_click(move |_, window, cx| {
                    view.update(cx, |this, cx| this.confirm_delete(index, window, cx))
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
                    .on_ok(move |_, _, cx| view.update(cx, |this, cx| this.create_project(cx)))
            }
        });
        cx.defer_in(window, move |_, window, cx| {
            input.update(cx, |state, cx| state.focus(window, cx));
        });
    }

    /// Add a placeholder image to the library, and to the project being viewed.
    fn import_sample(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.assets.len();
        let number = 1 + self
            .assets
            .iter()
            .filter(|a| a.name.starts_with("Import "))
            .count();
        let hue = (count as f32 * 0.37).fract();
        self.assets.push(Asset {
            name: format!("Import {number}").into(),
            canvas: false,
            dimensions: "4000 × 3000",
            hues: (hue, (hue + 0.45).fract()),
            look: Look::default(),
            trashed: false,
        });
        if let LibraryView::Project(project) = self.view {
            self.projects[project].assets.push(count);
        }
        self.sort = 0;
        self.select_asset(count, window, cx);
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

fn empty(title: &'static str, detail: &'static str) -> Div {
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
    index: usize,
    asset: &Asset,
    placed: &Asset,
    width: f32,
    selected: bool,
    cx: &mut Context<Workspace>,
) -> Stateful<Div> {
    div()
        .id(("asset", index))
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
                .child(
                    div()
                        .rounded(px(7.))
                        .overflow_hidden()
                        .child(samples::thumbnail(asset, placed, width)),
                ),
        )
        .child(
            div()
                .px_1()
                .flex()
                .flex_col()
                .child(div().text_sm().truncate().child(asset.name.clone()))
                .child(div().text_xs().text_color(rgb(FAINT)).child(
                    match (asset.canvas, asset.trashed) {
                        (_, true) => "30 days left",
                        (true, _) => "Canvas",
                        _ => "Image",
                    },
                )),
        )
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            this.select_asset(index, window, cx);
            if event.click_count() == 2 {
                let mode = if this.assets[index].canvas {
                    Mode::Canvas
                } else {
                    Mode::Adjust
                };
                this.set_mode(mode, cx);
            }
        }))
}
