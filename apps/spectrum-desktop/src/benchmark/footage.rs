//! Footage in the player: opening, scrubbing, playing, and the transport's
//! shortcuts, on the heaviest common camera format: 4K at 59.94 fps in
//! 10-bit 4:2:2 HEVC at a camera's bitrate, with sound.
use super::{
    Outcome,
    scenarios::{This, ms, outcome, pause, read, until},
};
use crate::workspace::{Open, Workspace};
use anyhow::{Context as _, Result, bail};
use gpui::*;
use spectrum_media::Time;
use std::path::{Path, PathBuf};

/// Seconds of generated footage.
const SECONDS: u32 = 6;

/// A Sony-style clip made with the `ffmpeg` command: moving test patterns
/// with grain, so it codes like a real picture rather than flat color.
pub(super) fn generate(directory: &Path) -> Result<PathBuf> {
    let path = directory.join("C0001.MP4");
    let picture = format!("testsrc2=s=3840x2160:r=60000/1001:d={SECONDS},noise=alls=10:allf=t");
    let sound = format!("sine=frequency=440:sample_rate=48000:duration={SECONDS}");
    let status = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", &picture, "-f", "lavfi", "-i", &sound])
        .args([
            "-c:v",
            "libx265",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv422p10le",
        ])
        .args(["-b:v", "150M", "-x265-params", "log-level=error:keyint=60"])
        .args(["-tag:v", "hvc1", "-c:a", "pcm_s24le", "-ac", "2"])
        .arg(&path)
        .status()
        .context("the benchmark makes its footage with the ffmpeg command; install FFmpeg")?;
    if !status.success() {
        bail!("ffmpeg could not make the benchmark footage");
    }
    Ok(path)
}

/// Ten seconds of stereo sound at 44.1 kHz as a WAV file: a tone that
/// swells and fades, so its waveform has a shape.
pub(super) fn song(directory: &Path) -> Result<PathBuf> {
    let path = directory.join("song.wav");
    let rate = 44_100u32;
    let frames = rate * 10;
    let mut bytes = Vec::with_capacity(44 + frames as usize * 4);
    let data = frames * 4;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&2u16.to_le_bytes()); // stereo
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for frame in 0..frames {
        let t = frame as f32 / rate as f32;
        let level = (t * std::f32::consts::PI / 10.).sin() * 0.6;
        let sample = ((t * 440. * std::f32::consts::TAU).sin() * level * 32767.) as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(&path, bytes)?;
    Ok(path)
}

fn settled(ws: &Workspace) -> bool {
    ws.player.as_ref().is_some_and(|player| player.settled())
}

fn shown(this: &This, cx: &mut AsyncWindowContext) -> Option<i64> {
    read(this, cx, |ws| {
        ws.player.as_ref()?.shown.as_ref().map(|(_, index)| *index)
    })
    .flatten()
}

pub(super) async fn open(
    this: &This,
    cx: &mut AsyncWindowContext,
    project: spectrum_library::ProjectId,
    footage: spectrum_library::AssetId,
) -> Outcome {
    crate::perf::drain();
    this.update_in(cx, |ws, window, cx| {
        ws.enter_project(project, window, cx);
        ws.open_item(Open::Media(footage), window, cx);
    })
    .ok();
    match until(this, cx, 20_000, settled).await {
        Some(took) => outcome("footage_open", vec![("elapsed", ms(took))], None),
        None => outcome(
            "footage_open",
            vec![],
            Some("the footage never showed".into()),
        ),
    }
}

/// Drags the scrub bar across the clip and back at 60 moves a second, as a
/// hand does, then waits for the frame under the pointer.
pub(super) async fn scrub(this: &This, cx: &mut AsyncWindowContext) -> Outcome {
    crate::perf::drain();
    let duration = read(this, cx, |ws| ws.player.as_ref().map(|p| p.duration()))
        .flatten()
        .unwrap_or_default();
    let mut seen = std::collections::BTreeSet::new();
    let steps = 120;
    for step in 0..=steps {
        let share = 1. - (step as f64 / steps as f64 * 2. - 1.).abs() * 0.9;
        let time = Time((duration.0 as f64 * (0.95 - share * 0.9).abs()) as i64);
        this.update(cx, |ws, cx| {
            ws.seek_player(time, cx);
        })
        .ok();
        pause(cx, 16).await;
        seen.extend(shown(this, cx));
    }
    let caught_up = until(this, cx, 10_000, settled).await;
    // The paused frame then redraws from the original at full quality.
    let sharp = until(this, cx, 10_000, |ws| {
        ws.player
            .as_ref()
            .is_some_and(|p| p.settled() && p.shown_sharp)
    })
    .await;
    let mut extra = vec![("frames_seen", seen.len() as f64)];
    if let Some(took) = caught_up {
        extra.push(("caught_up", ms(took)));
    }
    if let Some(took) = sharp {
        extra.push(("sharp", ms(took)));
    }
    let problem = if caught_up.is_none() {
        Some("the picture never caught up with the scrub".into())
    } else if sharp.is_none() {
        Some("the paused frame never redrew from the original".into())
    } else if seen.len() < 8 {
        Some(format!(
            "only {} frames showed during a two-second scrub",
            seen.len()
        ))
    } else {
        None
    };
    outcome("footage_scrub", extra, problem)
}

/// The footage's frame rate and last frame.
fn timing(this: &This, cx: &mut AsyncWindowContext) -> (spectrum_media::FrameRate, i64) {
    read(this, cx, |ws| {
        let timing = ws.player.as_ref()?.timing;
        Some((timing.rate, timing.frames - 1))
    })
    .flatten()
    .unwrap_or((spectrum_media::FrameRate::new(60000, 1001), 0))
}

/// Plays three seconds from the start, then checks that each transport
/// shortcut moves the picture as it should.
pub(super) async fn play(this: &This, cx: &mut AsyncWindowContext) -> Outcome {
    let (rate, _) = timing(this, cx);
    this.update(cx, |ws, cx| {
        ws.seek_player(Time::ZERO, cx);
    })
    .ok();
    until(this, cx, 10_000, settled).await;
    crate::perf::drain();
    this.update(cx, |ws, cx| {
        ws.toggle_play(cx);
    })
    .ok();
    pause(cx, 3_000).await;
    this.update(cx, |ws, cx| {
        ws.toggle_play(cx);
    })
    .ok();
    let samples = crate::perf::drain();
    let played = shown(this, cx).unwrap_or_default();
    let mut problems = Vec::new();
    let expected = rate.fps() * 3.;
    if (played as f64) < expected * 0.85 || (played as f64) > expected * 1.15 {
        problems.push(format!("playing three seconds reached frame {played}"));
    }
    problems.extend(controls(this, cx).await);
    let mut result = outcome("footage_play", vec![], None);
    for (kind, values) in samples {
        result.samples.entry(kind).or_default().extend(values);
    }
    result.problem = (!problems.is_empty()).then(|| problems.join("; "));
    result
}

/// Moves the playhead with `act`, waits for the picture, and returns the
/// frame shown.
async fn after(
    this: &This,
    cx: &mut AsyncWindowContext,
    act: impl FnOnce(&mut Workspace, &mut Context<Workspace>),
) -> i64 {
    this.update(cx, |ws, cx| act(ws, cx)).ok();
    until(this, cx, 10_000, settled).await;
    shown(this, cx).unwrap_or(-1)
}

/// Home, the arrows, a typed time, End, and J, K, and L each land where
/// they should.
async fn controls(this: &This, cx: &mut AsyncWindowContext) -> Vec<String> {
    let (rate, last) = timing(this, cx);
    let mut problems = Vec::new();
    let mut expect = |name: &str, at: i64, wanted: i64| {
        if at != wanted {
            problems.push(format!("{name} showed frame {at}, not {wanted}"));
        }
    };
    let at = after(this, cx, |ws, cx| {
        ws.seek_player(Time::ZERO, cx);
    })
    .await;
    expect("Home", at, 0);
    let at = after(this, cx, |ws, cx| {
        ws.step_frames(1, cx);
    })
    .await;
    expect("Right arrow", at, 1);
    let at = after(this, cx, |ws, cx| {
        ws.step_frames(10, cx);
    })
    .await;
    expect("Shift-Right arrow", at, 11);
    let at = after(this, cx, |ws, cx| {
        ws.step_frames(-1, cx);
    })
    .await;
    expect("Left arrow", at, 10);
    let typed = Time::parse("00:00:02:00", Some(rate)).unwrap_or_default();
    let at = after(this, cx, |ws, cx| {
        ws.seek_player(typed, cx);
    })
    .await;
    expect("a typed time", at, rate.frame_at(typed));
    let at = after(this, cx, |ws, cx| {
        let end = ws.player.as_ref().map_or(Time::ZERO, |p| p.end());
        ws.seek_player(end, cx);
    })
    .await;
    expect("End", at, last);
    // L plays forward and J backward for a second; K stops.
    let second = rate.fps().round() as i64;
    for direction in [1, -1] {
        let start = if direction > 0 { second } else { last - second };
        after(this, cx, |ws, cx| {
            ws.seek_player(rate.time_of(start), cx);
        })
        .await;
        this.update(cx, |ws, cx| {
            ws.shuttle(direction as i32, cx);
        })
        .ok();
        pause(cx, 1_000).await;
        let at = after(this, cx, |ws, cx| {
            ws.shuttle(0, cx);
        })
        .await;
        let moved = (at - start) * direction;
        if moved < second / 2 || moved > second * 3 / 2 {
            let key = if direction > 0 { "L" } else { "J" };
            problems.push(format!("{key} for a second moved {moved} frames"));
        }
    }
    problems
}

/// Opens a song, waits for its waveform, and plays a second and a half.
pub(super) async fn audio(
    this: &This,
    cx: &mut AsyncWindowContext,
    project: spectrum_library::ProjectId,
    song: spectrum_library::AssetId,
) -> Outcome {
    crate::perf::drain();
    this.update_in(cx, |ws, window, cx| {
        ws.enter_project(project, window, cx);
        ws.open_item(Open::Media(song), window, cx);
    })
    .ok();
    let drawn = until(this, cx, 20_000, |ws| {
        ws.player_controls
            .as_ref()
            .is_some_and(|c| c.waveform.is_some())
    })
    .await;
    this.update(cx, |ws, cx| {
        ws.toggle_play(cx);
    })
    .ok();
    pause(cx, 1_500).await;
    this.update(cx, |ws, cx| {
        ws.toggle_play(cx);
    })
    .ok();
    let at = read(this, cx, |ws| ws.player.as_ref().map(|p| p.now().seconds()))
        .flatten()
        .unwrap_or_default();
    let problem = if drawn.is_none() {
        Some("the waveform never showed".into())
    } else if !(1.1..=1.9).contains(&at) {
        Some(format!("playing a second and a half reached {at:.2} s"))
    } else {
        None
    };
    let extra = drawn
        .map(|took| vec![("elapsed", ms(took))])
        .unwrap_or_default();
    outcome("audio_play", extra, problem)
}
