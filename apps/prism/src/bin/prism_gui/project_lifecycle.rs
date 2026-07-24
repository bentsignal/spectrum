use super::dialogs::{
    ModalAction, RENAME_DOCUMENT_ID, modal_action, modal_text_input, reset_modal_text_input,
};
use super::*;

#[derive(Clone, Debug)]
pub(super) struct NewDocumentDialog {
    pub(super) name: String,
    pub(super) width: u32,
    pub(super) height: u32,
}

impl Default for NewDocumentDialog {
    fn default() -> Self {
        Self {
            name: "Untitled artwork".into(),
            width: 1920,
            height: 1080,
        }
    }
}

const KEEP_ONLY_TAB_STATUS: &str =
    "This project is still open. Open or create another document before closing its tab.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TabClosePlan {
    Close { position: usize },
    KeepOnlyTab,
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TabCloseAffordance {
    pub(super) enabled: bool,
    pub(super) hover_text: &'static str,
}

fn tab_close_plan(tab_ids: &[u64], id: u64) -> TabClosePlan {
    let Some(position) = tab_ids.iter().position(|tab| *tab == id) else {
        return TabClosePlan::Missing;
    };
    if tab_ids.len() == 1 {
        TabClosePlan::KeepOnlyTab
    } else {
        TabClosePlan::Close { position }
    }
}

fn begin_tab_close(
    tab_ids: &[u64],
    id: u64,
    status: &mut String,
    status_error: &mut bool,
) -> Option<usize> {
    match tab_close_plan(tab_ids, id) {
        TabClosePlan::Close { position } => Some(position),
        TabClosePlan::KeepOnlyTab => {
            *status = KEEP_ONLY_TAB_STATUS.into();
            *status_error = true;
            None
        }
        TabClosePlan::Missing => {
            *status = "That document tab is no longer open".into();
            *status_error = true;
            None
        }
    }
}

pub(super) fn tab_close_affordance(tab_count: usize) -> TabCloseAffordance {
    if tab_count <= 1 {
        TabCloseAffordance {
            enabled: false,
            hover_text: "Keep at least one document open",
        }
    } else {
        TabCloseAffordance {
            enabled: true,
            hover_text: "Close document",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct MoveProjectDialog {
    destination_directory: Option<PathBuf>,
}

fn local_actor(session: spectrum_revisions::SessionId) -> spectrum_revisions::Actor {
    spectrum_revisions::Actor {
        id: format!("local:human:{session}"),
        display_name: "Local User".into(),
        kind: spectrum_revisions::ActorKind::Human,
    }
}

fn local_session_id() -> anyhow::Result<spectrum_revisions::SessionId> {
    let directory = eframe::storage_dir("Spectrum")
        .ok_or_else(|| anyhow::anyhow!("Spectrum could not locate its local application folder"))?;
    spectrum_revisions::local_session_id(&directory).map_err(Into::into)
}

pub(super) fn open_local_workspace(path: &Path) -> anyhow::Result<Workspace> {
    let session = local_session_id()?;
    Workspace::open_as(path, local_actor(session), session)
}

pub(super) fn create_managed_workspace(document: Document) -> anyhow::Result<Workspace> {
    let directory = managed_projects_directory()?;
    std::fs::create_dir_all(&directory)?;
    let path = available_project_path(&directory, &document.name);
    let session = local_session_id()?;
    Workspace::create_durable(document, &path, local_actor(session), session)
}

fn managed_projects_directory() -> anyhow::Result<PathBuf> {
    eframe::storage_dir("Prism")
        .map(|directory| directory.join("Projects"))
        .ok_or_else(|| anyhow::anyhow!("Prism could not locate its local application folder"))
}

fn available_project_path(directory: &Path, document_name: &str) -> PathBuf {
    let stem = safe_project_stem(document_name);
    let initial = directory.join(format!("{stem}.prism"));
    if !initial.exists() {
        return initial;
    }
    for copy in 2..u32::MAX {
        let candidate = directory.join(format!("{stem} {copy}.prism"));
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!(
        "{stem}-{}.prism",
        spectrum_revisions::ProjectId::new()
    ))
}

fn safe_project_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .filter(|character| {
            !character.is_control() && !matches!(character, '/' | '\\' | ':' | '\0')
        })
        .take(96)
        .collect();
    let stem = stem.trim().trim_matches('.');
    if stem.is_empty() {
        "Untitled artwork".into()
    } else {
        stem.into()
    }
}

impl PrismApp {
    pub(super) fn close_active_tab(&mut self) {
        self.close_tab(self.active_tab_id);
    }

    pub(super) fn close_tab(&mut self, id: u64) {
        self.clear_font_hover_preview();
        // Decide before settling inline text: refusing a close must not commit or cancel any live
        // project interaction.
        let Some(position) =
            begin_tab_close(&self.tab_ids, id, &mut self.status, &mut self.status_error)
        else {
            return;
        };

        self.settle_inline_text_editor();
        self.cancel_brush();
        self.cancel_lasso();
        let dirty = if id == self.active_tab_id {
            self.workspace.is_dirty()
        } else {
            self.inactive_workspaces
                .get(&id)
                .is_some_and(Workspace::is_dirty)
        };
        if dirty {
            self.activate_tab(id);
            self.status = "This legacy document must be converted before its tab can close".into();
            self.status_error = true;
            return;
        }

        let closed_name = if id == self.active_tab_id {
            self.workspace.document.name.clone()
        } else {
            self.inactive_workspaces.get(&id).map_or_else(
                || "document".into(),
                |workspace| workspace.document.name.clone(),
            )
        };
        self.tab_ids.remove(position);
        if id == self.active_tab_id {
            let replacement_id = self.tab_ids[position.min(self.tab_ids.len() - 1)];
            if let Some(replacement) = self.inactive_workspaces.remove(&replacement_id) {
                self.workspace = replacement;
                self.active_tab_id = replacement_id;
                // Install the replacement source set before forgetting the old tab so an
                // overlapping ready provider and its generation survive atomically.
                self.sync_active_raster_sources();
            }
        } else {
            self.inactive_workspaces.remove(&id);
        }
        self.raster_sources.remove_tab(id);
        self.history.workspace_changed();
        self.sync_active_raster_sources();
        self.reset_canvas_cache();
        self.layer_thumbnails.clear();
        self.fit_requested = true;
        self.pan = Vec2::ZERO;
        self.drag = None;
        self.smart_guides = SmartGuides::default();
        self.alignment_reference = None;
        self.status = format!("Closed {closed_name}");
        self.status_error = false;
    }

    pub(super) fn begin_rename_document(&mut self) {
        self.settle_inline_text_editor();
        self.rename_document = Some(self.workspace.document.name.clone());
    }

    pub(super) fn rename_document_dialog(&mut self, context: &egui::Context) {
        let Some(mut name) = self.rename_document.take() else {
            return;
        };
        let mut keep_open = true;
        let mut rename = false;
        egui::Window::new("Rename document")
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .fixed_size(Vec2::new(480.0, 178.0))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label(
                    RichText::new(
                        "Changes the title shown in Prism. The .prism filename and location stay unchanged; use Move Project to relocate the file.",
                    )
                    .color(MUTED),
                );
                ui.add_space(8.0);
                modal_text_input(ui, &mut name, RENAME_DOCUMENT_ID, false);
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, "Rename").clicked() {
                        rename = true;
                        keep_open = false;
                    }
                    if quiet_button(ui, "Cancel").clicked() {
                        keep_open = false;
                    }
                });
                match modal_action(ui) {
                    ModalAction::Confirm => {
                        rename = true;
                        keep_open = false;
                    }
                    ModalAction::Cancel => keep_open = false,
                    ModalAction::None => {}
                }
            });

        if rename {
            if self.execute(Command::RenameDocument { name: name.clone() }) {
                reset_modal_text_input(context, RENAME_DOCUMENT_ID);
            } else {
                keep_open = true;
            }
        }
        if keep_open {
            self.rename_document = Some(name);
        } else {
            reset_modal_text_input(context, RENAME_DOCUMENT_ID);
        }
    }

    pub(super) fn open_project_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Prism project", &["prism", "mica"])
            .pick_file()
        {
            self.open_path(&path);
        }
    }

    pub(super) fn sync_agent_collaborations(&mut self, context: &egui::Context) {
        let now = std::time::Instant::now();
        if now < self.collaboration_poll_at {
            return;
        }
        self.collaboration_poll_at = now + std::time::Duration::from_millis(100);

        match self.workspace.sync_together() {
            Ok(spectrum_revisions::CollaborationSync::Advanced { collaboration, .. }) => {
                let agent = collaboration_agent_name(&self.workspace, collaboration.agent_session);
                self.finish_durable_revision_advance();
                self.apply_canvas_invalidation(CanvasInvalidation::All);
                self.sync_active_raster_sources();
                self.history.mark_stale();
                self.status = format!("Following {agent}'s latest changes");
                self.status_error = false;
                context.request_repaint();
            }
            Ok(spectrum_revisions::CollaborationSync::Split(collaboration)) => {
                let agent = collaboration_agent_name(&self.workspace, collaboration.agent_session);
                self.history.mark_stale();
                self.status = format!("You and {agent} are now continuing separately");
                self.status_error = false;
                context.request_repaint();
            }
            Ok(
                spectrum_revisions::CollaborationSync::Idle
                | spectrum_revisions::CollaborationSync::Waiting(_),
            ) => {}
            Err(error) => {
                self.status = format!("Could not follow agent changes: {error:#}");
                self.status_error = true;
            }
        }

        for (tab_id, workspace) in &mut self.inactive_workspaces {
            if matches!(
                workspace.sync_together(),
                Ok(spectrum_revisions::CollaborationSync::Advanced { .. })
            ) {
                self.raster_sources
                    .set_tab_document(*tab_id, &workspace.document);
            }
        }
    }

    pub(super) fn receive_open_documents(&mut self, context: &egui::Context) {
        let now = std::time::Instant::now();
        self.pending_open_documents.extend(
            self.open_document_receiver
                .try_iter()
                .map(|path| (now + std::time::Duration::from_millis(100), path)),
        );
        if !self.workspace_initialized {
            if self
                .pending_open_documents
                .front()
                .is_some_and(|(ready_at, _)| *ready_at <= now)
            {
                let (_, path) = self
                    .pending_open_documents
                    .pop_front()
                    .expect("front was just checked");
                self.replace_with_opened_project(&path);
                self.workspace_initialized = true;
            } else if self.pending_open_documents.is_empty() && now >= self.startup_project_ready_at
            {
                match create_managed_workspace(Document::default()) {
                    Ok(workspace) => {
                        self.settle_inline_text_editor();
                        self.cancel_brush();
                        self.workspace = workspace;
                        self.sync_active_raster_sources();
                    }
                    Err(error) => {
                        self.status = format!("Could not create local project: {error:#}");
                        self.status_error = true;
                    }
                }
                self.workspace_initialized = true;
            }
        }
        while self.workspace_initialized
            && self
                .pending_open_documents
                .front()
                .is_some_and(|(ready_at, _)| *ready_at <= now)
        {
            let (_, path) = self
                .pending_open_documents
                .pop_front()
                .expect("front was just checked");
            self.open_path(&path);
        }
        if let Some(delay) = next_open_document_repaint_delay(
            now,
            self.workspace_initialized,
            self.startup_project_ready_at,
            self.pending_open_documents.front(),
        ) {
            context.request_repaint_after(delay);
        }
    }

    fn replace_with_opened_project(&mut self, path: &Path) {
        match open_local_workspace(path) {
            Ok(workspace) => {
                self.settle_inline_text_editor();
                self.cancel_brush();
                self.workspace = workspace;
                self.sync_active_raster_sources();
                self.status = format!("Opened {}", path.display());
                self.status_error = false;
            }
            Err(error) => {
                self.status = format!("Could not open project: {error:#}");
                self.status_error = true;
            }
        }
    }

    pub(super) fn open_path(&mut self, path: &Path) {
        match open_local_workspace(path) {
            Ok(workspace) => {
                self.add_workspace_tab(workspace);
                self.status = format!("Opened {}", path.display());
                self.status_error = false;
            }
            Err(error) => {
                self.status = format!("Could not open project: {error:#}");
                self.status_error = true;
            }
        }
    }

    pub(super) fn new_document(&mut self, draft: NewDocumentDialog) {
        match create_managed_workspace(Document::new(draft.name, draft.width, draft.height)) {
            Ok(workspace) => {
                let path = workspace.project_path.clone();
                self.add_workspace_tab(workspace);
                self.status = path.map_or_else(
                    || "Created a new Prism project".into(),
                    |path| format!("Created {}", path.display()),
                );
                self.status_error = false;
            }
            Err(error) => {
                self.status = format!("Could not create project: {error:#}");
                self.status_error = true;
            }
        }
    }

    pub(super) fn begin_move_project(&mut self) {
        self.settle_inline_text_editor();
        self.move_project_dialog = Some(MoveProjectDialog::default());
    }

    pub(super) fn move_project_dialog(&mut self, context: &egui::Context) {
        let Some(mut dialog) = self.move_project_dialog.take() else {
            return;
        };
        let Some(current) = self.workspace.project_path.clone() else {
            self.status = "This project does not have a durable location".into();
            self.status_error = true;
            return;
        };
        let mut keep_open = true;
        let mut move_now = false;
        egui::Window::new("Move Prism project")
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .fixed_size(Vec2::new(520.0, 228.0))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label(RichText::new("Current location").strong());
                ui.label(
                    RichText::new(current.display().to_string())
                        .monospace()
                        .color(MUTED),
                );
                if quiet_button(ui, "Show in Finder").clicked()
                    && let Err(error) = reveal_project(&current)
                {
                    self.status = format!("Could not reveal project: {error:#}");
                    self.status_error = true;
                }

                ui.add_space(10.0);
                ui.label(RichText::new("Destination folder").strong());
                ui.label(
                    RichText::new(dialog.destination_directory.as_ref().map_or_else(
                        || "Choose where this project should live".into(),
                        |path| path.display().to_string(),
                    ))
                    .monospace()
                    .color(MUTED),
                );
                if secondary_button(ui, "Choose folder…")
                    .on_hover_text("Select the destination folder")
                    .clicked()
                    && let Some(destination) = rfd::FileDialog::new()
                        .set_directory(current.parent().unwrap_or_else(|| Path::new(".")))
                        .pick_folder()
                {
                    dialog.destination_directory = Some(destination);
                }

                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(dialog.destination_directory.is_some(), |ui| {
                        if primary_button(ui, "Move project").clicked() {
                            move_now = true;
                            keep_open = false;
                        }
                    });
                    if quiet_button(ui, "Cancel").clicked() {
                        keep_open = false;
                    }
                });
            });

        if move_now {
            let directory = dialog
                .destination_directory
                .expect("move button requires a destination");
            let file_name = current
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("Untitled artwork.prism"));
            let destination = directory.join(file_name);
            match self.workspace.move_project(&destination) {
                Ok(path) => {
                    self.status = format!("Project now lives at {}", path.display());
                    self.status_error = false;
                }
                Err(error) => {
                    self.status = format!("Could not move project: {error:#}");
                    self.status_error = true;
                }
            }
        } else if keep_open {
            self.move_project_dialog = Some(dialog);
        }
    }
}

fn next_open_document_repaint_delay(
    now: std::time::Instant,
    workspace_initialized: bool,
    startup_project_ready_at: std::time::Instant,
    pending: Option<&(std::time::Instant, PathBuf)>,
) -> Option<std::time::Duration> {
    pending
        .map(|(ready_at, _)| ready_at.saturating_duration_since(now))
        .or_else(|| {
            (!workspace_initialized)
                .then(|| startup_project_ready_at.saturating_duration_since(now))
        })
}

fn collaboration_agent_name(
    workspace: &Workspace,
    session_id: spectrum_revisions::SessionId,
) -> String {
    workspace
        .history()
        .ok()
        .flatten()
        .and_then(|history| {
            history
                .sessions
                .into_iter()
                .find(|session| session.id == session_id)
        })
        .map_or_else(|| "the agent".into(), |session| session.actor.display_name)
}

#[cfg(target_os = "macos")]
fn reveal_project(path: &Path) -> anyhow::Result<()> {
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn reveal_project(path: &Path) -> anyhow::Result<()> {
    std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn()?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn reveal_project(path: &Path) -> anyhow::Result<()> {
    std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or_else(|| Path::new(".")))
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_names_are_safe_portable_file_names() {
        assert_eq!(
            safe_project_stem(" Campaign / Summer: 2026 "),
            "Campaign  Summer 2026"
        );
        assert_eq!(safe_project_stem("..."), "Untitled artwork");
    }

    #[test]
    fn lone_tab_close_is_explicit_and_cannot_mutate_its_workspace() {
        let workspace = Workspace::new(Document::new("Still open", 320, 200), None);
        let before = workspace.document.clone();
        let generation = workspace.document_generation();
        let mut status = "Ready".to_owned();
        let mut status_error = false;

        assert_eq!(
            begin_tab_close(&[7], 7, &mut status, &mut status_error),
            None
        );
        assert_eq!(status, KEEP_ONLY_TAB_STATUS);
        assert!(status_error);
        assert_eq!(workspace.document, before);
        assert_eq!(workspace.document_generation(), generation);
    }

    #[test]
    fn lone_tab_close_affordance_is_disabled_and_annotated() {
        let lone = tab_close_affordance(1);
        assert!(!lone.enabled);
        assert_eq!(lone.hover_text, "Keep at least one document open");

        let multiple = tab_close_affordance(2);
        assert!(multiple.enabled);
        assert_eq!(multiple.hover_text, "Close document");
    }

    #[test]
    fn tab_close_plan_rejects_stale_ids_without_changing_the_tab_list() {
        let tabs = vec![3, 9];
        assert_eq!(tab_close_plan(&tabs, 4), TabClosePlan::Missing);
        assert_eq!(tabs, vec![3, 9]);
        assert_eq!(
            tab_close_plan(&tabs, 9),
            TabClosePlan::Close { position: 1 }
        );
    }

    #[test]
    fn idle_project_does_not_force_periodic_full_app_repaints() {
        let now = std::time::Instant::now();
        assert_eq!(next_open_document_repaint_delay(now, true, now, None), None);
    }

    #[test]
    fn startup_and_debounced_open_documents_schedule_only_their_deadlines() {
        let now = std::time::Instant::now();
        let startup_delay = std::time::Duration::from_millis(250);
        assert_eq!(
            next_open_document_repaint_delay(now, false, now + startup_delay, None),
            Some(startup_delay)
        );

        let open_delay = std::time::Duration::from_millis(100);
        let pending = (now + open_delay, PathBuf::from("Artwork.prism"));
        assert_eq!(
            next_open_document_repaint_delay(now, true, now, Some(&pending)),
            Some(open_delay)
        );
    }
}
