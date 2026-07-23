use std::{path::PathBuf, time::Duration};

#[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
use std::collections::BTreeSet;

use spectrum_terminal::{TerminalContext, TerminalEvent, TerminalSession, TerminalSize};

use super::*;

const INITIAL_SIZE: TerminalSize = TerminalSize::new(32, 120);
const SCROLLBACK_ROWS: usize = 10_000;
const FONT_SIZE: f32 = 13.0;

pub(super) struct TerminalDock {
    visible: bool,
    session: Option<TerminalTab>,
    next_id: u64,
    native_sessions: bool,
    focus_requested: bool,
    close_confirmation: bool,
}

impl TerminalDock {
    pub(super) fn new(native_sessions: bool) -> Self {
        Self {
            visible: false,
            session: None,
            next_id: 1,
            native_sessions,
            focus_requested: false,
            close_confirmation: false,
        }
    }

    pub(super) fn visible(&self) -> bool {
        self.visible
    }

    #[cfg(target_os = "macos")]
    pub(super) fn native_session(&self) -> bool {
        self.session.as_ref().is_some_and(|session| session.native)
    }

    fn ensure_session(&mut self, launch: TerminalLaunch) {
        if self.session.is_some() {
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.session = Some(TerminalTab::spawn(id, launch, self.native_sessions));
        self.focus_requested = true;
    }

    fn close_now(&mut self) {
        self.session = None;
        self.close_confirmation = false;
    }

    fn replace_workspace_session(&mut self, launch: TerminalLaunch) {
        let visible = self.visible;
        self.close_now();
        if visible {
            self.ensure_session(launch);
        }
    }

    pub(super) fn shutdown(&mut self) {
        self.close_now();
        self.visible = false;
    }
}

struct TerminalLaunch {
    title: String,
    context: TerminalContext,
}

struct TerminalTab {
    id: u64,
    title: String,
    context: TerminalContext,
    process: Option<TerminalSession>,
    parser: vt100::Parser,
    size: TerminalSize,
    running: bool,
    message: Option<(String, bool)>,
    native: bool,
    last_activity: std::time::Instant,
}

impl TerminalTab {
    fn spawn(id: u64, launch: TerminalLaunch, native: bool) -> Self {
        let (process, running, message) = if native {
            (None, true, None)
        } else {
            match TerminalSession::spawn(launch.context.clone(), INITIAL_SIZE) {
                Ok(process) => (Some(process), true, None),
                Err(error) => (
                    None,
                    false,
                    Some((format!("Could not start shell: {error:#}"), true)),
                ),
            }
        };
        Self {
            id,
            title: launch.title,
            context: launch.context,
            process,
            parser: vt100::Parser::new(INITIAL_SIZE.rows, INITIAL_SIZE.cols, SCROLLBACK_ROWS),
            size: INITIAL_SIZE,
            running,
            message,
            native,
            last_activity: std::time::Instant::now(),
        }
    }

    fn poll(&mut self) -> bool {
        let Some(process) = self.process.as_mut() else {
            return false;
        };
        let mut changed = false;
        let follow_output = self.parser.screen().scrollback() == 0;
        for event in process.poll() {
            changed = true;
            match event {
                TerminalEvent::Output(bytes) => {
                    self.parser.process(&bytes);
                    if follow_output {
                        self.parser.screen_mut().set_scrollback(0);
                    }
                }
                TerminalEvent::Exited(exit) => {
                    self.running = false;
                    self.message = Some((
                        exit.signal.as_ref().map_or_else(
                            || format!("Shell exited ({})", exit.code),
                            |signal| format!("Shell ended ({signal})"),
                        ),
                        !exit.success(),
                    ));
                }
                TerminalEvent::Error(error) => self.message = Some((error, true)),
            }
        }
        if changed {
            self.last_activity = std::time::Instant::now();
        }
        changed
    }

    fn write(&mut self, bytes: &[u8]) {
        match self.process.as_mut().filter(|_| self.running) {
            Some(process) => {
                if let Err(error) = process.write(bytes) {
                    self.message = Some((format!("Terminal input failed: {error:#}"), true));
                } else {
                    self.last_activity = std::time::Instant::now();
                }
            }
            None if !self.native => self.message = Some(("Shell is not running".into(), true)),
            None => {}
        }
    }

    fn resize(&mut self, size: TerminalSize) {
        if self.native || self.size == size {
            return;
        }
        if let Some(process) = self.process.as_ref()
            && let Err(error) = process.resize(size)
        {
            self.message = Some((format!("Terminal resize failed: {error:#}"), true));
            return;
        }
        self.parser.screen_mut().set_size(size.rows, size.cols);
        self.size = size;
    }

    fn restart(&mut self) {
        if self.native {
            self.running = true;
            self.message = None;
            return;
        }
        if let Some(process) = self.process.as_mut() {
            let _ = process.terminate();
        }
        match TerminalSession::spawn(self.context.clone(), self.size) {
            Ok(process) => {
                self.process = Some(process);
                self.running = true;
                self.message = None;
                self.parser = vt100::Parser::new(self.size.rows, self.size.cols, SCROLLBACK_ROWS);
            }
            Err(error) => {
                self.process = None;
                self.running = false;
                self.message = Some((format!("Could not restart shell: {error:#}"), true));
            }
        }
    }

    #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
    fn fall_back_to_portable(&mut self, diagnostic: String) {
        self.native = false;
        match TerminalSession::spawn(self.context.clone(), self.size) {
            Ok(process) => {
                self.process = Some(process);
                self.running = true;
                self.message = Some((diagnostic, true));
            }
            Err(error) => {
                self.running = false;
                self.message = Some((
                    format!("{diagnostic}; portable fallback also failed: {error:#}"),
                    true,
                ));
            }
        }
    }
}

impl LumenApp {
    pub(super) fn hide_terminal(&mut self) {
        self.terminal.visible = false;
        self.terminal.focus_requested = false;
    }

    pub(super) fn toggle_terminal(&mut self) {
        if self.history_open {
            return;
        }
        self.terminal.visible = !self.terminal.visible;
        if self.terminal.visible {
            let launch = terminal_launch(&self.workspace);
            self.terminal.ensure_session(launch);
            self.terminal.focus_requested = true;
        }
    }

    pub(super) fn reset_terminal_for_workspace(&mut self) {
        #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
        if let Some(id) = self.terminal.session.as_ref().map(|session| session.id) {
            self.native_terminal.reset(id);
        }
        self.terminal
            .replace_workspace_session(terminal_launch(&self.workspace));
    }

    pub(super) fn poll_terminal(&mut self, context: &egui::Context) {
        #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
        {
            for event in self.native_terminal.poll() {
                match event {
                    spectrum_terminal::native_ghostty::NativeTerminalEvent::Title {
                        session_id,
                        title,
                    } => {
                        if let Some(session) = self
                            .terminal
                            .session
                            .as_mut()
                            .filter(|session| session.id == session_id)
                        {
                            session.title = title;
                            session.last_activity = std::time::Instant::now();
                        }
                    }
                    spectrum_terminal::native_ghostty::NativeTerminalEvent::Closed {
                        session_id,
                        process_alive,
                    } => {
                        if self
                            .terminal
                            .session
                            .as_ref()
                            .is_some_and(|session| session.id == session_id)
                        {
                            if process_alive {
                                self.terminal.close_confirmation = true;
                            } else {
                                self.terminal.close_now();
                                self.native_terminal.reset(session_id);
                            }
                        }
                    }
                }
            }
            let sessions = self
                .terminal
                .session
                .as_ref()
                .filter(|session| session.native)
                .map(|session| BTreeSet::from([session.id]))
                .unwrap_or_default();
            self.native_terminal.retain_sessions(&sessions);
        }
        let changed = self
            .terminal
            .session
            .as_mut()
            .is_some_and(TerminalTab::poll);
        if let Some(session) = &self.terminal.session
            && session.running
        {
            let active = self.terminal.visible
                && session.last_activity.elapsed() < Duration::from_millis(180);
            context.request_repaint_after(Duration::from_millis(if active { 16 } else { 250 }));
        }
        if changed {
            context.request_repaint();
        }
    }

    pub(super) fn terminal_panel(&mut self, root: &mut egui::Ui) {
        #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
        let modal_open = self.terminal_modal_open();
        let mut restart = false;
        let mut close = false;
        let mut confirm_close = false;
        let mut cancel_close = false;
        egui::Panel::top("terminal-controls")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 7))
                    .stroke(Stroke::new(1.0, Color32::from_gray(49))),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let title = self
                        .terminal
                        .session
                        .as_ref()
                        .map_or("Terminal", |session| session.title.as_str());
                    ui.label(RichText::new(title).strong().color(ACCENT));
                    if let Some(session) = &self.terminal.session
                        && let Some((message, is_error)) = &session.message
                    {
                        ui.separator();
                        ui.label(RichText::new(message).size(11.0).color(if *is_error {
                            Color32::LIGHT_RED
                        } else {
                            Color32::GRAY
                        }));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                        if ui.button("Restart").clicked() {
                            restart = true;
                        }
                        if ui
                            .button(format!("Hide  {}", terminal_shortcut_label()))
                            .clicked()
                        {
                            self.terminal.visible = false;
                        }
                    });
                });
                if self.terminal.close_confirmation {
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("Stop the running shell and close it?");
                        if ui.button("Cancel").clicked() {
                            cancel_close = true;
                        }
                        if ui.button("Stop & Close").clicked() {
                            confirm_close = true;
                        }
                    });
                }
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(CANVAS).inner_margin(0))
            .show(root, |ui| {
                let Some(session) = self.terminal.session.as_mut() else {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("No terminal session").color(Color32::GRAY));
                    });
                    return;
                };
                #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
                if session.native {
                    let rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(rect, 0.0, CANVAS);
                    ui.allocate_rect(rect, Sense::hover());
                    let presentation =
                        spectrum_terminal::native_ghostty::NativeSurfacePresentation::for_terminal(
                            rect,
                            self.terminal.visible,
                            true,
                            modal_open,
                            self.terminal.focus_requested,
                        );
                    match self
                        .native_terminal
                        .present(session.id, &session.context, presentation)
                    {
                        Ok(()) => self.terminal.focus_requested = false,
                        Err(error) => session.fall_back_to_portable(format!(
                            "Ghostty surface failed; using portable terminal: {error:#}"
                        )),
                    }
                    return;
                }
                show_portable_terminal(ui, session, &mut self.terminal.focus_requested);
            });

        if restart && let Some(session) = self.terminal.session.as_mut() {
            #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
            if session.native {
                self.native_terminal.reset(session.id);
            }
            session.restart();
            self.terminal.focus_requested = true;
        }
        if close && let Some(session) = self.terminal.session.as_ref() {
            #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
            if session.native {
                if let Err(error) = self.native_terminal.request_close(session.id)
                    && let Some(session) = self.terminal.session.as_mut()
                {
                    session.message = Some((format!("Close failed: {error:#}"), true));
                }
            } else {
                self.terminal.close_confirmation = session.running;
                if !session.running {
                    self.terminal.close_now();
                }
            }
            #[cfg(not(all(target_os = "macos", feature = "ghostty-terminal")))]
            {
                self.terminal.close_confirmation = session.running;
                if !session.running {
                    self.terminal.close_now();
                }
            }
        }
        if cancel_close {
            self.terminal.close_confirmation = false;
        }
        if confirm_close {
            #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
            if let Some(id) = self
                .terminal
                .session
                .as_ref()
                .filter(|session| session.native)
                .map(|session| session.id)
            {
                self.native_terminal.reset(id);
            }
            self.terminal.close_now();
        }
    }

    #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
    fn terminal_modal_open(&self) -> bool {
        self.reset_confirmation
            || self.remove_confirmation
            || self.pending_catalog_switch.is_some()
            || self.rename_batch.is_some()
            || self.export_open
    }

    #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
    pub(super) fn route_native_terminal_edit(
        &mut self,
        action: macos_menu_spec::NativeMenuAction,
    ) -> bool {
        if self.terminal_modal_open() || self.history_open || self.library_mode {
            return false;
        }
        let Some(session) = self.terminal.session.as_mut() else {
            return false;
        };
        if !self.terminal.visible || !session.native {
            return false;
        }
        let action = match action {
            macos_menu_spec::NativeMenuAction::Copy => {
                spectrum_terminal::native_ghostty::NativeEditAction::Copy
            }
            macos_menu_spec::NativeMenuAction::Paste => {
                spectrum_terminal::native_ghostty::NativeEditAction::Paste
            }
            _ => return false,
        };
        if let Err(error) = self.native_terminal.edit(session.id, action) {
            session.message = Some((format!("Terminal edit failed: {error:#}"), true));
        }
        true
    }
}

pub(super) fn terminal_shortcut_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘J"
    } else {
        "Ctrl+J"
    }
}

fn terminal_launch(workspace: &Workspace) -> TerminalLaunch {
    let catalog = workspace.catalog_path.as_deref().map(canonical_path);
    let working_directory = catalog
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_owned)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let mut context = TerminalContext::new(working_directory)
        .with_env("SPECTRUM_ACTIVE_APP", "lumen")
        .with_env("SPECTRUM_CATALOG_NAME", &workspace.project.name)
        .with_env("LUMEN_CATALOG_NAME", &workspace.project.name);
    if let Some(catalog) = catalog {
        context = context
            .with_env("SPECTRUM_CATALOG", catalog.as_os_str())
            .with_env("LUMEN_CATALOG", catalog.as_os_str());
    }
    if let Some(session) = workspace.session_id() {
        let session = session.to_string();
        context = context
            .with_env("SPECTRUM_SESSION", &session)
            .with_env("LUMEN_SESSION", &session);
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        context = context.with_cli_directory(directory);
    }
    TerminalLaunch {
        title: workspace.project.name.clone(),
        context,
    }
}

fn canonical_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()
                .map(|directory| directory.join(path))
                .unwrap_or_else(|_| path.to_owned())
        }
    })
}

fn show_portable_terminal(ui: &mut egui::Ui, session: &mut TerminalTab, focus: &mut bool) {
    let outer = ui.available_rect_before_wrap();
    ui.painter().rect_filled(outer, 0.0, CANVAS);
    let viewport = outer.shrink2(Vec2::new(8.0, 6.0));
    let font = egui::FontId::new(FONT_SIZE, egui::FontFamily::Monospace);
    let galley =
        ui.painter()
            .layout_no_wrap("M".into(), font.clone(), Color32::from_rgb(218, 216, 222));
    let cell = Vec2::new(galley.size().x.max(1.0), (galley.size().y + 2.0).ceil());
    let size = TerminalSize::new(
        (viewport.height() / cell.y).floor().clamp(2.0, 180.0) as u16,
        (viewport.width() / cell.x).floor().clamp(10.0, 300.0) as u16,
    );
    session.resize(size);
    let response = ui.interact(
        viewport,
        ui.id().with(("lumen-terminal", session.id)),
        Sense::click(),
    );
    if response.clicked() || *focus {
        response.request_focus();
        *focus = false;
    }
    paint_screen(ui, viewport, cell, &font, session, response.has_focus());
    if response.hovered() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll.abs() > 0.5 {
            let rows = (scroll.abs() / cell.y).ceil().max(1.0) as usize;
            let current = session.parser.screen().scrollback();
            session.parser.screen_mut().set_scrollback(if scroll > 0.0 {
                current.saturating_add(rows)
            } else {
                current.saturating_sub(rows)
            });
        }
    }
    if !response.has_focus() {
        return;
    }
    for event in ui.input(|input| input.events.clone()) {
        match event {
            egui::Event::Text(text) if !text.is_empty() => session.write(text.as_bytes()),
            egui::Event::Paste(text) => {
                if session.parser.screen().bracketed_paste() {
                    session.write(b"\x1b[200~");
                    session.write(text.as_bytes());
                    session.write(b"\x1b[201~");
                } else {
                    session.write(text.as_bytes());
                }
            }
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                if modifiers.command && key == egui::Key::J {
                    continue;
                }
                if let Some(bytes) =
                    terminal_key_bytes(key, modifiers, session.parser.screen().application_cursor())
                {
                    session.write(bytes);
                }
            }
            _ => {}
        }
    }
}

fn terminal_key_bytes(
    key: egui::Key,
    modifiers: egui::Modifiers,
    application_cursor: bool,
) -> Option<&'static [u8]> {
    if modifiers.ctrl && !modifiers.alt {
        return match key {
            egui::Key::A => Some(b"\x01"),
            egui::Key::C => Some(b"\x03"),
            egui::Key::D => Some(b"\x04"),
            egui::Key::L => Some(b"\x0c"),
            egui::Key::Z => Some(b"\x1a"),
            _ => None,
        };
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
        egui::Key::Delete => Some(b"\x1b[3~"),
        _ => None,
    }
}

fn paint_screen(
    ui: &egui::Ui,
    viewport: Rect,
    cell: Vec2,
    font: &egui::FontId,
    session: &TerminalTab,
    focused: bool,
) {
    let painter = ui.painter().with_clip_rect(viewport);
    let screen = session.parser.screen();
    for row in 0..session.size.rows {
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = f32::INFINITY;
        job.keep_trailing_whitespace = true;
        job.first_row_min_height = cell.y;
        for col in 0..session.size.cols {
            let Some(value) = screen.cell(row, col) else {
                continue;
            };
            if value.is_wide_continuation() {
                continue;
            }
            let mut foreground = terminal_color(value.fgcolor(), true);
            let mut background = terminal_color(value.bgcolor(), false);
            if value.inverse() {
                std::mem::swap(&mut foreground, &mut background);
            }
            if background == CANVAS {
                background = Color32::TRANSPARENT;
            }
            job.append(
                if value.has_contents() {
                    value.contents()
                } else {
                    " "
                },
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    line_height: Some(cell.y),
                    color: foreground,
                    background,
                    italics: value.italic(),
                    underline: if value.underline() {
                        Stroke::new(1.0, foreground)
                    } else {
                        Stroke::NONE
                    },
                    ..Default::default()
                },
            );
        }
        painter.galley(
            viewport.min + Vec2::new(0.0, f32::from(row) * cell.y),
            painter.layout_job(job),
            Color32::WHITE,
        );
    }
    if focused && screen.scrollback() == 0 && !screen.hide_cursor() {
        let (row, col) = screen.cursor_position();
        let cursor = Rect::from_min_size(
            viewport.min + Vec2::new(f32::from(col) * cell.x, f32::from(row) * cell.y),
            cell,
        );
        painter.rect_stroke(
            cursor,
            0.0,
            Stroke::new(1.4, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
}

fn terminal_color(color: vt100::Color, foreground: bool) -> Color32 {
    match color {
        vt100::Color::Default => {
            if foreground {
                Color32::from_rgb(218, 216, 222)
            } else {
                CANVAS
            }
        }
        vt100::Color::Rgb(red, green, blue) => Color32::from_rgb(red, green, blue),
        vt100::Color::Idx(index) => indexed_color(index),
    }
}

fn indexed_color(index: u8) -> Color32 {
    const ANSI: [(u8, u8, u8); 16] = [
        (20, 21, 25),
        (210, 91, 99),
        (126, 190, 128),
        (202, 172, 102),
        (103, 153, 207),
        (174, 132, 196),
        (104, 187, 190),
        (211, 211, 216),
        (104, 104, 112),
        (235, 119, 126),
        (151, 210, 151),
        (225, 197, 126),
        (130, 176, 224),
        (199, 158, 220),
        (132, 208, 210),
        (239, 238, 242),
    ];
    match index {
        0..=15 => {
            let (red, green, blue) = ANSI[usize::from(index)];
            Color32::from_rgb(red, green, blue)
        }
        16..=231 => {
            let value = index - 16;
            let component = |part: u8| if part == 0 { 0 } else { 55 + part * 40 };
            Color32::from_rgb(
                component(value / 36),
                component((value / 6) % 6),
                component(value % 6),
            )
        }
        232..=255 => Color32::from_gray(8 + (index - 232) * 10),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_terminal_uses_catalog_parent_and_cli_environment() {
        let directory = std::env::temp_dir().join("lumen-terminal-context-test");
        let path = directory.join("session.lumen");
        let workspace = Workspace::new(
            Project {
                name: "Wedding edits".into(),
                ..Project::default()
            },
            Some(path.clone()),
        );
        let launch = terminal_launch(&workspace);
        assert_eq!(launch.context.working_directory(), directory);
        assert_eq!(
            launch.context.environment("LUMEN_CATALOG"),
            Some(path.as_os_str())
        );
        assert_eq!(
            launch.context.environment("SPECTRUM_ACTIVE_APP"),
            Some(std::ffi::OsStr::new("lumen"))
        );
    }

    #[test]
    fn hiding_dock_preserves_session_identity() {
        let mut dock = TerminalDock::new(true);
        dock.ensure_session(TerminalLaunch {
            title: "Test".into(),
            context: TerminalContext::new(std::env::current_dir().unwrap()),
        });
        let id = dock.session.as_ref().unwrap().id;
        dock.visible = true;
        dock.visible = false;
        assert_eq!(dock.session.as_ref().unwrap().id, id);
        dock.shutdown();
    }

    #[test]
    fn workspace_change_replaces_visible_session_context() {
        let mut dock = TerminalDock::new(true);
        dock.visible = true;
        dock.ensure_session(TerminalLaunch {
            title: "First".into(),
            context: TerminalContext::new("/first"),
        });
        let first_id = dock.session.as_ref().unwrap().id;
        dock.replace_workspace_session(TerminalLaunch {
            title: "Second".into(),
            context: TerminalContext::new("/second"),
        });
        let replacement = dock.session.as_ref().unwrap();
        assert_ne!(replacement.id, first_id);
        assert_eq!(
            replacement.context.working_directory(),
            Path::new("/second")
        );
        dock.shutdown();
    }
}
