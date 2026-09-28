//! The real Spectrum library, read through the same engine as the CLI.
use crate::workspace::LibraryView;
use anyhow::Result;
use spectrum::library::{Service, default_root};
use spectrum_library::{Asset, AssetId, Project, ProjectId};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub struct Entry {
    pub asset: Asset,
    /// Unix seconds when it was imported; zero for work made in Spectrum.
    pub added: i64,
    /// For trashed assets, when they will be purged.
    pub purge_after: Option<i64>,
}

pub enum Thumb {
    Loading,
    /// A rendered file and when it became ready, so only fresh ones fade in.
    Ready(PathBuf, std::time::Instant),
    Failed,
}

impl Thumb {
    pub fn ready(path: PathBuf) -> Self {
        Thumb::Ready(path, std::time::Instant::now())
    }
}

pub struct Store {
    pub root: PathBuf,
    pub service: Service,
    pub projects: Vec<Project>,
    pub entries: Vec<Entry>,
    pub thumbs: HashMap<AssetId, Thumb>,
    /// Each project's most recently added asset, used as its cover.
    pub covers: HashMap<ProjectId, AssetId>,
}

/// Files and folders are expanded to the image files inside them.
pub fn importable(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    fn walk(path: &Path, out: &mut Vec<PathBuf>) {
        if path.is_dir() {
            let Ok(entries) = std::fs::read_dir(path) else {
                return;
            };
            let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            children.sort();
            for child in children {
                walk(&child, out);
            }
        } else if lumen_core::project::is_supported_image(path) {
            out.push(path.to_path_buf());
        }
    }
    let mut out = Vec::new();
    for path in paths {
        walk(&path, &mut out);
    }
    out
}

impl Store {
    pub fn is_image(&self, id: AssetId) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.asset.id == id && entry.asset.kind == "image")
    }

    pub fn open() -> Result<Self> {
        let root = default_root()?;
        let mut service = Service::open(&root)?;
        service.ensure_catalog()?;
        service.scan()?;
        service.purge_expired()?;
        Ok(Self {
            root: service.library.root().to_path_buf(),
            service,
            projects: Vec::new(),
            entries: Vec::new(),
            thumbs: HashMap::new(),
            covers: HashMap::new(),
        })
    }

    /// Reloads projects and the assets in `view`. Returns the view actually
    /// shown, which falls back to all assets if a project no longer exists.
    pub fn load(&mut self, view: LibraryView) -> Result<LibraryView> {
        self.service.scan()?;
        let library = &self.service.library;
        self.projects = library.projects()?;
        self.covers = self
            .projects
            .iter()
            .filter_map(|p| {
                let first = library.project_assets(p.id).ok()?.into_iter().next()?;
                Some((p.id, first.id))
            })
            .collect();
        let added: HashMap<AssetId, i64> = library
            .imports()?
            .into_iter()
            .flat_map(|batch| batch.assets.into_iter().map(move |id| (id, batch.created)))
            .collect();
        let view = match view {
            LibraryView::Project(id) if !self.projects.iter().any(|p| p.id == id) => {
                LibraryView::All
            }
            view => view,
        };
        let entry = |asset: Asset, purge_after| Entry {
            added: added.get(&asset.id).copied().unwrap_or(0),
            asset,
            purge_after,
        };
        let entries: Vec<Entry> = match view {
            LibraryView::All => library
                .list()?
                .into_iter()
                .map(|a| entry(a, None))
                .collect(),
            LibraryView::Unassigned => library
                .unassigned()?
                .into_iter()
                .map(|a| entry(a, None))
                .collect(),
            LibraryView::Trash => library
                .trashed()?
                .into_iter()
                .map(|t| entry(t.asset, Some(t.purge_after)))
                .collect(),
            LibraryView::Project(id) => {
                // In a project, "recently added" means added to the project.
                let joined: HashMap<AssetId, i64> =
                    library.project_added(id)?.into_iter().collect();
                library
                    .project_assets(id)?
                    .into_iter()
                    .map(|a| {
                        let mut e = entry(a, None);
                        e.added = joined.get(&e.asset.id).copied().unwrap_or(e.added);
                        e
                    })
                    .collect()
            }
        };
        self.entries = entries
            .into_iter()
            .filter(|e| matches!(e.asset.kind.as_str(), "image" | "canvas"))
            .collect();
        Ok(view)
    }

    pub fn project_name(&self, id: ProjectId) -> Option<&str> {
        self.projects
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.as_str())
    }
}
