use super::*;

impl LumenApp {
    pub(super) fn sync_agent_collaborations(&mut self, context: &egui::Context) {
        let now = Instant::now();
        if now < self.collaboration_poll_at {
            return;
        }
        self.collaboration_poll_at = now + Duration::from_millis(100);
        match self.workspace.sync_together() {
            Ok(spectrum_revisions::CollaborationSync::Advanced { .. }) => {
                self.reset_catalog_view(self.library_mode);
                self.status = "Following the agent's latest changes".into();
                self.error = false;
                context.request_repaint();
            }
            Ok(spectrum_revisions::CollaborationSync::Split(_)) => {
                self.status = "You and the agent are now continuing separately".into();
                self.error = false;
                context.request_repaint();
            }
            Ok(
                spectrum_revisions::CollaborationSync::Idle
                | spectrum_revisions::CollaborationSync::Waiting(_),
            ) => {}
            Err(error) => {
                self.status = format!("Could not follow agent changes: {error:#}");
                self.error = true;
            }
        }
        context.request_repaint_after(Duration::from_millis(100));
    }

    pub(super) fn execute(&mut self, command: Command) -> bool {
        match self.workspace.execute(command) {
            Ok(output) => {
                self.status = output.message;
                self.error = false;
                true
            }
            Err(error) => {
                self.status = format!("{error:#}");
                self.error = true;
                false
            }
        }
    }

    pub(super) fn execute_and_commit(&mut self, command: Command) -> bool {
        let succeeded = self.execute(command);
        if succeeded && let Some(error) = self.workspace.pending_publish_error() {
            self.status =
                format!("change committed locally, but project publication failed: {error}");
            self.error = true;
        }
        succeeded
    }

    pub(super) fn invalidate_selected(&mut self) {
        self.preview = None;
        self.preview_source = None;
        self.preview_fast_source = None;
        self.preview_id = None;
        self.preview_layout_size = None;
        self.original_preview = None;
        self.original_preview_id = None;
        self.histogram = None;
        if let Some(id) = self.workspace.project.selected {
            self.thumbnails.remove(&id);
        }
    }

    pub(super) fn sync_draft(&mut self) {
        let selected = self.workspace.project.selected;
        if self.draft_id != selected {
            self.draft_id = selected;
            self.draft = self
                .workspace
                .project
                .selected_photo()
                .map(|photo| photo.adjustments.clone())
                .unwrap_or_default();
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
            self.crop_mode = false;
            self.crop_drag = None;
            self.spot_mode = false;
            self.spot_stroke_start = None;
            if let Some(id) = selected
                && self.selected_ids.is_empty()
            {
                self.selected_ids.insert(id);
            }
            self.invalidate_selected();
        }
    }

    pub(super) fn finish_edit(&mut self, id: u64) {
        if self.execute_and_commit(Command::SetAdjustments {
            id,
            adjustments: self.draft.clone(),
        }) {
            self.thumbnails.remove(&id);
        }
    }

    pub(super) fn select(&mut self, id: u64) {
        self.selected_ids.clear();
        self.selected_ids.insert(id);
        if self.execute(Command::Select { id }) {
            self.draft_id = None;
            self.sync_draft();
        }
    }

    pub(super) fn select_in_filmstrip(
        &mut self,
        id: u64,
        index: usize,
        modifiers: egui::Modifiers,
    ) {
        if modifiers.shift {
            let last = self.workspace.project.photos.len().saturating_sub(1);
            let anchor = self.selection_anchor.unwrap_or(index).min(last);
            let (start, end) = if anchor <= index {
                (anchor, index)
            } else {
                (index, anchor)
            };
            if !modifiers.command {
                self.selected_ids.clear();
            }
            for photo in &self.workspace.project.photos[start..=end] {
                self.selected_ids.insert(photo.id);
            }
        } else if modifiers.command {
            if !self.selected_ids.remove(&id) {
                self.selected_ids.insert(id);
            }
            if self.selected_ids.is_empty() {
                self.selected_ids.insert(id);
            }
            self.selection_anchor = Some(index);
        } else {
            self.selected_ids.clear();
            self.selected_ids.insert(id);
            self.selection_anchor = Some(index);
        }
        let active = if self.selected_ids.contains(&id) {
            Some(id)
        } else {
            self.selected_ids.iter().next().copied()
        };
        if let Some(active) = active
            && self.execute(Command::Select { id: active })
        {
            self.draft_id = None;
            self.sync_draft();
        }
    }

    pub(super) fn selected_photo_ids(&self) -> Vec<u64> {
        if self.selected_ids.is_empty() {
            self.workspace.project.selected.into_iter().collect()
        } else {
            self.selected_ids.iter().copied().collect()
        }
    }

    pub(super) fn set_pick(&mut self, ids: Vec<u64>, state: PickState) {
        if !ids.is_empty() {
            self.execute_and_commit(Command::SetPick { ids, state });
        }
    }

    pub(super) fn visible_photo_ids(&self) -> Vec<u64> {
        self.workspace
            .project
            .photos
            .iter()
            .filter(|photo| {
                self.active_batch
                    .is_none_or(|batch_id| photo.batch_id == Some(batch_id))
            })
            .filter(|photo| match self.film_filter {
                FilmFilter::All => true,
                FilmFilter::Keeps => photo.pick == PickState::Keep,
                FilmFilter::Rejects => photo.pick == PickState::Reject,
            })
            .map(|photo| photo.id)
            .collect()
    }

    pub(super) fn import(&mut self, paths: Vec<PathBuf>) {
        let paths: Vec<_> = paths
            .into_iter()
            .filter(|path| is_supported_image(path))
            .collect();
        if paths.is_empty() {
            self.status = "No supported images were selected".into();
            self.error = true;
            return;
        }
        let batch_count = self.workspace.project.batches.len();
        self.status = "Reading photo metadata and importing...".into();
        if self.execute_and_commit(Command::Import { paths }) {
            self.thumbnails.clear();
            self.selected_ids.clear();
            self.selection_anchor = None;
            self.draft_id = None;
            self.sync_draft();
            let imported_batch = if self.workspace.project.batches.len() > batch_count {
                self.workspace.project.batches.last().map(|batch| batch.id)
            } else {
                None
            };
            self.active_batch = imported_batch.or_else(|| {
                self.workspace
                    .project
                    .selected_photo()
                    .and_then(|photo| photo.batch_id)
            });
            if let Some(batch_id) = imported_batch
                && let Some(photo_id) = self
                    .workspace
                    .project
                    .photos
                    .iter()
                    .find(|photo| photo.batch_id == Some(batch_id))
                    .map(|photo| photo.id)
            {
                self.select(photo_id);
            }
            self.library_mode = false;
        }
    }

    pub(super) fn import_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter(
                "Photos",
                &["jpg", "jpeg", "png", "tif", "tiff", "webp", "arw"],
            )
            .pick_files()
        {
            self.import(paths);
        }
    }

    pub(super) fn reset_catalog_view(&mut self, library_mode: bool) {
        self.thumbnails.clear();
        self.selected_ids.clear();
        self.selection_anchor = None;
        self.draft_id = None;
        self.preview = None;
        self.preview_source = None;
        self.preview_fast_source = None;
        self.original_preview = None;
        self.histogram = None;
        self.film_filter = FilmFilter::All;
        self.active_batch = None;
        self.library_mode = library_mode;
        self.sync_draft();
    }

    pub(super) fn remember_catalog(&mut self, path: PathBuf) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        self.recent_catalogs.retain(|recent| recent != &path);
        self.recent_catalogs.insert(0, path);
        self.recent_catalogs.truncate(8);
    }

    pub(super) fn request_catalog_switch(&mut self, action: CatalogSwitch) {
        self.apply_catalog_switch(action);
    }

    pub(super) fn receive_open_documents(&mut self, context: &egui::Context) {
        let paths: Vec<_> = self.open_document_receiver.try_iter().collect();
        for path in paths {
            let current = self
                .workspace
                .catalog_path
                .as_ref()
                .and_then(|current| std::fs::canonicalize(current).ok());
            let incoming = std::fs::canonicalize(&path).ok();
            if current.is_some() && current == incoming {
                continue;
            }
            self.apply_catalog_switch(CatalogSwitch::Open(path));
        }
        context.request_repaint_after(Duration::from_millis(50));
    }

    pub(super) fn apply_catalog_switch(&mut self, action: CatalogSwitch) {
        match action {
            CatalogSwitch::Open(path) => match open_local_workspace(&path) {
                Ok(workspace) => {
                    let opened = workspace.catalog_path.clone().unwrap_or(path);
                    self.workspace = workspace;
                    self.remember_catalog(opened);
                    self.reset_catalog_view(true);
                    self.status = "Opened Lumen project".into();
                    self.error = false;
                }
                Err(error) => {
                    self.status = format!("Could not open project: {error:#}");
                    self.error = true;
                }
            },
        }
    }

    pub(super) fn new_catalog(&mut self) {
        match create_managed_workspace(Project::new("Untitled shoot")) {
            Ok(workspace) => {
                let path = workspace.catalog_path.clone();
                self.workspace = workspace;
                if let Some(path) = path {
                    self.remember_catalog(path);
                }
                self.reset_catalog_view(true);
                self.status = "Created a new Lumen project".into();
                self.error = false;
            }
            Err(error) => {
                self.status = format!("Could not create project: {error:#}");
                self.error = true;
            }
        }
    }

    pub(super) fn open_catalog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Lumen project", &["lumen", "lumencatalog"])
            .pick_file()
        {
            self.request_catalog_switch(CatalogSwitch::Open(path));
        }
    }

    pub(super) fn move_project(&mut self) {
        let Some(destination) = rfd::FileDialog::new()
            .add_filter("Lumen project", &["lumen"])
            .set_file_name(format!("{}.lumen", self.workspace.project.name))
            .save_file()
        else {
            return;
        };
        match self.workspace.move_project(&destination) {
            Ok(path) => {
                self.remember_catalog(path.clone());
                self.status = format!("Moved project to {}", path.display());
                self.error = false;
            }
            Err(error) => {
                self.status = format!("Could not move project: {error:#}");
                self.error = true;
            }
        }
    }

    pub(super) fn open_export(&mut self) {
        if !self.selected_photo_ids().is_empty() {
            self.export_open = true;
        }
    }

    pub(super) fn begin_crop(&mut self) {
        self.crop_draft = self.draft.crop.unwrap_or_default();
        self.crop_mode = true;
        self.spot_mode = false;
        self.compare_mode = CompareMode::Edited;
        self.crop_drag = None;
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
        self.preview = None;
    }

    pub(super) fn cancel_crop(&mut self) {
        self.crop_mode = false;
        self.crop_drag = None;
        self.preview = None;
    }

    pub(super) fn apply_crop(&mut self) {
        let Some(id) = self.workspace.project.selected else {
            return;
        };
        self.draft.crop = Some(self.crop_draft.sanitized());
        self.crop_mode = false;
        self.crop_drag = None;
        self.preview = None;
        self.finish_edit(id);
    }

    pub(super) fn preview_adjustments(&self) -> Adjustments {
        let mut adjustments = self.draft.clone();
        if self.crop_mode {
            adjustments.crop = None;
        }
        adjustments
    }

    pub(super) fn ensure_preview(&mut self, context: &egui::Context) {
        let Some(id) = self.workspace.project.selected else {
            self.preview = None;
            return;
        };
        let preview_adjustments = self.preview_adjustments();
        let interacting = context.input(|input| input.pointer.primary_down());
        if self.preview.is_some()
            && self.preview_id == Some(id)
            && self.preview_adjustments == preview_adjustments
            && !(self.preview_fast && !interacting)
        {
            return;
        }
        let Some(photo) = self.workspace.project.selected_photo().cloned() else {
            return;
        };
        if self
            .preview_source
            .as_ref()
            .map(|(source_id, _)| *source_id)
            != Some(id)
        {
            match decode_photo(&photo, Some(1800)) {
                Ok(source) => {
                    let fast = if source.width() > 960 || source.height() > 960 {
                        source.resize(960, 960, FilterType::Triangle)
                    } else {
                        source.clone()
                    };
                    self.preview_fast_source = Some((id, fast));
                    self.preview_source = Some((id, source));
                }
                Err(error) => {
                    self.status = format!("preview failed: {error:#}");
                    self.error = true;
                    return;
                }
            }
        }
        let geometry_changed = self.preview_id == Some(id)
            && !same_preview_geometry(&self.preview_adjustments, &preview_adjustments);
        let use_fast = interacting && self.preview_id == Some(id) && !geometry_changed;
        let source = if use_fast {
            self.preview_fast_source.as_ref()
        } else {
            self.preview_source.as_ref()
        };
        if let Some((_, source)) = source {
            if self.original_preview_id != Some(id) {
                self.original_preview = Some(load_texture(
                    context,
                    format!("original-{id}"),
                    source.clone(),
                ));
                self.original_preview_id = Some(id);
            }
            let rendered = render_image(
                source.clone(),
                preview_adjustments.clone(),
                RenderOptions::default(),
            );
            self.histogram = Some(Histogram::from_image(&rendered));
            if !use_fast || self.preview_layout_size.is_none() {
                self.preview_layout_size =
                    Some(Vec2::new(rendered.width() as f32, rendered.height() as f32));
            }
            self.preview = Some(load_texture(context, format!("preview-{id}"), rendered));
            self.preview_id = Some(id);
            self.preview_fast = use_fast;
            self.preview_adjustments = preview_adjustments;
        }
    }

    pub(super) fn ensure_thumbnail(&mut self, context: &egui::Context, id: u64) {
        if self.thumbnails.contains_key(&id) {
            return;
        }
        let Ok(photo) = self.workspace.project.photo(id) else {
            return;
        };
        if let Ok(rendered) = render_photo(
            photo,
            RenderOptions {
                max_size: Some(240),
            },
        ) {
            self.thumbnails.insert(
                id,
                load_texture(context, format!("thumbnail-{id}"), rendered),
            );
        }
    }

    pub(super) fn handle_drop_and_shortcuts(&mut self, context: &egui::Context) {
        let dropped = context.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect::<Vec<_>>()
        });
        if !dropped.is_empty() {
            self.import(dropped);
        }
        if context.input(|input| input.modifiers.command && input.key_pressed(egui::Key::S)) {
            if context.input(|input| input.modifiers.shift) {
                self.move_project();
            } else {
                self.status = "Every completed action is already saved".into();
                self.error = false;
            }
        }
        #[cfg(not(target_os = "macos"))]
        if context.input(|input| input.modifiers.command && input.key_pressed(egui::Key::H)) {
            self.toggle_history();
        }
        if context.input(|input| input.modifiers.command && input.key_pressed(egui::Key::A))
            && !context.egui_wants_keyboard_input()
        {
            self.select_all_visible_photos();
        }
        #[cfg(not(target_os = "macos"))]
        {
            let history = context.input(|input| {
                if input.modifiers.command && input.key_pressed(egui::Key::Z) {
                    Some(input.modifiers.shift)
                } else {
                    None
                }
            });
            if let Some(forward) = history {
                let command = if forward {
                    Command::Redo
                } else {
                    Command::Undo
                };
                if self.execute(command) {
                    self.draft_id = None;
                    self.sync_draft();
                }
            }
        }
        let direction = context.input(|input| {
            if input.key_pressed(egui::Key::ArrowLeft) || input.key_pressed(egui::Key::ArrowUp) {
                -1
            } else if input.key_pressed(egui::Key::ArrowRight)
                || input.key_pressed(egui::Key::ArrowDown)
            {
                1
            } else {
                0
            }
        });
        if direction != 0 && !context.egui_wants_keyboard_input() {
            self.select_relative(direction);
        }
        if !context.egui_wants_keyboard_input() {
            let pick = context.input(|input| {
                if input.key_pressed(egui::Key::P) {
                    Some(PickState::Keep)
                } else if input.key_pressed(egui::Key::X) {
                    Some(PickState::Reject)
                } else if input.key_pressed(egui::Key::U) {
                    Some(PickState::Unmarked)
                } else {
                    None
                }
            });
            if let Some(state) = pick {
                self.set_pick(self.selected_photo_ids(), state);
            }
            if context.input(|input| input.key_pressed(egui::Key::C)) {
                self.compare_mode = if self.compare_mode == CompareMode::Edited {
                    CompareMode::SideBySide
                } else {
                    CompareMode::Edited
                };
            }
            if context.input(|input| input.key_pressed(egui::Key::Delete))
                && self.workspace.project.selected.is_some()
            {
                self.remove_confirmation = true;
            }
        }
    }

    pub(super) fn select_all_visible_photos(&mut self) {
        self.selected_ids = self.visible_photo_ids().into_iter().collect();
        self.selection_anchor = self.workspace.project.selected.and_then(|id| {
            self.workspace
                .project
                .photos
                .iter()
                .position(|photo| photo.id == id)
        });
    }

    pub(super) fn select_relative(&mut self, direction: i32) {
        let visible = self.visible_photo_ids();
        if visible.is_empty() {
            return;
        }
        let current = self
            .workspace
            .project
            .selected
            .and_then(|id| visible.iter().position(|visible| *visible == id))
            .unwrap_or(0) as i32;
        let index = (current + direction).clamp(0, visible.len() as i32 - 1) as usize;
        self.select(visible[index]);
    }
}

fn local_session_id() -> anyhow::Result<spectrum_revisions::SessionId> {
    let directory = eframe::storage_dir("Spectrum")
        .ok_or_else(|| anyhow::anyhow!("Spectrum could not locate its local application folder"))?;
    spectrum_revisions::local_session_id(&directory).map_err(Into::into)
}

fn local_actor(session: spectrum_revisions::SessionId) -> spectrum_revisions::Actor {
    spectrum_revisions::Actor {
        id: format!("local:human:{session}"),
        display_name: "Local User".into(),
        kind: spectrum_revisions::ActorKind::Human,
    }
}

pub(super) fn open_local_workspace(path: &Path) -> anyhow::Result<Workspace> {
    let session = local_session_id()?;
    Workspace::open_as(path, local_actor(session), session)
}

pub(super) fn create_workspace_at(project: Project, path: &Path) -> anyhow::Result<Workspace> {
    let session = local_session_id()?;
    Workspace::create_durable(project, path, local_actor(session), session)
}

pub(super) fn create_managed_workspace(project: Project) -> anyhow::Result<Workspace> {
    let directory = eframe::storage_dir("Lumen")
        .map(|directory| directory.join("Projects"))
        .ok_or_else(|| anyhow::anyhow!("Lumen could not locate its local application folder"))?;
    std::fs::create_dir_all(&directory)?;
    let path = available_project_path(&directory, &project.name);
    create_workspace_at(project, &path)
}

fn available_project_path(directory: &Path, name: &str) -> PathBuf {
    let stem = safe_project_stem(name);
    let initial = directory.join(format!("{stem}.lumen"));
    if !initial.exists() {
        return initial;
    }
    for copy in 2..u32::MAX {
        let candidate = directory.join(format!("{stem} {copy}.lumen"));
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!(
        "{stem}-{}.lumen",
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
        "Untitled shoot".into()
    } else {
        stem.into()
    }
}
