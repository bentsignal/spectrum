//! Command+C and Command+V on a canvas: the selected layer goes to the system
//! clipboard in Spectrum's layer transfer format (the same JSON as
//! `spectrum canvas layer-copy`), and pasting inserts it above the selection.
use crate::workspace::Workspace;
use gpui::*;
use spectrum_canvas::{Command, LayerTransfer};

impl Workspace {
    pub fn copy_layer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.copy_picked_color(cx) {
            return;
        }
        let Some(canvas) = &self.canvas else {
            return;
        };
        let Some(id) = canvas.selected else {
            return;
        };
        match LayerTransfer::from_document(&canvas.doc, id).and_then(|t| t.to_json()) {
            Ok(json) => cx.write_to_clipboard(ClipboardItem::new_string(json)),
            Err(error) => self.notify_error(error, window, cx),
        }
    }

    pub fn paste_layer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.canvas.is_none() {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        // Other clipboard text is not a layer; ignore it quietly.
        let Ok(transfer) = LayerTransfer::from_json(&text) else {
            return;
        };
        self.canvas_commands(
            vec![Command::InsertLayer {
                transfer: Box::new(transfer),
                index: None,
            }],
            window,
            cx,
        );
    }
}
