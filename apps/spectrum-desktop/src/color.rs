//! Color mode: real engine adjustments for the open image. A histogram sits
//! above one section of controls at a time, so the sidebar never stacks
//! every tool at once.
use crate::{
    color_fields::{self as fields, BAND_COLORS, BANDS, CURVES, FIELDS, GRADING, MIXER, RANGES},
    controls::{chip, group, segmented, slider_row},
    curves::CHANNELS,
    histogram,
    theme::*,
    workspace::{Open, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
};
use spectrum_assets::Service;
use spectrum_library::AssetId;

/// Signed slider readout that shows zero without a sign.
pub fn signed(value: f32, decimals: usize) -> String {
    if value.abs() < 0.005 {
        return "0".into();
    }
    format!("{:+.*}", decimals, value).replacen('-', "−", 1)
}

/// What Color mode edits.
#[derive(Clone, Copy, PartialEq)]
pub enum Target {
    Image(AssetId),
    /// A canvas layer's own adjustments.
    Layer(u64),
    /// The shared image behind a canvas layer, used everywhere it appears.
    Shared(AssetId),
}

impl Workspace {
    pub fn color_target(&self) -> Option<Target> {
        match self.open {
            Open::Image(id) => Some(Target::Image(id)),
            Open::Canvas(_) => {
                let canvas = self.canvas.as_ref()?;
                let layer = self.selected_layer()?;
                Some(match layer.image_asset {
                    Some(asset) if canvas.global => Target::Shared(asset),
                    _ => Target::Layer(layer.id),
                })
            }
            Open::Overview => None,
        }
    }

    /// Loads the selected layer's adjustments, or its image's when editing globally.
    pub fn load_layer_color(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.color_target() {
            Some(Target::Shared(asset)) => self.load_color(asset, window, cx),
            Some(Target::Layer(_)) => {
                self.image.adjust = self
                    .selected_layer()
                    .map(|l| l.adjustments.clone())
                    .unwrap_or_default();
                self.sync_color_sliders(window, cx);
            }
            _ => {}
        }
    }

    pub fn open_image(&self) -> Option<AssetId> {
        match self.open {
            Open::Image(id) => Some(id),
            _ => None,
        }
    }

    /// Loads the image's saved adjustments into the sliders.
    pub fn load_color(&mut self, id: AssetId, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(store) = &self.store else {
            return;
        };
        match store.service.image(id) {
            Ok(photo) => {
                self.image.adjust = photo.adjustments.clone();
                if self.open == Open::Image(id) {
                    self.image.edits.reset(photo.adjustments.clone());
                    self.image.photo_info = Some(photo);
                }
            }
            Err(error) => return self.notify_error(error, window, cx),
        }
        self.sync_color_sliders(window, cx);
    }

    /// Moves every slider to the model's value for the chosen band and range.
    pub fn sync_color_sliders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (field, state) in FIELDS.iter().zip(&self.image.color) {
            let value = fields::get(
                &self.image.adjust,
                field.key,
                self.image.mix_band,
                self.image.grade_range,
            );
            state.update(cx, |state, cx| state.set_value(value, window, cx));
        }
        let straighten = self.image.adjust.straighten;
        self.image
            .straighten
            .update(cx, |state, cx| state.set_value(straighten, window, cx));
        cx.notify();
    }

    /// A slider moved: write it into the model and send the model to the engine.
    pub fn color_slider_changed(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = self.image.color[index].read(cx).value().start();
        let (band, range) = (self.image.mix_band, self.image.grade_range);
        fields::set(
            &mut self.image.adjust,
            FIELDS[index].key,
            band,
            range,
            value,
        );
        self.schedule_color_edit(window, cx);
    }

    /// Shows and saves the model. Canvas layers go to the engine one edit at a
    /// time; changes made meanwhile go out together when the running one ends.
    pub fn schedule_color_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        match self.color_target() {
            Some(Target::Layer(id)) => {
                let adjustments = self.image.adjust.clone();
                return self.canvas_commands(
                    vec![spectrum_canvas::Command::SetLayerAdjustments { id, adjustments }],
                    window,
                    cx,
                );
            }
            Some(Target::Shared(asset)) => return self.edit_shared(asset, window, cx),
            _ => {}
        }
        // The open image redraws from memory now and saves once edits pause.
        self.render_preview(window, cx);
        self.schedule_save(window, cx);
    }

    /// Edits the image behind a layer; every canvas using it picks up the change.
    fn edit_shared(&mut self, asset: AssetId, window: &mut Window, cx: &mut Context<Self>) {
        if self.image.color_busy {
            self.image.color_dirty = true;
            return;
        }
        let Ok(store) = &self.store else {
            return;
        };
        let (root, adjustments) = (store.root.clone(), self.image.adjust.clone());
        // Show the edit now from the original image; the save follows.
        let original = store
            .service
            .library
            .get(asset)
            .and_then(|a| store.service.library.path(&a));
        if let (Some(canvas), Ok(original)) = (&mut self.canvas, original) {
            canvas.shared_edit = Some((asset, original, adjustments.clone()));
            self.render_canvas(window, cx);
            self.refresh_layers(window, cx);
        }
        self.image.color_busy = true;
        self.image.color_dirty = false;
        let edit = cx
            .background_executor()
            .spawn(async move { Service::open(&root)?.set_adjustments(asset, adjustments) });
        cx.spawn_in(window, async move |this, cx| {
            let result = edit.await;
            this.update_in(cx, |this, window, cx| {
                this.image.color_busy = false;
                if let Err(error) = result {
                    this.notify_error(error, window, cx);
                }
                if let Ok(store) = &mut this.store {
                    store.thumbs.remove(&asset);
                }
                if this.image.color_dirty {
                    return this.schedule_color_edit(window, cx);
                }
                // Saved: render from the image's new look again.
                if let Some(canvas) = &mut this.canvas {
                    canvas.shared_edit = None;
                }
                this.rerender_canvas(window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Steps the open image's history back or forward, then reloads it.
    pub fn step_color_history(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.open, Open::Canvas(_)) {
            let command = if forward {
                spectrum_canvas::Command::Redo
            } else {
                spectrum_canvas::Command::Undo
            };
            return self.canvas_commands(vec![command], window, cx);
        }
        self.step_image_history(forward, window, cx);
    }

    /// Reloads the open image's controls and render after an outside change.
    pub fn refresh_open_image(&mut self, id: AssetId, window: &mut Window, cx: &mut Context<Self>) {
        self.load_color(id, window, cx);
        if let Ok(store) = &mut self.store {
            store.thumbs.remove(&id);
        }
        self.render_preview(window, cx);
    }

    fn reset_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("reset-section")
            .ghost()
            .xsmall()
            .label("Reset")
            .text_color(rgb(MUTED))
            .on_click(cx.listener(|this, _, window, cx| {
                let (section, band, range) = (
                    this.image.color_section,
                    this.image.mix_band,
                    this.image.grade_range,
                );
                fields::reset_section(&mut this.image.adjust, section, band, range);
                this.sync_color_sliders(window, cx);
                this.schedule_color_edit(window, cx);
            }))
            .into_any_element()
    }

    fn sliders(&self, section: usize, cx: &App) -> Vec<Div> {
        FIELDS
            .iter()
            .zip(&self.image.color)
            .filter(|(field, _)| field.section == section)
            .map(|(field, state)| {
                let value = state.read(cx).value().start();
                slider_row(field.label, signed(value, field.decimals), state)
            })
            .collect()
    }

    fn section_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let section = self.image.color_section;
        let title = fields::SECTIONS[section];
        let reset = self.reset_button(cx);
        let view = cx.entity();
        match section {
            CURVES => group(title, Some(reset))
                .child(segmented(
                    "curve-channel",
                    CHANNELS.iter().map(|c| (None, *c)),
                    self.image.curve_channel,
                    move |index, _, cx| {
                        view.update(cx, |this, cx| {
                            this.image.curve_channel = index;
                            cx.notify();
                        })
                    },
                ))
                .child(self.curve_editor(256., cx))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(FAINT))
                        .child("Click to add a point. Drag to move it. Double-click to remove it."),
                )
                .into_any_element(),
            MIXER => {
                group(title, Some(reset))
                    .gap_4()
                    .child(div().flex().justify_between().children(
                        BAND_COLORS.iter().enumerate().map(|(index, color)| {
                            let selected = index == self.image.mix_band;
                            div()
                                .id(("band", index))
                                .size(px(26.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .border_2()
                                .border_color(if selected {
                                    rgb(0xececec).into()
                                } else {
                                    transparent_black()
                                })
                                .child(div().size(px(16.)).rounded_full().bg(rgb(*color)))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.image.mix_band = index;
                                    this.sync_color_sliders(window, cx);
                                }))
                        }),
                    ))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(BANDS[self.image.mix_band]),
                    )
                    .children(self.sliders(MIXER, cx))
                    .into_any_element()
            }
            GRADING => group(title, Some(reset))
                .gap_4()
                .child(segmented(
                    "grade-range",
                    RANGES.iter().map(|r| (None, *r)),
                    self.image.grade_range,
                    move |index, window, cx| {
                        view.update(cx, |this, cx| {
                            this.image.grade_range = index;
                            this.sync_color_sliders(window, cx);
                        })
                    },
                ))
                .children(self.sliders(GRADING, cx))
                .into_any_element(),
            _ => group(title, Some(reset))
                .gap_4()
                .children(self.sliders(section, cx))
                .into_any_element(),
        }
    }

    /// Above the sections on a canvas: which layer, and for image layers
    /// whether edits stay in this canvas or change the image everywhere.
    fn layer_scope(&self, cx: &mut Context<Self>) -> Option<Div> {
        let Open::Canvas(_) = self.open else {
            return None;
        };
        let layer = self.selected_layer()?;
        let global = self.canvas.as_ref().is_some_and(|c| c.global);
        let view = cx.entity();
        let scope = layer.image_asset.map(|asset| {
            let usage = self
                .store
                .as_ref()
                .ok()
                .and_then(|s| s.service.usage(asset).ok())
                .map_or(0, |u| u.dependents.len());
            let caption = if global {
                format!(
                    "Changes the image in every canvas that uses it ({usage}) and in the library."
                )
            } else {
                "Changes apply only to this canvas.".to_string()
            };
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(segmented(
                    "scope",
                    [(None, "Edit locally"), (None, "Edit globally")],
                    global as usize,
                    move |index, window, cx| {
                        view.update(cx, |this, cx| {
                            if let Some(canvas) = &mut this.canvas {
                                canvas.global = index == 1;
                            }
                            this.load_layer_color(window, cx);
                        })
                    },
                ))
                .child(div().text_xs().text_color(rgb(MUTED)).child(caption))
        });
        Some(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child(format!("Editing {}", layer.name)),
                )
                .children(scope),
        )
    }

    pub fn color_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if matches!(self.open, Open::Canvas(_)) && self.selected_layer().is_none() {
            return div()
                .text_sm()
                .text_color(rgb(FAINT))
                .child("Select a layer to adjust its color.");
        }
        let scope = self.layer_scope(cx);
        let histogram = self
            .image
            .preview
            .as_ref()
            .and_then(|p| p.histogram.clone());
        let body = self.section_body(cx);
        let chips: Vec<_> = fields::SECTIONS
            .iter()
            .enumerate()
            .map(|(index, name)| {
                chip(name, name, index == self.image.color_section)
                    .flex_none()
                    .px_2p5()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.image.color_section = index;
                        this.settle_next_frame();
                        cx.notify();
                    }))
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .children(scope)
            .when(matches!(self.open, Open::Image(_)), |el| {
                el.child(histogram::view(
                    histogram,
                    crate::workspace::SIDEBAR_WIDTH - 32.,
                    72.,
                ))
            })
            .child(div().flex().flex_wrap().gap_1p5().children(chips))
            .child(body)
    }

    /// The open image, fitted to the main area.
    pub fn image_view(&self, _: &mut Context<Self>) -> impl IntoElement {
        let preview = self.image.preview.as_ref();
        let image = preview.and_then(|p| p.image.clone());
        if self.image.compare {
            return div()
                .size_full()
                .p_8()
                .pb(px(24.))
                .flex()
                .gap_6()
                .child(Self::compare_half(
                    preview.and_then(|p| p.original.clone()),
                    "Original",
                ))
                .child(Self::compare_half(image, "Edited"));
        }
        let content = match image {
            Some(image) => img(image)
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None if preview.is_some_and(|p| p.failed) => div()
                .text_sm()
                .text_color(rgb(FAINT))
                .child("This image could not be rendered.")
                .into_any_element(),
            None => div().into_any_element(),
        };
        div()
            .size_full()
            .p_8()
            .pb(px(40.))
            .flex()
            .items_center()
            .justify_center()
            .child(content)
    }
}
