//! Color mode: real engine adjustments for the open image.
use crate::{
    controls::{group, slider_row},
    store::Thumb,
    theme::*,
    workspace::{Open, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
    slider::SliderState,
};
use spectrum::library::Service;
use spectrum_library::AssetId;

/// Long edge of the open image's render.
pub const LARGE: u32 = 2048;

pub struct Field {
    pub label: &'static str,
    /// The `AdjustmentPatch` field this slider sets.
    pub key: &'static str,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub decimals: usize,
}

const fn percent(label: &'static str, key: &'static str) -> Field {
    Field {
        label,
        key,
        min: -100.,
        max: 100.,
        step: 1.,
        decimals: 0,
    }
}

/// Light first, then color; the sidebar splits them after `LIGHT`.
pub const FIELDS: [Field; 8] = [
    Field {
        label: "Exposure",
        key: "exposure",
        min: -5.,
        max: 5.,
        step: 0.05,
        decimals: 2,
    },
    percent("Contrast", "contrast"),
    percent("Highlights", "highlights"),
    percent("Shadows", "shadows"),
    percent("Temperature", "temperature"),
    percent("Tint", "tint"),
    percent("Vibrance", "vibrance"),
    percent("Saturation", "saturation"),
];
const LIGHT: usize = 4;

/// Signed slider readout that shows zero without a sign.
pub fn signed(value: f32, decimals: usize) -> String {
    if value.abs() < 0.005 {
        return "0".into();
    }
    format!("{:+.*}", decimals, value).replacen('-', "−", 1)
}

impl Workspace {
    fn open_image(&self) -> Option<AssetId> {
        match self.open {
            Open::Image(id) => Some(id),
            _ => None,
        }
    }

    /// Loads the image's current adjustments into the sliders.
    pub fn load_color(&mut self, id: AssetId, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(store) = &self.store else {
            return;
        };
        let adjustments = match store.service.image(id) {
            Ok(photo) => serde_json::to_value(&photo.adjustments).unwrap_or_default(),
            Err(error) => return self.notify_error(error, window, cx),
        };
        for (field, state) in FIELDS.iter().zip(&self.color) {
            let value = adjustments[field.key].as_f64().unwrap_or(0.) as f32;
            state.update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }

    /// Applies slider values to the engine, one edit at a time.
    pub fn schedule_color_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        if self.color_busy {
            self.color_dirty = true;
            return;
        }
        let (Some(id), Ok(store)) = (self.open_image(), &self.store) else {
            return;
        };
        let mut patch = serde_json::Map::new();
        for (field, state) in FIELDS.iter().zip(&self.color) {
            patch.insert(field.key.into(), state.read(cx).value().start().into());
        }
        let patch = match serde_json::from_value(patch.into()) {
            Ok(patch) => patch,
            Err(error) => return self.notify_error(error, window, cx),
        };
        let root = store.root.clone();
        self.color_busy = true;
        self.color_dirty = false;
        let edit = cx.background_executor().spawn(async move {
            let service = Service::open(&root)?;
            service.adjust(id, patch)?;
            service.thumbnail(id, LARGE)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = edit.await;
            this.update_in(cx, |this, window, cx| {
                this.color_busy = false;
                match result {
                    Ok(path) => {
                        if let Ok(store) = &mut this.store {
                            store.large.insert(id, Thumb::ready(path));
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

    fn color_group(
        &self,
        title: &'static str,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Div {
        let states: Vec<Entity<SliderState>> = self.color[range.clone()].to_vec();
        let reset = Button::new(title)
            .ghost()
            .xsmall()
            .label("Reset")
            .text_color(rgb(MUTED))
            .on_click(cx.listener(move |this, _, window, cx| {
                for state in &states {
                    state.update(cx, |state, cx| state.set_value(0., window, cx));
                }
                this.schedule_color_edit(window, cx);
            }))
            .into_any_element();
        group(title, Some(reset))
            .gap_4()
            .children(range.map(|index| {
                let field = &FIELDS[index];
                let state = &self.color[index];
                let value = state.read(cx).value().start();
                slider_row(field.label, signed(value, field.decimals), state)
            }))
    }

    pub fn color_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(self.color_group("Light", 0..LIGHT, cx))
            .child(self.color_group("Color", LIGHT..FIELDS.len(), cx))
    }

    /// The open image, fitted to the main area.
    pub fn image_view(&self, id: AssetId, cx: &mut Context<Self>) -> impl IntoElement {
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
        let _ = cx;
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
            .spawn(async move { Service::open(&root)?.thumbnail(id, LARGE) });
        cx.spawn(async move |this, cx| {
            let thumb = match render.await {
                Ok(path) => Thumb::ready(path),
                Err(_) => Thumb::Failed,
            };
            this.update(cx, |this, cx| {
                if let Ok(store) = &mut this.store {
                    store.large.insert(id, thumb);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
