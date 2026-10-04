//! Style > Look's Inside and mask group: put a layer inside the one below it
//! (it shows only where that layer is: clipping), take it out, and show,
//! invert, or remove its mask, or mask it to the selection.
//! Each is an engine command, as `spectrum canvas clip`, `vector-mask`, and
//! `selection mask`.
use crate::{
    controls::{chip, group, toggle},
    theme::*,
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use prism_core::{Command, Layer};

impl Workspace {
    fn set_mask(
        &mut self,
        layer: &Layer,
        edit: impl FnOnce(&mut prism_core::VectorMask) -> bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut mask) = layer.vector_mask.clone() else {
            return;
        };
        let id = layer.id;
        let keep = edit(&mut mask);
        self.canvas_commands(
            vec![Command::SetVectorMask {
                id,
                mask: keep.then_some(mask),
            }],
            window,
            cx,
        );
    }

    pub fn mask_section(&self, layer: &Layer, cx: &mut Context<Self>) -> Div {
        let id = layer.id;
        let clipped = layer.clip_to_below;
        let mask = layer.vector_mask.as_ref().map(|m| (m.enabled, m.invert));
        let painted_only = layer.vector_mask.as_ref().is_some_and(|m| m.painted_only());
        let erased = layer
            .vector_mask
            .as_ref()
            .is_some_and(|m| m.alpha.is_some());
        let selection = self.has_canvas_selection();
        let layers = self.canvas.as_ref().map_or(&[][..], |c| &c.doc.layers[..]);
        let index = layers.iter().position(|l| l.id == id);
        // The layer below, which this one can go inside.
        let below = index
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| layers.get(i))
            .map(|l| l.name.clone());
        // Layers inside this one: those directly above that are clipped.
        let holds: Vec<String> = index.map_or(Vec::new(), |i| {
            layers[i + 1..]
                .iter()
                .take_while(|l| l.clip_to_below)
                .map(|l| l.name.clone())
                .collect()
        });
        let take_out = cx.listener(move |this: &mut Self, _: &ClickEvent, window, cx| {
            this.canvas_commands(
                vec![Command::SetClipping { id, enabled: false }],
                window,
                cx,
            )
        });
        let put_in = cx.listener(move |this: &mut Self, _: &ClickEvent, window, cx| {
            this.canvas_commands(vec![Command::SetClipping { id, enabled: true }], window, cx)
        });
        let note = |text: String| div().text_xs().text_color(rgb(FAINT)).child(text);
        let mut section = group("Inside and mask", None).gap_3();
        section = match (clipped, below) {
            (true, Some(below)) => section
                .child(note(format!("Shows only where “{below}” is.")))
                .child(chip("take-out", "Take out", false).on_click(take_out)),
            (false, Some(below)) => section
                .child(
                    chip("put-inside", "Put inside the layer below", false)
                        .on_click(put_in),
                )
                .child(note(format!(
                    "Shows this layer only where “{below}” is. You can also drop it on a layer in the list."
                ))),
            _ => section,
        };
        if !holds.is_empty() {
            section = section.child(note(format!("Holds {}.", holds.join(", "))));
        }
        let edit_mask = |edit: fn(&mut prism_core::VectorMask) -> bool| {
            cx.listener(move |this: &mut Self, _: &ClickEvent, window, cx| {
                if let Some(layer) = this.selected_layer().cloned() {
                    this.set_mask(&layer, edit, window, cx);
                }
            })
        };
        if let Some((enabled, _)) = mask {
            section = section.child(toggle("mask-enabled", "Show mask", enabled).on_click(
                edit_mask(|m| {
                    m.enabled = !m.enabled;
                    true
                }),
            ));
        }
        if erased {
            section = section
                .child(note(
                    "Parts are hidden or erased; the layer is still whole underneath.".into(),
                ))
                .child(
                    chip("mask-restore", "Bring back erased parts", false).on_click(cx.listener(
                        |this, _, window, cx| {
                            let Some(layer) = this.selected_layer().cloned() else {
                                return;
                            };
                            let Some(mask) = &layer.vector_mask else {
                                return;
                            };
                            let id = layer.id;
                            let mask = mask.without_painted();
                            this.canvas_commands(
                                vec![Command::SetVectorMask { id, mask }],
                                window,
                                cx,
                            );
                        },
                    )),
                );
        }
        if let Some((_, invert)) = mask.filter(|_| !painted_only) {
            section = section.child(
                div()
                    .flex()
                    .gap_1p5()
                    .child(
                        chip("mask-invert", "Invert", invert).on_click(edit_mask(|m| {
                            m.invert = !m.invert;
                            true
                        })),
                    )
                    .child(chip("mask-remove", "Remove", false).on_click(edit_mask(|_| false))),
            );
        }
        if selection {
            section =
                section.child(chip("mask-selection", "Mask to selection", false).on_click(
                    cx.listener(|this, _, window, cx| this.mask_to_selection(window, cx)),
                ));
        } else if mask.is_none() {
            section = section.child(
                div()
                    .text_xs()
                    .text_color(rgb(FAINT))
                    .child("Select an area with the Marquee or Lasso to mask this layer to it."),
            );
        }
        section
    }
}
