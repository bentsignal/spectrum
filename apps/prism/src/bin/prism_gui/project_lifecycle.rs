use super::*;

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
    pub(super) fn sync_agent_collaborations(&mut self, context: &egui::Context) {
        let now = std::time::Instant::now();
        if now < self.collaboration_poll_at {
            return;
        }
        self.collaboration_poll_at = now + std::time::Duration::from_millis(100);

        match self.workspace.sync_together() {
            Ok(spectrum_revisions::CollaborationSync::Advanced { collaboration, .. }) => {
                let agent = collaboration_agent_name(&self.workspace, collaboration.agent_session);
                self.apply_canvas_invalidation(CanvasInvalidation::All);
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

        for workspace in self.inactive_workspaces.values_mut() {
            let _ = workspace.sync_together();
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
                    Ok(workspace) => self.workspace = workspace,
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
        context.request_repaint_after(std::time::Duration::from_millis(50));
    }

    fn replace_with_opened_project(&mut self, path: &Path) {
        match open_local_workspace(path) {
            Ok(workspace) => {
                self.workspace = workspace;
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
                if quiet_button(ui, "Choose folder…").clicked()
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
}
