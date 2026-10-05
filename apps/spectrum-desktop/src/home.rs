//! Home's Projects mode: every project, searchable, as covers.
use crate::{
    grid::{block_width, frame},
    library::{described, empty},
    theme::*,
    workspace::{Mode, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariant},
    dialog::DialogButtonProps,
    input::Input,
    menu::{ContextMenuExt, PopupMenuItem},
};
use spectrum_library::{Project, ProjectId};

const COVER: f32 = 240.;

impl Workspace {
    pub fn projects_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(self.new_project_button(cx))
            .child(
                Input::new(&self.project_search)
                    .prefix(Icon::new(IconName::Search).small().text_color(rgb(MUTED)))
                    .cleanable(true),
            )
    }

    fn matching_projects(&self, cx: &App) -> Vec<Project> {
        let query = self.project_search.read(cx).value().to_lowercase();
        self.store
            .as_ref()
            .map(|s| {
                s.projects
                    .iter()
                    .filter(|p| p.name.to_lowercase().contains(&query))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn projects_grid(&self, available: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let Ok(store) = &self.store else {
            return empty("Spectrum could not open your library", "".into()).into_any_element();
        };
        if store.projects.is_empty() {
            return empty(
                "No projects yet",
                "Projects group assets from your library for a piece of work.".into(),
            )
            .child(
                Button::new("empty-project")
                    .icon(IconName::Plus)
                    .label("New project")
                    .on_click(cx.listener(|this, _, window, cx| this.open_new_project(window, cx))),
            )
            .into_any_element();
        }
        let projects = self.matching_projects(cx);
        if projects.is_empty() {
            return empty("No matches", "Try another search.".into()).into_any_element();
        }
        let cards: Vec<AnyElement> = projects
            .into_iter()
            .map(|project| {
                let cover = store
                    .covers
                    .get(&project.id)
                    .and_then(|asset| store.thumbs.get(asset));
                self.project_card(&project, cover, cx).into_any_element()
            })
            .collect();
        div()
            .id("projects")
            .size_full()
            .overflow_y_scroll()
            .px_6()
            .pt_2()
            .pb_8()
            .child(
                div()
                    .w(px(block_width(available - 48., COVER + 8.)))
                    .mx_auto()
                    .flex()
                    .flex_wrap()
                    .gap_x_4()
                    .gap_y_6()
                    .children(cards),
            )
            .into_any_element()
    }

    /// Click selects, Shift extends from the last click, Command toggles, and
    /// a double-click opens the project, matching asset grids.
    fn click_project(
        &mut self,
        id: ProjectId,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.modifiers();
        if event.click_count() == 2 && !modifiers.shift && !modifiers.secondary() {
            return self.enter_project(id, window, cx);
        }
        let order: Vec<ProjectId> = self.matching_projects(cx).iter().map(|p| p.id).collect();
        let range = self
            .project_anchor
            .filter(|_| modifiers.shift)
            .and_then(|anchor| {
                let a = order.iter().position(|x| *x == anchor)?;
                let b = order.iter().position(|x| *x == id)?;
                Some(order[a.min(b)..=a.max(b)].to_vec())
            });
        if let Some(range) = range {
            self.project_selection = range;
        } else if modifiers.secondary() {
            if let Some(index) = self.project_selection.iter().position(|x| *x == id) {
                self.project_selection.remove(index);
            } else {
                self.project_selection.push(id);
            }
            self.project_anchor = Some(id);
        } else {
            self.project_selection = vec![id];
            self.project_anchor = Some(id);
        }
        cx.notify();
    }

    fn project_card(
        &self,
        project: &Project,
        cover: Option<&crate::store::Thumb>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = project.id;
        let name = project.name.clone();
        let count = project.assets;
        let selected = self.project_selection.contains(&id);
        let view = cx.entity();
        div()
            .id(SharedString::from(format!("project-{id}")))
            .w(px(COVER + 8.))
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
                        frame(cover, COVER, COVER * 0.75).when(cover.is_none(), |el| {
                            el.flex().items_center().justify_center().child(
                                Icon::new(IconName::FolderClosed)
                                    .large()
                                    .text_color(rgb(0x3a3a3a)),
                            )
                        }),
                    ),
            )
            .child(
                div()
                    .px_1()
                    .flex()
                    .flex_col()
                    .child(div().text_sm().truncate().child(name.clone()))
                    .child(div().text_xs().text_color(rgb(FAINT)).child(format!(
                        "{count} asset{}",
                        if count == 1 { "" } else { "s" }
                    ))),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.click_project(id, event, window, cx)
            }))
            .context_menu(move |menu, _, cx| {
                // Act on the selection when this project is part of it.
                let targets: Vec<(ProjectId, String, usize)> = {
                    let this = view.read(cx);
                    let ids = if this.project_selection.contains(&id) {
                        this.project_selection.clone()
                    } else {
                        vec![id]
                    };
                    this.store
                        .as_ref()
                        .map(|s| {
                            s.projects
                                .iter()
                                .filter(|p| ids.contains(&p.id))
                                .map(|p| (p.id, p.name.clone(), p.assets))
                                .collect()
                        })
                        .unwrap_or_default()
                };
                let rename = view.clone();
                let delete = view.clone();
                let current = name.clone();
                let label = match targets.len() {
                    1 => "Delete project".to_string(),
                    n => format!("Delete {n} projects"),
                };
                let menu = menu.min_w(px(240.));
                let menu = if targets.len() == 1 {
                    menu.item(
                        PopupMenuItem::new("Rename…").on_click(move |_, window, cx| {
                            let current = current.clone();
                            rename.update(cx, |this, cx| {
                                this.open_rename(
                                    "Rename project",
                                    current,
                                    move |store, name| {
                                        store.service.library.rename_project(id, name).map(|_| ())
                                    },
                                    window,
                                    cx,
                                )
                            })
                        }),
                    )
                    .separator()
                } else {
                    menu
                };
                menu.item(
                    PopupMenuItem::element(move |_, _| {
                        described(label.clone(), "Their assets stay in your library.", DANGER)
                    })
                    .on_click(move |_, window, cx| {
                        let targets = targets.clone();
                        delete.update(cx, |this, cx| {
                            this.confirm_delete_projects(targets, window, cx)
                        })
                    }),
                )
            })
    }

    fn confirm_delete_projects(
        &mut self,
        targets: Vec<(ProjectId, String, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        let title = match targets.as_slice() {
            [(_, name, _)] => format!("Delete project \"{name}\"?"),
            many => format!("Delete {} projects?", many.len()),
        };
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let targets = targets.clone();
            dialog
                .title(title.clone())
                .w(px(420.))
                .child(div().text_sm().text_color(rgb(MUTED)).child(
                    "Assets stay in your library. Assets in no other project become unassigned.",
                ))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(ButtonVariant::Danger),
                )
                .on_ok(move |_, window, cx| {
                    let ids: Vec<ProjectId> = targets.iter().map(|t| t.0).collect();
                    view.update(cx, |this, cx| {
                        this.project_selection.clear();
                        this.change(window, cx, |store| {
                            ids.iter()
                                .try_for_each(|id| store.service.library.delete_project(*id))
                        });
                        this.go_home(Mode::Projects, window, cx);
                    });
                    true
                })
        });
    }
}
