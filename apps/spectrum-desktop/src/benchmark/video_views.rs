//! The timeline's view controls and speed's two views, each checked: a
//! video row grows from its top edge and an audio row from its bottom, the
//! divider and the grip resize the panes and the timeline, switching speed
//! views changes nothing, a double-click on the speed line in the other
//! view adds a keyframe that drags at once (saved, undone, redone), and
//! stopping returns to where play began.
use super::{
    scenarios::{This, pause, read, until},
    video::{check, doc, idle, restore},
};
use crate::{
    video_layout::{GRIP, HEADER, PANEL, TOP},
    workspace::Workspace,
};
use gpui::*;
use spectrum_library::AssetId;
use spectrum_media::Time;
use spectrum_video::{SpeedForm, TrackKind, Video};
use std::time::Instant;

/// Adds a speed keyframe at the playhead from the time remap view, which
/// keeps the clip's timing as remap keyframes first, in one edit.
pub(super) fn key_in_remap_view(
    ws: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    if let Some(video) = &mut ws.video {
        video.speed_view = SpeedForm::Remap;
    }
    ws.add_speed_key(window, cx);
    if let Some(video) = &mut ws.video {
        video.speed_view = SpeedForm::Velocity;
    }
}

/// The view checks, as problems found.
pub(super) async fn views(
    this: &This,
    cx: &mut AsyncWindowContext,
    video_id: AssetId,
) -> Vec<String> {
    let mut problems = Vec::new();
    let Some(before) = doc(this, cx) else {
        return vec!["Views: the video closed".into()];
    };
    // A track grows from its top edge, which follows the pointer.
    for kind in [TrackKind::Video, TrackKind::Audio] {
        let edge = read(this, cx, |ws| {
            let video = ws.video.as_ref()?;
            let origin = video.timeline.borrow().origin;
            let layout = video.layout();
            let row = layout
                .rows
                .iter()
                .copied()
                .find(|r| r.kind == kind && r.top > TOP)?;
            Some((
                row.track,
                row.height,
                origin + point(px(HEADER / 2.), px(row.top)),
            ))
        })
        .flatten();
        let Some((track, height, from)) = edge else {
            problems.push(format!("Row height: no {kind:?} row"));
            continue;
        };
        real_drag(this, cx, from, from - point(px(0.), px(40.)), 8).await;
        let grown = read(this, cx, |ws| {
            ws.video.as_ref()?.heights.get(&track).copied()
        })
        .flatten()
        .unwrap_or(height);
        if (grown - height - 40.).abs() > 1. {
            problems.push(format!(
                "Row height: the {kind:?} row went from {height} to {grown}, not 40 taller"
            ));
        }
    }
    // The grip: pulled far up with real pointer events, past where the
    // timeline was, it keeps following until the monitor is a sliver; a
    // double-click puts it back.
    let grip = read(this, cx, |ws| {
        let video = ws.video.as_ref()?;
        let bounds = *video.timeline.borrow();
        Some(bounds.origin + point(px(HEADER + 200.), px(GRIP / 2.)))
    })
    .flatten();
    if let Some(grip) = grip {
        real_drag(this, cx, grip, point(grip.x, px(0.)), 30).await;
        pause(cx, 100).await;
        let (panel, most) = read(this, cx, |ws| {
            let video = ws.video.as_ref()?;
            Some((video.panel, f32::from(video.timeline.borrow().size.height)))
        })
        .flatten()
        .unwrap_or((0., 0.));
        let tallest = this
            .update_in(cx, |_, window, _| crate::video_layout::panel_most(window))
            .unwrap_or(0.);
        if (panel - tallest).abs() > 2. || (most - tallest).abs() > 2. {
            problems.push(format!(
                "Grip: pulled to the top, the timeline is {panel} (drawn {most}), not {tallest}"
            ));
        }
        let top = read(this, cx, |ws| {
            Some(
                ws.video.as_ref()?.timeline.borrow().origin
                    + point(px(HEADER + 200.), px(GRIP / 2.)),
            )
        })
        .flatten()
        .unwrap_or(grip);
        click(this, cx, top, 2).await;
        pause(cx, 100).await;
        let panel = read(this, cx, |ws| Some(ws.video.as_ref()?.panel)).flatten();
        if panel != Some(PANEL) {
            problems.push(format!(
                "Grip: a double-click left it at {panel:?}, not {PANEL}"
            ));
        }
    }
    let same = doc(this, cx).is_some_and(|d| d.tracks == before.tracks);
    if !same {
        problems.push("Rows and grip: changed the edit".into());
    }
    this.update(cx, |ws, cx| {
        if let Some(video) = &mut ws.video {
            video.heights.clear();
            video.scroll_y = 0.;
            video.panel = PANEL;
        }
        cx.notify();
    })
    .ok();
    pause(cx, 100).await;
    // A track dragged by its name to the top: saved, undone, redone.
    let header = read(this, cx, |ws| {
        let video = ws.video.as_ref()?;
        let origin = video.timeline.borrow().origin;
        let row = video
            .layout()
            .rows
            .into_iter()
            .find(|r| r.kind == TrackKind::Audio)?;
        Some((
            origin + point(px(12.), px(row.top + row.height / 2.)),
            origin + point(px(12.), px(TOP + 2.)),
        ))
    })
    .flatten();
    match header {
        None => problems.push("Track order: no audio track".into()),
        Some((from, to)) => {
            real_drag(this, cx, from, to, 10).await;
            let started = Instant::now();
            match check(this, cx, video_id, &before, started).await {
                Err(problem) => problems.push(format!("Track order: {problem}")),
                Ok(()) => {
                    let top = doc(this, cx).and_then(|d| d.tracks.last().map(|t| t.kind));
                    if top != Some(TrackKind::Audio) {
                        problems.push("Track order: the audio track did not go to the top".into());
                    }
                }
            }
            restore(this, cx, &before).await;
        }
    }
    problems.extend(speed_views(this, cx, video_id, &before).await);
    problems.extend(return_to_start(this, cx).await);
    problems
}

/// The ramped clip, selected.
fn select_ramp(ws: &mut Workspace) -> Option<u64> {
    let video = ws.video.as_mut()?;
    let id = video.doc.track_named("V1")?.clips.last()?.id;
    video.selected = video.doc.linked(id);
    video.key = None;
    Some(id)
}

async fn speed_views(
    this: &This,
    cx: &mut AsyncWindowContext,
    video_id: AssetId,
    before: &Video,
) -> Vec<String> {
    let mut problems = Vec::new();
    // Switching views changes nothing.
    this.update_in(cx, |ws, window, cx| {
        select_ramp(ws);
        ws.set_speed_view(SpeedForm::Remap, window, cx);
    })
    .ok();
    pause(cx, 50).await;
    if doc(this, cx).is_none_or(|d| d.tracks != before.tracks) {
        problems.push("Speed views: switching changed the edit".into());
    }
    // A double-click on the remap line adds a keyframe and drags it.
    let spot = read(this, cx, |ws| {
        let video = ws.video.as_ref()?;
        let clip = ws.speed_clip()?;
        let track = video.doc.clip(clip.id)?.0.id;
        let row = video.layout().row(track)?;
        let origin = video.timeline.borrow().origin;
        let x = video.x_of(clip.start + Time(clip.duration.0 / 3));
        let boxed = (
            video.x_of(clip.start),
            row.top + 3.,
            video.x_of(clip.end()) - video.x_of(clip.start),
            row.height - 6.,
        );
        let line = ws.speed_geometry(&clip, boxed, None)?;
        let (_, y) = line
            .points
            .iter()
            .min_by(|a, b| (a.0 - x).abs().total_cmp(&(b.0 - x).abs()))?;
        Some(origin + point(px(x), px(*y)))
    })
    .flatten();
    let Some(at) = spot else {
        problems.push("Speed views: the ramp is not on the timeline".into());
        return problems;
    };
    this.update_in(cx, |ws, window, cx| {
        let event = MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 2,
            first_mouse: false,
        };
        ws.timeline_down(&event, window, cx);
        let event = MouseMoveEvent {
            position: at - point(px(0.), px(8.)),
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::default(),
        };
        ws.timeline_move(&event, window, cx);
        ws.timeline_release(window, cx);
    })
    .ok();
    let started = Instant::now();
    match check(this, cx, video_id, before, started).await {
        Err(problem) => problems.push(format!("Speed line double-click: {problem}")),
        Ok(()) => {
            let remap = doc(this, cx).and_then(|d| {
                let id = d.track_named("V1")?.clips.last()?.id;
                Some(matches!(
                    d.clip(id)?.1.speed,
                    spectrum_video::Speed::Remap { .. }
                ))
            });
            if remap != Some(true) {
                problems.push(
                    "Speed line double-click: the clip did not become remap keyframes".into(),
                );
            }
        }
    }
    restore(this, cx, before).await;
    this.update_in(cx, |ws, window, cx| {
        ws.set_speed_view(SpeedForm::Velocity, window, cx)
    })
    .ok();
    until(this, cx, 20_000, idle).await;
    problems
}

async fn return_to_start(this: &This, cx: &mut AsyncWindowContext) -> Vec<String> {
    let from = Time::from_seconds(1.);
    this.update(cx, |ws, cx| {
        if let Some(video) = &mut ws.video {
            video.return_to_start = true;
        }
        ws.seek_player(from, cx);
        ws.toggle_play(cx);
    })
    .ok();
    pause(cx, 400).await;
    this.update(cx, |ws, cx| {
        ws.toggle_play(cx);
    })
    .ok();
    let now = read(this, cx, |ws| Some(ws.video.as_ref()?.monitor.now())).flatten();
    match now {
        Some(now) if now == from => Vec::new(),
        other => vec![format!(
            "Return to start: stopped at {other:?}, not {from:?}"
        )],
    }
}

/// A preview render: the first second rendered ahead, then played from
/// memory without falling behind; an edit to one clip loses only its
/// frames, and undoing it finds them again.
pub(super) async fn preview(this: &This, cx: &mut AsyncWindowContext) -> super::Outcome {
    use super::scenarios::{key, ms, outcome};
    until(this, cx, 10_000, idle).await;
    let second = read(this, cx, |ws| {
        let rate = ws.video.as_ref()?.doc.rate;
        Some(rate.time_of(rate.frame_at(Time::from_seconds(1.))))
    })
    .flatten()
    .unwrap_or(Time::from_seconds(1.));
    this.update(cx, |ws, cx| {
        ws.seek_player(Time::ZERO, cx);
        ws.set_in_point(cx);
        ws.seek_player(second, cx);
        ws.set_out_point(cx);
        ws.seek_player(Time::ZERO, cx);
        ws.render_video_preview(cx);
    })
    .ok();
    let started = Instant::now();
    let rendered = until(this, cx, 120_000, |ws| {
        ws.video
            .as_ref()
            .is_some_and(|v| v.preview.progress().is_none() && v.preview.len() > 0)
    })
    .await;
    let mut samples = vec![];
    let Some(_) = rendered else {
        return outcome(
            "video_preview",
            vec![],
            Some("the preview never rendered".into()),
        );
    };
    samples.push(("preview_render", ms(started.elapsed())));
    // Played from memory.
    until(this, cx, 10_000, |ws| {
        ws.video.as_ref().is_some_and(|v| v.monitor.settled())
    })
    .await;
    crate::perf::drain();
    this.update(cx, |ws, cx| {
        ws.toggle_play(cx);
    })
    .ok();
    pause(cx, 900).await;
    this.update(cx, |ws, cx| {
        ws.toggle_play(cx);
    })
    .ok();
    let played = crate::perf::drain();
    let hits = played.get("preview_frame").map_or(0, Vec::len);
    let mut result = outcome("video_preview", samples, None);
    for (kind, values) in played {
        result.samples.entry(kind).or_default().extend(values);
    }
    if hits < 20 {
        result.problem = Some(format!("only {hits} frames played from the render"));
    }
    // An edit to the first clip loses its frames; undo finds them.
    let probe = |ws: &Workspace| -> Option<bool> {
        let video = ws.video.as_ref()?;
        let size = crate::video_program::fitted(&video.doc, video.monitor.size);
        let key = crate::video_preview::frame_key(&video.doc, size, 10);
        Some(video.preview.has(key))
    };
    this.update_in(cx, |ws, window, cx| {
        let Some(clip) = ws
            .video
            .as_ref()
            .and_then(|v| v.doc.track_named("V1")?.clips.first().map(|c| c.id))
        else {
            return;
        };
        let command = spectrum_video::Command::SetClip {
            clip,
            name: None,
            enabled: None,
            opacity: Some(0.5),
            gain_db: None,
            interpolation: None,
            sound_follows_speed: None,
        };
        ws.video_commands(vec![command], window, cx);
        ws.focus_handle.focus(window);
    })
    .ok();
    until(this, cx, 20_000, idle).await;
    let changed = read(this, cx, probe).flatten();
    key(cx, "secondary-z");
    until(this, cx, 20_000, idle).await;
    let undone = read(this, cx, probe).flatten();
    if changed != Some(false) || undone != Some(true) {
        result.problem = Some(format!(
            "an edit should lose its frames and undo find them: after the edit {changed:?}, after undo {undone:?}"
        ));
    }
    this.update(cx, |ws, cx| ws.clear_in_out(cx)).ok();
    result
}

/// A press, moves, and a release through the timeline's handlers, the
/// moves reaching wherever they go (the window follows a drag anywhere).
async fn real_drag(
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
    pause(cx, 16).await;
    for step in 1..=steps.max(1) {
        let share = step as f32 / steps.max(1) as f32;
        this.update_in(cx, |ws, window, cx| {
            let event = MouseMoveEvent {
                position: from + (to - from) * share,
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
    pause(cx, 16).await;
}

/// A click (or double-click) on the timeline.
async fn click(this: &This, cx: &mut AsyncWindowContext, at: Point<Pixels>, count: usize) {
    for click_count in 1..=count {
        this.update_in(cx, |ws, window, cx| {
            let event = MouseDownEvent {
                button: MouseButton::Left,
                position: at,
                modifiers: Modifiers::default(),
                click_count,
                first_mouse: false,
            };
            ws.timeline_down(&event, window, cx);
            ws.timeline_release(window, cx);
        })
        .ok();
        pause(cx, 16).await;
    }
}
