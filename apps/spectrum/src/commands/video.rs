//! `spectrum video`: making and editing videos, as the desktop's timeline
//! does, and exporting them.
use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde_json::json;
use spectrum_assets::Service;
use spectrum_library::{AssetId, AssetKind, ProjectId};
use spectrum_media::{ExportSettings, FrameRate, Time, VideoCodec};
use spectrum_video::{Command, Curve, Interpolation, Placement, RemapKey, Speed, VelocityKey};
use std::path::PathBuf;

/// A video is an edit: tracks of clips that use footage, audio, images,
/// and canvases. Times take seconds (1.5), frames (36f), or timecode
/// (00:00:01:12).
#[derive(Subcommand)]
pub(super) enum Video {
    /// List video assets.
    List,
    /// Make a video. `--like` takes a footage asset's size and frame rate.
    New {
        name: String,
        #[arg(long, default_value_t = 1920)]
        width: u32,
        #[arg(long, default_value_t = 1080)]
        height: u32,
        /// Frames a second: 23.976, 24, 25, 29.97, 30, 50, 59.94, 60, or
        /// a fraction such as 30000/1001.
        #[arg(long, default_value = "30")]
        rate: String,
        #[arg(long, conflicts_with_all = ["width", "height", "rate"])]
        like: Option<AssetId>,
        #[arg(long = "project", id = "project_id")]
        project: Option<ProjectId>,
    },
    /// Show a video's tracks and clips as JSON.
    Inspect { video: AssetId },
    /// Add footage, audio, an image, or a canvas at a time. Footage brings
    /// its sound, linked.
    Add {
        video: AssetId,
        asset: AssetId,
        #[arg(long, default_value = "0")]
        at: String,
        /// The track to put it on; the lowest of its kind when omitted.
        #[arg(long)]
        track: Option<u64>,
        /// Push later clips along instead of overwriting them.
        #[arg(long)]
        insert: bool,
    },
    /// Set a clip's speed (its linked sound follows): velocity keyframes as
    /// time=percent pairs ("0=100,1.5s=25,3s=-100"), a time remap as
    /// clip time=source time pairs ("0=2s,1s=2s,3s=0"), or normal; or
    /// freeze a frame, or reverse the clip. Times count from the clip's
    /// start.
    Speed {
        video: AssetId,
        clip: u64,
        #[arg(long, conflicts_with_all = ["remap", "normal"])]
        velocity: Option<String>,
        #[arg(long, conflicts_with = "normal")]
        remap: Option<String>,
        #[arg(long)]
        normal: bool,
        /// Hold the frame at this time, for --hold or to the clip's end.
        #[arg(long)]
        freeze: Option<String>,
        /// How long a freeze lasts.
        #[arg(long, requires = "freeze")]
        hold: Option<String>,
        /// Play the clip's frames backward, after any other change.
        #[arg(long)]
        reverse: bool,
        /// Keep the clip's timing as velocity or remap keyframes (every
        /// frame stays where it is).
        #[arg(long = "as")]
        form: Option<String>,
        /// How each keyframe moves to the next: smooth, linear, fast, slow,
        /// sharp, or hold.
        #[arg(long, default_value = "smooth")]
        curve: String,
        /// Frames made in slow motion: off, blend, or motion.
        #[arg(long)]
        interpolate: Option<String>,
        /// Whether the sound follows the speed: follow or keep.
        #[arg(long)]
        sound: Option<String>,
    },
    /// Execute video Command JSON: one object, or an array applied as one
    /// edit. See `spectrum video schema`.
    Command { video: AssetId, json: String },
    /// Save the frame showing at a time as an image outside the library.
    Frame {
        video: AssetId,
        path: PathBuf,
        #[arg(long, default_value = "0")]
        at: String,
        #[arg(long)]
        max_size: Option<u32>,
    },
    /// Export the video outside the library.
    Export {
        video: AssetId,
        path: PathBuf,
        /// h264 (MP4), hevc (MP4), or prores (MOV).
        #[arg(long, default_value = "h264")]
        codec: String,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
        /// Bits a second for H.264 and HEVC.
        #[arg(long)]
        bitrate: Option<u64>,
        #[arg(long)]
        no_sound: bool,
    },
    /// Show the command formats with examples.
    Schema,
}

/// A typed frame rate: decimal (29.97) or a fraction (30000/1001).
pub(super) fn parse_rate(text: &str) -> Result<FrameRate> {
    let rate = match text.split_once('/') {
        Some((n, d)) => FrameRate::new(n.trim().parse()?, d.trim().parse()?),
        None => {
            let fps: f64 = text.trim().parse().context("not a frame rate")?;
            FrameRate::new((fps * 1000.).round() as u32, 1000).standardized()
        }
    };
    if rate.numerator == 0 || rate.denominator == 0 {
        bail!("{text} is not a frame rate");
    }
    Ok(rate)
}

fn videos(service: &Service) -> Result<serde_json::Value> {
    let assets: Vec<_> = service
        .library
        .list()?
        .into_iter()
        .filter(|asset| asset.kind == AssetKind::Video)
        .collect();
    Ok(serde_json::to_value(assets)?)
}

pub(super) fn run(service: &mut Service, command: Video) -> Result<serde_json::Value> {
    match command {
        Video::List => videos(service),
        Video::New {
            name,
            width,
            height,
            rate,
            like,
            project,
        } => {
            if let Some(project) = project {
                service.library.project(project)?;
            }
            let (width, height, rate) = match like {
                Some(footage) => {
                    let video = service
                        .media(footage)?
                        .0
                        .info
                        .video
                        .context("not footage")?;
                    (video.width, video.height, video.rate)
                }
                None => (width, height, parse_rate(&rate)?),
            };
            let asset = service.create_video(&name, width & !1, height & !1, rate)?;
            if let Some(project) = project {
                service.library.add_to_project(project, &[asset.id])?;
            }
            Ok(serde_json::to_value(asset)?)
        }
        Video::Inspect { video } => Ok(serde_json::to_value(service.video(video)?)?),
        Video::Add {
            video,
            asset,
            at,
            track,
            insert,
        } => {
            let rate = service.video(video)?.rate;
            let at = Time::parse(&at, Some(rate))?;
            let placement = if insert {
                Placement::Insert
            } else {
                Placement::Overwrite
            };
            let clips = service.add_to_video(video, asset, at, track, placement)?;
            Ok(json!({"video": video, "clips": clips}))
        }
        Video::Command { video, json } => {
            let commands: Vec<Command> = match serde_json::from_str::<serde_json::Value>(&json)? {
                serde_json::Value::Array(items) => items
                    .into_iter()
                    .map(serde_json::from_value)
                    .collect::<Result<_, _>>()?,
                value => vec![serde_json::from_value(value)?],
            };
            let outputs = service.edit_video(video, commands)?;
            Ok(json!({"video": video, "results": outputs}))
        }
        Video::Frame {
            video,
            path,
            at,
            max_size,
        } => {
            service.check_export(&path)?;
            let edit = service.video(video)?;
            let time = Time::parse(&at, Some(edit.rate))?;
            let max = max_size.unwrap_or(edit.width.max(edit.height));
            let scale = (f64::from(max) / f64::from(edit.width.max(edit.height))).min(1.);
            let size = (
                ((f64::from(edit.width) * scale).round() as u32).max(1),
                ((f64::from(edit.height) * scale).round() as u32).max(1),
            );
            let frame = service.video_frame(video, time, size)?;
            image::DynamicImage::ImageRgba8(frame)
                .to_rgb8()
                .save(&path)
                .with_context(|| format!("could not write {}", path.display()))?;
            Ok(json!({"exported": path, "frame": edit.rate.frame_at(time)}))
        }
        Video::Export {
            video,
            path,
            codec,
            width,
            height,
            bitrate,
            no_sound,
        } => {
            let edit = service.video(video)?;
            let codec = match codec.as_str() {
                "h264" => VideoCodec::H264,
                "hevc" | "h265" => VideoCodec::Hevc,
                "prores" => VideoCodec::ProRes,
                other => bail!("{other} is not h264, hevc, or prores"),
            };
            let settings = ExportSettings {
                codec,
                width: width.unwrap_or(edit.width) & !1,
                height: height.unwrap_or(edit.height) & !1,
                rate: edit.rate,
                bitrate,
                sound: !no_sound,
            };
            let started = std::time::Instant::now();
            let encoder = service.export_video(video, &path, settings, &mut |_| true)?;
            Ok(json!({
                "exported": path,
                "encoder": encoder,
                "seconds": started.elapsed().as_secs_f64(),
            }))
        }
        Video::Speed {
            video,
            clip,
            velocity,
            remap,
            normal,
            freeze,
            hold,
            reverse,
            form,
            curve,
            interpolate,
            sound,
        } => {
            let edit = service.video(video)?;
            let rate = edit.rate;
            let curve = match curve.as_str() {
                "smooth" => Curve::Smooth,
                "linear" => Curve::Linear,
                "fast" => Curve::Fast,
                "slow" => Curve::Slow,
                "sharp" => Curve::Sharp,
                "hold" => Curve::Hold,
                other => bail!("{other} is not a curve"),
            };
            let pairs = |text: &str| -> Result<Vec<(Time, String)>> {
                text.split(',')
                    .map(|pair| {
                        let (at, value) = pair
                            .split_once('=')
                            .context("keyframes are time=value pairs")?;
                        Ok((Time::parse(at, Some(rate))?, value.trim().to_string()))
                    })
                    .collect()
            };
            let mut commands = Vec::new();
            if let Some(text) = velocity {
                let keys = pairs(&text)?
                    .into_iter()
                    .map(|(at, value)| {
                        let percent = value.trim_end_matches('%').parse()?;
                        Ok(VelocityKey { at, percent, curve })
                    })
                    .collect::<Result<_>>()?;
                commands.push(Command::SetSpeed {
                    clip,
                    speed: Speed::Velocity {
                        keys,
                        origin: Time::ZERO,
                    },
                });
            } else if let Some(text) = remap {
                let keys = pairs(&text)?
                    .into_iter()
                    .map(|(at, value)| {
                        Ok(RemapKey {
                            at,
                            source: Time::parse(&value, Some(rate))?,
                            curve,
                        })
                    })
                    .collect::<Result<_>>()?;
                commands.push(Command::SetSpeed {
                    clip,
                    speed: Speed::Remap {
                        keys,
                        origin: Time::ZERO,
                    },
                });
            } else if normal {
                commands.push(Command::SetSpeed {
                    clip,
                    speed: Speed::Normal,
                });
            }
            if let Some(at) = freeze {
                let start = edit.clip(clip).context("no such clip")?.1.start;
                commands.push(Command::Freeze {
                    clip,
                    at: start + Time::parse(&at, Some(rate))?,
                    length: hold.map(|h| Time::parse(&h, Some(rate))).transpose()?,
                });
            }
            if reverse {
                commands.push(Command::Reverse { clip });
            }
            if let Some(form) = form {
                let to = match form.as_str() {
                    "velocity" => spectrum_video::SpeedForm::Velocity,
                    "remap" => spectrum_video::SpeedForm::Remap,
                    other => bail!("{other} is not velocity or remap"),
                };
                commands.push(Command::ConvertSpeed { clip, to });
            }
            let interpolation = interpolate
                .map(|text| match text.as_str() {
                    "off" => Ok(Interpolation::Off),
                    "blend" => Ok(Interpolation::Blend),
                    "motion" => Ok(Interpolation::Motion),
                    other => Err(anyhow::anyhow!("{other} is not off, blend, or motion")),
                })
                .transpose()?;
            let follows = sound
                .map(|text| match text.as_str() {
                    "follow" => Ok(true),
                    "keep" => Ok(false),
                    other => Err(anyhow::anyhow!("{other} is not follow or keep")),
                })
                .transpose()?;
            if interpolation.is_some() || follows.is_some() {
                for id in edit.linked(clip) {
                    commands.push(Command::SetClip {
                        clip: id,
                        name: None,
                        enabled: None,
                        opacity: None,
                        gain_db: None,
                        interpolation,
                        sound_follows_speed: follows,
                    });
                }
            }
            if commands.is_empty() {
                bail!(
                    "give --velocity, --remap, --normal, --freeze, --reverse, --interpolate, or --sound"
                );
            }
            let outputs = service.edit_video(video, commands)?;
            Ok(json!({"video": video, "clip": clip, "results": outputs}))
        }
        Video::Schema => Ok(schema()),
    }
}

fn schema() -> serde_json::Value {
    json!({
        "times": "flicks (1/705600000 s) in JSON; typed times in flags take 1.5, 36f, or 00:00:01:12",
        "examples": [
            {"command": "split", "at": 705600000, "clips": []},
            {"command": "trim", "clip": 3, "edge": "end", "by": -352800000, "ripple": true},
            {"command": "move", "clips": [3], "by": 705600000, "tracks": 1},
            {"command": "roll", "clip": 3, "by": 23520000},
            {"command": "slip", "clip": 3, "by": 23520000},
            {"command": "slide", "clip": 3, "by": 23520000},
            {"command": "delete", "clips": [3], "ripple": true},
            {"command": "set_clip", "clip": 3, "opacity": 0.5, "gain_db": -6.0, "interpolation": "motion", "sound_follows_speed": true},
            {"command": "set_speed", "clip": 3, "speed": {"kind": "velocity", "origin": 0, "keys": [
                {"at": 0, "percent": 100.0, "curve": "smooth"},
                {"at": 705600000, "percent": -100.0, "curve": "linear"}]}},
            {"command": "set_speed", "clip": 3, "speed": {"kind": "remap", "origin": 0, "keys": [
                {"at": 0, "source": 1411200000, "curve": "hold"}]}},
            {"command": "freeze", "clip": 3, "at": 705600000, "length": 705600000},
            {"command": "reverse", "clip": 3},
            {"command": "convert_speed", "clip": 3, "to": "remap"},
            {"command": "set_track", "track": 1, "muted": true},
            {"command": "add_track", "kind": "video"},
            {"command": "undo"}
        ],
        "commands": ["rename", "set_format", "add_track", "remove_track", "set_track", "add_clip",
            "move", "trim", "roll", "slip", "slide", "split", "delete", "link", "unlink",
            "set_clip", "set_speed", "freeze", "reverse", "convert_speed", "undo", "redo"],
    })
}
