use super::*;
use std::sync::{Arc, Mutex};

struct QueryBackend {
    context: TerminalContext,
    events: Vec<TerminalEvent>,
    writes: Arc<Mutex<Vec<Vec<u8>>>>,
    running: bool,
}

impl spectrum_terminal::TerminalSessionBackend for QueryBackend {
    fn context(&self) -> &TerminalContext {
        &self.context
    }

    fn process_id(&self) -> Option<u32> {
        None
    }

    fn write(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        self.writes.lock().unwrap().push(bytes.to_vec());
        Ok(())
    }

    fn resize(&self, _size: TerminalSize) -> anyhow::Result<()> {
        Ok(())
    }

    fn poll(&mut self) -> Vec<TerminalEvent> {
        std::mem::take(&mut self.events)
    }

    fn is_running(&mut self) -> bool {
        self.running
    }

    fn terminate(&mut self) -> anyhow::Result<()> {
        self.running = false;
        Ok(())
    }
}

fn test_launch() -> TerminalLaunch {
    TerminalLaunch {
        title: "Test project".into(),
        context: TerminalContext::new(std::env::current_dir().unwrap()),
    }
}

fn tab_with_process(process: TerminalSession) -> TerminalTab {
    let size = TerminalSize::new(24, 80);
    TerminalTab {
        id: 1,
        title: "Protocol".into(),
        context_title: "Test".into(),
        context: TerminalContext::new(std::env::current_dir().unwrap()),
        process: Some(process),
        parser: terminal_parser(size),
        size,
        running: true,
        message: None,
        selection: None,
        mouse_buttons: [false; 3],
        last_mouse_cell: None,
        last_activity: std::time::Instant::now(),
        native: false,
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
fn terminal_shortcut_help_is_on_demand_without_persistent_help_rows() {
    let guidance = terminal_rail_guidance(shortcuts::SHORTCUT_LABELS);
    assert_eq!(
        guidance.persistent_rows,
        [format!("{}  editor", shortcuts::SHORTCUT_LABELS.terminal)]
    );
    assert_eq!(
        guidance.on_demand_rows,
        [
            shortcuts::SHORTCUT_LABELS.terminal_meta.to_owned(),
            shortcuts::SHORTCUT_LABELS.terminal_clipboard.to_owned(),
            "Shift+PageUp/PageDown scroll".to_owned(),
        ]
    );
    for help in &guidance.on_demand_rows {
        assert!(!guidance.persistent_rows[0].contains(help));
    }
}

#[test]
fn existing_terminal_footer_exposes_on_demand_help_accessibly() {
    use egui::accesskit::{Action, Role};

    let context = egui::Context::default();
    context.enable_accesskit();
    let guidance = terminal_rail_guidance(shortcuts::SHORTCUT_LABELS);
    let expected_label = format!(
        "{}. Terminal shortcuts: {}",
        guidance.persistent_rows[0],
        guidance.on_demand_rows.join("; ")
    );
    let mut semantics = None;
    let _ = context.run_ui(egui::RawInput::default(), |ui| {
        let response = terminal_guidance_footer(ui, &guidance);
        response.request_focus();
        semantics = context.accesskit_node_builder(response.id, |node| {
            (
                node.role(),
                node.label().map(str::to_owned),
                node.supports_action(Action::Click),
                node.supports_action(Action::Focus),
            )
        });
    });
    assert_eq!(
        semantics,
        Some((Role::Button, Some(expected_label), true, true))
    );
}

#[test]
fn parser_queries_write_bounded_replies_back_through_the_portable_backend() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let backend = QueryBackend {
        context: TerminalContext::new(std::env::current_dir().unwrap()),
        events: vec![TerminalEvent::Output(b"\x1b[3;5H\x1b[5n\x1b[6n".to_vec())],
        writes: writes.clone(),
        running: true,
    };
    let mut tab = tab_with_process(TerminalSession::from_backend(backend));

    assert!(tab.poll());
    assert_eq!(
        *writes.lock().unwrap(),
        [b"\x1b[0n".to_vec(), b"\x1b[3;5R".to_vec()]
    );
    assert!(tab.message.is_none());
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

#[test]
fn wide_glyph_continuations_normalize_to_one_copyable_character() {
    let mut parser = terminal_parser(TerminalSize::new(2, 12));
    parser.process("A界B".as_bytes());
    assert!(parser.screen().cell(0, 2).unwrap().is_wide_continuation());
    let selected = normalize_selection_cell(parser.screen(), CellPosition { row: 0, col: 2 });
    assert_eq!(selected, CellPosition { row: 0, col: 1 });
    assert_eq!(
        parser
            .screen()
            .contents_between(0, selected.col, 0, selected.col + 1),
        "界"
    );
}
