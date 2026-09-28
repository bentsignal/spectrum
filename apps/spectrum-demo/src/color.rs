//! Color mode: real engine adjustments for the open image. A histogram sits
//! above one section of controls at a time, so the sidebar never stacks
//! every tool at once.
use crate::{
    color_fields::{self as fields, BAND_COLORS, BANDS, CURVES, FIELDS, GRADING, MIXER, RANGES},
    controls::{chip, group, segmented, slider_row},
    curves::CHANNELS,
    histogram::{self, Histogram},
    store::Thumb,
    theme::*,
    workspace::{Open, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
};
use spectrum::library::Service;
use spectrum_library::AssetId;
use std::sync::Arc;

/// Long edge of the open image's render.
pub const LARGE: u32 = 2048;

/// Signed slider readout that shows zero without a sign.
pub fn signed(value: f32, decimals: usize) -> String {
    if value.abs() < 0.005 {
        return "0".into();
    }
    format!("{:+.*}", decimals, value).replacen('-', "−", 1)
}

/// Renders the open image and measures its histogram, off the main thread.
fn render_large(
    root: &std::path::Path,
    id: AssetId,
) -> anyhow::Result<(std::path::PathBuf, Histogram)> {
    let path = Service::open(root)?.thumbnail(id, LARGE)?;
    let histogram = Histogram::from_file(&path)?;
    Ok((path, histogram))
}

impl Workspace {
    fn open_image(&self) -> Option<AssetId> {
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
            Ok(photo) => self.adjust = photo.adjustments,
            Err(error) => return self.notify_error(error, window, cx),
        }
        self.sync_color_sliders(window, cx);
    }

    /// Moves every slider to the model's value for the chosen band and range.
    pub fn sync_color_sliders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (field, state) in FIELDS.iter().zip(&self.color) {
            let value = fields::get(&self.adjust, field.key, self.mix_band, self.grade_range);
            state.update(cx, |state, cx| state.set_value(value, window, cx));
        }
        cx.notify();
    }

    /// A slider moved: write it into the model and send the model to the engine.
    pub fn color_slider_changed(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = self.color[index].read(cx).value().start();
        let (band, range) = (self.mix_band, self.grade_range);
        fields::set(&mut self.adjust, FIELDS[index].key, band, range, value);
        self.schedule_color_edit(window, cx);
    }

    /// Applies the model to the engine, one edit at a time; changes made
    /// meanwhile go out together when the running edit finishes.
    pub fn schedule_color_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        if self.color_busy {
            self.color_dirty = true;
            return;
        }
        let (Some(id), Ok(store)) = (self.open_image(), &self.store) else {
            return;
        };
        let root = store.root.clone();
        let adjustments = self.adjust.clone();
        self.color_busy = true;
        self.color_dirty = false;
        let edit = cx.background_executor().spawn(async move {
            Service::open(&root)?.set_adjustments(id, adjustments)?;
            render_large(&root, id)
        });
        self.finish_render(id, edit, window, cx);
    }

    /// Steps the open image's history back or forward, then reloads it.
    pub fn step_color_history(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(id), Ok(store)) = (self.open_image(), &self.store) else {
            return;
        };
        if self.color_busy {
            return;
        }
        if let Err(error) = store.service.step_history(id, forward) {
            return self.notify_error(error, window, cx);
        }
        self.load_color(id, window, cx);
        let root = store_root(self);
        self.color_busy = true;
        let render = cx
            .background_executor()
            .spawn(async move { render_large(&root, id) });
        self.finish_render(id, render, window, cx);
    }

    fn finish_render(
        &mut self,
        id: AssetId,
        task: Task<anyhow::Result<(std::path::PathBuf, Histogram)>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.color_busy = false;
                match result {
                    Ok((path, histogram)) => {
                        if let Ok(store) = &mut this.store {
                            store.large.insert(id, Thumb::ready(path));
                            store.histograms.insert(id, Arc::new(histogram));
                            store.thumbs.remove(&id);
                        }
                    }
                    Err(error) => this.notify_error(error, window, cx),
                }
                if this.color_dirty {
                    this.schedule_color_edit(window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn reset_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("reset-section")
            .ghost()
            .xsmall()
            .label("Reset")
            .text_color(rgb(MUTED))
            .on_click(cx.listener(|this, _, window, cx| {
                let (section, band, range) = (this.color_section, this.mix_band, this.grade_range);
                fields::reset_section(&mut this.adjust, section, band, range);
                this.sync_color_sliders(window, cx);
                this.schedule_color_edit(window, cx);
            }))
            .into_any_element()
    }

    fn sliders(&self, section: usize, cx: &App) -> Vec<Div> {
        FIELDS
            .iter()
            .zip(&self.color)
            .filter(|(field, _)| field.section == section)
            .map(|(field, state)| {
                let value = state.read(cx).value().start();
                slider_row(field.label, signed(value, field.decimals), state)
            })
            .collect()
    }

    fn section_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let section = self.color_section;
        let title = fields::SECTIONS[section];
        let reset = self.reset_button(cx);
        let view = cx.entity();
        match section {
            CURVES => group(title, Some(reset))
                .child(segmented(
                    "curve-channel",
                    CHANNELS.iter().map(|c| (None, *c)),
                    self.curve_channel,
                    move |index, _, cx| {
                        view.update(cx, |this, cx| {
                            this.curve_channel = index;
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
                            let selected = index == self.mix_band;
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
                                    this.mix_band = index;
                                    this.sync_color_sliders(window, cx);
                                }))
                        }),
                    ))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(BANDS[self.mix_band]),
                    )
                    .children(self.sliders(MIXER, cx))
                    .into_any_element()
            }
            GRADING => group(title, Some(reset))
                .gap_4()
                .child(segmented(
                    "grade-range",
                    RANGES.iter().map(|r| (None, *r)),
                    self.grade_range,
                    move |index, window, cx| {
                        view.update(cx, |this, cx| {
                            this.grade_range = index;
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

    pub fn color_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let histogram = self
            .open_image()
            .and_then(|id| self.store.as_ref().ok()?.histograms.get(&id).cloned());
        let body = self.section_body(cx);
        let chips: Vec<_> = fields::SECTIONS
            .iter()
            .enumerate()
            .map(|(index, name)| {
                chip(name, name, index == self.color_section)
                    .flex_none()
                    .px_2p5()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.color_section = index;
                        this.settle_next_frame();
                        cx.notify();
                    }))
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(histogram::view(
                histogram,
                crate::workspace::SIDEBAR_WIDTH - 32.,
                72.,
            ))
            .child(div().flex().flex_wrap().gap_1p5().children(chips))
            .child(body)
    }

    /// The open image, fitted to the main area.
    pub fn image_view(&self, id: AssetId, _: &mut Context<Self>) -> impl IntoElement {
        let large = self.store.as_ref().ok().and_then(|s| s.large.get(&id));
        let content = match large {
            Some(Thumb::Ready(path, _)) => img(path.clone())
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            Some(Thumb::Failed) => div()
                .text_sm()
                .text_color(rgb(FAINT))
                .child("This image could not be rendered.")
                .into_any_element(),
            _ => div()
                .text_sm()
                .text_color(rgb(FAINT))
                .child("Rendering…")
                .into_any_element(),
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

    /// Starts the large render for the open image if it has none.
    pub fn request_large(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Ok(store)) = (self.open_image(), &mut self.store) else {
            return;
        };
        if store.large.contains_key(&id) {
            return;
        }
        store.large.insert(id, Thumb::Loading);
        let root = store.root.clone();
        let render = cx
            .background_executor()
            .spawn(async move { render_large(&root, id) });
        cx.spawn(async move |this, cx| {
            let result = render.await;
            this.update(cx, |this, cx| {
                if let Ok(store) = &mut this.store {
                    match result {
                        Ok((path, histogram)) => {
                            store.large.insert(id, Thumb::ready(path));
                            store.histograms.insert(id, Arc::new(histogram));
                        }
                        Err(_) => {
                            store.large.insert(id, Thumb::Failed);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn store_root(workspace: &Workspace) -> std::path::PathBuf {
    workspace
        .store
        .as_ref()
        .map(|s| s.root.clone())
        .unwrap_or_default()
}
