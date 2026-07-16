use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use image::GenericImageView;
use serde::{Deserialize, Serialize};

use crate::Adjustments;

pub const CATALOG_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Photo {
    pub id: u64,
    pub path: PathBuf,
    pub name: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub adjustments: Adjustments,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub next_id: u64,
    pub selected: Option<u64>,
    pub photos: Vec<Photo>,
}

impl Default for Project {
    fn default() -> Self {
        Self::new("Untitled")
    }
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            version: CATALOG_VERSION,
            name: name.into(),
            next_id: 1,
            selected: None,
            photos: Vec::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes =
            fs::read(path).with_context(|| format!("could not read catalog {}", path.display()))?;
        let project: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid catalog {}", path.display()))?;
        if project.version > CATALOG_VERSION {
            bail!(
                "catalog version {} is newer than this app supports ({CATALOG_VERSION})",
                project.version
            );
        }
        Ok(project)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        let json = serde_json::to_vec_pretty(self)?;
        fs::write(&temporary, json)
            .with_context(|| format!("could not write {}", temporary.display()))?;
        fs::rename(&temporary, path)
            .with_context(|| format!("could not replace catalog {}", path.display()))?;
        Ok(())
    }

    pub fn photo(&self, id: u64) -> Result<&Photo> {
        self.photos
            .iter()
            .find(|photo| photo.id == id)
            .with_context(|| format!("photo {id} is not in this catalog"))
    }

    pub fn photo_mut(&mut self, id: u64) -> Result<&mut Photo> {
        self.photos
            .iter_mut()
            .find(|photo| photo.id == id)
            .with_context(|| format!("photo {id} is not in this catalog"))
    }

    pub fn selected_photo(&self) -> Option<&Photo> {
        self.selected.and_then(|id| self.photo(id).ok())
    }

    pub fn import(&mut self, paths: &[PathBuf]) -> Result<Vec<u64>> {
        let mut imported = Vec::new();
        for input in paths {
            if !is_supported_image(input) {
                bail!("unsupported image type: {}", input.display());
            }
            let path = fs::canonicalize(input)
                .with_context(|| format!("could not find image {}", input.display()))?;
            if let Some(existing) = self.photos.iter().find(|photo| photo.path == path) {
                imported.push(existing.id);
                continue;
            }
            let dimensions = image::ImageReader::open(&path)
                .with_context(|| format!("could not open {}", path.display()))?
                .with_guessed_format()?
                .decode()
                .with_context(|| format!("could not decode {}", path.display()))?
                .dimensions();
            let id = self.next_id;
            self.next_id += 1;
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            self.photos.push(Photo {
                id,
                path,
                name,
                width: dimensions.0,
                height: dimensions.1,
                adjustments: Adjustments::default(),
            });
            imported.push(id);
        }
        if self.selected.is_none() {
            self.selected = imported.first().copied();
        }
        Ok(imported)
    }
}

pub fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "tif" | "tiff" | "webp"
            )
        })
}
