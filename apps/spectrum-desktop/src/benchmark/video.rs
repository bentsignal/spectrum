//! The video editor on a 4K edit of camera footage with cuts, a photo over
//! it, and a song: opening, scrubbing, playing across cuts, dragging a
//! trim, and every timeline tool and clip edit (placing media, freezing,
//! reversing), each checked to work, be saved, and undo and redo.
use super::{
    Outcome,
    scenarios::{This, key, ms, outcome, pause, read, until},
};
use crate::{
    video_layout::HEADER,
    video_state::Tool,
    workspace::{Open, Workspace},
};
use anyhow::{Context as _, Result};
use gpui::*;
use spectrum_assets::Service;
use spectrum_library::{AssetId, ProjectId};
use spectrum_media::Time;
use spectrum_video::{Command, Edge, Placement, Video};
use std::time::Instant;

/// A 4K edit: the footage cut in three on V1 and again after it, a photo
/// over the second cut on V2, and the song under all of it.
pub(super) fn make(
    service: &mut Service,
    project: ProjectId,
    footage: AssetId,
    photo: AssetId,
    song: AssetId,
) -> Result<AssetId> {
    let video = service.media(footage)?.0.info.video.context("footage")?;
    let edit = service.create_video("Benchmark edit", video.width, video.height, video.rate)?;
    service.library.add_to_project(project, &[edit.id])?;
    let rate = video.rate;
    let seconds = |s: f64| rate.time_of((s * rate.fps()).round() as i64);
    let v1 = service.video(edit.id)?.tracks[0].id;
    let mut commands = vec![
        service.clip_command(footage, Time::ZERO, Some(v1), Placement::Overwrite)?,
        service.clip_command(footage, seconds(6.), Some(v1), Placement::Overwrite)?,
        Command::Split {
            at: seconds(2.),
            clips: vec![],
        },
        Command::Split {
            at: seconds(4.),
            clips: vec![],
        },
    ];
    commands.push(service.clip_command(photo, seconds(3.), None, Placement::Overwrite)?);
    commands.push(service.clip_command(song, Time::ZERO, None, Placement::Overwrite)?);
    service.edit_video(edit.id, commands)?;
    // The photo shows for two seconds over the cut at four; the footage
    // after six slows to a quarter, then plays backward.
    let doc = service.video(edit.id)?;
    let still = doc.tracks[1].clips[0].id;
    let ramped = doc.tracks[0].clips.last().context("clips")?.id;
    let key = |at: f64, percent: f64| spectrum_video::VelocityKey {
        at: seconds(at),
        percent,
        curve: spectrum_video::Curve::Smooth,
    };
    service.edit_video(
        edit.id,
        vec![
            Command::Trim {
                clip: still,
                edge: Edge::End,
                by: seconds(-3.),
                ripple: false,
            },
            Command::SetSpeed {
                clip: ramped,
                speed: spectrum_video::Speed::Velocity {
                    keys: vec![key(0., 100.), key(1., 25.), key(2.5, -100.)],
                    origin: Time::ZERO,
                },
            },
        ],
    )?;
    Ok(edit.id)
}

pub(super) fn settled(ws: &Workspace) -> bool {
    ws.video.as_ref().is_some_and(|v| v.monitor.settled())
}

pub(super) fn idle(ws: &Workspace) -> bool {
    ws.video
        .as_ref()
        .is_some_and(|v| v.idle() && v.drag.is_none() && v.monitor.settled())
}

pub(super) fn doc(this: &This, cx: &mut AsyncWindowContext) -> Option<Video> {
    read(this, cx, |ws| ws.video.as_ref().map(|v| v.doc.clone())).flatten()
}

pub(super) async fn open(
    this: &This,
    cx: &mut AsyncWindowContext,
    project: ProjectId,
    video: AssetId,
) -> Outcome {
    crate::perf::drain();
    this.update_in(cx, |ws, window, cx| {
        ws.enter_project(project, window, cx);
        ws.open_item(Open::Video(video), window, cx);
    })
    .ok();
    match until(this, cx, 20_000, settled).await {
        Some(took) => outcome("video_open", vec![("elapsed", ms(took))], None),
        None => outcome("video_open", vec![], Some("the video never showed".into())),
    }
}

/// Drags the playhead across the edit and back, as on the ruler.
pub(super) async fn scrub(this: &This, cx: &mut AsyncWindowContext) -> Outcome {
    crate::perf::drain();
    let duration = read(this, cx, |ws| ws.video.as_ref().map(|v| v.doc.duration()))
        .flatten()
        .unwrap_or_default();
    let mut seen = std::collections::BTreeSet::new();
    for step in 0..=120 {
        let share = 1. - (f64::from(step) / 60. - 1.).abs();
        let time = Time((duration.0 as f64 * share * 0.95) as i64);
        this.update(cx, |ws, cx| {
            ws.seek_player(time, cx);
        })
        .ok();
        pause(cx, 16).await;
        seen.extend(
            read(this, cx, |ws| {
                ws.video.as_ref()?.monitor.shown.as_ref().map(|(_, i)| *i)
            })
            .flatten(),
        );
    }
    let caught_up = until(this, cx, 10_000, settled).await;
    let mut extra = vec![("frames_seen", seen.len() as f64)];
    extra.extend(caught_up.map(|took| ("caught_up", ms(took))));
    let problem = if caught_up.is_none() {
        Some("the monitor never caught up with the playhead".into())
    } else if seen.len() < 8 {
        Some(format!(
            "only {} frames showed during the scrub",
            seen.len()
        ))
    } else {
        None
    };
    outcome("video_scrub", extra, problem)
}

/// Plays three seconds from the start, across two cuts and the photo.
pub(super) async fn play(this: &This, cx: &mut AsyncWindowContext) -> Outcome {
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
    let (shown, fps) = read(this, cx, |ws| {
        let video = ws.video.as_ref()?;
        Some((
            video.monitor.shown.as_ref().map_or(0, |(_, i)| *i),
            video.doc.rate.fps(),
        ))
    })
    .flatten()
    .unwrap_or((0, 60.));
    let problem = ((shown as f64) < fps * 3. * 0.85)
        .then(|| format!("playing three seconds reached frame {shown}"));
    outcome("video_play", vec![], problem)
}

/// Plays through the speed ramp (slowing to a quarter, then backward), then
/// pauses in the slow part, where the paused frame is made between source
/// frames by following their motion.
pub(super) async fn speed(this: &This, cx: &mut AsyncWindowContext) -> Outcome {
    let rate = read(this, cx, |ws| ws.video.as_ref().map(|v| v.doc.rate)).flatten();
    let Some(rate) = rate else {
        return outcome("video_speed", vec![], Some("the video closed".into()));
    };
    let at = |s: f64| rate.time_of((s * rate.fps()).round() as i64);
    this.update(cx, |ws, cx| {
        ws.seek_player(at(6.), cx);
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
    let played = crate::perf::drain();
    until(this, cx, 10_000, settled).await;
    // A quarter speed lands between source frames.
    let started = Instant::now();
    this.update(cx, |ws, cx| {
        ws.seek_player(at(7.3), cx);
    })
    .ok();
    // Until the frame shows made by following the motion, after the quick
    // one.
    let slow = until(this, cx, 10_000, |ws| {
        ws.video
            .as_ref()
            .is_some_and(|v| v.monitor.settled() && v.monitor.shown_sharp)
    })
    .await;
    let mut result = outcome("video_speed", vec![], None);
    for (kind, values) in played {
        result.samples.entry(kind).or_default().extend(values);
    }
    match slow {
        Some(_) => result
            .samples
            .entry("slow_frame")
            .or_default()
            .push(ms(started.elapsed())),
        None => result.problem = Some("the slow-motion frame never showed".into()),
    }
    result
}

/// Where a time on a track is in the window.
pub(super) fn spot(ws: &Workspace, track: u64, time: Time) -> Option<Point<Pixels>> {
    let video = ws.video.as_ref()?;
    let bounds = *video.timeline.borrow();
    let row = video.layout().row(track)?;
    let (top, height) = (row.top, row.height);
    let x = HEADER + ((time.seconds() - video.scroll) as f32) * video.zoom;
    Some(bounds.origin + point(px(x), px(top + height / 2.)))
}

/// Presses on the timeline at `from`, moves to `to` over `steps` moves at
/// 60 a second, and lets go, through the timeline's own handlers.
pub(super) async fn drag(
    this: &This,
    cx: &mut AsyncWindowContext,
    from: Point<Pixels>,
    to: Point<Pixels>,
    steps: u32,
) {
    this.update_in(cx, |ws, window, cx| {
        let event = MouseDownEvent {
            button: MouseButton::Left,
            position: from,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        };
        ws.timeline_down(&event, window, cx);
    })
    .ok();
    for step in 1..=steps.max(1) {
        let share = step as f32 / steps.max(1) as f32;
        let position = from + (to - from) * share;
        this.update_in(cx, |ws, window, cx| {
            let event = MouseMoveEvent {
                position,
                pressed_button: Some(MouseButton::Left),
                modifiers: Modifiers::default(),
            };
            ws.timeline_move(&event, window, cx);
        })
        .ok();
        pause(cx, 16).await;
    }
    this.update_in(cx, |ws, window, cx| ws.timeline_release(window, cx))
        .ok();
}

/// The first clip on the first video track: its track, start, and end.
fn first_clip(video: &Video, index: usize) -> Option<(u64, u64, Time, Time)> {
    let track = &video.tracks[0];
    let clip = track.clips.get(index)?;
    Some((track.id, clip.id, clip.start, clip.end()))
}

/// Drags the end of the first clip back and forth, as a person trims.
pub(super) async fn trim(this: &This, cx: &mut AsyncWindowContext) -> Outcome {
    until(this, cx, 10_000, idle).await;
    crate::perf::drain();
    let Some(video) = doc(this, cx) else {
        return outcome("video_trim", vec![], Some("the video closed".into()));
    };
    let (track, _, _, end) = first_clip(&video, 0).unwrap_or_default();
    let positions = read(this, cx, |ws| {
        Some((spot(ws, track, end)?, spot(ws, track, Time(end.0 / 2))?))
    })
    .flatten();
    let Some((from, to)) = positions else {
        return outcome(
            "video_trim",
            vec![],
            Some("the clip is not on the timeline".into()),
        );
    };
    drag(this, cx, from, to, 60).await;
    let saved = until(this, cx, 20_000, idle).await;
    let shown = doc(this, cx);
    let problem = match (saved, shown) {
        (None, _) => Some("the trim was not saved".into()),
        (_, Some(shown)) if first_clip(&shown, 0).map(|c| c.3) == Some(end) => {
            Some("dragging the clip's end did not trim it".into())
        }
        _ => None,
    };
    // Back as it was, for the checks after.
    this.update_in(cx, |ws, window, _| ws.focus_handle.focus(window))
        .ok();
    key(cx, "secondary-z");
    until(this, cx, 20_000, idle).await;
    outcome("video_trim", vec![], problem)
}

/// Each tool and edit, used once, then checked: it changed the edit, was
/// saved, and undoes and redoes.
pub(super) async fn tools(this: &This, cx: &mut AsyncWindowContext, video_id: AssetId) -> Outcome {
    until(this, cx, 10_000, idle).await;
    crate::perf::drain();
    let mut problems = Vec::new();
    type Act = Box<dyn Fn(&Workspace) -> Option<(Point<Pixels>, Point<Pixels>)>>;
    let at = |index: usize, share: f64| -> Act {
        Box::new(move |ws: &Workspace| {
            let video = ws.video.as_ref()?;
            let (track, _, start, end) = first_clip(&video.doc, index)?;
            let time = Time(start.0 + ((end.0 - start.0) as f64 * share) as i64);
            let from = spot(ws, track, time)?;
            Some((from, from + point(px(40.), px(0.))))
        })
    };
    let edge = |index: usize, end_edge: bool, by: f32| -> Act {
        Box::new(move |ws: &Workspace| {
            let video = ws.video.as_ref()?;
            let (track, _, start, end) = first_clip(&video.doc, index)?;
            let from = spot(ws, track, if end_edge { end } else { start })?;
            Some((from, from + point(px(by), px(0.))))
        })
    };
    let checks: Vec<(&str, Tool, Act)> = vec![
        ("Select: move", Tool::Select, at(1, 0.5)),
        ("Select: trim", Tool::Select, edge(0, true, -30.)),
        ("Ripple edit", Tool::Ripple, edge(1, true, -30.)),
        ("Rolling edit", Tool::Roll, edge(0, true, 25.)),
        ("Slip", Tool::Slip, at(1, 0.5)),
        ("Slide", Tool::Slide, at(1, 0.5)),
        ("Razor", Tool::Razor, at(0, 0.5)),
    ];
    for (name, tool, act) in checks {
        let before = doc(this, cx)
            .unwrap_or_else(|| Video::new("", 2, 2, spectrum_media::FrameRate::new(30, 1)));
        this.update(cx, |ws, cx| ws.set_video_tool(tool, cx)).ok();
        let Some((from, to)) = read(this, cx, |ws| act(ws)).flatten() else {
            problems.push(format!("{name}: nothing to use it on"));
            continue;
        };
        let steps = if tool == Tool::Razor { 0 } else { 20 };
        drag(this, cx, from, to, steps).await;
        // Timed from the release, as a person feels it.
        let started = Instant::now();
        if let Err(problem) = check(this, cx, video_id, &before, started).await {
            problems.push(format!("{name}: {problem}"));
        }
        restore(this, cx, &before).await;
    }
    // The edits a key or a button makes.
    type Button = fn(&mut Workspace, &mut Window, &mut Context<Workspace>);
    let keyed: [(&str, &str, Option<Button>); 10] = [
        ("Cut at playhead", "s", None),
        ("Delete", "delete", None),
        ("Ripple delete", "shift-delete", None),
        ("Insert media", ",", None),
        ("Overwrite media", ".", None),
        ("Enable or disable", "shift-e", None),
        ("Link or unlink", "secondary-l", None),
        ("Freeze frame", "", Some(Workspace::freeze_frame)),
        ("Reverse", "", Some(Workspace::reverse_clip)),
        (
            "Keyframe added in the other view",
            "",
            Some(super::video_views::key_in_remap_view),
        ),
    ];
    for (name, keys, button) in keyed {
        let before = doc(this, cx)
            .unwrap_or_else(|| Video::new("", 2, 2, spectrum_media::FrameRate::new(30, 1)));
        this.update_in(cx, |ws, window, cx| {
            ws.set_video_tool(Tool::Select, cx);
            if let Some(video) = &mut ws.video {
                // The second clip, selected, with the playhead inside it.
                let clip = video.doc.tracks[0]
                    .clips
                    .get(1)
                    .map(|c| (c.id, c.start + Time(c.duration.0 / 2)));
                if let Some((id, middle)) = clip {
                    video.selected = video.doc.linked(id);
                    video.monitor.seek(middle);
                }
                // The footage, chosen in the Media sidebar for , and .
                video.media = video.doc.tracks[0].clips.first().map(|c| c.source.asset());
            }
            ws.focus_handle.focus(window);
        })
        .ok();
        let started = Instant::now();
        match button {
            Some(press) => {
                this.update_in(cx, press).ok();
            }
            None => key(cx, keys),
        }
        if let Err(problem) = check(this, cx, video_id, &before, started).await {
            problems.push(format!("{name}: {problem}"));
        }
        restore(this, cx, &before).await;
    }
    // Footage dragged from the sidebar onto the empty track above.
    let before = doc(this, cx)
        .unwrap_or_else(|| Video::new("", 2, 2, spectrum_media::FrameRate::new(30, 1)));
    let started = Instant::now();
    let dropped = this
        .update_in(cx, |ws, window, cx| {
            let video = ws.video.as_ref()?;
            let footage = video.doc.tracks[0].clips.first()?.source.asset();
            let track = video.doc.tracks.get(1)?;
            let at = track.clips.last().map_or(Time::ZERO, |c| c.end()) + Time::from_seconds(1.);
            let position = spot(ws, track.id, at)?;
            ws.drop_media(footage, position, window, cx);
            Some(())
        })
        .ok()
        .flatten();
    match dropped {
        None => problems.push("Drop media: no room to drop it".into()),
        Some(()) => {
            if let Err(problem) = check(this, cx, video_id, &before, started).await {
                problems.push(format!("Drop media: {problem}"));
            }
        }
    }
    restore(this, cx, &before).await;
    problems.extend(super::video_views::views(this, cx, video_id).await);
    let problem = (!problems.is_empty()).then(|| problems.join("; "));
    outcome("video_tools", vec![], problem)
}

/// Undoes a check's edit, so the next starts from the same video.
pub(super) async fn restore(this: &This, cx: &mut AsyncWindowContext, before: &Video) {
    this.update_in(cx, |ws, window, _| ws.focus_handle.focus(window))
        .ok();
    if doc(this, cx).as_ref().map(|v| &v.tracks) != Some(&before.tracks) {
        key(cx, "secondary-z");
        until(this, cx, 20_000, idle).await;
    }
}

/// Waits for an edit to show and save, then checks it changed the video,
/// matches what was saved, and undoes and redoes.
pub(super) async fn check(
    this: &This,
    cx: &mut AsyncWindowContext,
    video: AssetId,
    before: &Video,
    started: Instant,
) -> Result<(), String> {
    until(this, cx, 20_000, settled)
        .await
        .ok_or("was not shown within 20 s")?;
    crate::perf::record("feature_shown", started.elapsed());
    until(this, cx, 20_000, idle)
        .await
        .ok_or("was not saved within 20 s")?;
    crate::perf::record("feature_saved", started.elapsed());
    let after = doc(this, cx).ok_or("the video closed")?;
    if after.tracks == before.tracks {
        return Err("changed nothing".into());
    }
    let saved = read(this, cx, |ws| {
        ws.store
            .as_ref()
            .ok()?
            .service
            .video_view(video)
            .ok()
            .map(|(v, _)| v)
    })
    .flatten();
    if saved.as_ref() != Some(&after) {
        return Err("what was saved differs from what is shown".into());
    }
    key(cx, "secondary-z");
    until(this, cx, 20_000, idle).await;
    if doc(this, cx).as_ref().map(|v| &v.tracks) != Some(&before.tracks) {
        return Err("undo did not restore the edit".into());
    }
    key(cx, "secondary-shift-z");
    until(this, cx, 20_000, idle).await;
    if doc(this, cx).as_ref().map(|v| &v.tracks) != Some(&after.tracks) {
        return Err("redo did not bring the change back".into());
    }
    Ok(())
}
