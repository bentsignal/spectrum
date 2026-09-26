use super::*;
impl LumenApp {
    pub(crate) fn library_selected(&self) -> Option<(PathBuf, u64)> {
        Some((
            self.workspace.catalog_path.clone()?,
            self.workspace.project.selected?,
        ))
    }
    pub(crate) fn library_open(&mut self, path: PathBuf, id: u64) {
        self.apply_catalog_switch(CatalogSwitch::Open(path));
        self.select(id);
        self.library_mode = false;
    }
}
