//! Importing any supported file, and inspecting footage and audio.
use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde_json::json;
use spectrum_assets::Service;
use spectrum_library::{AssetId, AssetKind, ProjectId};
use spectrum_media::Time;
use std::path::PathBuf;

/// Footage is imported video with its sound. Import with `spectrum import`.
#[derive(Subcommand)]
pub(super) enum Footage {
    /// List footage assets.
    List,
    /// Show what footage holds: size, frame rate, frames, codec, sound tracks.
    Info { asset: AssetId },
    /// Save the frame showing at a time as an image outside the library.
    Frame {
        asset: AssetId,
        /// A .png or .jpg path.
        path: PathBuf,
        /// Seconds (1.5), frames (36f), or timecode (00:00:01:12).
        #[arg(long, default_value = "0")]
        at: String,
        /// Longest edge in pixels; full size when omitted.
        #[arg(long)]
        max_size: Option<u32>,
    },
    /// Make the playback copy that heavy footage (larger than HD, or more
    /// than 8 bits) scrubs and plays from. The app makes it on its own.
    Prepare { asset: AssetId },
    /// Copy the imported file, unchanged, outside the library.
    Export { asset: AssetId, path: PathBuf },
}

/// Audio is an imported sound file. Import with `spectrum import`.
#[derive(Subcommand)]
pub(super) enum Audio {
    /// List audio assets.
    List,
    /// Show what audio holds: duration, codec, rate, channels, tracks.
    Info { asset: AssetId },
    /// Copy the imported file, unchanged, outside the library.
    Export { asset: AssetId, path: PathBuf },
}

/// Imports files as one batch, creating a project first when asked; the
/// new project is removed again if the import fails.
pub(super) fn import(
    service: &mut Service,
    paths: Vec<PathBuf>,
    project: Option<ProjectId>,
    new_project: Option<String>,
) -> Result<serde_json::Value> {
    let created = new_project
        .map(|name| service.library.create_project(&name))
        .transpose()?
        .map(|p| p.id);
    let project = created.or(project);
    let (batch, assets) = match service.import_into(paths, project) {
        Ok(imported) => imported,
        Err(error) => {
            if let Some(id) = created {
                service.library.delete_project(id)?;
            }
            return Err(error);
        }
    };
    Ok(json!({"batch": batch.id, "project": project, "assets": assets}))
}

fn list(service: &Service, kind: AssetKind) -> Result<serde_json::Value> {
    let assets: Vec<_> = service
        .library
        .list()?
        .into_iter()
        .filter(|asset| asset.kind == kind)
        .collect();
    Ok(serde_json::to_value(assets)?)
}

fn info(service: &Service, asset: AssetId, kind: AssetKind) -> Result<serde_json::Value> {
    let found = service.library.get(asset)?;
    if found.kind != kind {
        bail!("{} is a {}, not {kind}", found.name, found.kind);
    }
    let (media, _) = service.media(asset)?;
    let mut value = serde_json::to_value(&media.info)?;
    value["asset"] = json!(asset);
    value["name"] = json!(found.name);
    value["original_name"] = json!(media.original_name);
    value["bytes"] = json!(media.bytes);
    value["duration_seconds"] = json!(media.info.duration.seconds());
    if kind == AssetKind::Footage {
        value["playback_copy"] = json!(if service.playback_copy(asset)?.is_some() {
            "ready"
        } else if service.needs_playback_copy(asset)? {
            "needed"
        } else {
            "not needed"
        });
    }
    Ok(value)
}

pub(super) fn run_footage(service: &Service, command: Footage) -> Result<serde_json::Value> {
    match command {
        Footage::List => list(service, AssetKind::Footage),
        Footage::Info { asset } => info(service, asset, AssetKind::Footage),
        Footage::Frame {
            asset,
            path,
            at,
            max_size,
        } => {
            service.check_export(&path)?;
            let (media, _) = service.media(asset)?;
            let rate = media.info.video.as_ref().map(|video| video.rate);
            let time = Time::parse(&at, rate)?;
            let (frame, hardware) = service.footage_frame_from(asset, time, max_size)?;
            image::DynamicImage::ImageRgba8(frame.image)
                .to_rgb8()
                .save(&path)
                .with_context(|| format!("could not write {}", path.display()))?;
            Ok(json!({
                "exported": path,
                "frame": frame.index,
                "time_seconds": frame.time.seconds(),
                "decoded_by": hardware.unwrap_or("software"),
            }))
        }
        Footage::Prepare { asset } => {
            info(service, asset, AssetKind::Footage)?;
            let copy = service.make_playback_copy(asset, &mut |_| true)?;
            Ok(json!({"asset": asset, "playback_copy": copy}))
        }
        Footage::Export { asset, path } => {
            info(service, asset, AssetKind::Footage)?;
            service.export_original(asset, &path)?;
            Ok(json!({"exported": path}))
        }
    }
}

pub(super) fn run_audio(service: &Service, command: Audio) -> Result<serde_json::Value> {
    match command {
        Audio::List => list(service, AssetKind::Audio),
        Audio::Info { asset } => info(service, asset, AssetKind::Audio),
        Audio::Export { asset, path } => {
            info(service, asset, AssetKind::Audio)?;
            service.export_original(asset, &path)?;
            Ok(json!({"exported": path}))
        }
    }
}
