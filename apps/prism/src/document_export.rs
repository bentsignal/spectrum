use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, bail};
use image::ImageEncoder;
use same_file::Handle;

use crate::{Document, LayerKind, RasterSourceResolver, render_document_with_sources};

static EXPORT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

pub fn export_document(document: &Document, path: &Path, quality: u8) -> Result<()> {
    let cache_root = crate::default_raster_backing_cache_root()?;
    let sources = crate::prepare_export_raster_sources(document, &cache_root)?;
    export_document_with_sources(document, path, quality, &sources)
}

pub fn export_document_with_sources(
    document: &Document,
    path: &Path,
    quality: u8,
    raster_sources: &dyn RasterSourceResolver,
) -> Result<()> {
    export_document_with_sources_impl(document, path, quality, raster_sources, |_, _| Ok(()))
}

fn export_document_with_sources_impl(
    document: &Document,
    path: &Path,
    quality: u8,
    raster_sources: &dyn RasterSourceResolver,
    before_replace: impl FnOnce(&Path, &Path) -> Result<()>,
) -> Result<()> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "jpg" | "jpeg" | "png") {
        bail!("export path must end in .png, .jpg, or .jpeg");
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let canonical_parent = fs::canonicalize(parent)?;
    let destination =
        canonical_parent.join(path.file_name().context("export path needs a file name")?);
    refuse_source_alias(document, &destination)?;

    let image = render_document_with_sources(document, None, raster_sources)?;
    let temporary = export_temporary_path(&canonical_parent, &destination)?;
    let result = (|| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| format!("could not create {}", temporary.display()))?;
        let writer = BufWriter::new(file);
        let file = encode_image(writer, image, &extension, quality)?;
        file.sync_all()?;
        before_replace(&temporary, &destination)?;
        replace_export(&temporary, &destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn refuse_source_alias(document: &Document, destination: &Path) -> Result<()> {
    let destination_handle = match OpenOptions::new().read(true).write(true).open(destination) {
        Ok(file) => Some(Handle::from_file(file)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let mut sources = Vec::new();
    for layer in &document.layers {
        if let LayerKind::Raster {
            path,
            original_path,
        } = &layer.kind
        {
            sources.push(path);
            if let Some(original_path) = original_path {
                sources.push(original_path);
            }
        }
    }
    sources.extend(document.sampled_sources.values().map(|source| &source.path));
    for source in sources {
        let exact_destination = fs::canonicalize(source)
            .ok()
            .is_some_and(|canonical| canonical == destination);
        let aliases_destination = destination_handle.as_ref().is_some_and(|destination| {
            File::open(source)
                .ok()
                .and_then(|source| Handle::from_file(source).ok())
                .is_some_and(|source| source == *destination)
        });
        if exact_destination || aliases_destination {
            bail!(
                "refusing to overwrite raster source {}; choose a new export path",
                source.display()
            );
        }
    }
    Ok(())
}

fn export_temporary_path(parent: &Path, destination: &Path) -> Result<PathBuf> {
    let name = destination
        .file_name()
        .context("export destination needs a file name")?
        .to_string_lossy();
    for _ in 0..16 {
        let candidate = parent.join(format!(
            ".{name}.{}.{}.tmp",
            std::process::id(),
            EXPORT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        if candidate.symlink_metadata().is_err() {
            return Ok(candidate);
        }
    }
    bail!("could not allocate a private export temporary path")
}

fn encode_image(
    mut writer: BufWriter<File>,
    image: image::DynamicImage,
    extension: &str,
    quality: u8,
) -> Result<File> {
    match extension {
        "jpg" | "jpeg" => {
            let rgb = image.to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, quality.clamp(1, 100))
                .write_image(
                    &rgb,
                    rgb.width(),
                    rgb.height(),
                    image::ExtendedColorType::Rgb8,
                )?;
        }
        "png" => {
            let rgba = image.to_rgba8();
            image::codecs::png::PngEncoder::new(&mut writer).write_image(
                &rgba,
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )?;
        }
        _ => unreachable!("extension was validated before rendering"),
    }
    writer.flush()?;
    writer
        .into_inner()
        .map_err(|error| error.into_error().into())
}

#[cfg(not(target_os = "windows"))]
fn replace_export(temporary: &Path, destination: &Path) -> Result<()> {
    fs::rename(temporary, destination)
        .with_context(|| format!("could not replace {}", destination.display()))
}

#[cfg(target_os = "windows")]
fn replace_export(temporary: &Path, destination: &Path) -> Result<()> {
    if !destination.exists() {
        fs::rename(temporary, destination)?;
        return Ok(());
    }
    let backup = destination.with_extension("prism-export-backup");
    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    fs::rename(destination, &backup)?;
    if let Err(error) = fs::rename(temporary, destination) {
        let _ = fs::rename(&backup, destination);
        return Err(error.into());
    }
    fs::remove_file(backup)?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use image::{Rgba, RgbaImage};
    use sha2::{Digest, Sha256};
    use spectrum_imaging::Adjustments;

    use super::*;
    use crate::{BrushSample, BrushStroke, BrushStyle, SampledSourceSnapshot, Transform};

    fn test_directory(label: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "prism-export-{label}-{}-{stamp}",
            std::process::id()
        ))
    }

    fn sampled_document(source: &Path) -> Document {
        let bytes = fs::read(source).unwrap();
        let content_hash = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let snapshot = SampledSourceSnapshot {
            version: crate::SAMPLED_SOURCE_VERSION,
            source_layer_id: 1,
            source_layer_name: "Removed source".into(),
            path: source.to_owned(),
            content_hash,
            width: 4,
            height: 4,
            anchor_local: [1.5, 1.5],
            source_transform: Transform::default(),
            adjustments: Adjustments::default(),
            pixel_mask: None,
            vector_mask: None,
        };
        let marker = BrushStroke::new_clone_stamp(
            BrushStyle::default(),
            [BrushSample {
                x: 1.5,
                y: 1.5,
                pressure: 1.0,
            }],
            snapshot.clone(),
        )
        .unwrap();
        let source_id = serde_json::to_value(marker).unwrap()["source"]["source_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut encoded = serde_json::to_value(Document::new("Alias safety", 4, 4)).unwrap();
        encoded["sampled_sources"] = serde_json::json!({(source_id): snapshot});
        serde_json::from_value(encoded).unwrap()
    }

    #[test]
    fn sampled_source_hardlink_and_symlink_aliases_are_refused_without_mutation() {
        let directory = test_directory("aliases");
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("sampled.png");
        RgbaImage::from_pixel(4, 4, Rgba([17, 31, 47, 255]))
            .save(&source)
            .unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
        let before = fs::read(&source).unwrap();
        let before_mode = fs::metadata(&source).unwrap().permissions().mode();
        let document = sampled_document(&source);
        let cache = directory.join("cache");
        let providers = crate::prepare_export_raster_sources(&document, &cache).unwrap();

        let hardlink = directory.join("hardlink.png");
        fs::hard_link(&source, &hardlink).unwrap();
        assert!(export_document_with_sources(&document, &hardlink, 92, &providers).is_err());
        assert_eq!(fs::read(&source).unwrap(), before);
        assert_eq!(
            fs::metadata(&source).unwrap().permissions().mode(),
            before_mode
        );
        fs::remove_file(&hardlink).unwrap();

        let symlink_path = directory.join("symlink.png");
        symlink(&source, &symlink_path).unwrap();
        assert!(export_document_with_sources(&document, &symlink_path, 92, &providers).is_err());
        assert_eq!(fs::read(&source).unwrap(), before);
        assert_eq!(
            fs::metadata(&source).unwrap().permissions().mode(),
            before_mode
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn destination_hardlink_swap_before_atomic_replace_never_writes_source_inode() {
        let directory = test_directory("race");
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("sampled.png");
        RgbaImage::from_pixel(4, 4, Rgba([91, 52, 13, 255]))
            .save(&source)
            .unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
        let before = fs::read(&source).unwrap();
        let before_mode = fs::metadata(&source).unwrap().permissions().mode();
        let document = sampled_document(&source);
        let providers =
            crate::prepare_export_raster_sources(&document, &directory.join("cache")).unwrap();
        let destination = directory.join("race.png");
        fs::write(&destination, b"previous destination").unwrap();

        export_document_with_sources_impl(
            &document,
            &destination,
            92,
            &providers,
            |_, destination| {
                fs::remove_file(destination)?;
                fs::hard_link(&source, destination)?;
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(fs::read(&source).unwrap(), before);
        assert_eq!(
            fs::metadata(&source).unwrap().permissions().mode(),
            before_mode
        );
        assert!(!same_file::is_same_file(&source, &destination).unwrap());
        let output = image::open(&destination).unwrap();
        assert_eq!((output.width(), output.height()), (4, 4));
        assert!(
            fs::read_dir(&directory)
                .unwrap()
                .flatten()
                .all(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp"))
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
