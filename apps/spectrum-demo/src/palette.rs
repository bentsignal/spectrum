//! A search palette for choosing library views and projects, centered over
//! the content area so it never covers the sidebar.
use crate::{
    theme::*,
    workspace::{LibraryView, SIDEBAR_WIDTH, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable,
    input::{Input, InputEvent},
};

#[derive(Clone)]
enum Choice {
    View(LibraryView),
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

    pub fn palette_input(&mut self, event: &InputEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let InputEvent::Change = event {
            self.palette_index = 0;
            cx.notify();
        }
    }

    /// Views and projects matching the query, then project creation.
    fn palette_items(&self, cx: &App) -> Vec<Item> {
        let query = self.palette_query.read(cx).value().trim().to_string();
        let lower = query.to_lowercase();
        let matches = |label: &str| label.to_lowercase().contains(&lower);
        let mut items: Vec<Item> = [
            (IconName::LayoutDashboard, "All assets", LibraryView::All),
            (IconName::Inbox, "Unassigned", LibraryView::Unassigned),
            (IconName::Delete, "Trash", LibraryView::Trash),
        ]
        .into_iter()
        .filter(|(_, label, _)| matches(label))
        .map(|(icon, label, view)| Item {
            icon,
            label: label.into(),
            detail: None,
            choice: Choice::View(view),
        })
        .collect();
        if let Ok(store) = &self.store {
            items.extend(
                store
                    .projects
                    .iter()
                    .filter(|p| matches(&p.name))
                    .map(|p| Item {
                        icon: IconName::FolderClosed,
                        label: p.name.clone().into(),
                        detail: Some(format!("{}", p.assets).into()),
                        choice: Choice::View(LibraryView::Project(p.id)),
                    }),
            );
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
        }
        items
    }

    fn choose(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) {
        self.close_palette(cx);
        match choice {
            Choice::View(view) => self.show(view, window, cx),
            Choice::NewProject => self.open_new_project(window, cx),
            Choice::Create(name) => {
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
        let fixed = items
            .iter()
            .take_while(|i| {
                !matches!(
                    i.choice,
                    Choice::View(LibraryView::Project(_)) | Choice::NewProject | Choice::Create(_)
                )
            })
            .count();
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
                .when(index + 1 == fixed && fixed < count, |el| el.mb_1())
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
                                        "escape" => this.close_palette(cx),
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
