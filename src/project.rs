use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use rawler::{RawLoader, decoders::RawDecodeParams, formats::tiff::Rational, rawsource::RawSource};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::Adjustments;

pub const CATALOG_VERSION: u32 = 4;
pub const MAX_HISTORY: usize = 200;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: u64,
    pub label: String,
    pub adjustments: Adjustments,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: u64,
    pub name: String,
    pub adjustments: Adjustments,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PickState {
    #[default]
    Unmarked,
    Keep,
    Reject,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhotoMetadata {
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<u32>,
    pub focal_length_mm: Option<f32>,
    pub aperture: Option<f32>,
    pub shutter_seconds: Option<f32>,
    pub captured_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Photo {
    pub id: u64,
    pub path: PathBuf,
    pub name: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub adjustments: Adjustments,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    #[serde(default)]
    pub history_cursor: usize,
    #[serde(default = "default_history_id")]
    pub next_history_id: u64,
    #[serde(default)]
    pub pick: PickState,
    #[serde(default)]
    pub metadata: PhotoMetadata,
}

fn default_history_id() -> u64 {
    1
}

impl Photo {
    pub fn new(id: u64, path: PathBuf, name: String, width: u32, height: u32) -> Self {
        let adjustments = Adjustments::default();
        Self {
            id,
            path,
            name,
            width,
            height,
            format: String::new(),
            adjustments: adjustments.clone(),
            history: vec![HistoryEntry {
                id: 1,
                label: "Imported".into(),
                adjustments,
            }],
            history_cursor: 0,
            next_history_id: 2,
            pick: PickState::Unmarked,
            metadata: PhotoMetadata::default(),
        }
    }

    pub fn is_raw(&self) -> bool {
        self.format.eq_ignore_ascii_case("arw") || is_raw_image(&self.path)
    }

    pub fn commit_adjustments(&mut self, label: impl Into<String>, adjustments: Adjustments) {
        let adjustments = adjustments.sanitized();
        if self.adjustments == adjustments {
            return;
        }
        self.history.truncate(self.history_cursor.saturating_add(1));
        self.history.push(HistoryEntry {
            id: self.next_history_id,
            label: label.into(),
            adjustments: adjustments.clone(),
        });
        self.next_history_id += 1;
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
        self.history_cursor = self.history.len() - 1;
        self.adjustments = adjustments;
    }

    pub fn can_history_back(&self) -> bool {
        self.history_cursor > 0
    }
    pub fn can_history_forward(&self) -> bool {
        self.history_cursor + 1 < self.history.len()
    }

    pub fn history_back(&mut self) -> bool {
        if !self.can_history_back() {
            return false;
        }
        self.history_cursor -= 1;
        self.adjustments = self.history[self.history_cursor].adjustments.clone();
        true
    }

    pub fn history_forward(&mut self) -> bool {
        if !self.can_history_forward() {
            return false;
        }
        self.history_cursor += 1;
        self.adjustments = self.history[self.history_cursor].adjustments.clone();
        true
    }

    pub fn history_jump(&mut self, index: usize) -> Result<()> {
        let entry = self
            .history
            .get(index)
            .with_context(|| format!("history entry {index} does not exist"))?;
        self.history_cursor = index;
        self.adjustments = entry.adjustments.clone();
        Ok(())
    }

    fn migrate(&mut self) {
        if self.format.is_empty() {
            self.format = extension(&self.path);
        }
        if self.history.is_empty() {
            let current = self.adjustments.clone().sanitized();
            self.history.push(HistoryEntry {
                id: 1,
                label: "Imported".into(),
                adjustments: Adjustments::default(),
            });
            if current != Adjustments::default() {
                self.history.push(HistoryEntry {
                    id: 2,
                    label: "Migrated edits".into(),
                    adjustments: current.clone(),
                });
            }
            self.adjustments = current;
            self.history_cursor = self.history.len() - 1;
            self.next_history_id = self.history.last().map_or(1, |entry| entry.id + 1);
        } else {
            self.history_cursor = self.history_cursor.min(self.history.len() - 1);
            self.adjustments = self.history[self.history_cursor]
                .adjustments
                .clone()
                .sanitized();
            self.next_history_id = self
                .next_history_id
                .max(self.history.iter().map(|entry| entry.id).max().unwrap_or(0) + 1);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub next_id: u64,
    pub selected: Option<u64>,
    pub photos: Vec<Photo>,
    #[serde(default)]
    pub presets: Vec<Preset>,
    #[serde(default = "default_preset_id")]
    pub next_preset_id: u64,
}

fn default_preset_id() -> u64 {
    1
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
            presets: Vec::new(),
            next_preset_id: 1,
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes =
            fs::read(path).with_context(|| format!("could not read catalog {}", path.display()))?;
        let mut project: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid catalog {}", path.display()))?;
        if project.version > CATALOG_VERSION {
            bail!(
                "catalog version {} is newer than this app supports ({CATALOG_VERSION})",
                project.version
            );
        }
        project.version = CATALOG_VERSION;
        for photo in &mut project.photos {
            photo.migrate();
        }
        let missing_raw: Vec<_> = project
            .photos
            .iter()
            .enumerate()
            .filter(|(_, photo)| photo.is_raw() && photo.metadata == PhotoMetadata::default())
            .map(|(index, photo)| (index, photo.path.clone()))
            .collect();
        if !missing_raw.is_empty() {
            let loader = RawLoader::new();
            let refreshed: Vec<_> = missing_raw
                .par_iter()
                .filter_map(|(index, path)| {
                    source_info(path, Some(&loader))
                        .ok()
                        .map(|(width, height, metadata)| (*index, width, height, metadata))
                })
                .collect();
            for (index, width, height, metadata) in refreshed {
                project.photos[index].width = width;
                project.photos[index].height = height;
                project.photos[index].metadata = metadata;
            }
        }
        project.next_preset_id = project.next_preset_id.max(
            project
                .presets
                .iter()
                .map(|preset| preset.id)
                .max()
                .unwrap_or(0)
                + 1,
        );
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
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("could not write {}", temporary.display()))?;
        #[cfg(target_os = "windows")]
        if path.exists() {
            fs::remove_file(path)
                .with_context(|| format!("could not replace catalog {}", path.display()))?;
        }
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
        let loader = paths
            .iter()
            .any(|path| is_raw_image(path))
            .then(RawLoader::new);
        let prepared: Vec<PreparedPhoto> = paths
            .par_iter()
            .map(|input| prepare_photo(input, loader.as_ref()))
            .collect::<Result<_>>()?;
        let mut known: HashMap<PathBuf, u64> = self
            .photos
            .iter()
            .map(|photo| (photo.path.clone(), photo.id))
            .collect();
        let mut imported = Vec::new();
        for prepared in prepared {
            if let Some(id) = known.get(&prepared.path).copied() {
                imported.push(id);
                continue;
            }
            let id = self.next_id;
            self.next_id += 1;
            let mut photo = Photo::new(
                id,
                prepared.path.clone(),
                prepared.name,
                prepared.width,
                prepared.height,
            );
            photo.format = prepared.format;
            photo.metadata = prepared.metadata;
            self.photos.push(photo);
            known.insert(prepared.path, id);
            imported.push(id);
        }
        if self.selected.is_none() {
            self.selected = imported.first().copied();
        }
        Ok(imported)
    }

    pub fn preset(&self, id: u64) -> Result<&Preset> {
        self.presets
            .iter()
            .find(|preset| preset.id == id)
            .with_context(|| format!("preset {id} is not in this catalog"))
    }

    pub fn save_preset(
        &mut self,
        name: impl Into<String>,
        adjustments: &Adjustments,
    ) -> Result<u64> {
        let name = name.into().trim().to_owned();
        if name.is_empty() {
            bail!("preset name cannot be empty");
        }
        let id = self.next_preset_id;
        self.next_preset_id += 1;
        self.presets.push(Preset {
            id,
            name,
            adjustments: adjustments.as_preset(),
        });
        Ok(id)
    }

    pub fn delete_preset(&mut self, id: u64) -> Result<()> {
        self.preset(id)?;
        self.presets.retain(|preset| preset.id != id);
        Ok(())
    }
}

struct PreparedPhoto {
    path: PathBuf,
    name: String,
    format: String,
    width: u32,
    height: u32,
    metadata: PhotoMetadata,
}

fn prepare_photo(input: &Path, loader: Option<&RawLoader>) -> Result<PreparedPhoto> {
    if !is_supported_image(input) {
        bail!("unsupported image type: {}", input.display());
    }
    let path = fs::canonicalize(input)
        .with_context(|| format!("could not find image {}", input.display()))?;
    let (width, height, metadata) = source_info(&path, loader)?;
    Ok(PreparedPhoto {
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        format: extension(&path),
        path,
        width,
        height,
        metadata,
    })
}

fn source_info(path: &Path, loader: Option<&RawLoader>) -> Result<(u32, u32, PhotoMetadata)> {
    if is_raw_image(path) {
        let loader = loader.expect("RAW imports initialize a metadata loader");
        let source = RawSource::new(path)
            .with_context(|| format!("could not open Sony RAW {}", path.display()))?;
        let decoder = loader
            .get_decoder(&source)
            .with_context(|| format!("could not inspect Sony RAW {}", path.display()))?;
        let params = RawDecodeParams::default();
        // Dummy decode parses dimensions and validates the RAW container without
        // allocating or demosaicing its full pixel plane.
        let raw = decoder
            .raw_image(&source, &params, true)
            .with_context(|| format!("could not inspect Sony RAW {}", path.display()))?;
        let raw_metadata = decoder
            .raw_metadata(&source, &params)
            .with_context(|| format!("could not read Sony RAW metadata {}", path.display()))?;
        let transpose = raw.orientation.to_flips().0;
        let dimensions = raw.crop_area.or(raw.active_area).map_or_else(
            || (raw.width as u32, raw.height as u32),
            |area| (area.d.w as u32, area.d.h as u32),
        );
        let exif = raw_metadata.exif;
        let lens = exif.lens_model.clone().or_else(|| {
            raw_metadata
                .lens
                .as_ref()
                .map(|lens| lens.lens_name.clone())
        });
        let metadata = PhotoMetadata {
            camera_make: Some(raw_metadata.make),
            camera_model: Some(raw_metadata.model),
            lens,
            iso: exif
                .iso_speed
                .or(exif.recommended_exposure_index)
                .or(exif.iso_speed_ratings.map(u32::from)),
            focal_length_mm: rational_value(exif.focal_length),
            aperture: rational_value(exif.fnumber.or(exif.aperture_value)),
            shutter_seconds: rational_value(exif.exposure_time),
            captured_at: exif.date_time_original.or(exif.create_date),
        };
        let oriented = if transpose {
            (dimensions.1, dimensions.0)
        } else {
            dimensions
        };
        Ok((oriented.0, oriented.1, metadata))
    } else {
        let dimensions = image::ImageReader::open(path)
            .with_context(|| format!("could not open {}", path.display()))?
            .with_guessed_format()?
            .into_dimensions()
            .with_context(|| format!("could not read {}", path.display()))?;
        Ok((dimensions.0, dimensions.1, PhotoMetadata::default()))
    }
}

fn rational_value(value: Option<Rational>) -> Option<f32> {
    value.and_then(|value| (value.d != 0).then_some(value.n as f32 / value.d as f32))
}

pub fn is_raw_image(path: &Path) -> bool {
    extension(path) == "arw"
}

pub fn is_supported_image(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "jpg" | "jpeg" | "png" | "tif" | "tiff" | "webp" | "arw"
    )
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn recognizes_sony_raw_case_insensitively() {
        assert!(is_supported_image(Path::new("portrait.ARW")));
        assert!(is_raw_image(Path::new("portrait.arw")));
    }

    #[test]
    fn history_is_persistent_and_navigable() {
        let mut photo = Photo::new(1, "test.jpg".into(), "test.jpg".into(), 10, 10);
        photo.commit_adjustments(
            "Exposure",
            Adjustments {
                exposure: 1.0,
                ..Default::default()
            },
        );
        photo.commit_adjustments("Reset all edits", Adjustments::default());
        assert_eq!(photo.history.len(), 3);
        assert!(photo.history_back());
        assert_eq!(photo.adjustments.exposure, 1.0);
        assert!(photo.history_forward());
        assert_eq!(photo.adjustments.exposure, 0.0);
    }

    #[test]
    fn catalog_round_trips() {
        let directory = test_directory("catalog-round-trip");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("test.lumencatalog");
        let mut project = Project::new("Round trip");
        project.selected = Some(42);
        project.next_id = 43;
        project.save(&path).unwrap();
        assert_eq!(Project::load(&path).unwrap(), project);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn presets_round_trip_and_allocate_ids() {
        let mut project = Project::new("Presets");
        let first = project
            .save_preset(
                "Warm",
                &Adjustments {
                    temperature: 22.0,
                    ..Default::default()
                },
            )
            .unwrap();
        let second = project
            .save_preset(
                "Cool",
                &Adjustments {
                    temperature: -18.0,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!((first, second), (1, 2));
        assert_eq!(project.preset(2).unwrap().name, "Cool");
        project.delete_preset(1).unwrap();
        assert_eq!(project.presets.len(), 1);
    }

    fn test_directory(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("lumen-{label}-{}-{unique}", std::process::id()))
    }
}
