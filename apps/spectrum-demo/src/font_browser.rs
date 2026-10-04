//! Choosing a text layer's font from the fonts installed on this computer.
//! Every family is listed in its own typeface; hovering one, or moving to it
//! with the arrow keys, shows it on the canvas at once, without applying it.
//! A click or Enter applies it (`ImportFont` then `SetTextTypography`, as
//! `spectrum canvas font-import` and `typography` do); Escape puts it back.
use crate::{theme::*, workspace::Workspace};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable,
    input::{self, Input},
};
use prism_core::{Command, Document, FontAsset, LayerKind};
use std::{path::Path, path::PathBuf, sync::Arc};

/// An installed family and the file of its regular face.
pub struct Family {
    pub name: SharedString,
    pub path: PathBuf,
    /// Has Latin letters; other fonts show boxes for English text.
    pub latin: bool,
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
            typography: prism_core::TextTypography {
                font_id: Some(font_id),
                ..typography
            },
        },
    ])
}

/// A copy of `doc` with `layer` in the font at `path`, for previews.
pub fn with_font(doc: &Document, layer: u64, path: &Path) -> anyhow::Result<Document> {
    let mut local = prism_core::Workspace::new(doc.clone(), None);
    local.execute_batch(font_commands(doc, layer, path)?)?;
    Ok(local.document)
}

/// The family a text layer uses now.
pub fn current_family(doc: &Document, typography: &prism_core::TextTypography) -> SharedString {
    typography
        .font_id
        .and_then(|id| doc.font_assets.iter().find(|f| f.id == id))
        .map_or("Ubuntu Light".into(), |font| {
            format!("{} {}", font.family, font.style).into()
        })
}

impl Workspace {
    /// Opens the browser for the selected text layer, loading the installed
    /// fonts the first time.
    pub fn open_fonts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.font_open = true;
        self.font_hover = None;
        self.font_query
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.font_query
            .update(cx, |state, cx| state.focus(window, cx));
        if self.fonts.is_none() {
            let task = cx.background_executor().spawn(async {
                let fonts = prism_core::system_fonts();
                let mut families: Vec<Family> = Vec::new();
                for font in &fonts {
                    if families.last().is_some_and(|f| f.name == font.family) {
                        continue;
                    }
                    let faces = fonts.iter().filter(|f| f.family == font.family);
                    if let Some(face) = prism_core::regular_face(faces) {
                        families.push(Family {
                            name: face.family.clone().into(),
                            path: face.path.clone(),
                            latin: face.latin,
                        });
                    }
                }
                families
            });
            cx.spawn(async move |this, cx| {
                let families = task.await;
                this.update(cx, |this, cx| {
                    this.fonts = Some(Arc::new(families));
                    this.font_highlight = this.current_font_index().unwrap_or(0);
                    this.font_scroll
                        .scroll_to_item(this.font_highlight, ScrollStrategy::Center);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        } else {
            self.font_highlight = self.current_font_index().unwrap_or(0);
            self.font_scroll
                .scroll_to_item(self.font_highlight, ScrollStrategy::Center);
        }
        cx.notify();
    }

    /// Families matching the search, as indexes into the full list.
    fn font_matches(&self, cx: &App) -> Vec<usize> {
        let Some(fonts) = &self.fonts else {
            return Vec::new();
        };
        let query = self.font_query.read(cx).value().trim().to_lowercase();
        fonts
            .iter()
            .enumerate()
            .filter(|(_, f)| query.is_empty() || f.name.to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect()
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
        self.fonts
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
        let handle = self.font_scroll.0.borrow().base_handle.clone();
        let bounds = handle.bounds();
        if !bounds.contains(&position) {
            return;
        }
        let row = f32::from(position.y - bounds.top() - handle.offset().y) / ROW;
        let hovered = self.font_matches(cx).get(row.floor() as usize).copied();
        if hovered.is_some() && hovered != self.font_hover {
            self.font_hover = hovered;
            self.preview_font(window, cx);
        }
    }

    /// Shows the hovered family, or else the highlighted one, on the canvas.
    fn preview_font(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let matches = self.font_matches(cx);
        let shown = self
            .font_hover
            .or_else(|| matches.get(self.font_highlight).copied());
        let path = shown.and_then(|i| self.fonts.as_ref()?.get(i).map(|f| f.path.clone()));
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
            .fonts
            .as_ref()
            .and_then(|f| f.get(index))
            .map(|f| f.path.clone());
        self.close_fonts(window, cx);
        let (Some(path), Some(canvas)) = (path, &self.canvas) else {
            return;
        };
        let Some(layer) = canvas.selected else {
            return;
        };
        match font_commands(&canvas.doc, layer, &path) {
            Ok(commands) => self.canvas_commands(commands, window, cx),
            Err(error) => self.notify_error(error, window, cx),
        }
    }

    /// Closes the browser and drops any preview.
    pub fn close_fonts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.font_open = false;
        self.font_hover = None;
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
        self.font_hover = None;
        self.font_highlight =
            (self.font_highlight as isize + step).clamp(0, count as isize - 1) as usize;
        self.font_scroll.scroll_to_item(
            self.font_highlight,
            if step > 0 {
                ScrollStrategy::Bottom
            } else {
                ScrollStrategy::Top
            },
        );
        self.preview_font(window, cx);
    }

    /// The Font row, and the browser below it while open.
    pub fn font_field(&self, family: SharedString, cx: &mut Context<Self>) -> Div {
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
                Icon::new(if self.font_open {
                    IconName::ChevronUp
                } else {
                    IconName::ChevronDown
                })
                .small()
                .text_color(rgb(MUTED)),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                if this.font_open {
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
            .when(self.font_open, |el| el.child(self.font_list(cx)))
    }

    fn font_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let matches = Arc::new(self.font_matches(cx));
        let fonts = self.fonts.clone();
        let (highlight, hover) = (self.font_highlight, self.font_hover);
        let blocked = self.font_blocked.clone();
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
                            el.children(note.map(|note| {
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(rgb(FAINT))
                                    .child(note)
                            }))
                        })
                        .on_hover(move |hovered, window, cx| {
                            enter.update(cx, |this, cx| {
                                if *hovered {
                                    this.font_hover = Some(index);
                                } else if this.font_hover == Some(index) {
                                    this.font_hover = None;
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
        .track_scroll(self.font_scroll.clone())
        .h(px(ROW * 9.));
        let loading = self.fonts.is_none();
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
                Input::new(&self.font_query)
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
                self.font_highlight = 0;
                self.font_hover = None;
                self.font_scroll.scroll_to_item(0, ScrollStrategy::Top);
                self.preview_font(window, cx);
            }
            input::InputEvent::PressEnter { .. } => {
                let matches = self.font_matches(cx);
                if let Some(&index) = matches.get(self.font_highlight) {
                    self.apply_font(index, window, cx);
                }
            }
            _ => {}
        }
    }
}
