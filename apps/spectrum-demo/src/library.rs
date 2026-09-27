use crate::{
    controls::{Field, group},
    samples::{self, Asset, Look},
    theme::*,
    workspace::{Mode, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    dialog::DialogButtonProps,
    input::Input,
    slider::Slider,
};

const SORTS: [&str; 3] = ["Recently added", "Name", "Kind"];

impl Workspace {
    pub fn library_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let projects = self.projects.iter().map(|p| p.name.clone()).collect();
        let project = Field::new("project", self.projects[self.project].name.clone())
            .action("New project…", {
                let view = view.clone();
                move |window, cx| view.update(cx, |this, cx| this.open_new_project(window, cx))
            })
            .options(
                projects,
                {
                    let view = view.clone();
                    move |cx| view.read(cx).project
                },
                {
                    let view = view.clone();
                    move |index, _, cx| {
                        view.update(cx, |this, cx| {
                            this.project = index;
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
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(
                group("Project", None).child(project).child(
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
                            .child(
                                Checkbox::new("photos")
                                    .label("Photos")
                                    .text_sm()
                                    .checked(self.show_photos)
                                    .on_click(cx.listener(|this, checked, _, cx| {
                                        this.show_photos = *checked;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Checkbox::new("canvases")
                                    .label("Canvases")
                                    .text_sm()
                                    .checked(self.show_canvases)
                                    .on_click(cx.listener(|this, checked, _, cx| {
                                        this.show_canvases = *checked;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(group("Sort by", None).child(sort))
            .child(group("Thumbnail size", None).child(Slider::new(&self.thumbnail)))
    }

    pub fn library(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let project = &self.projects[self.project];
        let query = self.search.read(cx).value().to_lowercase();
        let mut items: Vec<usize> = project
            .assets
            .iter()
            .copied()
            .filter(|&index| {
                let asset = &self.assets[index];
                let kind = if asset.canvas {
                    self.show_canvases
                } else {
                    self.show_photos
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
        if project.assets.is_empty() {
            return empty(
                "No assets yet",
                "Import photos and canvases into this project.",
            )
            .child(
                Button::new("empty-import")
                    .icon(IconName::Plus)
                    .label("Import")
                    .on_click(cx.listener(|this, _, window, cx| this.import_sample(window, cx))),
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
                        card(
                            index,
                            &self.assets[index],
                            placed,
                            width,
                            index == self.selected,
                            cx,
                        )
                    })),
            )
            .into_any_element()
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

    /// Add a placeholder photo to the library and the current project.
    fn import_sample(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.assets.len();
        let hue = (count as f32 * 0.37).fract();
        self.assets.push(Asset {
            name: format!("Import {}", count - 5).into(),
            canvas: false,
            dimensions: "4000 × 3000",
            hues: (hue, (hue + 0.45).fract()),
            look: Look::default(),
        });
        self.projects[self.project].assets.push(count);
        self.sort = 0;
        self.select_asset(count, window, cx);
    }
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
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(FAINT))
                        .child(if asset.canvas { "Canvas" } else { "Photo" }),
                ),
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
