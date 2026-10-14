//! Editing a video: every change is a command kept as a durable revision,
//! from the desktop's timeline and from `spectrum video` alike.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use spectrum_document::{FileVisitor, Model, Step};
use spectrum_time::{FrameRate, Time};

use crate::{Clip, Edge, Source, Speed, TrackKind, Video};

/// Where a new clip goes and what it does to what is there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// Replaces what is under it.
    #[default]
    Overwrite,
    /// Pushes later clips on every unlocked track along to make room.
    Insert,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    Rename {
        name: String,
    },
    /// Frame size and rate. Clip edges move to the new rate's frames.
    SetFormat {
        width: u32,
        height: u32,
        rate: FrameRate,
    },
    AddTrack {
        kind: TrackKind,
    },
    /// Removes a track and its clips.
    RemoveTrack {
        track: u64,
    },
    SetTrack {
        track: u64,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        muted: Option<bool>,
        #[serde(default)]
        locked: Option<bool>,
    },
    /// Adds a clip of `source` at `at` on `track` (the lowest unlocked track
    /// of its kind when omitted). Footage brings its first sound track,
    /// linked, onto `sound_track` unless `sound` is false.
    AddClip {
        source: Source,
        #[serde(default)]
        track: Option<u64>,
        at: Time,
        #[serde(default)]
        source_in: Time,
        duration: Time,
        #[serde(default)]
        source_duration: Option<Time>,
        #[serde(default)]
        name: String,
        #[serde(default)]
        placement: Placement,
        #[serde(default = "yes")]
        sound: bool,
        #[serde(default)]
        sound_track: Option<u64>,
    },
    /// Moves clips (with their linked clips) in time and between tracks.
    Move {
        clips: Vec<u64>,
        by: Time,
        #[serde(default)]
        tracks: i32,
    },
    Trim {
        clip: u64,
        edge: Edge,
        by: Time,
        #[serde(default)]
        ripple: bool,
    },
    /// Moves the cut after `clip`.
    Roll {
        clip: u64,
        by: Time,
    },
    Slip {
        clip: u64,
        by: Time,
    },
    Slide {
        clip: u64,
        by: Time,
    },
    /// Cuts `clips` (every clip there when empty) at `at`.
    Split {
        at: Time,
        #[serde(default)]
        clips: Vec<u64>,
    },
    Delete {
        clips: Vec<u64>,
        #[serde(default)]
        ripple: bool,
    },
    Link {
        clips: Vec<u64>,
    },
    Unlink {
        clips: Vec<u64>,
    },
    SetClip {
        clip: u64,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        enabled: Option<bool>,
        #[serde(default)]
        opacity: Option<f32>,
        #[serde(default)]
        gain_db: Option<f32>,
        #[serde(default)]
        interpolation: Option<crate::Interpolation>,
        #[serde(default)]
        sound_follows_speed: Option<bool>,
    },
    SetSpeed {
        clip: u64,
        speed: Speed,
    },
    /// Holds the frame showing at `at` (on the timeline) for `length`, or
    /// to the clip's end, then plays on; the clip keeps its length.
    Freeze {
        clip: u64,
        at: Time,
        #[serde(default)]
        length: Option<Time>,
    },
    /// Plays the clip's frames backward, ramps and all.
    Reverse {
        clip: u64,
    },
    /// Keeps the clip's timing as velocity or time remap keyframes; every
    /// frame stays where it was (within a quarter frame).
    ConvertSpeed {
        clip: u64,
        to: crate::SpeedForm,
    },
    Undo,
    Redo,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandOutput {
    pub ok: bool,
    pub action: String,
    pub message: String,
    /// Clips the command made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clip_ids: Vec<u64>,
    /// How far an edit moved, after its edges stopped at neighbors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved: Option<Time>,
}

fn output(action: &str, message: impl Into<String>) -> CommandOutput {
    CommandOutput {
        ok: true,
        action: action.into(),
        message: message.into(),
        clip_ids: Vec::new(),
        moved: None,
    }
}

/// Spectrum's video documents.
pub struct VideoModel;

/// A video, in memory or durably in its revision file.
pub type Workspace = spectrum_document::Workspace<VideoModel>;

impl Model for VideoModel {
    type Document = Video;
    type Command = Command;
    type Output = CommandOutput;

    const APPLICATION: &'static str = "spectrum.video";
    const NOUN: &'static str = "video";

    fn apply(video: &mut Video, command: Command) -> Result<CommandOutput> {
        apply(video, command)
    }

    fn describe(output: &CommandOutput) -> String {
        output.message.clone()
    }

    fn stepped(step: Step) -> CommandOutput {
        match step {
            Step::Undo => output("undo", "Went back one edit"),
            Step::Redo => output("redo", "Went forward one edit"),
        }
    }

    fn step(command: &Command) -> Option<Step> {
        match command {
            Command::Undo => Some(Step::Undo),
            Command::Redo => Some(Step::Redo),
            _ => None,
        }
    }

    fn document_files(_: &mut Video, _: &mut FileVisitor) -> Result<()> {
        Ok(())
    }

    fn command_files(_: &mut Command, _: &mut FileVisitor) -> Result<()> {
        Ok(())
    }
}

fn moved(action: &str, message: &str, by: Time) -> CommandOutput {
    CommandOutput {
        moved: Some(by),
        ..output(action, message)
    }
}

/// Applies one command, all or nothing: a command that fails partway (on a
/// locked track, say) leaves the video as it was.
pub fn apply(video: &mut Video, command: Command) -> Result<CommandOutput> {
    let mut draft = video.clone();
    let output = apply_to(&mut draft, command)?;
    *video = draft;
    Ok(output)
}

fn apply_to(video: &mut Video, command: Command) -> Result<CommandOutput> {
    Ok(match command {
        Command::Rename { name } => {
            let name = name.trim();
            if name.is_empty() {
                bail!("a video needs a name");
            }
            video.name = name.into();
            output("rename", "Renamed the video")
        }
        Command::SetFormat {
            width,
            height,
            rate,
        } => {
            if width < 2 || height < 2 || width > 16384 || height > 16384 {
                bail!("a video is 2 to 16384 pixels on each side");
            }
            if rate.numerator == 0 || rate.denominator == 0 || rate.fps() > 240. {
                bail!("{rate} is not a frame rate Spectrum edits at");
            }
            video.width = width;
            video.height = height;
            video.rate = rate;
            for clip in video.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
                let end = snap_to(rate, clip.end());
                clip.start = snap_to(rate, clip.start);
                clip.duration = (end - clip.start).max(rate.frame_duration());
            }
            output("set_format", format!("{width} × {height} at {rate}"))
        }
        Command::AddTrack { kind } => {
            let id = video.add_track(kind);
            CommandOutput {
                clip_ids: vec![id],
                ..output("add_track", "Added a track")
            }
        }
        Command::RemoveTrack { track } => {
            let kind = video.track(track).context("no such track")?.kind;
            if video.tracks.iter().filter(|t| t.kind == kind).count() == 1 {
                bail!("a video keeps at least one track of each kind");
            }
            video.tracks.retain(|t| t.id != track);
            output("remove_track", "Removed a track")
        }
        Command::SetTrack {
            track,
            name,
            muted,
            locked,
        } => {
            let track = video.track_mut(track).context("no such track")?;
            if let Some(name) = name.filter(|name| !name.trim().is_empty()) {
                track.name = name.trim().into();
            }
            if let Some(muted) = muted {
                track.muted = muted;
            }
            if let Some(locked) = locked {
                track.locked = locked;
            }
            output("set_track", "Changed a track")
        }
        Command::AddClip {
            source,
            track,
            at,
            source_in,
            duration,
            source_duration,
            name,
            placement,
            sound,
            sound_track,
        } => add_clip(
            video,
            NewClip {
                source,
                track,
                at,
                source_in,
                duration,
                source_duration,
                name,
                placement,
                sound,
                sound_track,
            },
        )?,
        Command::Move { clips, by, tracks } => {
            video.move_clips(&clips, by, tracks)?;
            output("move", "Moved clips")
        }
        Command::Trim {
            clip,
            edge,
            by,
            ripple,
        } => {
            let by = video.trim(clip, edge, by, ripple)?;
            moved(
                "trim",
                if ripple { "Ripple trimmed" } else { "Trimmed" },
                by,
            )
        }
        Command::Roll { clip, by } => moved("roll", "Rolled a cut", video.roll(clip, by)?),
        Command::Slip { clip, by } => moved("slip", "Slipped a clip", video.slip(clip, by)?),
        Command::Slide { clip, by } => moved("slide", "Slid a clip", video.slide(clip, by)?),
        Command::Split { at, clips } => {
            let made = video.split(at, &clips)?;
            if made.is_empty() {
                bail!("no clip to cut there");
            }
            CommandOutput {
                clip_ids: made,
                ..output("split", "Cut clips")
            }
        }
        Command::Delete { clips, ripple } => {
            video.delete(&clips, ripple)?;
            output(
                "delete",
                if ripple {
                    "Ripple deleted"
                } else {
                    "Deleted clips"
                },
            )
        }
        Command::Link { clips } => {
            video.link(&clips, true)?;
            output("link", "Linked clips")
        }
        Command::Unlink { clips } => {
            video.link(&clips, false)?;
            output("unlink", "Unlinked clips")
        }
        Command::SetClip {
            clip,
            name,
            enabled,
            opacity,
            gain_db,
            interpolation,
            sound_follows_speed,
        } => {
            let c = video.clip_mut(clip).context("no such clip")?;
            if let Some(interpolation) = interpolation {
                c.interpolation = interpolation;
            }
            if let Some(follows) = sound_follows_speed {
                c.sound_follows_speed = follows;
            }
            if let Some(name) = name {
                c.name = name;
            }
            if let Some(enabled) = enabled {
                c.enabled = enabled;
            }
            if let Some(opacity) = opacity {
                c.opacity = opacity.clamp(0., 1.);
            }
            if let Some(gain) = gain_db {
                c.gain_db = gain.clamp(-60., 24.);
            }
            output("set_clip", "Changed a clip")
        }
        Command::SetSpeed { clip, speed } => {
            let speed = speed.checked()?;
            for id in video.with_links(&[clip]) {
                video.clip_mut(id).context("no such clip")?.speed = speed.clone();
            }
            output("set_speed", "Changed a clip's speed")
        }
        Command::Freeze { clip, at, length } => {
            let found = video.clip(clip).context("no such clip")?.1;
            if !found.covers(at) {
                bail!("the clip is not at {}", at.timecode(video.rate));
            }
            if length.is_some_and(|length| length <= Time::ZERO) {
                bail!("a freeze lasts longer than nothing");
            }
            let speed = found.speed.frozen(at - found.start, length);
            for id in video.with_links(&[clip]) {
                video.clip_mut(id).context("no such clip")?.speed = speed.clone();
            }
            output("freeze", "Froze a frame")
        }
        Command::Reverse { clip } => {
            let frame = video.rate.frame_duration();
            for id in video.with_links(&[clip]) {
                let clip = video.clip_mut(id).context("no such clip")?;
                let (speed, moved) = clip.speed.reversed(clip.duration, frame);
                clip.speed = speed;
                // Sound kept at normal speed plays on from the same place.
                if clip.sound_follows_speed || !matches!(clip.source, Source::Sound { .. }) {
                    clip.source_in =
                        Time::from_seconds(clip.wrap(clip.source_in.seconds() + moved));
                }
            }
            output("reverse", "Reversed a clip")
        }
        Command::ConvertSpeed { clip, to } => {
            // A quarter frame: every frame shows the same moment.
            let tolerance = video.rate.frame_duration().seconds() / 4.;
            let mut exact = true;
            for id in video.with_links(&[clip]) {
                let clip = video.clip_mut(id).context("no such clip")?;
                let converted = clip.converted(to, tolerance);
                exact &= converted.exact;
                clip.speed = converted.speed.checked()?;
                clip.source_in = converted.source_in;
            }
            let message = if exact {
                "Kept the clip's timing as other keyframes"
            } else {
                "Kept the clip's timing; a jump or a speed past 1000% became the nearest velocity"
            };
            output("convert_speed", message)
        }
        Command::Undo | Command::Redo => bail!("history is stepped by the workspace"),
    })
}

fn snap_to(rate: FrameRate, time: Time) -> Time {
    rate.time_of(rate.frame_at(time + Time(rate.frame_duration().0 / 2)))
}

struct NewClip {
    source: Source,
    track: Option<u64>,
    at: Time,
    source_in: Time,
    duration: Time,
    source_duration: Option<Time>,
    name: String,
    placement: Placement,
    sound: bool,
    sound_track: Option<u64>,
}

/// The track a new clip goes on: the one asked for, or the lowest unlocked
/// track of `kind` that is free over `[from, to)`, made if none is.
fn target(
    video: &mut Video,
    kind: TrackKind,
    wanted: Option<u64>,
    from: Time,
    to: Time,
) -> Result<u64> {
    if let Some(id) = wanted {
        let track = video.track(id).context("no such track")?;
        if track.kind != kind {
            bail!("{} is not a {kind:?} track", track.name);
        }
        return Ok(id);
    }
    let free = video.tracks.iter().find(|t| {
        t.kind == kind && !t.locked && t.clips.iter().all(|c| c.end() <= from || c.start >= to)
    });
    match free {
        Some(track) => Ok(track.id),
        None => Ok(video.add_track(kind)),
    }
}

fn add_clip(video: &mut Video, new: NewClip) -> Result<CommandOutput> {
    let frame = video.rate.frame_duration();
    let at = video.snap(new.at.max(Time::ZERO));
    let mut duration = video.snap(new.duration).max(frame);
    if let Some(length) = new.source_duration {
        let left = (length - new.source_in).max(Time::ZERO);
        duration = duration.min(video.snap(left).max(frame));
    }
    let footage = matches!(new.source, Source::Footage { .. });
    let link = (footage && new.sound).then(|| video.take_id());
    let picture_track = target(video, new.source.kind(), new.track, at, at + duration)?;
    let mut made = Vec::new();
    let mut clip = Clip {
        id: video.take_id(),
        name: new.name.clone(),
        source: new.source,
        start: at,
        duration,
        source_in: new.source_in.max(Time::ZERO),
        source_duration: new.source_duration,
        link,
        enabled: true,
        opacity: 1.,
        gain_db: 0.,
        speed: Speed::Normal,
        interpolation: crate::Interpolation::default(),
        sound_follows_speed: true,
    };
    made.push(clip.id);
    let insert = new.placement == Placement::Insert;
    video.put(picture_track, clip.clone(), insert)?;
    if let Some(link) = link {
        let sound_track = target(video, TrackKind::Audio, new.sound_track, at, at + duration)?;
        clip.id = video.take_id();
        clip.source = Source::Sound {
            asset: new.source.asset(),
            track: 0,
        };
        clip.link = Some(link);
        made.push(clip.id);
        // Inserting already made room on every track.
        if insert {
            video
                .track_mut(sound_track)
                .context("no such track")?
                .place(clip);
        } else {
            video.put(sound_track, clip, false)?;
        }
    }
    Ok(CommandOutput {
        clip_ids: made,
        ..output("add_clip", format!("Added {}", new.name))
    })
}
