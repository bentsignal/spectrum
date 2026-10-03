//! Dragging rows in the layer list: a line shows where the layer will land,
//! above or below the row under the pointer. Anywhere above the list means
//! the top, anywhere below means the bottom, and the drop is taken anywhere
//! in the window.
use crate::{canvas_layers::LayerRow, workspace::Workspace};
use gpui::{prelude::*, *};
use prism_core::Command;

/// Row height plus the gap between rows.
const PITCH: f32 = 38.;

impl Workspace {
    /// The rows, with the drop line while a row is dragged.
    pub fn layer_list(&self, rows: Vec<Stateful<Div>>, cx: &mut Context<Self>) -> Stateful<Div> {
        let count = rows.len();
        let slot = self
            .canvas
            .as_ref()
            .and_then(|c| c.drop_slot)
            .filter(|_| cx.has_active_drag());
        div()
            .id("layer-list")
            .relative()
            .flex()
            .flex_col()
            .gap_0p5()
            .children(rows)
            .on_drag_move::<LayerRow>(cx.listener(
                move |this, event: &DragMoveEvent<LayerRow>, _, cx| {
                    let y = f32::from(event.event.position.y - event.bounds.top());
                    let slot = (y / PITCH).round().clamp(0., count as f32) as usize;
                    if let Some(canvas) = &mut this.canvas
                        && canvas.drop_slot != Some(slot)
                    {
                        canvas.drop_slot = Some(slot);
                        cx.notify();
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
    }

    /// Moves the dragged layer to the drop line's place.
    pub fn drop_layer(&mut self, row: &LayerRow, window: &mut Window, cx: &mut Context<Self>) {
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        let Some(slot) = canvas.drop_slot.take() else {
            return;
        };
        // The list shows the top layer first; the engine counts from the bottom.
        let count = canvas.doc.layers.len();
        let Some(from) = canvas.doc.layers.iter().position(|l| l.id == row.0) else {
            return;
        };
        let shown_from = count - 1 - from;
        let shown_to = if slot > shown_from { slot - 1 } else { slot };
        let index = count - 1 - shown_to.min(count - 1);
        if index != from {
            self.canvas_commands(vec![Command::MoveLayer { id: row.0, index }], window, cx);
        }
        cx.notify();
    }
}
