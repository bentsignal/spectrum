//! Adds assets already in the library to the open project.
use crate::{
    grid::frame,
    theme::*,
    workspace::{Place, SIDEBAR_WIDTH, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
};
use spectrum_library::Asset;

const TILE: f32 = 128.;

impl Workspace {
    /// Opens the picker with library assets that are not in the project yet.
    pub fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Place::Project(project), Ok(store)) = (self.place, &self.store) else {
            return;
        };
        let library = &store.service.library;
        let candidates = library.project_assets(project).and_then(|members| {
            Ok(library
                .list()?
                .into_iter()
                .filter(|a| {
                    matches!(
                        a.kind,
                        spectrum_library::AssetKind::Image | spectrum_library::AssetKind::Canvas
                    )
                })
                .filter(|a| !members.iter().any(|m| m.id == a.id))
                .collect::<Vec<Asset>>())
        });
        match candidates {
            Ok(assets) => self.picker_assets = assets,
            Err(error) => return self.notify_error(error, window, cx),
        }
        self.picker_place = false;
        self.picker_replace = None;
        self.show_picker(window, cx);
    }

    /// Opens the picker to place the project's images on the open canvas.
    pub fn open_place_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(store) = &self.store else {
            return;
        };
        self.picker_assets = store
            .entries
            .iter()
            .filter(|e| {
                e.asset.kind == spectrum_library::AssetKind::Image && e.purge_after.is_none()
            })
            .map(|e| e.asset.clone())
            .collect();
        self.picker_place = true;
        self.picker_replace = None;
        self.show_picker(window, cx);
    }

    /// Opens the picker to swap the image behind a canvas layer, which also
    /// fixes a missing-image placeholder.
    pub fn open_replace_picker(&mut self, layer: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.open_place_picker(window, cx);
        self.picker_replace = Some(layer);
    }

    fn show_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker_selected.clear();
        self.picker_anchor = None;
        self.picker_open = true;
        self.picker_query.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    pub fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker_open = false;
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn add_picked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Place::Project(project) = self.place else {
            return;
        };
        let ids = std::mem::take(&mut self.picker_selected);
        self.close_picker(window, cx);
        if let (Some(layer), Some(asset)) = (self.picker_replace.take(), ids.first()) {
            let command = spectrum_canvas::Command::LinkImage {
                id: layer,
                asset: *asset,
            };
            return self.canvas_commands(vec![command], window, cx);
        }
        if self.picker_place {
            return self.place_images(ids, window, cx);
        }
        self.change(window, cx, |store| {
            store.service.library.add_to_project(project, &ids)
        });
    }

    /// Library assets matching the picker's search, in display order.
    fn picker_matches(&self, cx: &App) -> Vec<&Asset> {
        let query = self.picker_query.read(cx).value().to_lowercase();
        self.picker_assets
            .iter()
            .filter(|a| a.name.to_lowercase().contains(&query))
            .collect()
    }

    /// Click toggles one asset; Shift-click adds the range from the last click.
    fn pick(&mut self, id: spectrum_library::AssetId, shift: bool, cx: &mut Context<Self>) {
        if self.picker_replace.is_some() {
            self.picker_selected = vec![id];
            cx.notify();
            return;
        }
        let order: Vec<_> = self.picker_matches(cx).iter().map(|a| a.id).collect();
        let range = self.picker_anchor.filter(|_| shift).and_then(|anchor| {
            let a = order.iter().position(|x| *x == anchor)?;
            let b = order.iter().position(|x| *x == id)?;
            Some(order[a.min(b)..=a.max(b)].to_vec())
        });
        match range {
            Some(range) => {
                for id in range {
                    if !self.picker_selected.contains(&id) {
                        self.picker_selected.push(id);
                    }
                }
            }
            None => {
                if let Some(index) = self.picker_selected.iter().position(|x| *x == id) {
                    self.picker_selected.remove(index);
                } else {
                    self.picker_selected.push(id);
                }
            }
        }
        self.picker_anchor = Some(id);
        cx.notify();
    }

    pub fn picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let thumbs = self.store.as_ref().ok().map(|s| &s.thumbs);
        let tiles: Vec<AnyElement> = self
            .picker_matches(cx)
            .into_iter()
            .map(|asset| {
                let id = asset.id;
                let selected = self.picker_selected.contains(&id);
                div()
                    .id(SharedString::from(format!("pick-{id}")))
                    .w(px(TILE + 8.))
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .relative()
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
                            .child(frame(thumbs.and_then(|t| t.get(&id)), TILE, TILE * 0.75))
                            .when(selected, |el| {
                                el.child(
                                    div()
                                        .absolute()
                                        .top(px(8.))
                                        .right(px(8.))
                                        .size(px(20.))
                                        .rounded_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .bg(rgb(0xececec))
                                        .child(
                                            Icon::new(IconName::Check)
                                                .xsmall()
                                                .text_color(rgb(0x141414)),
                                        ),
                                )
                            }),
                    )
                    .child(div().px_1().text_xs().truncate().child(asset.name.clone()))
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        this.pick(id, event.modifiers().shift, cx)
                    }))
                    .into_any_element()
            })
            .collect();
        let count = self.picker_selected.len();
        let empty = tiles.is_empty();
        let (left, right) = if self.sidebar_right {
            (0., SIDEBAR_WIDTH)
        } else {
            (SIDEBAR_WIDTH, 0.)
        };
        div()
            .id("picker-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .bg(hsla(0., 0., 0., 0.45))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_picker(window, cx)),
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
                    .pt(px(72.))
                    .px_6()
                    .child(
                        div()
                            .id("picker")
                            .w_full()
                            .max_w(px(760.))
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
                                |this, event: &KeyDownEvent, window, cx| {
                                    if event.keystroke.key == "escape" {
                                        this.close_picker(window, cx);
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                            .child(
                                div()
                                    .px_2()
                                    .py_1p5()
                                    .border_b_1()
                                    .border_color(rgb(BORDER))
                                    .child(
                                        Input::new(&self.picker_query).appearance(false).prefix(
                                            Icon::new(IconName::Search)
                                                .small()
                                                .text_color(rgb(MUTED)),
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .id("picker-grid")
                                    .max_h(px(440.))
                                    .overflow_y_scroll()
                                    .p_4()
                                    .when(empty, |el| {
                                        el.py_8()
                                            .flex()
                                            .justify_center()
                                            .text_sm()
                                            .text_color(rgb(FAINT))
                                            .child(if self.picker_place {
                                                "This project has no images yet."
                                            } else {
                                                "Every library asset is already in this project."
                                            })
                                    })
                                    .when(!empty, |el| {
                                        el.child(div().flex().flex_wrap().gap_3().children(tiles))
                                    }),
                            )
                            .child(
                                div()
                                    .px_4()
                                    .py_3()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .border_t_1()
                                    .border_color(rgb(BORDER))
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_sm()
                                            .text_color(rgb(MUTED))
                                            .child(format!("{count} selected")),
                                    )
                                    .child(Button::new("picker-cancel").label("Cancel").on_click(
                                        cx.listener(|this, _, window, cx| {
                                            this.close_picker(window, cx)
                                        }),
                                    ))
                                    .child(
                                        Button::new("picker-add")
                                            .primary()
                                            .label(if self.picker_replace.is_some() {
                                                "Replace image".to_string()
                                            } else if count == 1 {
                                                if self.picker_place {
                                                    "Place 1 image"
                                                } else {
                                                    "Add 1 asset"
                                                }
                                                .to_string()
                                            } else {
                                                format!(
                                                    "{} {count} {}",
                                                    if self.picker_place { "Place" } else { "Add" },
                                                    if self.picker_place {
                                                        "images"
                                                    } else {
                                                        "assets"
                                                    }
                                                )
                                            })
                                            .disabled(count == 0)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.add_picked(window, cx)
                                            })),
                                    ),
                            ),
                    ),
            )
    }
}
