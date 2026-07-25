use super::*;

use terminal::{CellPosition, TerminalSelection, TerminalTab};
use terminal_protocol::{
    MouseButton, MouseEventKind, MouseReport, encode_mouse_report, is_meta_text_key,
    terminal_key_bytes, terminal_text_bytes,
};
use vt100::MouseProtocolMode;

pub(super) fn handle_terminal_input(
    ui: &mut egui::Ui,
    response: &egui::Response,
    viewport: Rect,
    cell_size: Vec2,
    session: &mut TerminalTab,
) {
    handle_pointer(ui, response, viewport, cell_size, session);
    if !response.has_focus() {
        return;
    }
    let (events, mut text_modifiers) = ui.input(|input| (input.events.clone(), input.modifiers));
    let mut suppress_meta_text = false;
    for event in events {
        match event {
            egui::Event::Copy => match clipboard_event_route(
                ClipboardEventKind::Copy,
                ui.input(|input| input.modifiers),
            ) {
                ClipboardRoute::Clipboard => copy_selection(ui, session),
                ClipboardRoute::Control(byte) => {
                    session.write(&[byte]);
                    request_output_poll(ui);
                }
            },
            egui::Event::Cut => match clipboard_event_route(
                ClipboardEventKind::Cut,
                ui.input(|input| input.modifiers),
            ) {
                ClipboardRoute::Clipboard => copy_selection(ui, session),
                ClipboardRoute::Control(byte) => {
                    session.write(&[byte]);
                    request_output_poll(ui);
                }
            },
            egui::Event::Paste(text) => {
                match clipboard_event_route(
                    ClipboardEventKind::Paste,
                    ui.input(|input| input.modifiers),
                ) {
                    ClipboardRoute::Clipboard => {
                        session.selection = None;
                        session.write(&paste_bytes(
                            &text,
                            session.parser.screen().bracketed_paste(),
                        ));
                    }
                    ClipboardRoute::Control(byte) => session.write(&[byte]),
                }
                request_output_poll(ui);
            }
            egui::Event::Text(text) if !text.is_empty() => {
                if suppress_meta_text {
                    suppress_meta_text = false;
                    continue;
                }
                if let Some(bytes) = terminal_text_bytes(&text, text_modifiers) {
                    session.selection = None;
                    session.write(&bytes);
                    request_output_poll(ui);
                }
            }
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                text_modifiers = modifiers;
                if is_terminal_toggle_key(modifiers, key) {
                    continue;
                }
                if is_clipboard_key(modifiers, key) {
                    continue;
                }
                if modifiers.shift
                    && !modifiers.alt
                    && !modifiers.ctrl
                    && !modifiers.mac_cmd
                    && matches!(key, egui::Key::PageUp | egui::Key::PageDown)
                {
                    let current = session.parser.screen().scrollback();
                    let amount = usize::from(session.size.rows.saturating_sub(2));
                    let offset = if key == egui::Key::PageUp {
                        current.saturating_add(amount)
                    } else {
                        current.saturating_sub(amount)
                    };
                    session.parser.screen_mut().set_scrollback(offset);
                    continue;
                }
                if let Some(bytes) =
                    terminal_key_bytes(key, modifiers, session.parser.screen().application_cursor())
                {
                    suppress_meta_text = modifiers.alt && is_meta_text_key(key);
                    session.selection = None;
                    session.write(&bytes);
                    request_output_poll(ui);
                }
            }
            _ => {}
        }
    }
}

#[cfg(target_os = "macos")]
fn is_terminal_toggle_key(modifiers: egui::Modifiers, key: egui::Key) -> bool {
    modifiers.mac_cmd && !modifiers.alt && key == egui::Key::J
}

#[cfg(not(target_os = "macos"))]
fn is_terminal_toggle_key(modifiers: egui::Modifiers, key: egui::Key) -> bool {
    modifiers.ctrl && modifiers.shift && !modifiers.alt && key == egui::Key::J
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClipboardEventKind {
    Copy,
    Cut,
    Paste,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClipboardRoute {
    Clipboard,
    Control(u8),
}

fn clipboard_event_route(kind: ClipboardEventKind, modifiers: egui::Modifiers) -> ClipboardRoute {
    #[cfg(target_os = "macos")]
    let raw_control = modifiers.ctrl && !modifiers.mac_cmd;
    #[cfg(not(target_os = "macos"))]
    let raw_control = modifiers.ctrl && !modifiers.shift;
    if raw_control {
        ClipboardRoute::Control(match kind {
            ClipboardEventKind::Copy => 3,
            ClipboardEventKind::Cut => 24,
            ClipboardEventKind::Paste => 22,
        })
    } else {
        ClipboardRoute::Clipboard
    }
}

fn request_output_poll(ui: &egui::Ui) {
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(16));
}

#[cfg(target_os = "macos")]
fn is_clipboard_key(modifiers: egui::Modifiers, key: egui::Key) -> bool {
    modifiers.mac_cmd && !modifiers.alt && matches!(key, egui::Key::C | egui::Key::V | egui::Key::X)
}

#[cfg(not(target_os = "macos"))]
fn is_clipboard_key(modifiers: egui::Modifiers, key: egui::Key) -> bool {
    modifiers.ctrl
        && modifiers.shift
        && !modifiers.alt
        && matches!(key, egui::Key::C | egui::Key::V | egui::Key::X)
}

fn handle_pointer(
    ui: &egui::Ui,
    response: &egui::Response,
    viewport: Rect,
    cell_size: Vec2,
    session: &mut TerminalTab,
) {
    if pointer_route(session.parser.screen().mouse_protocol_mode()) == PointerRoute::Reported {
        handle_reported_pointer(ui, viewport, cell_size, session);
        return;
    }
    session.mouse_buttons = [false; 3];
    session.last_mouse_cell = None;
    if response.drag_started()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let position =
            session.normalize_selection_cell(cell_at(pointer, viewport, cell_size, session.size));
        session.selection = Some(TerminalSelection {
            anchor: position,
            head: position,
        });
        ui.ctx().request_repaint();
    } else if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let position =
            session.normalize_selection_cell(cell_at(pointer, viewport, cell_size, session.size));
        if let Some(selection) = session.selection.as_mut() {
            selection.head = position;
        }
        ui.ctx().request_repaint();
    }

    if response.hovered() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll.abs() > 0.5 {
            let rows = (scroll.abs() / cell_size.y).ceil().max(1.0) as usize;
            let current = session.parser.screen().scrollback();
            let offset = if scroll > 0.0 {
                current.saturating_add(rows)
            } else {
                current.saturating_sub(rows)
            };
            session.parser.screen_mut().set_scrollback(offset);
            session.selection = None;
            ui.ctx().request_repaint();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerRoute {
    Local,
    Reported,
}

fn pointer_route(mode: MouseProtocolMode) -> PointerRoute {
    if mode == MouseProtocolMode::None {
        PointerRoute::Local
    } else {
        PointerRoute::Reported
    }
}

fn handle_reported_pointer(
    ui: &egui::Ui,
    viewport: Rect,
    cell_size: Vec2,
    session: &mut TerminalTab,
) {
    session.selection = None;
    let mode = session.parser.screen().mouse_protocol_mode();
    let encoding = session.parser.screen().mouse_protocol_encoding();
    let (events, hover_position) =
        ui.input(|input| (input.events.clone(), input.pointer.hover_pos()));
    for event in events {
        match event {
            egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            } => {
                let Some((button, index)) = pointer_button(button) else {
                    continue;
                };
                if pressed && !viewport.contains(pos) {
                    continue;
                }
                if !pressed && !session.mouse_buttons[index] {
                    continue;
                }
                session.mouse_buttons[index] = pressed;
                let position = cell_at(pos, viewport, cell_size, session.size);
                session.last_mouse_cell = Some(position);
                write_mouse_report(
                    ui,
                    session,
                    mode,
                    encoding,
                    MouseReport {
                        kind: if pressed {
                            MouseEventKind::Press
                        } else {
                            MouseEventKind::Release
                        },
                        button: Some(button),
                        position,
                        modifiers,
                    },
                );
            }
            egui::Event::PointerMoved(pos) => {
                if !viewport.contains(pos) && !session.mouse_buttons.iter().any(|pressed| *pressed)
                {
                    continue;
                }
                let position = cell_at(pos, viewport, cell_size, session.size);
                if session.last_mouse_cell == Some(position) {
                    continue;
                }
                session.last_mouse_cell = Some(position);
                let button = pressed_mouse_button(session.mouse_buttons);
                write_mouse_report(
                    ui,
                    session,
                    mode,
                    encoding,
                    MouseReport {
                        kind: MouseEventKind::Motion,
                        button,
                        position,
                        modifiers: ui.input(|input| input.modifiers),
                    },
                );
            }
            egui::Event::MouseWheel {
                delta, modifiers, ..
            } => {
                let Some(pointer) = hover_position.filter(|pointer| viewport.contains(*pointer))
                else {
                    continue;
                };
                let Some(button) = wheel_button(delta) else {
                    continue;
                };
                let position = cell_at(pointer, viewport, cell_size, session.size);
                let dominant = delta.x.abs().max(delta.y.abs());
                let cell_extent = if delta.x.abs() > delta.y.abs() {
                    cell_size.x
                } else {
                    cell_size.y
                };
                let reports = (dominant / cell_extent).ceil().clamp(1.0, 8.0) as usize;
                for _ in 0..reports {
                    write_mouse_report(
                        ui,
                        session,
                        mode,
                        encoding,
                        MouseReport {
                            kind: MouseEventKind::Wheel,
                            button: Some(button),
                            position,
                            modifiers,
                        },
                    );
                }
            }
            _ => {}
        }
    }
}

fn pointer_button(button: egui::PointerButton) -> Option<(MouseButton, usize)> {
    match button {
        egui::PointerButton::Primary => Some((MouseButton::Primary, 0)),
        egui::PointerButton::Middle => Some((MouseButton::Middle, 1)),
        egui::PointerButton::Secondary => Some((MouseButton::Secondary, 2)),
        egui::PointerButton::Extra1 | egui::PointerButton::Extra2 => None,
    }
}

fn pressed_mouse_button(buttons: [bool; 3]) -> Option<MouseButton> {
    if buttons[0] {
        Some(MouseButton::Primary)
    } else if buttons[1] {
        Some(MouseButton::Middle)
    } else if buttons[2] {
        Some(MouseButton::Secondary)
    } else {
        None
    }
}

fn wheel_button(delta: Vec2) -> Option<MouseButton> {
    if delta.x.abs() > delta.y.abs() {
        if delta.x > 0.0 {
            Some(MouseButton::WheelLeft)
        } else if delta.x < 0.0 {
            Some(MouseButton::WheelRight)
        } else {
            None
        }
    } else if delta.y > 0.0 {
        Some(MouseButton::WheelUp)
    } else if delta.y < 0.0 {
        Some(MouseButton::WheelDown)
    } else {
        None
    }
}

fn write_mouse_report(
    ui: &egui::Ui,
    session: &mut TerminalTab,
    mode: vt100::MouseProtocolMode,
    encoding: vt100::MouseProtocolEncoding,
    report: MouseReport,
) {
    if let Some(bytes) = encode_mouse_report(mode, encoding, report) {
        session.write(&bytes);
        request_output_poll(ui);
    }
}

fn copy_selection(ui: &egui::Ui, session: &TerminalTab) {
    if let Some(text) = session.selected_text().filter(|text| !text.is_empty()) {
        ui.ctx().copy_text(text);
    }
}

pub(super) fn cell_at(
    pointer: Pos2,
    viewport: Rect,
    cell_size: Vec2,
    size: spectrum_terminal::TerminalSize,
) -> CellPosition {
    let relative = (pointer - viewport.min).max(Vec2::ZERO);
    CellPosition {
        row: ((relative.y / cell_size.y).floor() as u16).min(size.rows.saturating_sub(1)),
        col: ((relative.x / cell_size.x).floor() as u16).min(size.cols.saturating_sub(1)),
    }
}

pub(super) fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut bytes = Vec::with_capacity(text.len() + 12);
        bytes.extend_from_slice(b"\x1b[200~");
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        bytes
    } else {
        text.as_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_clamps_pointer_to_terminal_cells() {
        let viewport = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(80.0, 40.0));
        let size = spectrum_terminal::TerminalSize::new(4, 8);
        assert_eq!(
            cell_at(Pos2::new(35.0, 35.0), viewport, Vec2::new(10.0, 10.0), size),
            CellPosition { row: 1, col: 2 }
        );
        assert_eq!(
            cell_at(
                Pos2::new(999.0, 999.0),
                viewport,
                Vec2::new(10.0, 10.0),
                size
            ),
            CellPosition { row: 3, col: 7 }
        );
    }

    #[test]
    fn terminal_keys_use_control_and_cursor_sequences() {
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(
            terminal_key_bytes(egui::Key::C, ctrl, false),
            Some(b"\x03".to_vec())
        );
        assert_eq!(
            terminal_key_bytes(egui::Key::ArrowUp, egui::Modifiers::default(), false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            terminal_key_bytes(egui::Key::ArrowUp, egui::Modifiers::default(), true),
            Some(b"\x1bOA".to_vec())
        );
    }

    #[test]
    fn bracketed_paste_wraps_payload_exactly() {
        assert_eq!(paste_bytes("one\ntwo", true), b"\x1b[200~one\ntwo\x1b[201~");
        assert_eq!(paste_bytes("plain", false), b"plain");
    }

    #[test]
    fn raw_control_c_is_never_mistaken_for_clipboard_copy() {
        let ctrl = egui::Modifiers {
            ctrl: true,
            command: cfg!(not(target_os = "macos")),
            ..Default::default()
        };
        assert!(!is_clipboard_key(ctrl, egui::Key::C));
        assert_eq!(
            terminal_key_bytes(egui::Key::C, ctrl, false),
            Some(b"\x03".to_vec())
        );
    }

    #[test]
    fn backend_copy_cut_paste_events_preserve_terminal_controls() {
        let raw_control = egui::Modifiers {
            ctrl: true,
            command: cfg!(not(target_os = "macos")),
            ..Default::default()
        };
        assert_eq!(
            clipboard_event_route(ClipboardEventKind::Copy, raw_control),
            ClipboardRoute::Control(3)
        );
        assert_eq!(
            clipboard_event_route(ClipboardEventKind::Cut, raw_control),
            ClipboardRoute::Control(24)
        );
        assert_eq!(
            clipboard_event_route(ClipboardEventKind::Paste, raw_control),
            ClipboardRoute::Control(22)
        );
    }

    #[test]
    fn platform_clipboard_chord_routes_backend_events_to_clipboard() {
        let mut modifiers = egui::Modifiers::default();
        #[cfg(target_os = "macos")]
        {
            modifiers.mac_cmd = true;
            modifiers.command = true;
        }
        #[cfg(not(target_os = "macos"))]
        {
            modifiers.ctrl = true;
            modifiers.command = true;
            modifiers.shift = true;
        }
        assert_eq!(
            clipboard_event_route(ClipboardEventKind::Copy, modifiers),
            ClipboardRoute::Clipboard
        );
        assert_eq!(
            clipboard_event_route(ClipboardEventKind::Paste, modifiers),
            ClipboardRoute::Clipboard
        );
    }

    #[test]
    fn platform_terminal_toggle_is_not_replayed_into_the_pty() {
        let mut modifiers = egui::Modifiers::default();
        #[cfg(target_os = "macos")]
        {
            modifiers.mac_cmd = true;
            modifiers.command = true;
        }
        #[cfg(not(target_os = "macos"))]
        {
            modifiers.ctrl = true;
            modifiers.command = true;
            modifiers.shift = true;
        }
        assert!(is_terminal_toggle_key(modifiers, egui::Key::J));
        assert!(!is_terminal_toggle_key(modifiers, egui::Key::K));
        modifiers.alt = true;
        assert!(!is_terminal_toggle_key(modifiers, egui::Key::J));
    }

    #[test]
    fn negotiated_mouse_modes_route_exclusively_between_local_and_pty_behavior() {
        let mut parser = vt100::Parser::new(4, 8, 0);
        assert_eq!(
            pointer_route(parser.screen().mouse_protocol_mode()),
            PointerRoute::Local
        );
        parser.process(b"\x1b[?1000h\x1b[?1006h");
        assert_eq!(
            pointer_route(parser.screen().mouse_protocol_mode()),
            PointerRoute::Reported
        );
        assert_eq!(
            parser.screen().mouse_protocol_encoding(),
            vt100::MouseProtocolEncoding::Sgr
        );
        parser.process(b"\x1b[?1000l\x1b[?1006l");
        assert_eq!(
            pointer_route(parser.screen().mouse_protocol_mode()),
            PointerRoute::Local
        );
    }

    #[test]
    fn pointer_buttons_and_wheels_map_to_xterm_order() {
        assert_eq!(
            pointer_button(egui::PointerButton::Primary),
            Some((MouseButton::Primary, 0))
        );
        assert_eq!(
            pointer_button(egui::PointerButton::Middle),
            Some((MouseButton::Middle, 1))
        );
        assert_eq!(
            pointer_button(egui::PointerButton::Secondary),
            Some((MouseButton::Secondary, 2))
        );
        assert_eq!(
            pressed_mouse_button([false, true, false]),
            Some(MouseButton::Middle)
        );
        assert_eq!(
            wheel_button(Vec2::new(0.0, 12.0)),
            Some(MouseButton::WheelUp)
        );
        assert_eq!(
            wheel_button(Vec2::new(-8.0, 0.0)),
            Some(MouseButton::WheelRight)
        );
    }
}
