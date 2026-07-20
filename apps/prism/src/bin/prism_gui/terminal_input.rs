use super::*;

use terminal::{CellPosition, TerminalSelection, TerminalTab};

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
    let events = ui.input(|input| input.events.clone());
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
                session.selection = None;
                session.write(text.as_bytes());
                request_output_poll(ui);
            }
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                if is_terminal_toggle_key(modifiers, key) {
                    continue;
                }
                if is_clipboard_key(modifiers, key) {
                    continue;
                }
                if modifiers.shift && matches!(key, egui::Key::PageUp | egui::Key::PageDown) {
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
                    session.selection = None;
                    session.write(bytes);
                    request_output_poll(ui);
                }
            }
            _ => {}
        }
    }
}

#[cfg(target_os = "macos")]
fn is_terminal_toggle_key(modifiers: egui::Modifiers, key: egui::Key) -> bool {
    modifiers.mac_cmd && key == egui::Key::J
}

#[cfg(not(target_os = "macos"))]
fn is_terminal_toggle_key(modifiers: egui::Modifiers, key: egui::Key) -> bool {
    modifiers.ctrl && modifiers.shift && key == egui::Key::J
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
    modifiers.mac_cmd && matches!(key, egui::Key::C | egui::Key::V | egui::Key::X)
}

#[cfg(not(target_os = "macos"))]
fn is_clipboard_key(modifiers: egui::Modifiers, key: egui::Key) -> bool {
    modifiers.ctrl && modifiers.shift && matches!(key, egui::Key::C | egui::Key::V | egui::Key::X)
}

fn handle_pointer(
    ui: &egui::Ui,
    response: &egui::Response,
    viewport: Rect,
    cell_size: Vec2,
    session: &mut TerminalTab,
) {
    if response.drag_started()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let position = cell_at(pointer, viewport, cell_size, session.size);
        session.selection = Some(TerminalSelection {
            anchor: position,
            head: position,
        });
        ui.ctx().request_repaint();
    } else if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
        && let Some(selection) = session.selection.as_mut()
    {
        selection.head = cell_at(pointer, viewport, cell_size, session.size);
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

pub(super) fn terminal_key_bytes(
    key: egui::Key,
    modifiers: egui::Modifiers,
    application_cursor: bool,
) -> Option<&'static [u8]> {
    if modifiers.ctrl && !modifiers.alt {
        return control_byte(key);
    }
    let csi = !application_cursor;
    match key {
        egui::Key::Enter => Some(b"\r"),
        egui::Key::Tab => Some(b"\t"),
        egui::Key::Backspace => Some(b"\x7f"),
        egui::Key::Escape => Some(b"\x1b"),
        egui::Key::ArrowUp if csi => Some(b"\x1b[A"),
        egui::Key::ArrowDown if csi => Some(b"\x1b[B"),
        egui::Key::ArrowRight if csi => Some(b"\x1b[C"),
        egui::Key::ArrowLeft if csi => Some(b"\x1b[D"),
        egui::Key::ArrowUp => Some(b"\x1bOA"),
        egui::Key::ArrowDown => Some(b"\x1bOB"),
        egui::Key::ArrowRight => Some(b"\x1bOC"),
        egui::Key::ArrowLeft => Some(b"\x1bOD"),
        egui::Key::Home => Some(b"\x1b[H"),
        egui::Key::End => Some(b"\x1b[F"),
        egui::Key::Insert => Some(b"\x1b[2~"),
        egui::Key::Delete => Some(b"\x1b[3~"),
        egui::Key::PageUp => Some(b"\x1b[5~"),
        egui::Key::PageDown => Some(b"\x1b[6~"),
        _ => None,
    }
}

fn control_byte(key: egui::Key) -> Option<&'static [u8]> {
    match key {
        egui::Key::A => Some(b"\x01"),
        egui::Key::B => Some(b"\x02"),
        egui::Key::C => Some(b"\x03"),
        egui::Key::D => Some(b"\x04"),
        egui::Key::E => Some(b"\x05"),
        egui::Key::F => Some(b"\x06"),
        egui::Key::G => Some(b"\x07"),
        egui::Key::H => Some(b"\x08"),
        egui::Key::I => Some(b"\x09"),
        egui::Key::J => Some(b"\x0a"),
        egui::Key::K => Some(b"\x0b"),
        egui::Key::L => Some(b"\x0c"),
        egui::Key::M => Some(b"\x0d"),
        egui::Key::N => Some(b"\x0e"),
        egui::Key::O => Some(b"\x0f"),
        egui::Key::P => Some(b"\x10"),
        egui::Key::Q => Some(b"\x11"),
        egui::Key::R => Some(b"\x12"),
        egui::Key::S => Some(b"\x13"),
        egui::Key::T => Some(b"\x14"),
        egui::Key::U => Some(b"\x15"),
        egui::Key::V => Some(b"\x16"),
        egui::Key::W => Some(b"\x17"),
        egui::Key::X => Some(b"\x18"),
        egui::Key::Y => Some(b"\x19"),
        egui::Key::Z => Some(b"\x1a"),
        egui::Key::OpenBracket => Some(b"\x1b"),
        egui::Key::Backslash => Some(b"\x1c"),
        egui::Key::CloseBracket => Some(b"\x1d"),
        _ => None,
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
            Some(&b"\x03"[..])
        );
        assert_eq!(
            terminal_key_bytes(egui::Key::ArrowUp, egui::Modifiers::default(), false),
            Some(&b"\x1b[A"[..])
        );
        assert_eq!(
            terminal_key_bytes(egui::Key::ArrowUp, egui::Modifiers::default(), true),
            Some(&b"\x1bOA"[..])
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
            Some(&b"\x03"[..])
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
    }
}
