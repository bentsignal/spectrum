//! Style > Look's Mask and clipping group: clip a layer to the one below it,
//! and show, invert, or remove its vector mask, or mask it to the selection.
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
        let selection = self.has_canvas_selection();
        let mut section = group("Mask and clipping", None).gap_3().child(
            toggle("clip-below", "Clip to layer below", clipped).on_click(cx.listener(
                move |this, _, window, cx| {
                    this.canvas_commands(
                        vec![Command::SetClipping {
                            id,
                            enabled: !clipped,
                        }],
                        window,
                        cx,
                    )
                },
            )),
        );
        let edit_mask = |edit: fn(&mut prism_core::VectorMask) -> bool| {
            cx.listener(move |this: &mut Self, _: &ClickEvent, window, cx| {
                if let Some(layer) = this.selected_layer().cloned() {
                    this.set_mask(&layer, edit, window, cx);
                }
            })
        };
        if let Some((enabled, invert)) = mask {
            section = section
                .child(
                    toggle("mask-enabled", "Show mask", enabled).on_click(edit_mask(|m| {
                        m.enabled = !m.enabled;
                        true
                    })),
                )
                .child(
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
