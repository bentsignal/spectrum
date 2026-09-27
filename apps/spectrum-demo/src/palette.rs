//! The Command+K palette for projects, places, and assets, centered over the
//! content area so it never covers the sidebar.
use crate::{
    theme::*,
    workspace::{LibraryView, Mode, Open, Place, SIDEBAR_WIDTH, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable, WindowExt,
    input::{Input, InputEvent},
    notification::Notification,
};
use spectrum_library::{AssetId, ProjectId};

#[derive(Clone)]
enum Choice {
    AddTo(ProjectId, SharedString),
    Home,
    Place(LibraryView),
    Enter(ProjectId),
    Open(Open),
    NewProject,
    Create(String),
}

struct Item {
    icon: IconName,
    label: SharedString,
    detail: Option<SharedString>,
    choice: Choice,
}

impl Workspace {
    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_adding.clear();
        self.show_palette(window, cx);
    }

    /// Opens the palette to choose, or create, a project for `ids`.
    pub fn open_add_to_project(
        &mut self,
        ids: Vec<AssetId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.palette_adding = ids;
        self.show_palette(window, cx);
    }

    pub fn added_notice(
        &mut self,
        count: usize,
        project: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let noun = if count == 1 { "asset" } else { "assets" };
        window.push_notification(
            Notification::success(format!("Added {count} {noun} to {project}.")),
            cx,
        );
    }

    fn show_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = true;
        self.palette_index = 0;
        self.palette_query.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    pub fn close_palette(&mut self, cx: &mut Context<Self>) {
        self.palette_open = false;
        cx.notify();
    }

    /// Returns focus to the workspace so shortcuts keep working.
    fn refocus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    pub fn palette_input(&mut self, event: &InputEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let InputEvent::Change = event {
            self.palette_index = 0;
            cx.notify();
        }
    }

    /// The open project's items, then Home's places, then projects.
    fn palette_items(&self, cx: &App) -> Vec<Item> {
        let query = self.palette_query.read(cx).value().trim().to_string();
        let lower = query.to_lowercase();
        let matches = |label: &str| label.to_lowercase().contains(&lower);
        let Ok(store) = &self.store else {
            return Vec::new();
        };
        let mut items = Vec::new();
        if !self.palette_adding.is_empty() {
            items.extend(
                store
                    .projects
                    .iter()
                    .filter(|p| self.place != Place::Project(p.id) && matches(&p.name))
                    .map(|p| Item {
                        icon: IconName::FolderClosed,
                        label: p.name.clone().into(),
                        detail: Some(
                            format!("{} asset{}", p.assets, if p.assets == 1 { "" } else { "s" })
                                .into(),
                        ),
                        choice: Choice::AddTo(p.id, p.name.clone().into()),
                    }),
            );
        } else if let Place::Project(_) = self.place {
            if matches("Overview") {
                items.push(Item {
                    icon: IconName::LayoutDashboard,
                    label: "Overview".into(),
                    detail: Some(self.view_name()),
                    choice: Choice::Open(Open::Overview),
                });
            }
            items.extend(
                store
                    .entries
                    .iter()
                    .filter(|e| e.asset.kind == "image" && matches(&e.asset.name))
                    .map(|e| Item {
                        icon: IconName::Frame,
                        label: e.asset.name.clone().into(),
                        detail: Some("Image".into()),
                        choice: Choice::Open(Open::Image(e.asset.id)),
                    }),
            );
        }
        if self.palette_adding.is_empty() {
            items.extend(
                [
                    (IconName::FolderClosed, "Projects", Choice::Home),
                    (
                        IconName::LayoutDashboard,
                        "All assets",
                        Choice::Place(LibraryView::All),
                    ),
                    (
                        IconName::Inbox,
                        "Unassigned",
                        Choice::Place(LibraryView::Unassigned),
                    ),
                    (IconName::Delete, "Trash", Choice::Place(LibraryView::Trash)),
                ]
                .into_iter()
                .filter(|(_, label, _)| matches(label))
                .map(|(icon, label, choice)| Item {
                    icon,
                    label: label.into(),
                    detail: Some("Home".into()),
                    choice,
                }),
            );
            items.extend(
                store
                    .projects
                    .iter()
                    .filter(|p| matches(&p.name))
                    .map(|p| Item {
                        icon: IconName::FolderClosed,
                        label: p.name.clone().into(),
                        detail: Some(
                            format!("{} asset{}", p.assets, if p.assets == 1 { "" } else { "s" })
                                .into(),
                        ),
                        choice: Choice::Enter(p.id),
                    }),
            );
        }
        let exact = store
            .projects
            .iter()
            .any(|p| p.name.to_lowercase() == lower);
        items.push(if query.is_empty() || exact {
            Item {
                icon: IconName::Plus,
                label: "New project…".into(),
                detail: None,
                choice: Choice::NewProject,
            }
        } else {
            Item {
                icon: IconName::Plus,
                label: format!("Create project \"{query}\"").into(),
                detail: None,
                choice: Choice::Create(query.clone()),
            }
        });
        items
    }

    fn choose(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) {
        self.close_palette(cx);
        self.refocus(window);
        match choice {
            Choice::Home => self.go_home(Mode::Projects, window, cx),
            Choice::Place(view) => {
                self.go_home(Mode::Assets, window, cx);
                self.show(view, window, cx);
            }
            Choice::Enter(id) => self.enter_project(id, window, cx),
            Choice::Open(open) => self.open_item(open, window, cx),
            Choice::AddTo(project, name) => {
                let ids = std::mem::take(&mut self.palette_adding);
                self.change(window, cx, |store| {
                    store.service.library.add_to_project(project, &ids)
                });
                self.added_notice(ids.len(), &name, window, cx);
            }
            Choice::NewProject => {
                let ids = std::mem::take(&mut self.palette_adding);
                self.open_new_project(window, cx);
                self.pending_add = ids;
            }
            Choice::Create(name) => {
                self.pending_add = std::mem::take(&mut self.palette_adding);
                self.new_project_name
                    .update(cx, |state, cx| state.set_value(name, window, cx));
                self.create_project(window, cx);
            }
        }
    }

    pub fn palette(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.palette_items(cx);
        let count = items.len();
        let selected = self.palette_index.min(count.saturating_sub(1));
        let rows = items.into_iter().enumerate().map(|(index, item)| {
            let choice = item.choice.clone();
            div()
                .id(("palette-item", index))
                .h(px(36.))
                .px_3()
                .flex()
                .items_center()
                .gap_3()
                .rounded_md()
                .text_sm()
                .when(index == selected, |el| el.bg(rgb(SELECTED)))
                .when(index != selected, |el| el.hover(|el| el.bg(rgb(HOVER))))
                .child(Icon::new(item.icon).small().text_color(rgb(MUTED)))
                .child(div().flex_1().truncate().child(item.label))
                .children(
                    item.detail
                        .map(|d| div().text_xs().text_color(rgb(FAINT)).child(d)),
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.choose(choice.clone(), window, cx)),
                )
        });
        let (left, right) = if self.sidebar_right {
            (0., SIDEBAR_WIDTH)
        } else {
            (SIDEBAR_WIDTH, 0.)
        };
        div()
            .id("palette-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .bg(hsla(0., 0., 0., 0.45))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_palette(cx)),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(left))
                    .right(px(right))
                    .flex()
                    .justify_center()
                    .items_start()
                    .pt(px(96.))
                    .px_6()
                    .child(
                        div()
                            .id("palette")
                            .w_full()
                            .max_w(px(560.))
                            .flex()
                            .flex_col()
                            .rounded_xl()
                            .border_1()
                            .border_color(rgb(0x2e2e2e))
                            .bg(rgb(0x191919))
                            .shadow_lg()
                            .overflow_hidden()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .capture_key_down(cx.listener(
                                move |this, event: &KeyDownEvent, window, cx| {
                                    match event.keystroke.key.as_str() {
                                        "down" => {
                                            this.palette_index =
                                                (selected + 1).min(count.saturating_sub(1))
                                        }
                                        "up" => this.palette_index = selected.saturating_sub(1),
                                        "escape" => {
                                            this.close_palette(cx);
                                            this.refocus(window);
                                        }
                                        "enter" => {
                                            let items = this.palette_items(cx);
                                            if let Some(item) = items.into_iter().nth(selected) {
                                                this.choose(item.choice, window, cx);
                                            }
                                        }
                                        _ => return,
                                    }
                                    cx.stop_propagation();
                                    cx.notify();
                                },
                            ))
                            .when(!self.palette_adding.is_empty(), |el| {
                                let n = self.palette_adding.len();
                                el.child(
                                    div().px_4().pt_3().text_xs().text_color(rgb(MUTED)).child(
                                        format!(
                                            "Add {n} asset{} to a project",
                                            if n == 1 { "" } else { "s" }
                                        ),
                                    ),
                                )
                            })
                            .child(
                                div()
                                    .px_2()
                                    .py_1p5()
                                    .border_b_1()
                                    .border_color(rgb(BORDER))
                                    .child(
                                        Input::new(&self.palette_query).appearance(false).prefix(
                                            Icon::new(IconName::Search)
                                                .small()
                                                .text_color(rgb(MUTED)),
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .id("palette-list")
                                    .max_h(px(380.))
                                    .overflow_y_scroll()
                                    .p_1p5()
                                    .flex()
                                    .flex_col()
                                    .gap_0p5()
                                    .children(rows),
                            )
                            .child(
                                div()
                                    .px_4()
                                    .py_2()
                                    .border_t_1()
                                    .border_color(rgb(BORDER))
                                    .text_xs()
                                    .text_color(rgb(FAINT))
                                    .child("↑↓ to move · Enter to open · Esc to close"),
                            ),
                    ),
            )
    }
}
