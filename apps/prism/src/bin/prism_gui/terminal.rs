use super::*;

use spectrum_terminal::{TerminalContext, TerminalEvent, TerminalSession, TerminalSize};

const INITIAL_ROWS: u16 = 32;
const INITIAL_COLS: u16 = 120;
const SCROLLBACK_ROWS: usize = 10_000;
const SESSION_RAIL_WIDTH: f32 = 176.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct CellPosition {
    pub(super) row: u16,
    pub(super) col: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TerminalSelection {
    pub(super) anchor: CellPosition,
    pub(super) head: CellPosition,
}

impl TerminalSelection {
    pub(super) fn ordered(self) -> (CellPosition, CellPosition) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub(super) fn contains(self, position: CellPosition) -> bool {
        let (start, end) = self.ordered();
        position >= start && position <= end
    }
}

pub(super) struct TerminalDock {
    visible: bool,
    pub(super) sessions: Vec<TerminalTab>,
    pub(super) active: usize,
    next_id: u64,
    pub(super) focus_terminal: bool,
    pending_close: Option<usize>,
}

impl Default for TerminalDock {
    fn default() -> Self {
        Self {
            visible: false,
            sessions: Vec::new(),
            active: 0,
            next_id: 1,
            focus_terminal: false,
            pending_close: None,
        }
    }
}

pub(super) struct TerminalTab {
    pub(super) id: u64,
    pub(super) title: String,
    context_title: String,
    context: TerminalContext,
    pub(super) process: Option<TerminalSession>,
    pub(super) parser: vt100::Parser,
    pub(super) size: TerminalSize,
    pub(super) running: bool,
    pub(super) message: Option<(String, bool)>,
    pub(super) selection: Option<TerminalSelection>,
    pub(super) last_activity: std::time::Instant,
}

impl TerminalTab {
    fn spawn(id: u64, context_title: String, context: TerminalContext) -> Self {
        let size = TerminalSize::new(INITIAL_ROWS, INITIAL_COLS);
        let result = TerminalSession::spawn(context.clone(), size);
        let (process, running, message) = match result {
            Ok(process) => (Some(process), true, None),
            Err(error) => (
                None,
                false,
                Some((format!("could not start shell: {error:#}"), true)),
            ),
        };
        Self {
            id,
            title: format!("Terminal {id}"),
            context_title,
            context,
            process,
            parser: vt100::Parser::new(size.rows, size.cols, SCROLLBACK_ROWS),
            size,
            running,
            message,
            selection: None,
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
                    let message = exit.signal.as_ref().map_or_else(
                        || format!("process exited ({})", exit.code),
                        |signal| format!("process ended ({signal})"),
                    );
                    self.message = Some((message, !exit.success()));
                }
                TerminalEvent::Error(error) => self.message = Some((error, true)),
            }
        }
        if changed {
            self.last_activity = std::time::Instant::now();
        }
        changed
    }

    pub(super) fn write(&mut self, bytes: &[u8]) {
        match self.process.as_mut().filter(|_| self.running) {
            Some(process) => {
                if let Err(error) = process.write(bytes) {
                    self.message = Some((format!("input failed: {error:#}"), true));
                } else {
                    self.last_activity = std::time::Instant::now();
                }
            }
            None => self.message = Some(("shell is not running".into(), true)),
        }
    }

    pub(super) fn resize(&mut self, size: TerminalSize) {
        if size == self.size {
            return;
        }
        if let Some(process) = self.process.as_ref()
            && let Err(error) = process.resize(size)
        {
            self.message = Some((format!("resize failed: {error:#}"), true));
            return;
        }
        self.parser.screen_mut().set_size(size.rows, size.cols);
        self.size = size;
        self.selection = None;
    }

    pub(super) fn clear(&mut self) {
        self.parser = vt100::Parser::new(self.size.rows, self.size.cols, SCROLLBACK_ROWS);
        self.write(&[12]);
        self.selection = None;
        self.message = None;
    }

    pub(super) fn interrupt(&mut self) {
        self.write(&[3]);
    }

    pub(super) fn restart(&mut self) {
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
                self.message = Some((format!("restart failed: {error:#}"), true));
            }
        }
    }

    pub(super) fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection?.ordered();
        Some(self.parser.screen().contents_between(
            start.row,
            start.col,
            end.row,
            end.col.saturating_add(1).min(self.size.cols),
        ))
    }
}

impl TerminalDock {
    pub(super) fn visible(&self) -> bool {
        self.visible
    }

    fn new_session(&mut self, launch: TerminalLaunch) {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions
            .push(TerminalTab::spawn(id, launch.title, launch.context));
        self.active = self.sessions.len() - 1;
        self.focus_terminal = true;
        self.pending_close = None;
    }

    fn request_close(&mut self, index: usize) {
        if self
            .sessions
            .get(index)
            .is_some_and(|session| session.running)
        {
            self.pending_close = Some(index);
        } else {
            self.close_now(index);
        }
    }

    fn close_now(&mut self, index: usize) {
        if index >= self.sessions.len() {
            return;
        }
        self.sessions.remove(index);
        self.active = self.active.min(self.sessions.len().saturating_sub(1));
        self.focus_terminal = true;
        self.pending_close = None;
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        for session in &mut self.sessions {
            changed |= session.poll();
        }
        changed
    }

    pub(super) fn shutdown(&mut self) {
        self.sessions.clear();
    }
}

struct TerminalLaunch {
    title: String,
    context: TerminalContext,
}

impl PrismApp {
    #[cfg(not(target_os = "macos"))]
    pub(super) fn terminal_status_control(&mut self, ui: &mut egui::Ui) {
        if ui
            .selectable_label(self.terminal.visible(), "Terminal  ⌘J")
            .clicked()
        {
            self.toggle_terminal();
        }
        ui.separator();
    }

    pub(super) fn toggle_terminal(&mut self) {
        self.terminal.visible = !self.terminal.visible;
        if self.terminal.visible {
            if self.terminal.sessions.is_empty() {
                let launch = terminal_launch(&self.workspace);
                self.terminal.new_session(launch);
            }
            self.terminal.focus_terminal = true;
        }
    }

    pub(super) fn poll_terminals(&mut self, context: &egui::Context) {
        let changed = self.terminal.poll();
        if self.terminal.sessions.iter().any(|session| session.running) {
            let recently_active = self.terminal.sessions.iter().any(|session| {
                session.last_activity.elapsed() < std::time::Duration::from_millis(180)
            });
            context.request_repaint_after(terminal_poll_interval(
                self.terminal.visible,
                recently_active,
            ));
        }
        if changed {
            context.request_repaint();
        }
    }

    pub(super) fn terminal_panel(&mut self, root: &mut egui::Ui) {
        self.terminal_session_rail(root);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(INK).inner_margin(0))
            .show(root, |ui| {
                let Some(session) = self.terminal.sessions.get_mut(self.terminal.active) else {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("No terminals").color(MUTED));
                    });
                    return;
                };
                terminal_render::show_terminal(ui, session, &mut self.terminal.focus_terminal);
            });
    }

    fn terminal_session_rail(&mut self, root: &mut egui::Ui) {
        let sessions: Vec<_> = self
            .terminal
            .sessions
            .iter()
            .map(|session| {
                (
                    session.id,
                    session.title.clone(),
                    session.context_title.clone(),
                    session.running,
                )
            })
            .collect();
        let mut new_session = false;
        let mut close = None;
        let mut interrupt = None;
        let mut clear = None;
        let mut restart = None;
        let mut hide = false;
        let mut cancel_close = false;
        let mut confirm_close = None;
        egui::Panel::right("terminal-session-rail")
            .default_size(SESSION_RAIL_WIDTH)
            .min_size(SESSION_RAIL_WIDTH)
            .max_size(SESSION_RAIL_WIDTH)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(8, 8))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 5.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("TERMINALS").size(10.0).color(SUBTLE));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("×")
                            .on_hover_text("Close terminal")
                            .clicked()
                        {
                            close = Some(self.terminal.active);
                        }
                        if ui.small_button("+").on_hover_text("New terminal").clicked() {
                            new_session = true;
                        }
                        if ui.small_button("‹").on_hover_text("Hide · ⌘J").clicked() {
                            hide = true;
                        }
                    });
                });
                ui.add_space(4.0);
                for (index, (_, title, context_title, running)) in sessions.iter().enumerate() {
                    let marker = if *running { "●" } else { "○" };
                    let response = ui
                        .selectable_label(
                            index == self.terminal.active,
                            RichText::new(format!("{marker}  {title}")).size(11.0),
                        )
                        .on_hover_text(context_title);
                    if response.clicked() {
                        self.terminal.active = index;
                        self.terminal.focus_terminal = true;
                    }
                    response.context_menu(|ui| {
                        if *running && ui.button("Interrupt").clicked() {
                            interrupt = Some(index);
                            ui.close();
                        }
                        if ui.button("Clear display").clicked() {
                            clear = Some(index);
                            ui.close();
                        }
                        if !*running && ui.button("Restart").clicked() {
                            restart = Some(index);
                            ui.close();
                        }
                        if ui.button("Close").clicked() {
                            close = Some(index);
                            ui.close();
                        }
                    });
                }
                if let Some(index) = self.terminal.pending_close
                    && let Some((_, title, _, _)) = sessions.get(index)
                {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.label(
                        RichText::new(format!("Stop {title} and close it?"))
                            .size(10.0)
                            .color(ACCENT_WARM),
                    );
                    ui.label(
                        RichText::new("Its shell or foreground process will stop.")
                            .size(10.0)
                            .color(SUBTLE),
                    );
                    ui.horizontal(|ui| {
                        if ui.small_button("Cancel").clicked() {
                            cancel_close = true;
                        }
                        if ui.small_button("Stop & close").clicked() {
                            confirm_close = Some(index);
                        }
                    });
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(RichText::new("⌘J  editor").size(10.0).color(SUBTLE));
                    if let Some(session) = self.terminal.sessions.get(self.terminal.active)
                        && let Some((message, is_error)) = &session.message
                    {
                        ui.label(RichText::new(message).size(10.0).color(if *is_error {
                            DANGER
                        } else {
                            SUBTLE
                        }));
                    }
                });
            });
        if new_session {
            self.terminal.new_session(terminal_launch(&self.workspace));
        }
        if let Some(index) = interrupt {
            self.terminal.sessions[index].interrupt();
        }
        if let Some(index) = clear {
            self.terminal.sessions[index].clear();
        }
        if let Some(index) = restart {
            self.terminal.sessions[index].restart();
        }
        if let Some(index) = close {
            self.terminal.request_close(index);
        }
        if cancel_close {
            self.terminal.pending_close = None;
        }
        if let Some(index) = confirm_close {
            self.terminal.close_now(index);
        }
        if hide {
            self.terminal.visible = false;
        }
    }
}

fn terminal_poll_interval(visible: bool, recently_active: bool) -> std::time::Duration {
    std::time::Duration::from_millis(if visible && recently_active { 16 } else { 250 })
}

fn terminal_launch(workspace: &Workspace) -> TerminalLaunch {
    let project_path = workspace.project_path.as_deref().map(|path| {
        std::fs::canonicalize(path).unwrap_or_else(|_| {
            if path.is_absolute() {
                path.to_owned()
            } else {
                std::env::current_dir()
                    .map(|directory| directory.join(path))
                    .unwrap_or_else(|_| path.to_owned())
            }
        })
    });
    let project = project_path.as_deref();
    let working_directory = project
        .and_then(Path::parent)
        .map(Path::to_owned)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let mut context = TerminalContext::new(&working_directory)
        .with_env("SPECTRUM_ACTIVE_APP", "prism")
        .with_env("SPECTRUM_DOCUMENT", &workspace.document.name)
        .with_env("PRISM_DOCUMENT", &workspace.document.name);
    if let Some(project) = project {
        context = context
            .with_env("SPECTRUM_PROJECT", project.as_os_str())
            .with_env("PRISM_PROJECT", project.as_os_str());
    }
    if let Some(session) = workspace.session_id() {
        let session = session.to_string();
        context = context
            .with_env("SPECTRUM_SESSION", &session)
            .with_env("PRISM_SESSION", &session);
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        context = context.with_cli_directory(directory);
    }
    TerminalLaunch {
        title: workspace.document.name.clone(),
        context,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_launch() -> TerminalLaunch {
        TerminalLaunch {
            title: "Test project".into(),
            context: TerminalContext::new(std::env::current_dir().unwrap()),
        }
    }

    #[test]
    fn hiding_terminal_preserves_every_session() {
        let mut dock = TerminalDock::default();
        dock.new_session(test_launch());
        let process_id = dock.sessions[0]
            .process
            .as_ref()
            .and_then(TerminalSession::process_id);
        dock.visible = true;
        dock.visible = false;
        assert_eq!(dock.sessions.len(), 1);
        assert_eq!(
            dock.sessions[0]
                .process
                .as_ref()
                .and_then(TerminalSession::process_id),
            process_id
        );
        dock.shutdown();
    }

    #[test]
    fn closing_active_session_selects_a_surviving_tab() {
        let mut dock = TerminalDock::default();
        dock.new_session(test_launch());
        dock.new_session(test_launch());
        dock.sessions[1].running = false;
        dock.request_close(1);
        assert_eq!(dock.sessions.len(), 1);
        assert_eq!(dock.active, 0);
        dock.shutdown();
    }

    #[test]
    fn running_session_requires_confirmation_before_close() {
        let mut dock = TerminalDock::default();
        dock.new_session(test_launch());
        dock.request_close(0);
        assert_eq!(dock.sessions.len(), 1);
        assert_eq!(dock.pending_close, Some(0));
        dock.close_now(0);
        assert!(dock.sessions.is_empty());
    }

    #[test]
    fn terminal_polling_bursts_only_while_visible_and_active() {
        assert_eq!(
            terminal_poll_interval(true, true),
            std::time::Duration::from_millis(16)
        );
        assert_eq!(
            terminal_poll_interval(true, false),
            std::time::Duration::from_millis(250)
        );
        assert_eq!(
            terminal_poll_interval(false, true),
            std::time::Duration::from_millis(250)
        );
    }

    #[test]
    fn active_project_context_is_passed_as_data() {
        let workspace = Workspace::new(
            Document::new("$(unsafe) artwork", 100, 100),
            Some(PathBuf::from("/tmp/project with spaces.prism")),
        );
        let launch = terminal_launch(&workspace);
        assert_eq!(launch.context.working_directory(), Path::new("/tmp"));
        assert_eq!(
            launch.context.environment("PRISM_PROJECT"),
            Some(std::ffi::OsStr::new("/tmp/project with spaces.prism"))
        );
        assert_eq!(
            launch.context.environment("PRISM_DOCUMENT"),
            Some(std::ffi::OsStr::new("$(unsafe) artwork"))
        );
    }

    #[test]
    fn selection_orders_reverse_drags_and_includes_cells() {
        let selection = TerminalSelection {
            anchor: CellPosition { row: 4, col: 9 },
            head: CellPosition { row: 2, col: 3 },
        };
        assert_eq!(selection.ordered().0, CellPosition { row: 2, col: 3 });
        assert!(selection.contains(CellPosition { row: 3, col: 0 }));
        assert!(!selection.contains(CellPosition { row: 1, col: 9 }));
    }
}
