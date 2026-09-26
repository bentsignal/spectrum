use super::*;
impl PrismApp {
    pub(crate) fn library_set_exporter(&mut self, exporter: LibraryExporter) {
        self.library_exporter = Some(exporter);
    }
    pub(crate) fn library_path(&self) -> Option<PathBuf> {
        self.workspace.project_path.clone()
    }
    pub(crate) fn library_selected(
        &self,
    ) -> Option<(u64, Option<spectrum_library::AssetId>, PathBuf)> {
        let layer = self
            .workspace
            .document
            .layer(self.workspace.document.selected?)
            .ok()?;
        let LayerKind::Raster { path, .. } = &layer.kind else {
            return None;
        };
        Some((layer.id, layer.image_asset, path.clone()))
    }
    pub(crate) fn library_open(&mut self, path: &Path) {
        self.open_path(path);
    }
    pub(crate) fn library_place(
        &mut self,
        asset: spectrum_library::AssetId,
        path: PathBuf,
        name: String,
    ) {
        self.execute(Command::AddLinkedImage { path, name, asset });
    }
    pub(crate) fn library_link(&mut self, id: u64, asset: spectrum_library::AssetId) {
        self.execute(Command::LinkImage { id, asset });
    }
    pub(crate) fn library_refresh(
        &mut self,
        previews: &HashMap<spectrum_library::AssetId, PathBuf>,
    ) {
        // Derived pixels are not a user edit and must not generate history entries.
        let mut changed = false;
        for workspace in
            std::iter::once(&mut self.workspace).chain(self.inactive_workspaces.values_mut())
        {
            if workspace.interaction_active() {
                continue;
            }
            for layer in &mut workspace.document.layers {
                if let Some(path) = layer.image_asset.and_then(|id| previews.get(&id))
                    && let LayerKind::Raster {
                        path: current,
                        original_path,
                    } = &mut layer.kind
                    && current != path
                {
                    *current = path.clone();
                    *original_path = None;
                    changed = true;
                }
            }
        }
        if changed {
            self.reset_canvas_cache();
            self.sync_active_raster_sources();
        }
    }
}
impl PrismApp {
    pub(crate) fn library_references(&self) -> Vec<spectrum_library::AssetId> {
        let ids: HashSet<_> = std::iter::once(&self.workspace)
            .chain(self.inactive_workspaces.values())
            .flat_map(|w| w.document.layers.iter().filter_map(|l| l.image_asset))
            .collect();
        ids.into_iter().collect()
    }
}

pub(crate) type CanvasSummary = (PathBuf, String, Vec<(String, spectrum_library::AssetId)>);
impl PrismApp {
    pub(crate) fn library_summaries(&self) -> Vec<CanvasSummary> {
        std::iter::once(&self.workspace)
            .chain(self.inactive_workspaces.values())
            .filter_map(|workspace| {
                if workspace.interaction_active() {
                    return None;
                }
                let path = workspace.project_path.clone()?;
                let links = workspace
                    .document
                    .layers
                    .iter()
                    .filter_map(|layer| layer.image_asset.map(|id| (layer.id.to_string(), id)))
                    .collect();
                Some((path, workspace.document.name.clone(), links))
            })
            .collect()
    }
}
