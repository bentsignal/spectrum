use std::collections::VecDeque;

use super::*;
use terminal::CellPosition;
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

const MAX_QUERY_REPLIES: usize = 32;
const MAX_QUERY_REPLY_BYTES: usize = 4 * 1024;

#[derive(Debug, Default)]
pub(super) struct TerminalCallbacks {
    replies: VecDeque<Vec<u8>>,
    reply_bytes: usize,
}

impl TerminalCallbacks {
    fn push_reply(&mut self, reply: Vec<u8>) {
        if self.replies.len() >= MAX_QUERY_REPLIES
            || self.reply_bytes.saturating_add(reply.len()) > MAX_QUERY_REPLY_BYTES
        {
            return;
        }
        self.reply_bytes += reply.len();
        self.replies.push_back(reply);
    }

    pub(super) fn take_replies(&mut self) -> VecDeque<Vec<u8>> {
        self.reply_bytes = 0;
        std::mem::take(&mut self.replies)
    }
}

impl vt100::Callbacks for TerminalCallbacks {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        first_intermediate: Option<u8>,
        second_intermediate: Option<u8>,
        params: &[&[u16]],
        command: char,
    ) {
        let single_param = || match params {
            [] => Some(0),
            [[value]] => Some(*value),
            _ => None,
        };
        match (
            first_intermediate,
            second_intermediate,
            command,
            single_param(),
        ) {
            (None, None, 'c', Some(0)) => self.push_reply(b"\x1b[?1;2c".to_vec()),
            (Some(b'>'), None, 'c', Some(0)) => {
                self.push_reply(b"\x1b[>0;136;0c".to_vec());
            }
            (None, None, 'n', Some(5)) => self.push_reply(b"\x1b[0n".to_vec()),
            (None, None, 'n', Some(6)) => {
                let (row, col) = screen.cursor_position();
                self.push_reply(format!("\x1b[{};{}R", row + 1, col + 1).into_bytes());
            }
            (Some(b'?'), None, 'n', Some(6)) => {
                let (row, col) = screen.cursor_position();
                self.push_reply(format!("\x1b[?{};{}R", row + 1, col + 1).into_bytes());
            }
            (None, None, 't', Some(18)) => {
                let (rows, cols) = screen.size();
                self.push_reply(format!("\x1b[8;{rows};{cols}t").into_bytes());
            }
            _ => {}
        }
    }
}

pub(super) fn terminal_key_bytes(
    key: egui::Key,
    modifiers: egui::Modifiers,
    application_cursor: bool,
) -> Option<Vec<u8>> {
    if modifiers.mac_cmd {
        return None;
    }
    if modifiers.ctrl
        && let Some(control) = control_byte(key)
    {
        return Some(meta_prefix(vec![control], modifiers.alt));
    }
    if modifiers.alt
        && let Some(printable) = printable_key_byte(key, modifiers.shift)
    {
        return Some(vec![0x1b, printable]);
    }

    let modifier = xterm_modifier(modifiers);
    let modified = modifier != 1;
    let bytes = match key {
        egui::Key::Enter => meta_prefix(b"\r".to_vec(), modifiers.alt),
        egui::Key::Tab if modifiers.shift && !modifiers.alt && !modifiers.ctrl => {
            b"\x1b[Z".to_vec()
        }
        egui::Key::Tab if modified => format!("\x1b[1;{modifier}Z").into_bytes(),
        egui::Key::Tab => b"\t".to_vec(),
        egui::Key::Backspace => meta_prefix(b"\x7f".to_vec(), modifiers.alt),
        egui::Key::Escape => b"\x1b".to_vec(),
        egui::Key::ArrowUp => cursor_key(b'A', modifier, application_cursor),
        egui::Key::ArrowDown => cursor_key(b'B', modifier, application_cursor),
        egui::Key::ArrowRight => cursor_key(b'C', modifier, application_cursor),
        egui::Key::ArrowLeft => cursor_key(b'D', modifier, application_cursor),
        egui::Key::Home => cursor_key(b'H', modifier, application_cursor),
        egui::Key::End => cursor_key(b'F', modifier, application_cursor),
        egui::Key::Insert => tilde_key(2, modifier),
        egui::Key::Delete => tilde_key(3, modifier),
        egui::Key::PageUp => tilde_key(5, modifier),
        egui::Key::PageDown => tilde_key(6, modifier),
        egui::Key::F1 => function_key(1, modifier),
        egui::Key::F2 => function_key(2, modifier),
        egui::Key::F3 => function_key(3, modifier),
        egui::Key::F4 => function_key(4, modifier),
        egui::Key::F5 => function_key(5, modifier),
        egui::Key::F6 => function_key(6, modifier),
        egui::Key::F7 => function_key(7, modifier),
        egui::Key::F8 => function_key(8, modifier),
        egui::Key::F9 => function_key(9, modifier),
        egui::Key::F10 => function_key(10, modifier),
        egui::Key::F11 => function_key(11, modifier),
        egui::Key::F12 => function_key(12, modifier),
        _ => return None,
    };
    Some(bytes)
}

pub(super) fn is_meta_text_key(key: egui::Key) -> bool {
    printable_key_byte(key, false).is_some()
}

pub(super) fn terminal_text_bytes(text: &str, modifiers: egui::Modifiers) -> Option<Vec<u8>> {
    if text.is_empty() || modifiers.mac_cmd {
        return None;
    }
    Some(meta_prefix(text.as_bytes().to_vec(), modifiers.alt))
}

fn cursor_key(final_byte: u8, modifier: u8, application_cursor: bool) -> Vec<u8> {
    if modifier != 1 {
        format!("\x1b[1;{modifier}{}", char::from(final_byte)).into_bytes()
    } else if application_cursor {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

fn tilde_key(code: u8, modifier: u8) -> Vec<u8> {
    if modifier == 1 {
        format!("\x1b[{code}~").into_bytes()
    } else {
        format!("\x1b[{code};{modifier}~").into_bytes()
    }
}

fn function_key(number: u8, modifier: u8) -> Vec<u8> {
    if number <= 4 {
        let final_byte = b'P' + number - 1;
        if modifier == 1 {
            vec![0x1b, b'O', final_byte]
        } else {
            format!("\x1b[1;{modifier}{}", char::from(final_byte)).into_bytes()
        }
    } else {
        let code = match number {
            5 => 15,
            6 => 17,
            7 => 18,
            8 => 19,
            9 => 20,
            10 => 21,
            11 => 23,
            12 => 24,
            _ => unreachable!("caller limits function keys to F1-F12"),
        };
        tilde_key(code, modifier)
    }
}

fn xterm_modifier(modifiers: egui::Modifiers) -> u8 {
    1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.ctrl)
}

fn meta_prefix(mut bytes: Vec<u8>, meta: bool) -> Vec<u8> {
    if meta {
        bytes.insert(0, 0x1b);
    }
    bytes
}

fn control_byte(key: egui::Key) -> Option<u8> {
    match key {
        egui::Key::A => Some(0x01),
        egui::Key::B => Some(0x02),
        egui::Key::C => Some(0x03),
        egui::Key::D => Some(0x04),
        egui::Key::E => Some(0x05),
        egui::Key::F => Some(0x06),
        egui::Key::G => Some(0x07),
        egui::Key::H => Some(0x08),
        egui::Key::I => Some(0x09),
        egui::Key::J => Some(0x0a),
        egui::Key::K => Some(0x0b),
        egui::Key::L => Some(0x0c),
        egui::Key::M => Some(0x0d),
        egui::Key::N => Some(0x0e),
        egui::Key::O => Some(0x0f),
        egui::Key::P => Some(0x10),
        egui::Key::Q => Some(0x11),
        egui::Key::R => Some(0x12),
        egui::Key::S => Some(0x13),
        egui::Key::T => Some(0x14),
        egui::Key::U => Some(0x15),
        egui::Key::V => Some(0x16),
        egui::Key::W => Some(0x17),
        egui::Key::X => Some(0x18),
        egui::Key::Y => Some(0x19),
        egui::Key::Z => Some(0x1a),
        egui::Key::OpenBracket => Some(0x1b),
        egui::Key::Backslash => Some(0x1c),
        egui::Key::CloseBracket => Some(0x1d),
        _ => None,
    }
}

fn printable_key_byte(key: egui::Key, shift: bool) -> Option<u8> {
    let letter = match key {
        egui::Key::A => Some(b'a'),
        egui::Key::B => Some(b'b'),
        egui::Key::C => Some(b'c'),
        egui::Key::D => Some(b'd'),
        egui::Key::E => Some(b'e'),
        egui::Key::F => Some(b'f'),
        egui::Key::G => Some(b'g'),
        egui::Key::H => Some(b'h'),
        egui::Key::I => Some(b'i'),
        egui::Key::J => Some(b'j'),
        egui::Key::K => Some(b'k'),
        egui::Key::L => Some(b'l'),
        egui::Key::M => Some(b'm'),
        egui::Key::N => Some(b'n'),
        egui::Key::O => Some(b'o'),
        egui::Key::P => Some(b'p'),
        egui::Key::Q => Some(b'q'),
        egui::Key::R => Some(b'r'),
        egui::Key::S => Some(b's'),
        egui::Key::T => Some(b't'),
        egui::Key::U => Some(b'u'),
        egui::Key::V => Some(b'v'),
        egui::Key::W => Some(b'w'),
        egui::Key::X => Some(b'x'),
        egui::Key::Y => Some(b'y'),
        egui::Key::Z => Some(b'z'),
        _ => None,
    };
    if let Some(letter) = letter {
        return Some(if shift {
            letter.to_ascii_uppercase()
        } else {
            letter
        });
    }
    match key {
        egui::Key::Space => Some(b' '),
        egui::Key::Num0 => Some(if shift { b')' } else { b'0' }),
        egui::Key::Num1 => Some(if shift { b'!' } else { b'1' }),
        egui::Key::Num2 => Some(if shift { b'@' } else { b'2' }),
        egui::Key::Num3 => Some(if shift { b'#' } else { b'3' }),
        egui::Key::Num4 => Some(if shift { b'$' } else { b'4' }),
        egui::Key::Num5 => Some(if shift { b'%' } else { b'5' }),
        egui::Key::Num6 => Some(if shift { b'^' } else { b'6' }),
        egui::Key::Num7 => Some(if shift { b'&' } else { b'7' }),
        egui::Key::Num8 => Some(if shift { b'*' } else { b'8' }),
        egui::Key::Num9 => Some(if shift { b'(' } else { b'9' }),
        egui::Key::Backtick => Some(if shift { b'~' } else { b'`' }),
        egui::Key::Minus => Some(if shift { b'_' } else { b'-' }),
        egui::Key::Equals => Some(if shift { b'+' } else { b'=' }),
        egui::Key::OpenBracket => Some(if shift { b'{' } else { b'[' }),
        egui::Key::CloseBracket => Some(if shift { b'}' } else { b']' }),
        egui::Key::Backslash => Some(if shift { b'|' } else { b'\\' }),
        egui::Key::Semicolon => Some(if shift { b':' } else { b';' }),
        egui::Key::Quote => Some(if shift { b'"' } else { b'\'' }),
        egui::Key::Comma => Some(if shift { b'<' } else { b',' }),
        egui::Key::Period => Some(if shift { b'>' } else { b'.' }),
        egui::Key::Slash => Some(if shift { b'?' } else { b'/' }),
        egui::Key::Colon => Some(b':'),
        egui::Key::Pipe => Some(b'|'),
        egui::Key::Questionmark => Some(b'?'),
        egui::Key::Exclamationmark => Some(b'!'),
        egui::Key::OpenCurlyBracket => Some(b'{'),
        egui::Key::CloseCurlyBracket => Some(b'}'),
        egui::Key::Plus => Some(b'+'),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MouseButton {
    Primary,
    Middle,
    Secondary,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MouseEventKind {
    Press,
    Release,
    Motion,
    Wheel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MouseReport {
    pub(super) kind: MouseEventKind,
    pub(super) button: Option<MouseButton>,
    pub(super) position: CellPosition,
    pub(super) modifiers: egui::Modifiers,
}

pub(super) fn encode_mouse_report(
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
    report: MouseReport,
) -> Option<Vec<u8>> {
    if !mouse_mode_accepts(mode, report) {
        return None;
    }
    let mut code = match report.button {
        Some(MouseButton::Primary) => 0,
        Some(MouseButton::Middle) => 1,
        Some(MouseButton::Secondary) => 2,
        Some(MouseButton::WheelUp) => 64,
        Some(MouseButton::WheelDown) => 65,
        Some(MouseButton::WheelLeft) => 66,
        Some(MouseButton::WheelRight) => 67,
        None => 3,
    };
    if report.kind == MouseEventKind::Motion {
        code += 32;
    }
    code += u16::from(report.modifiers.shift) * 4;
    code += u16::from(report.modifiers.alt) * 8;
    code += u16::from(report.modifiers.ctrl) * 16;
    let x = report.position.col.saturating_add(1);
    let y = report.position.row.saturating_add(1);
    match encoding {
        MouseProtocolEncoding::Sgr => {
            let final_byte = if report.kind == MouseEventKind::Release {
                'm'
            } else {
                'M'
            };
            Some(format!("\x1b[<{code};{x};{y}{final_byte}").into_bytes())
        }
        MouseProtocolEncoding::Default => {
            if x > 223 || y > 223 {
                return None;
            }
            if report.kind == MouseEventKind::Release {
                code = 3
                    + u16::from(report.modifiers.shift) * 4
                    + u16::from(report.modifiers.alt) * 8
                    + u16::from(report.modifiers.ctrl) * 16;
            }
            Some(vec![
                0x1b,
                b'[',
                b'M',
                (code + 32) as u8,
                (x + 32) as u8,
                (y + 32) as u8,
            ])
        }
        MouseProtocolEncoding::Utf8 => {
            if x > 2_015 || y > 2_015 {
                return None;
            }
            if report.kind == MouseEventKind::Release {
                code = 3
                    + u16::from(report.modifiers.shift) * 4
                    + u16::from(report.modifiers.alt) * 8
                    + u16::from(report.modifiers.ctrl) * 16;
            }
            let mut bytes = b"\x1b[M".to_vec();
            for value in [code + 32, x + 32, y + 32] {
                bytes.extend(char::from_u32(u32::from(value))?.to_string().as_bytes());
            }
            Some(bytes)
        }
    }
}

fn mouse_mode_accepts(mode: MouseProtocolMode, report: MouseReport) -> bool {
    match mode {
        MouseProtocolMode::None => false,
        MouseProtocolMode::Press => {
            matches!(report.kind, MouseEventKind::Press | MouseEventKind::Wheel)
        }
        MouseProtocolMode::PressRelease => {
            matches!(
                report.kind,
                MouseEventKind::Press | MouseEventKind::Release | MouseEventKind::Wheel
            )
        }
        MouseProtocolMode::ButtonMotion => {
            report.kind != MouseEventKind::Motion || report.button.is_some()
        }
        MouseProtocolMode::AnyMotion => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modifiers(shift: bool, alt: bool, ctrl: bool) -> egui::Modifiers {
        egui::Modifiers {
            shift,
            alt,
            ctrl,
            command: ctrl,
            ..Default::default()
        }
    }

    #[test]
    fn function_shift_tab_and_modified_navigation_use_xterm_sequences() {
        let function_keys = [
            (egui::Key::F1, b"\x1bOP".as_slice()),
            (egui::Key::F2, b"\x1bOQ".as_slice()),
            (egui::Key::F3, b"\x1bOR".as_slice()),
            (egui::Key::F4, b"\x1bOS".as_slice()),
            (egui::Key::F5, b"\x1b[15~".as_slice()),
            (egui::Key::F6, b"\x1b[17~".as_slice()),
            (egui::Key::F7, b"\x1b[18~".as_slice()),
            (egui::Key::F8, b"\x1b[19~".as_slice()),
            (egui::Key::F9, b"\x1b[20~".as_slice()),
            (egui::Key::F10, b"\x1b[21~".as_slice()),
            (egui::Key::F11, b"\x1b[23~".as_slice()),
            (egui::Key::F12, b"\x1b[24~".as_slice()),
        ];
        for (key, expected) in function_keys {
            assert_eq!(
                terminal_key_bytes(key, egui::Modifiers::NONE, false).unwrap(),
                expected
            );
        }
        assert_eq!(
            terminal_key_bytes(egui::Key::Tab, modifiers(true, false, false), false).unwrap(),
            b"\x1b[Z"
        );
        assert_eq!(
            terminal_key_bytes(egui::Key::ArrowLeft, modifiers(true, true, true), true).unwrap(),
            b"\x1b[1;8D"
        );
        assert_eq!(
            terminal_key_bytes(egui::Key::Delete, modifiers(false, true, false), false).unwrap(),
            b"\x1b[3;3~"
        );
        assert_eq!(
            terminal_key_bytes(egui::Key::F5, modifiers(true, false, false), false).unwrap(),
            b"\x1b[15;2~"
        );
        assert_eq!(
            terminal_key_bytes(egui::Key::ArrowUp, egui::Modifiers::NONE, true).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            terminal_key_bytes(egui::Key::Home, modifiers(false, false, true), false).unwrap(),
            b"\x1b[1;5H"
        );
        assert_eq!(
            terminal_key_bytes(egui::Key::End, egui::Modifiers::NONE, true).unwrap(),
            b"\x1bOF"
        );
    }

    #[test]
    fn meta_text_and_control_are_escape_prefixed_but_command_is_reserved() {
        assert_eq!(
            terminal_key_bytes(egui::Key::X, modifiers(false, true, false), false).unwrap(),
            b"\x1bx"
        );
        assert_eq!(
            terminal_key_bytes(egui::Key::Num1, modifiers(true, true, false), false).unwrap(),
            b"\x1b!"
        );
        assert_eq!(
            terminal_text_bytes("x", modifiers(false, true, false)).unwrap(),
            b"\x1bx"
        );
        assert_eq!(
            terminal_key_bytes(egui::Key::C, modifiers(false, true, true), false).unwrap(),
            b"\x1b\x03"
        );
        let command = egui::Modifiers {
            mac_cmd: true,
            command: true,
            ..Default::default()
        };
        assert!(terminal_text_bytes("q", command).is_none());
        assert!(terminal_key_bytes(egui::Key::Q, command, false).is_none());
    }

    #[test]
    fn sgr_mouse_reports_press_release_motion_wheel_and_one_based_coordinates() {
        let position = CellPosition { row: 4, col: 8 };
        let report = |kind, button| MouseReport {
            kind,
            button,
            position,
            modifiers: modifiers(true, false, true),
        };
        assert_eq!(
            encode_mouse_report(
                MouseProtocolMode::PressRelease,
                MouseProtocolEncoding::Sgr,
                report(MouseEventKind::Press, Some(MouseButton::Primary)),
            )
            .unwrap(),
            b"\x1b[<20;9;5M"
        );
        assert_eq!(
            encode_mouse_report(
                MouseProtocolMode::PressRelease,
                MouseProtocolEncoding::Sgr,
                report(MouseEventKind::Release, Some(MouseButton::Primary)),
            )
            .unwrap(),
            b"\x1b[<20;9;5m"
        );
        assert!(
            encode_mouse_report(
                MouseProtocolMode::PressRelease,
                MouseProtocolEncoding::Sgr,
                report(MouseEventKind::Motion, Some(MouseButton::Primary)),
            )
            .is_none()
        );
        assert_eq!(
            encode_mouse_report(
                MouseProtocolMode::ButtonMotion,
                MouseProtocolEncoding::Sgr,
                report(MouseEventKind::Motion, Some(MouseButton::Primary)),
            )
            .unwrap(),
            b"\x1b[<52;9;5M"
        );
        assert_eq!(
            encode_mouse_report(
                MouseProtocolMode::AnyMotion,
                MouseProtocolEncoding::Sgr,
                report(MouseEventKind::Wheel, Some(MouseButton::WheelDown)),
            )
            .unwrap(),
            b"\x1b[<85;9;5M"
        );
    }

    #[test]
    fn legacy_mouse_encodings_fail_closed_outside_their_coordinate_range() {
        let report = MouseReport {
            kind: MouseEventKind::Press,
            button: Some(MouseButton::Secondary),
            position: CellPosition { row: 4, col: 299 },
            modifiers: egui::Modifiers::NONE,
        };
        assert!(
            encode_mouse_report(
                MouseProtocolMode::Press,
                MouseProtocolEncoding::Default,
                report
            )
            .is_none()
        );
        assert!(
            encode_mouse_report(
                MouseProtocolMode::Press,
                MouseProtocolEncoding::Utf8,
                report
            )
            .is_some()
        );
    }

    #[test]
    fn mouse_modes_and_legacy_encodings_follow_the_negotiated_protocol() {
        let base = MouseReport {
            kind: MouseEventKind::Press,
            button: Some(MouseButton::Primary),
            position: CellPosition { row: 4, col: 8 },
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(
            encode_mouse_report(
                MouseProtocolMode::Press,
                MouseProtocolEncoding::Default,
                base
            )
            .unwrap(),
            vec![0x1b, b'[', b'M', 32, 41, 37]
        );
        assert!(
            encode_mouse_report(
                MouseProtocolMode::Press,
                MouseProtocolEncoding::Sgr,
                MouseReport {
                    kind: MouseEventKind::Release,
                    ..base
                },
            )
            .is_none()
        );
        assert_eq!(
            encode_mouse_report(
                MouseProtocolMode::AnyMotion,
                MouseProtocolEncoding::Sgr,
                MouseReport {
                    kind: MouseEventKind::Motion,
                    button: None,
                    ..base
                },
            )
            .unwrap(),
            b"\x1b[<35;9;5M"
        );
        assert!(
            encode_mouse_report(
                MouseProtocolMode::ButtonMotion,
                MouseProtocolEncoding::Sgr,
                MouseReport {
                    kind: MouseEventKind::Motion,
                    button: None,
                    ..base
                },
            )
            .is_none()
        );
    }

    #[test]
    fn terminal_queries_are_bounded_and_report_cursor_and_size() {
        let mut parser = vt100::Parser::new_with_callbacks(24, 80, 0, TerminalCallbacks::default());
        parser.process(b"\x1b[4;7H\x1b[5n\x1b[6n\x1b[?6n\x1b[c\x1b[>c\x1b[18t");
        assert_eq!(
            parser.callbacks_mut().take_replies(),
            VecDeque::from([
                b"\x1b[0n".to_vec(),
                b"\x1b[4;7R".to_vec(),
                b"\x1b[?4;7R".to_vec(),
                b"\x1b[?1;2c".to_vec(),
                b"\x1b[>0;136;0c".to_vec(),
                b"\x1b[8;24;80t".to_vec(),
            ])
        );
        parser.process(&b"\x1b[5n".repeat(MAX_QUERY_REPLIES + 20));
        let replies = parser.callbacks_mut().take_replies();
        assert_eq!(replies.len(), MAX_QUERY_REPLIES);
        assert!(replies.iter().map(Vec::len).sum::<usize>() <= MAX_QUERY_REPLY_BYTES);
    }
}
