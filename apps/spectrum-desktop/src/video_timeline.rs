//! The timeline: a ruler, video tracks (the top one in front) over audio
//! tracks, their clips, and the playhead. Pressing a clip selects it and
//! starts the current tool's drag (move or trim with Select, ripple trim,
//! roll, slip, slide); the Razor cuts where clicked. Edges and moves snap
//! to the playhead and other clips' edges.
use crate::{
    theme::*,
    video_state::{Drag, DragKind, Tool},
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use spectrum_media::Time;
use spectrum_video::{Command, Edge, TrackKind, Video};
use std::time::Instant;

pub const HEADER: f32 = 76.;
pub const RULER: f32 = 26.;
const VIDEO_ROW: f32 = 54.;
const AUDIO_ROW: f32 = 42.;
/// How near an edge, in points, counts as grabbing it.
const EDGE: f32 = 6.;
/// How near, in points, an edge snaps.
const SNAP: f32 = 8.;

/// The rows from the top: video tracks (highest first), then audio.
pub fn rows(video: &Video) -> Vec<(u64, TrackKind, f32, f32)> {
    let mut rows = Vec::new();
    let mut y = RULER;
    let tracks = video
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .rev()
        .chain(video.tracks.iter().filter(|t| t.kind == TrackKind::Audio));
    for track in tracks {
        let height = if track.kind == TrackKind::Video {
            VIDEO_ROW
        } else {
            AUDIO_ROW
        };
        rows.push((track.id, track.kind, y, height));
        y += height;
    }
    rows
}

/// What is under a point on the timeline.
enum Hit {
    Ruler,
    Clip { clip: u64, edge: Option<Edge> },
    Empty,
}

impl Workspace {
    pub(crate) fn timeline_x(&self, time: Time) -> f32 {
        let video = self.video.as_ref().expect("video");
        HEADER + ((time.seconds() - video.scroll) as f32) * video.zoom
    }

    /// The timeline time under a window point.
    fn timeline_time(&self, position: Point<Pixels>) -> Time {
        let video = self.video.as_ref().expect("video");
        let bounds = *video.timeline.borrow();
        let x = f32::from(position.x - bounds.origin.x) - HEADER;
        Time::from_seconds((f64::from(x / video.zoom) + video.scroll).max(0.))
    }

    fn hit(&self, position: Point<Pixels>) -> Hit {
        let video = self.video.as_ref().expect("video");
        let bounds = *video.timeline.borrow();
        let (x, y) = (
            f32::from(position.x - bounds.origin.x),
            f32::from(position.y - bounds.origin.y),
        );
        if y < RULER {
            return Hit::Ruler;
        }
        let Some((track, ..)) = rows(&video.doc)
            .into_iter()
            .find(|(_, _, top, height)| y >= *top && y < top + height)
        else {
            return Hit::Empty;
        };
        let Some(track) = video.doc.track(track) else {
            return Hit::Empty;
        };
        for clip in &track.clips {
            let (left, right) = (self.timeline_x(clip.start), self.timeline_x(clip.end()));
            if x >= left - EDGE && x <= right + EDGE {
                let edge = if (x - left).abs() <= EDGE {
                    Some(Edge::Start)
                } else if (x - right).abs() <= EDGE {
                    Some(Edge::End)
                } else if x < left || x > right {
                    continue;
                } else {
                    None
                };
                return Hit::Clip {
                    clip: clip.id,
                    edge,
                };
            }
        }
        Hit::Empty
    }

    pub fn timeline_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.speed_down(event, window, cx) {
            return;
        }
        let hit = self.hit(event.position);
        let time = self.timeline_time(event.position);
        let shift = event.modifiers.shift;
        let Some(video) = &mut self.video else {
            return;
        };
        let kind = match hit {
            Hit::Ruler | Hit::Empty => {
                if matches!(hit, Hit::Empty) && !shift {
                    video.selected.clear();
                }
                DragKind::Playhead
            }
            Hit::Clip { clip, edge } => {
                if video.tool == Tool::Razor {
                    let at = video.doc.snap(time);
                    self.video_commands(
                        vec![Command::Split {
                            at,
                            clips: vec![clip],
                        }],
                        window,
                        cx,
                    );
                    return;
                }
                let linked = video.doc.linked(clip);
                if shift {
                    if video.selected.contains(&clip) {
                        video.selected.retain(|id| !linked.contains(id));
                    } else {
                        video.selected.extend(linked);
                    }
                } else if !video.selected.contains(&clip) {
                    video.selected = linked;
                }
                match (video.tool, edge) {
                    (Tool::Select, Some(edge)) => DragKind::Trim {
                        clip,
                        edge,
                        ripple: false,
                    },
                    (Tool::Ripple, Some(edge)) => DragKind::Trim {
                        clip,
                        edge,
                        ripple: true,
                    },
                    (Tool::Roll, Some(Edge::End)) => DragKind::Roll { clip },
                    (Tool::Roll, Some(Edge::Start)) => {
                        // The cut before this clip belongs to the clip before.
                        let (track, c) = video.doc.clip(clip).expect("clip");
                        match track.clips.iter().find(|o| o.end() == c.start) {
                            Some(before) => DragKind::Roll { clip: before.id },
                            None => return,
                        }
                    }
                    (Tool::Slip, _) => DragKind::Slip { clip },
                    (Tool::Slide, _) => DragKind::Slide { clip },
                    _ => DragKind::Move,
                }
            }
        };
        let row = f32::from(event.position.y);
        video.drag = Some(Drag {
            base: video.doc.clone(),
            kind,
            from: (time.seconds(), row),
            command: None,
            started: Instant::now(),
        });
        if kind == DragKind::Playhead {
            self.seek_player(time, cx);
        }
        self.sync_video_controls(window, cx);
        cx.notify();
    }

    pub fn timeline_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.video.as_ref().and_then(|v| v.drag.as_ref()) else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            return self.timeline_release(window, cx);
        }
        let time = self.timeline_time(event.position);
        let (kind, from) = (drag.kind, drag.from);
        if kind == DragKind::Playhead {
            self.seek_player(time, cx);
            return;
        }
        let by = self.snapped_by(Time::from_seconds(time.seconds() - from.0), kind);
        // Rows dragged up move pictures up a track.
        let rows = ((from.1 - f32::from(event.position.y)) / VIDEO_ROW).round() as i32;
        let Some(selected) = self.video.as_ref().map(|v| v.selected.clone()) else {
            return;
        };
        let command = match kind {
            DragKind::Move => Command::Move {
                clips: selected,
                by,
                tracks: rows,
            },
            DragKind::Trim { clip, edge, ripple } => Command::Trim {
                clip,
                edge,
                by,
                ripple,
            },
            DragKind::Roll { clip } => Command::Roll { clip, by },
            // Dragging right moves the picture right: an earlier in point.
            DragKind::Slip { clip } => Command::Slip { clip, by: -by },
            DragKind::Slide { clip } => Command::Slide { clip, by },
            DragKind::Key { clip, index } => {
                let Some(command) = self.key_drag(clip, index, time, event.position) else {
                    return;
                };
                command
            }
            DragKind::Playhead => return,
        };
        let Some(video) = &mut self.video else {
            return;
        };
        let drag = video.drag.as_mut().expect("drag");
        let mut doc = drag.base.clone();
        if spectrum_video::apply(&mut doc, command.clone()).is_ok() {
            drag.command = Some(command);
            self.show_video_doc(doc);
        }
        cx.notify();
    }

    /// Lets go of a drag, saving the edit it made.
    pub fn timeline_release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(video) = &mut self.video else {
            return;
        };
        let Some(drag) = video.drag.take() else {
            return;
        };
        crate::perf::record("video_drag", drag.started.elapsed());
        if let Some(command) = drag.command {
            // The edit applies again, to the edit from before the drag.
            video.doc = drag.base;
            self.video_commands(vec![command], window, cx);
        }
        cx.notify();
    }

    /// `by`, moved so the edges it moves land on the playhead or another
    /// clip's edge when they come near.
    fn snapped_by(&self, by: Time, kind: DragKind) -> Time {
        let Some(video) = &self.video else {
            return by;
        };
        let Some(drag) = &video.drag else {
            return by;
        };
        let doc = &drag.base;
        let by = doc.snap(by);
        if !video.snapping {
            return by;
        }
        let moving: Vec<u64> = match kind {
            DragKind::Move => video.selected.clone(),
            DragKind::Trim { clip, .. }
            | DragKind::Roll { clip }
            | DragKind::Slip { clip }
            | DragKind::Slide { clip } => doc.linked(clip),
            DragKind::Playhead | DragKind::Key { .. } => return by,
        };
        let edges: Vec<Time> = moving
            .iter()
            .filter_map(|id| doc.clip(*id).map(|(_, c)| c))
            .flat_map(|c| match kind {
                DragKind::Trim {
                    edge: Edge::Start, ..
                } => vec![c.start],
                DragKind::Trim {
                    edge: Edge::End, ..
                }
                | DragKind::Roll { .. } => vec![c.end()],
                DragKind::Slip { .. } => vec![],
                _ => vec![c.start, c.end()],
            })
            .collect();
        let mut targets = vec![Time::ZERO, video.monitor.now()];
        targets.extend(
            doc.tracks
                .iter()
                .flat_map(|t| t.clips.iter())
                .filter(|c| !moving.contains(&c.id))
                .flat_map(|c| [c.start, c.end()]),
        );
        let reach = f64::from(SNAP / video.zoom);
        let mut best: Option<(f64, Time)> = None;
        for edge in &edges {
            for target in &targets {
                let gap = (*target - (*edge + by)).seconds();
                if gap.abs() <= reach && best.is_none_or(|(b, _)| gap.abs() < b.abs()) {
                    best = Some((gap, *target - *edge));
                }
            }
        }
        best.map_or(by, |(_, snapped)| snapped)
    }

    /// Scrolls the timeline sideways, or zooms it around the pointer with
    /// the Command or Alt key held.
    pub fn timeline_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let time = self.timeline_time(event.position);
        let delta = event.delta.pixel_delta(window.line_height());
        let Some(video) = &mut self.video else {
            return;
        };
        if event.modifiers.platform || event.modifiers.alt || event.modifiers.control {
            let factor = (1. + f32::from(delta.y) / 200.).clamp(0.5, 2.);
            let x = (time.seconds() - video.scroll) as f32 * video.zoom;
            video.zoom = (video.zoom * factor).clamp(2., 2000.);
            video.scroll = (time.seconds() - f64::from(x / video.zoom)).max(0.);
        } else {
            let sideways = if f32::from(delta.x).abs() > f32::from(delta.y).abs() {
                f32::from(delta.x)
            } else {
                f32::from(delta.y)
            };
            video.scroll = (video.scroll - f64::from(sideways / video.zoom)).max(0.);
        }
        cx.notify();
    }

    /// Presses on a speed line: a keyframe starts its drag; a double-click
    /// on the line adds one there. Returns whether the line took the press.
    fn speed_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(clip) = self.speed_clip().filter(|c| !c.speed.is_normal()) else {
            return false;
        };
        let Some(video) = &self.video else {
            return false;
        };
        let bounds = *video.timeline.borrow();
        let (x, y) = (
            f32::from(event.position.x - bounds.origin.x),
            f32::from(event.position.y - bounds.origin.y),
        );
        let Some((_, _, top, row)) = rows(&video.doc).into_iter().find(|(id, ..)| {
            video
                .doc
                .track(*id)
                .is_some_and(|t| t.clips.iter().any(|c| c.id == clip.id))
        }) else {
            return false;
        };
        let (left, right) = (self.timeline_x(clip.start), self.timeline_x(clip.end()));
        let boxed = (left, top + 3., right - left, row - 6.);
        if let Some(index) = self.speed_key_at(&clip, boxed, x, y) {
            let time = self.timeline_time(event.position);
            let Some(video) = &mut self.video else {
                return false;
            };
            video.key = Some((clip.id, index));
            video.drag = Some(Drag {
                base: video.doc.clone(),
                kind: DragKind::Key {
                    clip: clip.id,
                    index,
                },
                from: (time.seconds(), f32::from(event.position.y)),
                command: None,
                started: Instant::now(),
            });
            self.sync_key_fields(window, cx);
            cx.notify();
            return true;
        }
        if event.click_count == 2 && x >= left && x <= right && y >= top && y <= top + row {
            let time = self.timeline_time(event.position);
            self.add_speed_key_at(time, window, cx);
            return true;
        }
        false
    }

    /// The speed command a keyframe drag to `position` makes.
    fn key_drag(
        &self,
        clip: u64,
        index: usize,
        time: Time,
        position: Point<Pixels>,
    ) -> Option<Command> {
        let video = self.video.as_ref()?;
        let drag = video.drag.as_ref()?;
        let base = drag.base.clip(clip)?.1.clone();
        let bounds = *video.timeline.borrow();
        let (_, _, top, row) = rows(&drag.base).into_iter().find(|(id, ..)| {
            drag.base
                .track(*id)
                .is_some_and(|t| t.clips.iter().any(|c| c.id == clip))
        })?;
        let y = f32::from(position.y - bounds.origin.y);
        let speed = self.dragged_speed(&base, index, drag.base.snap(time), y, (top + 3., row - 6.));
        Some(Command::SetSpeed { clip, speed })
    }

    /// Moves the playhead to the next (or previous) cut on any track.
    pub fn jump_to_cut(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(video) = &self.video else {
            return;
        };
        let now = video.monitor.now();
        let cuts = video
            .doc
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .flat_map(|c| [c.start, c.end()]);
        let target = if forward {
            cuts.filter(|t| *t > now).min()
        } else {
            cuts.filter(|t| *t < now).max()
        };
        if let Some(target) = target {
            self.seek_player(target, cx);
        }
    }

    /// Zooms the timeline in (positive) or out by steps, or to fit (0).
    pub fn zoom_timeline(&mut self, steps: i32, cx: &mut Context<Self>) {
        let Some(video) = &mut self.video else {
            return;
        };
        let width = f32::from(video.timeline.borrow().size.width) - HEADER;
        if steps == 0 {
            let seconds = video.doc.duration().seconds().max(1.) as f32;
            video.zoom = (width.max(200.) / seconds * 0.95).clamp(2., 2000.);
            video.scroll = 0.;
        } else {
            let playhead = video.monitor.now().seconds();
            let x = (playhead - video.scroll) as f32 * video.zoom;
            video.zoom = (video.zoom * 1.5f32.powi(steps)).clamp(2., 2000.);
            video.scroll = (playhead - f64::from(x / video.zoom)).max(0.);
        }
        cx.notify();
    }

    pub fn timeline(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(video) = &self.video else {
            return div().into_any_element();
        };
        let doc = &video.doc;
        let rows = rows(doc);
        let height = rows.last().map_or(RULER, |(_, _, y, h)| y + h) + 24.;
        let bounds = video.timeline.clone();
        let width = f32::from(bounds.borrow().size.width).max(400.);
        let visible = (
            video.scroll,
            video.scroll + f64::from((width - HEADER) / video.zoom),
        );
        let mut layer = div()
            .id("timeline")
            .relative()
            .w_full()
            .h(px(height))
            .overflow_hidden()
            .bg(rgb(BACKGROUND))
            .child(
                canvas(move |b, _, _| *bounds.borrow_mut() = b, |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(self.ruler(width, visible));
        let mut lines = Vec::new();
        for (track_id, kind, top, row) in &rows {
            let Some(track) = doc.track(*track_id) else {
                continue;
            };
            layer = layer.child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(*top))
                    .w_full()
                    .h(px(*row))
                    .border_b_1()
                    .border_color(rgb(BORDER)),
            );
            for clip in track
                .clips
                .iter()
                .filter(|c| c.end().seconds() >= visible.0 && c.start.seconds() <= visible.1)
            {
                let left = self.timeline_x(clip.start).max(HEADER);
                let right = self.timeline_x(clip.end());
                let selected = video.selected.contains(&clip.id);
                let fill = match (kind, clip.enabled) {
                    (_, false) => 0x262626,
                    (TrackKind::Video, _) => 0x36404f,
                    (TrackKind::Audio, _) => 0x34473d,
                };
                layer = layer.child(
                    div()
                        .absolute()
                        .left(px(left))
                        .top(px(top + 3.))
                        .w(px((right - left).max(2.)))
                        .h(px(row - 6.))
                        .rounded(px(4.))
                        .bg(rgb(fill))
                        .border_1()
                        .border_color(rgb(if selected { 0xececec } else { 0x4a4a4a }))
                        .px_1p5()
                        .py_1()
                        .overflow_hidden()
                        .text_xs()
                        .text_color(rgb(TEXT))
                        .child(div().truncate().child(clip.name.clone())),
                );
                let key = video.key.filter(|(id, _)| *id == clip.id).map(|(_, i)| i);
                if let Some(line) =
                    self.speed_line(clip, (left, top + 3., right - left, row - 6.), key)
                {
                    lines.push(line);
                }
            }
            let name = track.name.clone();
            let (muted, locked) = (track.muted, track.locked);
            let id = *track_id;
            layer = layer.child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(*top))
                    .w(px(HEADER))
                    .h(px(*row))
                    .bg(rgb(SIDEBAR))
                    .border_r_1()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .text_xs()
                    .child(div().w(px(22.)).text_color(rgb(MUTED)).child(name))
                    .child(toggle(
                        ("mute", id),
                        match (kind, muted) {
                            (TrackKind::Video, false) => {
                                gpui_component::Icon::new(gpui_component::IconName::Eye)
                            }
                            (TrackKind::Video, true) => {
                                gpui_component::Icon::new(gpui_component::IconName::EyeOff)
                            }
                            (_, false) => crate::icons::icon("volume"),
                            (_, true) => crate::icons::icon("volume-off"),
                        },
                        !muted,
                        cx,
                        move |this, window, cx| {
                            this.video_commands(
                                vec![Command::SetTrack {
                                    track: id,
                                    name: None,
                                    muted: Some(!muted),
                                    locked: None,
                                }],
                                window,
                                cx,
                            );
                        },
                    ))
                    .child(toggle(
                        ("lock", id),
                        crate::icons::icon(if locked { "lock" } else { "lock-open" }),
                        locked,
                        cx,
                        move |this, window, cx| {
                            this.video_commands(
                                vec![Command::SetTrack {
                                    track: id,
                                    name: None,
                                    muted: None,
                                    locked: Some(!locked),
                                }],
                                window,
                                cx,
                            );
                        },
                    )),
            );
        }
        layer = layer.children(lines);
        let playhead = self.timeline_x(video.monitor.now());
        if playhead >= HEADER {
            layer = layer.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(playhead))
                    .w(px(1.5))
                    .bg(rgb(0xff5a5a)),
            );
        }
        layer
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.timeline_down(event, window, cx)
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.timeline_move(event, window, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.timeline_release(window, cx)),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                this.timeline_wheel(event, window, cx)
            }))
            .into_any_element()
    }

    fn ruler(&self, width: f32, (from, to): (f64, f64)) -> Div {
        let video = self.video.as_ref().expect("video");
        // A label every 80 points or more, on a round number of seconds.
        let step = [
            0.1, 0.25, 0.5, 1., 2., 5., 10., 15., 30., 60., 120., 300., 600.,
        ]
        .into_iter()
        .find(|step| step * f64::from(video.zoom) >= 80.)
        .unwrap_or(1200.);
        let mut ruler = div()
            .absolute()
            .left_0()
            .top_0()
            .w(px(width))
            .h(px(RULER))
            .bg(rgb(SIDEBAR))
            .border_b_1()
            .border_color(rgb(BORDER));
        // Ticks fall on whole timecode frames, so 59.94 fps labels read
        // 00:00:01:00 rather than 00:00:00:59.
        let rate = video.doc.rate;
        let per = ((step * f64::from(rate.nominal())).round() as i64).max(1);
        let mut frame = (rate.frame_at(Time::from_seconds(from)) / per) * per;
        while rate.time_of(frame).seconds() <= to {
            let time = rate.time_of(frame);
            let x = self.timeline_x(time);
            if x >= HEADER {
                ruler = ruler.child(
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(4.))
                        .pl_1()
                        .border_l_1()
                        .border_color(rgb(FAINT))
                        .text_xs()
                        .text_color(rgb(FAINT))
                        .child(time.timecode(rate)),
                );
            }
            frame += per;
        }
        ruler
    }
}

fn toggle(
    id: (&'static str, u64),
    icon: gpui_component::Icon,
    on: bool,
    cx: &mut Context<Workspace>,
    act: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("{}-{}", id.0, id.1)))
        .size(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .text_color(rgb(if on { TEXT } else { FAINT }))
        .hover(|el| el.bg(rgb(HOVER)))
        .child(icon.size(px(12.)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, window, cx| act(this, window, cx)))
}
