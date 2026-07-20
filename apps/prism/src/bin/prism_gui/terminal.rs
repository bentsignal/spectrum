use super::*;

use spectrum_terminal::{TerminalContext, TerminalEvent, TerminalSession, TerminalSize};

const INITIAL_ROWS: u16 = 18;
const INITIAL_COLS: u16 = 120;
const SCROLLBACK_ROWS: usize = 4_000;

pub(super) struct TerminalDock {
    visible: bool,
    sessions: Vec<TerminalTab>,
    active: usize,
    next_id: u64,
    focus_input: bool,
}

impl Default for TerminalDock {
    fn default() -> Self {
        Self {
            visible: false,
            sessions: Vec::new(),
            active: 0,
            next_id: 1,
            focus_input: false,
        }
    }
}

struct TerminalTab {
    id: u64,
    title: String,
    context: TerminalContext,
    context_summary: String,
    process: Option<TerminalSession>,
    parser: vt100::Parser,
    size: TerminalSize,
    input: String,
    running: bool,
    status: String,
    status_error: bool,
}

impl TerminalTab {
    fn spawn(id: u64, title: String, context: TerminalContext, context_summary: String) -> Self {
        let size = TerminalSize::new(INITIAL_ROWS, INITIAL_COLS);
        let process = TerminalSession::spawn(context.clone(), size);
        let (process, running, status, status_error) = match process {
            Ok(process) => (Some(process), true, "Shell ready".into(), false),
            Err(error) => (
                None,
                false,
                format!("Could not start terminal: {error:#}"),
                true,
            ),
        };
        Self {
            id,
            title,
            context,
            context_summary,
            process,
            parser: vt100::Parser::new(size.rows, size.cols, SCROLLBACK_ROWS),
            size,
            input: String::new(),
            running,
            status,
            status_error,
        }
    }

    fn poll(&mut self) {
        let Some(process) = self.process.as_mut() else {
            return;
        };
        for event in process.poll() {
            match event {
                TerminalEvent::Output(bytes) => self.parser.process(&bytes),
                TerminalEvent::Exited(exit) => {
                    self.running = false;
                    self.status_error = !exit.success();
                    self.status = exit.signal.map_or_else(
                        || format!("Shell exited with code {}", exit.code),
                        |signal| format!("Shell terminated by {signal}"),
                    );
                }
                TerminalEvent::Error(error) => {
                    self.status = error;
                    self.status_error = true;
                }
            }
        }
    }

    fn send_input(&mut self) {
        let line = self.input.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return;
        }
        let bytes = format!("{line}\n");
        match self.process.as_mut() {
            Some(process) if self.running => match process.write(bytes.as_bytes()) {
                Ok(()) => {
                    self.input.clear();
                    self.status = "Input sent".into();
                    self.status_error = false;
                }
                Err(error) => {
                    self.status = format!("Could not send terminal input: {error:#}");
                    self.status_error = true;
                }
            },
            _ => {
                self.status = "Restart this terminal before sending input".into();
                self.status_error = true;
            }
        }
    }

    fn resize(&mut self, size: TerminalSize) {
        if size == self.size {
            return;
        }
        if let Some(process) = self.process.as_ref()
            && let Err(error) = process.resize(size)
        {
            self.status = format!("Could not resize terminal: {error:#}");
            self.status_error = true;
            return;
        }
        self.parser.screen_mut().set_size(size.rows, size.cols);
        self.size = size;
    }

    fn clear(&mut self) {
        self.parser = vt100::Parser::new(self.size.rows, self.size.cols, SCROLLBACK_ROWS);
        if let Some(process) = self.process.as_mut()
            && self.running
        {
            let _ = process.write(&[12]);
        }
        self.status = "Terminal display cleared; the shell is still running".into();
        self.status_error = false;
    }

    fn stop(&mut self) {
        if let Some(process) = self.process.as_mut() {
            match process.terminate() {
                Ok(()) => {
                    self.running = false;
                    self.status = "Terminal stopped".into();
                    self.status_error = false;
                }
                Err(error) => {
                    self.status = format!("Could not stop terminal: {error:#}");
                    self.status_error = true;
                }
            }
        }
    }

    fn restart(&mut self) {
        self.stop();
        match TerminalSession::spawn(self.context.clone(), self.size) {
            Ok(process) => {
                self.process = Some(process);
                self.running = true;
                self.status = "Shell restarted in the original project context".into();
                self.status_error = false;
            }
            Err(error) => {
                self.process = None;
                self.running = false;
                self.status = format!("Could not restart terminal: {error:#}");
                self.status_error = true;
            }
        }
    }
}

impl TerminalDock {
    pub(super) fn visible(&self) -> bool {
        self.visible
    }

    fn new_session(&mut self, launch: TerminalLaunch) {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.push(TerminalTab::spawn(
            id,
            format!("{} · {id}", launch.title),
            launch.context,
            launch.summary,
        ));
        self.active = self.sessions.len() - 1;
        self.focus_input = true;
    }

    fn close_active(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        self.sessions.remove(self.active);
        self.active = self.active.min(self.sessions.len().saturating_sub(1));
    }

    fn poll(&mut self) {
        for session in &mut self.sessions {
            session.poll();
        }
    }

    pub(super) fn shutdown(&mut self) {
        self.sessions.clear();
    }
}

struct TerminalLaunch {
    title: String,
    context: TerminalContext,
    summary: String,
}

impl PrismApp {
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
                let launch = self.terminal_launch();
                self.terminal.new_session(launch);
            }
            self.terminal.focus_input = true;
            self.status = "Terminal opened · active sessions keep running while hidden".into();
            self.status_error = false;
        }
    }

    pub(super) fn poll_terminals(&mut self, context: &egui::Context) {
        self.terminal.poll();
        if self.terminal.sessions.iter().any(|session| session.running) {
            let interval = if self.terminal.visible { 33 } else { 250 };
            context.request_repaint_after(std::time::Duration::from_millis(interval));
        }
    }

    fn terminal_launch(&self) -> TerminalLaunch {
        terminal_launch(&self.workspace)
    }

    pub(super) fn terminal_panel(&mut self, root: &mut egui::Ui) {
        if !self.terminal.visible {
            return;
        }
        egui::Panel::bottom("prism-terminal")
            .resizable(true)
            .default_size(280.0)
            .min_size(170.0)
            .max_size(640.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(8)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| self.terminal_contents(ui));
    }

    fn terminal_contents(&mut self, ui: &mut egui::Ui) {
        let tabs: Vec<_> = self
            .terminal
            .sessions
            .iter()
            .map(|session| (session.id, session.title.clone(), session.running))
            .collect();
        let mut new_session = false;
        let mut close_active = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("TERMINAL").size(10.0).strong().color(ACCENT));
            for (index, (_, title, running)) in tabs.iter().enumerate() {
                let marker = if *running { "●" } else { "○" };
                if ui
                    .selectable_label(index == self.terminal.active, format!("{marker} {title}"))
                    .clicked()
                {
                    self.terminal.active = index;
                    self.terminal.focus_input = true;
                }
            }
            if ui
                .small_button("+")
                .on_hover_text("New terminal for the active project")
                .clicked()
            {
                new_session = true;
            }
            if ui
                .add_enabled(!tabs.is_empty(), egui::Button::new("×"))
                .on_hover_text("Close active terminal and stop its process")
                .clicked()
            {
                close_active = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Hide  ⌘J").clicked() {
                    self.terminal.visible = false;
                }
            });
        });
        if new_session {
            let launch = self.terminal_launch();
            self.terminal.new_session(launch);
        }
        if close_active {
            self.terminal.close_active();
        }

        let Some(session) = self.terminal.sessions.get_mut(self.terminal.active) else {
            ui.centered_and_justified(|ui| {
                ui.label("No terminal sessions · choose + to open one for this project");
            });
            return;
        };
        ui.label(
            RichText::new(&session.context_summary)
                .monospace()
                .size(10.0)
                .color(MUTED),
        );
        ui.horizontal(|ui| {
            if ui.small_button("Clear").clicked() {
                session.clear();
            }
            if session.running {
                if ui.small_button("Interrupt").clicked()
                    && let Some(process) = session.process.as_mut()
                    && let Err(error) = process.write(&[3])
                {
                    session.status = format!("Could not interrupt terminal: {error:#}");
                    session.status_error = true;
                }
                if ui.small_button("Stop").clicked() {
                    session.stop();
                }
            } else if ui.small_button("Restart").clicked() {
                session.restart();
            }
            let contents = session.parser.screen().contents();
            if ui.small_button("Copy all").clicked() {
                ui.ctx().copy_text(contents);
                session.status = "Terminal contents copied".into();
                session.status_error = false;
            }
            ui.label(
                RichText::new(&session.status)
                    .size(10.0)
                    .color(if session.status_error { DANGER } else { MUTED }),
            );
        });

        let output_height = (ui.available_height() - 38.0).max(60.0);
        let rows = (output_height / 15.0).floor().clamp(4.0, 120.0) as u16;
        let cols = (ui.available_width() / 7.5).floor().clamp(30.0, 240.0) as u16;
        session.resize(TerminalSize::new(rows, cols));
        egui::Frame::new()
            .fill(INK)
            .inner_margin(8)
            .stroke(Stroke::new(1.0, BORDER))
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .stick_to_bottom(true)
                    .max_height(output_height)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(session.parser.screen().contents())
                                    .monospace()
                                    .size(12.0)
                                    .color(TEXT),
                            )
                            .selectable(true)
                            .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
            });
        ui.horizontal(|ui| {
            ui.label(RichText::new("›").monospace().color(ACCENT));
            let response = ui.add_enabled(
                session.running,
                egui::TextEdit::singleline(&mut session.input)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("Send input to the shell or active coding agent…")
                    .desired_width(f32::INFINITY),
            );
            if self.terminal.focus_input {
                response.request_focus();
                self.terminal.focus_input = false;
            }
            let send = ui.add_enabled(session.running, egui::Button::new("Send"));
            if (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                || send.clicked()
            {
                session.send_input();
                response.request_focus();
            }
        });
    }
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
    let summary = format!(
        "session context: cwd {}  ·  PRISM_PROJECT {}  ·  PRISM_SESSION {}",
        working_directory.display(),
        project.map_or_else(|| "not available".into(), |path| path.display().to_string()),
        workspace
            .session_id()
            .map_or_else(|| "not available".into(), |session| session.to_string())
    );
    TerminalLaunch {
        title: workspace.document.name.clone(),
        context,
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hiding_terminal_preserves_every_session() {
        let mut dock = TerminalDock::default();
        dock.new_session(TerminalLaunch {
            title: "Test project".into(),
            context: TerminalContext::new(std::env::current_dir().unwrap()),
            summary: "test context".into(),
        });
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
    fn relative_projects_become_unambiguous_terminal_paths() {
        let workspace = Workspace::new(
            Document::new("Relative", 100, 100),
            Some(PathBuf::from("fixtures/relative.prism")),
        );
        let launch = terminal_launch(&workspace);
        let expected = std::env::current_dir()
            .unwrap()
            .join("fixtures/relative.prism");

        assert_eq!(
            launch.context.environment("PRISM_PROJECT"),
            Some(expected.as_os_str())
        );
        assert_eq!(
            launch.context.working_directory(),
            expected.parent().unwrap()
        );
    }
}
