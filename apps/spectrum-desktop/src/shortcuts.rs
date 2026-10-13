//! The window's shortcuts: modes and sections, selection, nudging (or
//! frame steps in the player), undo, the palette, and the one-key tools.
use crate::{
    ClearSelection, CopyEdits, CopyLayer, DeleteSelection, Deselect, EditText, InvertSelection,
    Mode1, Mode2, Mode3, Mode4, Mode5, Mode6, NudgeDown, NudgeDownFar, NudgeLeft, NudgeLeftFar,
    NudgeRight, NudgeRightFar, NudgeUp, NudgeUpFar, OpenPalette, OpenTools, PasteEdits, PasteLayer,
    Redo, Section1, Section2, Section3, Section4, Section5, Section6, SelectAll, ToggleGuides,
    ToggleSnapping, Undo,
    workspace::{Open, Place, Workspace},
};
use gpui::{prelude::*, *};

impl Workspace {
    pub(crate) fn shortcuts(&self, el: Div, cx: &mut Context<Self>) -> Div {
        el.on_action(cx.listener(|this, _: &Mode1, window, cx| this.nth_mode(0, window, cx)))
            .on_action(cx.listener(|this, _: &EditText, window, cx| this.edit_text(window, cx)))
            .on_action(cx.listener(|this, _: &OpenTools, window, cx| this.open_tools(window, cx)))
            .on_action(cx.listener(|this, _: &CopyLayer, window, cx| this.copy_layer(window, cx)))
            .on_action(cx.listener(|this, _: &PasteLayer, window, cx| this.paste_layer(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleGuides, _, cx| this.toggle_guides(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleSnapping, window, cx| {
                    this.toggle_snapping(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &Section1, _, cx| this.nth_section(0, cx)))
            .on_action(cx.listener(|this, _: &Section2, _, cx| this.nth_section(1, cx)))
            .on_action(cx.listener(|this, _: &Section3, _, cx| this.nth_section(2, cx)))
            .on_action(cx.listener(|this, _: &Section4, _, cx| this.nth_section(3, cx)))
            .on_action(cx.listener(|this, _: &Section5, _, cx| this.nth_section(4, cx)))
            .on_action(cx.listener(|this, _: &Section6, _, cx| this.nth_section(5, cx)))
            .on_action(cx.listener(|this, _: &Mode2, window, cx| this.nth_mode(1, window, cx)))
            .on_action(cx.listener(|this, _: &Mode3, window, cx| this.nth_mode(2, window, cx)))
            .on_action(cx.listener(|this, _: &Mode4, window, cx| this.nth_mode(3, window, cx)))
            .on_action(cx.listener(|this, _: &Mode5, window, cx| this.nth_mode(4, window, cx)))
            .on_action(cx.listener(|this, _: &Mode6, window, cx| this.nth_mode(5, window, cx)))
            .on_action(cx.listener(|this, _: &OpenPalette, window, cx| {
                if this.palette_open {
                    this.close_palette(cx)
                } else {
                    this.open_palette(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &SelectAll, window, cx| {
                if matches!(this.open, Open::Canvas(_)) && this.place != Place::Home {
                    this.select_canvas(window, cx)
                } else {
                    this.select_all(cx)
                }
            }))
            .on_action(cx.listener(|this, _: &Deselect, window, cx| this.deselect(window, cx)))
            .on_action(cx.listener(|this, _: &InvertSelection, window, cx| {
                this.invert_selection(window, cx)
            }))
            // Enter closes a shape being drawn with the Pen; fields and
            // lists that use Enter handle it before it gets here.
            // Any key during a Command hold is a shortcut, not a hold.
            .capture_key_down(cx.listener(|this, _: &KeyDownEvent, _, _| this.hold_token += 1))
            .on_modifiers_changed(
                cx.listener(|this, event: &ModifiersChangedEvent, window, cx| {
                    this.modifiers_changed(&event.modifiers, window, cx)
                }),
            )
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                if event.keystroke.key == "space" {
                    this.hold_space(false, cx);
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.focus_handle.is_focused(window) && this.player_key(&event.keystroke, cx) {
                    // The player took it.
                } else if event.keystroke.key == "space" && this.focus_handle.is_focused(window) {
                    this.hold_space(true, cx);
                } else if event.keystroke.key == "enter"
                    && this.canvas.as_ref().is_some_and(|c| !c.pen.is_empty())
                {
                    this.finish_pen(window, cx);
                } else if this.focus_handle.is_focused(window) {
                    // Photoshop's one-key tools, only while no field has focus.
                    this.tool_key(&event.keystroke, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NudgeLeft, w, cx| {
                if !this.step_frames(-1, cx) {
                    this.nudge(-1., 0., w, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &NudgeRight, w, cx| {
                if !this.step_frames(1, cx) {
                    this.nudge(1., 0., w, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &NudgeUp, w, cx| {
                if this.mode_menu {
                    this.step_mode(-1, w, cx)
                } else {
                    this.nudge(0., -1., w, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &NudgeDown, w, cx| {
                if this.mode_menu {
                    this.step_mode(1, w, cx)
                } else {
                    this.nudge(0., 1., w, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &crate::ModeUp, w, cx| this.step_mode(-1, w, cx)))
            .on_action(cx.listener(|this, _: &crate::ModeDown, w, cx| this.step_mode(1, w, cx)))
            .on_action(cx.listener(|this, _: &NudgeLeftFar, w, cx| {
                if !this.step_frames(-10, cx) {
                    this.nudge(-10., 0., w, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &NudgeRightFar, w, cx| {
                if !this.step_frames(10, cx) {
                    this.nudge(10., 0., w, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &NudgeUpFar, w, cx| this.nudge(0., -10., w, cx)))
            .on_action(cx.listener(|this, _: &NudgeDownFar, w, cx| this.nudge(0., 10., w, cx)))
            .on_action(cx.listener(|this, _: &DeleteSelection, window, cx| {
                this.delete_selection(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &Undo, window, cx| {
                    this.step_color_history(false, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &Redo, window, cx| this.step_color_history(true, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &CopyEdits, window, cx| this.copy_edits(None, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PasteEdits, window, cx| this.paste_edits(None, window, cx)),
            )
            .on_action(cx.listener(|this, _: &crate::SampleColor, window, cx| {
                this.start_sampling(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearSelection, window, cx| {
                if this.stop_sampling(cx) {
                    // Escape ended taking a color for a picker.
                } else if this.canvas.as_ref().is_some_and(|c| !c.pen.is_empty()) {
                    if let Some(canvas) = &mut this.canvas {
                        canvas.pen.clear();
                    }
                    cx.notify()
                } else if !this.picker_open && !this.palette_open && this.has_canvas_selection() {
                    this.deselect(window, cx)
                } else if this.picker_open {
                    this.close_picker(window, cx)
                } else if this.palette_open {
                    this.close_palette(cx)
                } else {
                    this.clear_selection(cx)
                }
            }))
    }
}
