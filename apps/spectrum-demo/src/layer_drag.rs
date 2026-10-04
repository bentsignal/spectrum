//! Dragging rows in the layer list: a line shows where the layer will land,
//! above or below the row under the pointer. Anywhere above the list means
//! the top, anywhere below means the bottom, and the drop is taken anywhere
//! in the window. Over the middle of a row, the layer goes inside that one
//! instead: it shows only where that layer is (clipping), and dropping it
//! between rows takes it back out.
use crate::{canvas_layers::LayerRow, workspace::Workspace};
use gpui::{prelude::*, *};
use prism_core::Command;

/// Row height plus the gap between rows.
const PITCH: f32 = 38.;

impl Workspace {
    /// The rows, with the drop line while a row is dragged.
    pub fn layer_list(&self, rows: Vec<AnyElement>, cx: &mut Context<Self>) -> Stateful<Div> {
        let count = rows.len();
        let dragging = cx.has_active_drag();
        let slot = self
            .canvas
            .as_ref()
            .and_then(|c| c.drop_slot)
            .filter(|_| dragging);
        let inside = self
            .canvas
            .as_ref()
            .and_then(|c| c.drop_inside)
            .filter(|_| dragging);
        let shown: Vec<(u64, String)> = self.canvas.as_ref().map_or(Vec::new(), |c| {
            c.doc
                .layers
                .iter()
                .rev()
                .map(|l| (l.id, l.name.clone()))
                .collect()
        });
        // Each layer holding others, and those inside it, share a box: the
        // ones inside are listed above it, indented, as they draw over it.
        let boxes: Vec<(usize, usize)> = self.canvas.as_ref().map_or(Vec::new(), |c| {
            let layers = &c.doc.layers;
            (0..layers.len())
                .filter(|&i| !layers[i].clip_to_below)
                .filter_map(|i| {
                    let inside = layers[i + 1..]
                        .iter()
                        .take_while(|l| l.clip_to_below)
                        .count();
                    (inside > 0).then(|| (layers.len() - 1 - i - inside, inside + 1))
                })
                .collect()
        });
        div()
            .id("layer-list")
            .relative()
            .flex()
            .flex_col()
            .gap_0p5()
            .children(boxes.into_iter().map(|(first, rows)| {
                div()
                    .absolute()
                    .left(px(-4.))
                    .right(px(-4.))
                    .top(px(first as f32 * PITCH - 4.))
                    .h(px(rows as f32 * PITCH + 6.))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(0x333333))
                    .bg(rgb(0x1c1c1c))
            }))
            .children(rows)
            .on_drag_move::<LayerRow>(cx.listener(
                move |this, event: &DragMoveEvent<LayerRow>, _, cx| {
                    let y = f32::from(event.event.position.y - event.bounds.top());
                    let dragged = event.drag(cx).0;
                    // The middle of another row means inside it.
                    let row = (y / PITCH).floor();
                    let within = y / PITCH - row;
                    let target =
                        (row >= 0. && (row as usize) < count && (0.3..0.7).contains(&within))
                            .then_some(row as usize)
                            .filter(|&row| shown.get(row).is_some_and(|(id, _)| *id != dragged));
                    let slot = (y / PITCH).round().clamp(0., count as f32) as usize;
                    if let Some(canvas) = &mut this.canvas {
                        let (slot, inside) = match target {
                            Some(row) => (None, Some(row)),
                            None => (Some(slot), None),
                        };
                        if canvas.drop_slot != slot || canvas.drop_inside != inside {
                            canvas.drop_slot = slot;
                            canvas.drop_inside = inside;
                            cx.notify();
                        }
                    }
                },
            ))
            .children(slot.map(|slot| {
                let top = (slot as f32 * PITCH - 2.).max(-1.);
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(top))
                    .h(px(2.))
                    .rounded_full()
                    .bg(rgb(0x4a7fe0))
            }))
            .children(inside.map(|row| {
                let name = self
                    .canvas
                    .as_ref()
                    .and_then(|c| c.doc.layers.iter().rev().nth(row))
                    .map_or(String::new(), |l| l.name.clone());
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(row as f32 * PITCH))
                    .h(px(PITCH - 2.))
                    .rounded_md()
                    .border_2()
                    .border_color(rgb(0x4a7fe0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .pr_10()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x8fb3f0))
                            .child(format!("Put inside {name}")),
                    )
            }))
    }

    /// Moves the dragged layer to the drop line's place, taking it out of
    /// the layer it was inside; or puts it inside the row it was dropped on.
    pub fn drop_layer(&mut self, row: &LayerRow, window: &mut Window, cx: &mut Context<Self>) {
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        let (inside, slot) = (canvas.drop_inside.take(), canvas.drop_slot.take());
        // The list shows the top layer first; the engine counts from the bottom.
        let count = canvas.doc.layers.len();
        let Some(from) = canvas.doc.layers.iter().position(|l| l.id == row.0) else {
            return;
        };
        let id = row.0;
        let clipped = canvas.doc.layers[from].clip_to_below;
        let mut commands = Vec::new();
        if let Some(target) = inside {
            // Directly above the target, clipped to it.
            let target = count - 1 - target.min(count - 1);
            let index = if from > target { target + 1 } else { target };
            if index != from {
                commands.push(Command::MoveLayer { id, index });
            }
            commands.push(Command::SetClipping { id, enabled: true });
        } else if let Some(slot) = slot {
            let shown_from = count - 1 - from;
            let shown_to = if slot > shown_from { slot - 1 } else { slot };
            let index = count - 1 - shown_to.min(count - 1);
            if index != from {
                commands.push(Command::MoveLayer { id, index });
                if clipped {
                    commands.push(Command::SetClipping { id, enabled: false });
                }
            }
        }
        if !commands.is_empty() {
            self.canvas_commands(commands, window, cx);
        }
        cx.notify();
    }
}
