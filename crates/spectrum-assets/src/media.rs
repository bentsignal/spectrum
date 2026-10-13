//! Footage and audio: imported into the library's media store, with a
//! document recording what each file holds, posters and waveforms for
//! grids, and the stored file for players to read.
use super::{Service, save_atomically};
use anyhow::{Context, Result, bail};
use spectrum_library::{Asset, AssetId, AssetKind};
use spectrum_media::{Media, MediaInfo, MediaModel, Time};
use std::path::{Path, PathBuf};

/// Where a poster frame is taken: a tenth of the way in, past any fade from
/// black, but no later than two seconds.
fn poster_time(info: &MediaInfo) -> Time {
    Time(info.duration.0 / 10).min(Time::from_seconds(2.0))
}

impl Service {
    /// Copies a footage or audio file into the media store and adds its
    /// asset. `progress` hears the bytes copied so far.
    pub(crate) fn create_media(
        &self,
        name: &str,
        source: &Path,
        info: MediaInfo,
        progress: &mut dyn FnMut(u64),
    ) -> Result<Asset> {
        let kind = if info.is_footage() {
            AssetKind::Footage
        } else {
            AssetKind::Audio
        };
        let stored = self.library.ingest(source, progress)?;
        let media = Media {
            file: stored.file,
            original_name: source
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            bytes: stored.bytes,
            info,
        };
        self.create_media_document(name, kind, media)
    }

    fn create_media_document(&self, name: &str, kind: AssetKind, media: Media) -> Result<Asset> {
        let path = self.new_document(kind);
        std::fs::create_dir_all(path.parent().context("document has no folder")?)?;
        let file = media.file.clone();
        spectrum_media::Workspace::create(&path, media, self.actor.clone(), self.session)?;
        let asset = self.library.register(kind, name, &path)?;
        self.library.use_media(asset.id, &file)?;
        Ok(asset)
    }

    /// Footage or audio, and where its file is.
    pub fn media(&self, id: AssetId) -> Result<(Media, PathBuf)> {
        let asset = self.library.get(id)?;
        if !matches!(asset.kind, AssetKind::Footage | AssetKind::Audio) {
            bail!("{} is a {}, not footage or audio", asset.name, asset.kind);
        }
        let media = self
            .read::<MediaModel>(&self.library.path(&asset)?, false)?
            .0;
        let path = self.library.media_path(&media.file)?;
        Ok((media, path))
    }

    /// The frame of footage showing at `time`, no larger than `max_size` on
    /// its long edge.
    pub fn footage_frame(
        &self,
        id: AssetId,
        time: Time,
        max_size: Option<u32>,
    ) -> Result<spectrum_media::Frame> {
        let (media, path) = self.media(id)?;
        if !media.info.is_footage() {
            bail!("audio has no picture");
        }
        spectrum_media::FrameReader::open(&path)?.frame_at(time, max_size)
    }

    /// A small poster of footage, or a waveform of audio, for grids.
    pub(crate) fn media_thumbnail(&self, id: AssetId, max: u32) -> Result<PathBuf> {
        let path = self
            .library
            .root()
            .join("previews")
            .join(format!("{id}-thumb-{max}.png"));
        if path.exists() {
            return Ok(path);
        }
        let (media, file) = self.media(id)?;
        let image = if media.info.is_footage() {
            // From the playback copy, or the original's nearest keyframe:
            // either decodes in a moment, so a grid of camera footage never
            // takes the cores editing needs.
            let at = poster_time(&media.info);
            let frame = match self.playback_copy(id)? {
                Some(copy) => {
                    let rotation = media.info.video.as_ref().map_or(0, |v| v.rotation);
                    spectrum_media::FrameReader::open(&copy)?
                        .with_rotation(rotation)
                        .frame_at(at, Some(max))?
                }
                None => spectrum_media::FrameReader::open(&file)?.keyframe_near(at, Some(max))?,
            };
            image::DynamicImage::ImageRgba8(frame.image)
        } else {
            waveform_picture(&file, &media.info, max)?
        };
        save_atomically(&image, &path)?;
        Ok(path)
    }

    /// Footage's playback copy, if it needs one and it is made.
    pub fn playback_copy(&self, id: AssetId) -> Result<Option<PathBuf>> {
        let (media, _) = self.media(id)?;
        if !spectrum_media::needs_playback_copy(&media.info) {
            return Ok(None);
        }
        let path = self.library.media_cache_path(&media.file, "playback.mkv")?;
        Ok(path.exists().then_some(path))
    }

    /// Whether footage needs a playback copy that is not made yet.
    pub fn needs_playback_copy(&self, id: AssetId) -> Result<bool> {
        let (media, _) = self.media(id)?;
        Ok(spectrum_media::needs_playback_copy(&media.info)
            && !self
                .library
                .media_cache_path(&media.file, "playback.mkv")?
                .exists())
    }

    /// Makes footage's playback copy if it needs one, returning it.
    /// `progress` hears the share done and stops the copy by returning false.
    pub fn make_playback_copy(
        &self,
        id: AssetId,
        progress: &mut dyn FnMut(f32) -> bool,
    ) -> Result<Option<PathBuf>> {
        let (media, file) = self.media(id)?;
        if !spectrum_media::needs_playback_copy(&media.info) {
            return Ok(None);
        }
        let path = self.library.media_cache_path(&media.file, "playback.mkv")?;
        if !path.exists() {
            spectrum_media::make_playback_copy(&file, &path, progress)?;
        }
        Ok(Some(path))
    }

    /// Copies an imported file, unchanged, to a path outside the library.
    pub fn export_original(&self, id: AssetId, destination: &Path) -> Result<()> {
        self.check_export(destination)?;
        let (_, file) = self.media(id)?;
        std::fs::copy(&file, destination)
            .with_context(|| format!("could not write {}", destination.display()))?;
        let mut permissions = std::fs::metadata(destination)?.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(destination, permissions)?;
        Ok(())
    }

    /// Another asset of the same footage or audio; the stored file is shared.
    pub(crate) fn copy_media(&self, id: AssetId, name: &str) -> Result<Asset> {
        let kind = self.library.get(id)?.kind;
        let (media, _) = self.media(id)?;
        self.create_media_document(name, kind, media)
    }
}

/// A waveform of a sound's first track, twice as wide as it is tall.
fn waveform_picture(file: &Path, info: &MediaInfo, max: u32) -> Result<image::DynamicImage> {
    let (width, height) = (max.max(16), (max / 2).max(8));
    let bars = (width / 3).max(1) as usize;
    let peaks = spectrum_media::waveform(file, 0, info.duration, bars)?;
    let mut picture = image::RgbaImage::from_pixel(width, height, image::Rgba([31, 31, 31, 255]));
    let middle = height as f32 / 2.0;
    for (bar, peak) in peaks.iter().enumerate() {
        let reach = (peak.sqrt() * middle * 0.9).max(1.0);
        for x in (bar as u32 * 3)..(bar as u32 * 3 + 2).min(width) {
            for y in (middle - reach) as u32..((middle + reach) as u32).min(height) {
                picture.put_pixel(x, y, image::Rgba([150, 150, 150, 255]));
            }
        }
    }
    Ok(image::DynamicImage::ImageRgba8(picture))
}
