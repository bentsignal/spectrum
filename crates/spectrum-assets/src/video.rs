//! Videos in the library: creating them, editing them through the same
//! sessions as every asset, keeping the library's record of what they use,
//! and adding assets to their timelines.
use super::Service;
use anyhow::{Context, Result, bail};
use spectrum_document::{Model, RevisionId, Workspace};
use spectrum_library::{Asset, AssetId, AssetKind};
use spectrum_time::{FrameRate, Time};
use spectrum_video::{Command, CommandOutput, Placement, Source, Video, VideoModel};

/// How long a still lasts when added to a video.
pub const STILL_SECONDS: f64 = 5.0;

impl Service {
    pub fn create_video(
        &self,
        name: &str,
        width: u32,
        height: u32,
        rate: FrameRate,
    ) -> Result<Asset> {
        let path = self.new_document(AssetKind::Video);
        std::fs::create_dir_all(path.parent().context("document has no folder")?)?;
        let video = Video::new(name, width, height, rate);
        spectrum_video::Workspace::create(&path, video, self.actor.clone(), self.session)?;
        self.library.register(AssetKind::Video, name, &path)
    }

    /// A video as this person or agent sees it.
    pub fn video(&self, id: AssetId) -> Result<Video> {
        Ok(self
            .read::<VideoModel>(&self.document(id, AssetKind::Video)?, false)?
            .0)
    }

    /// Opens a video for the person, following an agent they work together
    /// with, and the revision they are on.
    pub fn video_view(&self, id: AssetId) -> Result<(Video, RevisionId)> {
        let path = self.document(id, AssetKind::Video)?;
        Workspace::<VideoModel>::follow(&path, self.person)?;
        Workspace::<VideoModel>::read_session(&path, self.person)
    }

    pub fn edit_video(
        &mut self,
        id: AssetId,
        commands: Vec<Command>,
    ) -> Result<Vec<CommandOutput>> {
        Ok(self.edit_video_from(id, None, commands)?.0)
    }

    /// Edits a video from `base`. Consecutive edits apply together as one
    /// revision; undo and redo step on their own.
    pub fn edit_video_from(
        &mut self,
        id: AssetId,
        base: Option<RevisionId>,
        commands: Vec<Command>,
    ) -> Result<(Vec<CommandOutput>, RevisionId)> {
        let path = self.document(id, AssetKind::Video)?;
        let mut workspace = self.workspace::<VideoModel>(&path, base)?;
        let mut outputs = Vec::with_capacity(commands.len());
        let mut batch = Vec::new();
        for command in commands {
            if VideoModel::step(&command).is_some() {
                if !batch.is_empty() {
                    outputs.extend(workspace.execute_batch(std::mem::take(&mut batch))?);
                }
                outputs.push(workspace.execute(command)?);
            } else {
                batch.push(command);
            }
        }
        if !batch.is_empty() {
            outputs.extend(workspace.execute_batch(batch)?);
        }
        if let Some(error) = workspace.pending_publish_error() {
            bail!("the edit was saved but not published: {error}");
        }
        let revision = workspace.revision().context("video has no history")?;
        drop(workspace);
        self.index_video(id)?;
        Ok((outputs, revision))
    }

    /// Records which assets a video uses, as the person sees it and as this
    /// agent does.
    pub fn index_video(&mut self, id: AssetId) -> Result<()> {
        let path = self.document(id, AssetKind::Video)?;
        let (person, _) = Workspace::<VideoModel>::read_session(&path, self.person)?;
        let mut used = person.uses();
        let reader = self.reader::<VideoModel>(&path)?;
        if reader != self.person {
            let (agent, _) = Workspace::<VideoModel>::read_session(&path, reader)?;
            used.extend(
                agent
                    .uses()
                    .into_iter()
                    .map(|(slot, asset)| (format!("{reader}/{slot}"), asset)),
            );
        }
        // One record per asset is enough for the library.
        used.sort_by_key(|(_, asset)| *asset);
        used.dedup_by_key(|(_, asset)| *asset);
        self.library.references(id, &used)
    }

    /// The command that adds `asset` (footage, audio, an image, or a canvas)
    /// to a video at `at`: all of footage or audio, and five seconds of a
    /// still.
    pub fn clip_command(
        &self,
        asset: AssetId,
        at: Time,
        track: Option<u64>,
        placement: Placement,
    ) -> Result<Command> {
        let found = self.library.get(asset)?;
        let (source, duration, source_duration) = match found.kind {
            AssetKind::Footage => {
                let (media, _) = self.media(asset)?;
                let video = media.info.video.context("footage has a picture")?;
                let length = video.rate.time_of(video.frames as i64);
                (Source::Footage { asset }, length, Some(length))
            }
            AssetKind::Audio => {
                let (media, _) = self.media(asset)?;
                let length = media.info.duration;
                (Source::Sound { asset, track: 0 }, length, Some(length))
            }
            AssetKind::Image => (
                Source::Image { asset },
                Time::from_seconds(STILL_SECONDS),
                None,
            ),
            AssetKind::Canvas => (
                Source::Canvas { asset },
                Time::from_seconds(STILL_SECONDS),
                None,
            ),
            kind => bail!("{kind} cannot go in a video yet"),
        };
        let sound = match found.kind {
            AssetKind::Footage => !self.media(asset)?.0.info.audio.is_empty(),
            _ => false,
        };
        Ok(Command::AddClip {
            source,
            track,
            at,
            source_in: Time::ZERO,
            duration,
            source_duration,
            name: found.name,
            placement,
            sound,
            sound_track: None,
        })
    }

    /// Copies a video. Its images and canvases are copied too, so editing
    /// the copy's never changes the original's; footage and sound never
    /// change, so they are shared.
    pub(crate) fn copy_video(&mut self, id: AssetId, name: &str) -> Result<Asset> {
        let mut video = self.video(id)?;
        video.name = name.into();
        let mut copied = std::collections::HashMap::new();
        for clip in video.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
            let (Source::Image { asset } | Source::Canvas { asset }) = &mut clip.source else {
                continue;
            };
            if self.library.removed(*asset)?.is_some() {
                continue;
            }
            let copy = match copied.get(asset) {
                Some(copy) => *copy,
                None => {
                    let copy = self.copy(*asset)?.id;
                    copied.insert(*asset, copy);
                    copy
                }
            };
            *asset = copy;
        }
        let path = self.new_document(AssetKind::Video);
        spectrum_video::Workspace::create(&path, video.clone(), self.actor.clone(), self.session)?;
        let copy = self.library.register(AssetKind::Video, name, &path)?;
        let mut used = video.uses();
        used.sort_by_key(|(_, asset)| *asset);
        used.dedup_by_key(|(_, asset)| *asset);
        self.library.references(copy.id, &used)?;
        Ok(copy)
    }

    /// Adds an asset to a video at `at`, returning the clips made.
    pub fn add_to_video(
        &mut self,
        video: AssetId,
        asset: AssetId,
        at: Time,
        track: Option<u64>,
        placement: Placement,
    ) -> Result<Vec<u64>> {
        let command = self.clip_command(asset, at, track, placement)?;
        let outputs = self.edit_video(video, vec![command])?;
        Ok(outputs.into_iter().flat_map(|o| o.clip_ids).collect())
    }
}
