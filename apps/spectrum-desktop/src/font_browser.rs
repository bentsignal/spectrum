//! Choosing a text layer's font from the fonts installed on this computer.
//! Every family is listed in its own typeface; hovering one, or moving to it
//! with the arrow keys, shows it on the canvas at once, without applying it.
//! A click or Enter applies it (`ImportFont` then `SetTextTypography`, as
//! `spectrum canvas font-import` and `typography` do); Escape puts it back.
use crate::{theme::*, workspace::Workspace};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Selectable, Sizable,
    button::{Button, ButtonVariants},
    input::{self, Input},
};
use spectrum_canvas::{Command, Document, FontAsset, LayerKind};
use std::{path::Path, path::PathBuf, sync::Arc};

/// An installed family, the file of its regular face, and all its faces.
pub struct Family {
    pub name: SharedString,
    pub path: PathBuf,
    /// Has Latin letters; other fonts show boxes for English text.
    pub latin: bool,
    /// Each weight and style, lightest first, upright before italic.
    pub faces: Vec<(SharedString, PathBuf)>,
}

const ROW: f32 = 34.;

/// The commands that give `layer` the font at `path`, embedding it if the
/// canvas does not have it yet.
pub fn font_commands(doc: &Document, layer: u64, path: &Path) -> anyhow::Result<Vec<Command>> {
    let asset = FontAsset::import(doc.next_font_id, path)?;
    let font_id = doc
        .font_assets
        .iter()
        .find(|existing| existing.content_hash == asset.content_hash)
        .map_or(doc.next_font_id, |existing| existing.id);
    let typography = doc
        .layers
        .iter()
        .find(|l| l.id == layer)
        .and_then(|l| match &l.kind {
            LayerKind::Text { typography, .. } => Some(typography.clone()),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("layer {layer} is not text"))?;
    Ok(vec![
        Command::ImportFont {
            path: path.to_owned(),
            source_name: None,
        },
        Command::SetTextTypography {
            id: layer,
            typography: spectrum_canvas::TextTypography {
                font_id: Some(font_id),
                ..typography
            },
        },
    ])
}

/// A copy of `doc` with `layer` in the font at `path`, for previews.
pub fn with_font(doc: &Document, layer: u64, path: &Path) -> anyhow::Result<Document> {
    let mut local = spectrum_canvas::Workspace::new(doc.clone());
    local.execute_batch(font_commands(doc, layer, path)?)?;
    Ok(local.document)
}

/// The font a text layer uses now: its family, style, and file name.
#[derive(Default)]
pub struct CurrentFont {
    pub family: SharedString,
    pub style: SharedString,
    pub file: String,
}

pub fn current_family(doc: &Document, typography: &spectrum_canvas::TextTypography) -> CurrentFont {
    typography
        .font_id
        .and_then(|id| doc.font_assets.iter().find(|f| f.id == id))
        .map_or(
            CurrentFont {
                family: "Ubuntu".into(),
                style: "Light".into(),
                file: String::new(),
            },
            |font| CurrentFont {
                family: font.family.clone().into(),
                style: font.style.clone().into(),
                file: font.source_name.clone(),
            },
        )
}

impl Workspace {
    /// Opens the browser for the selected text layer, loading the installed
    /// fonts the first time.
    pub fn open_fonts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.canvas_ui.font_open = true;
        self.canvas_ui.font_hover = None;
        self.canvas_ui
            .font_query
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.canvas_ui
            .font_query
            .update(cx, |state, cx| state.focus(window, cx));
        if self.canvas_ui.fonts.is_none() {
            self.ensure_fonts(cx);
        } else {
            self.canvas_ui.font_highlight = self.current_font_position(cx);
            self.canvas_ui
                .font_scroll
                .scroll_to_item(self.canvas_ui.font_highlight, ScrollStrategy::Center);
        }
        cx.notify();
    }

    /// Finds the installed fonts in the background, once.
    pub fn ensure_fonts(&mut self, cx: &mut Context<Self>) {
        if self.canvas_ui.fonts.is_some() || self.canvas_ui.fonts_requested {
            return;
        }
        self.canvas_ui.fonts_requested = true;
        let task = cx.background_executor().spawn(async {
            let fonts = spectrum_canvas::system_fonts();
            let mut families: Vec<Family> = Vec::new();
            for font in &fonts {
                if families.last().is_some_and(|f| f.name == font.family) {
                    continue;
                }
                let faces: Vec<_> = fonts.iter().filter(|f| f.family == font.family).collect();
                if let Some(face) = spectrum_canvas::regular_face(faces.iter().copied()) {
                    families.push(Family {
                        name: face.family.clone().into(),
                        path: face.path.clone(),
                        latin: face.latin,
                        faces: faces
                            .iter()
                            .map(|f| (f.style.clone().into(), f.path.clone()))
                            .collect(),
                    });
                }
            }
            families
        });
        cx.spawn(async move |this, cx| {
            let families = task.await;
            this.update(cx, |this, cx| {
                this.canvas_ui.fonts = Some(Arc::new(families));
                if this.canvas_ui.font_open {
                    this.canvas_ui.font_highlight = this.current_font_position(cx);
                    this.canvas_ui
                        .font_scroll
                        .scroll_to_item(this.canvas_ui.font_highlight, ScrollStrategy::Center);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Families matching the search, as indexes into the full list.
    fn font_matches(&self, cx: &App) -> Vec<usize> {
        let Some(fonts) = &self.canvas_ui.fonts else {
            return Vec::new();
        };
        let query = self
            .canvas_ui
            .font_query
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let lists = &self.canvas_ui.font_lists;
        let shown = |family: &Family| {
            let hidden = lists.hidden.contains(family.name.as_ref());
            let matching = query.is_empty() || family.name.to_lowercase().contains(&query);
            // Showing hidden fonts lists only them, to bring them back.
            let listed = if lists.show_hidden {
                hidden
            } else if lists.favorites_only {
                lists.favorites.contains(family.name.as_ref())
            } else {
                !hidden && (family.latin || !lists.latin_only)
            };
            matching && listed
        };
        // Favorites first, each part in name order.
        let (mut favorites, mut rest): (Vec<usize>, Vec<usize>) = fonts
            .iter()
            .enumerate()
            .filter(|(_, f)| shown(f))
            .map(|(index, _)| index)
            .partition(|index| lists.favorites.contains(fonts[*index].name.as_ref()));
        favorites.append(&mut rest);
        favorites
    }

    /// Where the layer's current family is in the list shown.
    fn current_font_position(&self, cx: &App) -> usize {
        let current = self.current_font_index();
        self.font_matches(cx)
            .iter()
            .position(|index| Some(*index) == current)
            .unwrap_or(0)
    }

    /// Favorites or hides a family, or takes it back out.
    fn mark_font(&mut self, name: SharedString, favorite: bool, cx: &mut Context<Self>) {
        let lists = &mut self.canvas_ui.font_lists;
        let set = if favorite {
            &mut lists.favorites
        } else {
            &mut lists.hidden
        };
        crate::font_lists::FontLists::toggle(set, &name);
        if lists.hidden.is_empty() {
            lists.show_hidden = false;
        }
        if lists.favorites.is_empty() {
            lists.favorites_only = false;
        }
        lists.save();
        cx.notify();
    }

    fn current_font_index(&self) -> Option<usize> {
        let layer = self.selected_layer()?;
        let LayerKind::Text { typography, .. } = &layer.kind else {
            return None;
        };
        let canvas = self.canvas.as_ref()?;
        let font = typography
            .font_id
            .and_then(|id| canvas.doc.font_assets.iter().find(|f| f.id == id))?;
        self.canvas_ui
            .fonts
            .as_ref()?
            .iter()
            .position(|f| f.name.as_ref() == font.family)
    }

    /// Hovers the family at a window position after the list scrolled under
    /// a still pointer, which moves no hover by itself.
    fn hover_font_at(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.canvas_ui.font_scroll.0.borrow().base_handle.clone();
        let bounds = handle.bounds();
        if !bounds.contains(&position) {
            return;
        }
        let row = f32::from(position.y - bounds.top() - handle.offset().y) / ROW;
        let hovered = self.font_matches(cx).get(row.floor() as usize).copied();
        if hovered.is_some() && hovered != self.canvas_ui.font_hover {
            self.canvas_ui.font_hover = hovered;
            self.preview_font(window, cx);
        }
    }

    /// Shows the hovered family, or else the highlighted one, on the canvas.
    fn preview_font(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let matches = self.font_matches(cx);
        let shown = self
            .canvas_ui
            .font_hover
            .or_else(|| matches.get(self.canvas_ui.font_highlight).copied());
        let path = shown.and_then(|i| {
            self.canvas_ui
                .fonts
                .as_ref()?
                .get(i)
                .map(|f| f.path.clone())
        });
        let Some(layer) = self.selected_layer().map(|l| l.id) else {
            return;
        };
        if let Some(canvas) = &mut self.canvas {
            let wanted = path.map(|path| (layer, path));
            if canvas.font_preview != wanted {
                canvas.font_preview = wanted;
                self.refresh_layers(window, cx);
            }
        }
        cx.notify();
    }

    /// Applies the hovered or highlighted family and closes the browser.
    fn apply_font(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let path = self
            .canvas_ui
            .fonts
            .as_ref()
            .and_then(|f| f.get(index))
            .map(|f| f.path.clone());
        self.close_fonts(window, cx);
        if let Some(path) = path {
            self.apply_font_file(&path, window, cx);
        }
    }

    /// Gives the selected text layer the font in `path`.
    fn apply_font_file(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        let Some(layer) = canvas.selected else {
            return;
        };
        match font_commands(&canvas.doc, layer, path) {
            Ok(commands) => self.canvas_commands(commands, window, cx),
            Err(error) => self.notify_error(error, window, cx),
        }
    }

    /// Closes the browser and drops any preview.
    pub fn close_fonts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.canvas_ui.font_open = false;
        self.canvas_ui.font_hover = None;
        if let Some(canvas) = &mut self.canvas
            && canvas.font_preview.take().is_some()
        {
            self.refresh_layers(window, cx);
        }
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn step_font(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.font_matches(cx).len();
        if count == 0 {
            return;
        }
        self.canvas_ui.font_hover = None;
        self.canvas_ui.font_highlight =
            (self.canvas_ui.font_highlight as isize + step).clamp(0, count as isize - 1) as usize;
        self.canvas_ui.font_scroll.scroll_to_item(
            self.canvas_ui.font_highlight,
            if step > 0 {
                ScrollStrategy::Bottom
            } else {
                ScrollStrategy::Top
            },
        );
        self.preview_font(window, cx);
    }

    /// The Font row, and the browser below it while open.
    pub fn font_field(&self, current: CurrentFont, cx: &mut Context<Self>) -> Div {
        let CurrentFont {
            family,
            style,
            file,
        } = current;
        // The family's weights and styles, to pick among: found by name, or
        // by the file the canvas embedded.
        let is_file = |path: &PathBuf| path.file_name().is_some_and(|name| *name == *file);
        let faces = self
            .canvas_ui
            .fonts
            .as_ref()
            .and_then(|fonts| {
                fonts.iter().find(|f| f.name == family).or_else(|| {
                    fonts
                        .iter()
                        .find(|f| f.faces.iter().any(|(_, p)| is_file(p)))
                })
            })
            .map(|f| f.faces.clone())
            .filter(|faces| faces.len() > 1);
        let weight = faces.map(|faces| {
            let view = cx.entity();
            let names: Vec<SharedString> = faces.iter().map(|(name, _)| name.clone()).collect();
            let current = faces
                .iter()
                .position(|(_, path)| is_file(path))
                .or_else(|| names.iter().position(|name| *name == style))
                .unwrap_or(0);
            crate::controls::Field::new("font-weight", style.clone()).options(
                names,
                move |_| current,
                move |index, window, cx| {
                    let path = faces[index].1.clone();
                    view.update(cx, |this, cx| this.apply_font_file(&path, window, cx));
                },
            )
        });
        let field = div()
            .id("font-field")
            .h(px(34.))
            .px_3()
            .flex()
            .items_center()
            .justify_between()
            .rounded_md()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .hover(|el| el.bg(rgb(HOVER)))
            .child(
                div()
                    .text_sm()
                    .truncate()
                    .font_family(family.clone())
                    .child(family),
            )
            .child(
                Icon::new(if self.canvas_ui.font_open {
                    IconName::ChevronUp
                } else {
                    IconName::ChevronDown
                })
                .small()
                .text_color(rgb(MUTED)),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                if this.canvas_ui.font_open {
                    this.close_fonts(window, cx)
                } else {
                    this.open_fonts(window, cx)
                }
            }));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(field)
            .when(self.canvas_ui.font_open, |el| el.child(self.font_list(cx)))
            .children(weight)
    }

    fn font_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let matches = Arc::new(self.font_matches(cx));
        let fonts = self.canvas_ui.fonts.clone();
        let (highlight, hover) = (self.canvas_ui.font_highlight, self.canvas_ui.font_hover);
        let blocked = self.canvas_ui.font_blocked.clone();
        let favorites = Arc::new(self.canvas_ui.font_lists.favorites.clone());
        let hidden = Arc::new(self.canvas_ui.font_lists.hidden.clone());
        let view = cx.entity();
        let count = matches.len();
        let list = uniform_list("font-list", count, move |range, _, _| {
            let Some(fonts) = &fonts else {
                return Vec::new();
            };
            range
                .map(|position| {
                    let index = matches[position];
                    let family = &fonts[index];
                    let active = hover == Some(index) || (hover.is_none() && position == highlight);
                    let (enter, pick) = (view.clone(), view.clone());
                    let (star, hide) = (view.clone(), view.clone());
                    let favorite = favorites.contains(family.name.as_ref());
                    let is_hidden = hidden.contains(family.name.as_ref());
                    let (star_name, hide_name) = (family.name.clone(), family.name.clone());
                    // Favorite and hide, shown on the row under the pointer.
                    let marks =
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .when(active || favorite, |el| {
                                el.child(
                                    Button::new(("font-star", index))
                                        .ghost()
                                        .xsmall()
                                        .icon(Icon::new(IconName::Star).text_color(rgb(
                                            if favorite { 0xe5c34b } else { MUTED },
                                        )))
                                        .tooltip(if favorite {
                                            "Remove from favorites"
                                        } else {
                                            "Favorite: list it first"
                                        })
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            star.update(cx, |this, cx| {
                                                this.mark_font(star_name.clone(), true, cx)
                                            })
                                        }),
                                )
                            })
                            .when(active || is_hidden, |el| {
                                el.child(
                                    Button::new(("font-hide", index))
                                        .ghost()
                                        .xsmall()
                                        .icon(if is_hidden {
                                            IconName::Eye
                                        } else {
                                            IconName::EyeOff
                                        })
                                        .tooltip(if is_hidden {
                                            "Show in the list"
                                        } else {
                                            "Hide"
                                        })
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            hide.update(cx, |this, cx| {
                                                this.mark_font(hide_name.clone(), false, cx)
                                            })
                                        }),
                                )
                            });
                    div()
                        .id(("font", index))
                        .w_full()
                        .h(px(ROW))
                        .px_3()
                        .flex()
                        .items_center()
                        .rounded_md()
                        .when(active, |el| el.bg(rgb(SELECTED)))
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_base()
                                .truncate()
                                .when(blocked.contains(&family.path), |el| {
                                    el.text_color(rgb(FAINT))
                                })
                                .font_family(family.name.clone())
                                .child(family.name.clone()),
                        )
                        .map(|el| {
                            // Why a font may not show the text: it can't be
                            // embedded, or it has no Latin letters.
                            let note = if blocked.contains(&family.path) {
                                Some("Can't embed")
                            } else if !family.latin {
                                Some("No Latin letters")
                            } else {
                                None
                            };
                            el.children(note.filter(|_| !active).map(|note| {
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(rgb(FAINT))
                                    .child(note)
                            }))
                        })
                        .child(marks)
                        .on_hover(move |hovered, window, cx| {
                            enter.update(cx, |this, cx| {
                                if *hovered {
                                    this.canvas_ui.font_hover = Some(index);
                                } else if this.canvas_ui.font_hover == Some(index) {
                                    this.canvas_ui.font_hover = None;
                                }
                                this.preview_font(window, cx);
                            })
                        })
                        .on_click(move |_, window, cx| {
                            pick.update(cx, |this, cx| this.apply_font(index, window, cx))
                        })
                })
                .collect()
        })
        .track_scroll(self.canvas_ui.font_scroll.clone())
        .h(px(ROW * 9.));
        let loading = self.canvas_ui.fonts.is_none();
        div()
            .id("font-browser")
            // Scrolling here scrolls the list, never the sidebar behind it,
            // and the family that comes under the pointer shows at once.
            .on_scroll_wheel(cx.listener(|_, event: &ScrollWheelEvent, window, cx| {
                cx.stop_propagation();
                let position = event.position;
                cx.defer_in(window, move |this, window, cx| {
                    this.hover_font_at(position, window, cx)
                });
            }))
            .flex()
            .flex_col()
            .gap_1()
            .p_1()
            .rounded_md()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(0x161616))
            .on_action(
                cx.listener(|this, _: &crate::NudgeDown, window, cx| this.step_font(1, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &crate::NudgeUp, window, cx| this.step_font(-1, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &input::Escape, window, cx| this.close_fonts(window, cx)),
            )
            .child(
                Input::new(&self.canvas_ui.font_query)
                    .small()
                    .prefix(Icon::new(IconName::Search).small().text_color(rgb(MUTED))),
            )
            .when(loading, |el| {
                el.child(
                    div()
                        .p_3()
                        .text_sm()
                        .text_color(rgb(FAINT))
                        .child("Finding fonts…"),
                )
            })
            .when(!loading && count == 0, |el| {
                el.child(
                    div()
                        .p_3()
                        .text_sm()
                        .text_color(rgb(FAINT))
                        .child("No fonts match."),
                )
            })
            .when(count > 0, |el| el.child(list))
            .child(self.font_filters(cx))
    }

    /// Below the list: leave out fonts without Latin letters, and show the
    /// hidden ones to bring them back.
    fn font_filters(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let lists = &self.canvas_ui.font_lists;
        let hidden = lists.hidden.len();
        div()
            .flex()
            .items_center()
            .gap_1()
            .pt_1()
            .border_t_1()
            .border_color(rgb(BORDER))
            .when(!lists.favorites.is_empty(), |el| {
                el.child(
                    Button::new("font-favorites")
                        .ghost()
                        .xsmall()
                        .label("Favorites")
                        .selected(lists.favorites_only)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let lists = &mut this.canvas_ui.font_lists;
                            lists.favorites_only = !lists.favorites_only;
                            lists.show_hidden = false;
                            lists.save();
                            cx.notify();
                        })),
                )
            })
            .child(
                Button::new("font-latin")
                    .ghost()
                    .xsmall()
                    .label("Latin only")
                    .selected(lists.latin_only)
                    .tooltip("Leave out fonts without English letters")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.canvas_ui.font_lists.latin_only =
                            !this.canvas_ui.font_lists.latin_only;
                        this.canvas_ui.font_lists.save();
                        cx.notify();
                    })),
            )
            .when(hidden > 0, |el| {
                el.child(
                    Button::new("font-show-hidden")
                        .ghost()
                        .xsmall()
                        .label(format!("Hidden ({hidden})"))
                        .selected(lists.show_hidden)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let lists = &mut this.canvas_ui.font_lists;
                            lists.show_hidden = !lists.show_hidden;
                            lists.favorites_only = false;
                            cx.notify();
                        })),
                )
            })
    }

    /// The search field changed or was submitted.
    pub fn font_query_event(
        &mut self,
        event: &input::InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            input::InputEvent::Change => {
                self.canvas_ui.font_highlight = 0;
                self.canvas_ui.font_hover = None;
                self.canvas_ui
                    .font_scroll
                    .scroll_to_item(0, ScrollStrategy::Top);
                self.preview_font(window, cx);
            }
            input::InputEvent::PressEnter { .. } => {
                let matches = self.font_matches(cx);
                if let Some(&index) = matches.get(self.canvas_ui.font_highlight) {
                    self.apply_font(index, window, cx);
                }
            }
            _ => {}
        }
    }
}
