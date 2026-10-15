//! The timeline's input. Pressing a clip selects it and starts the current
//! tool's drag (move or trim with Select, ripple trim, roll, slip, slide);
//! the Razor cuts where clicked. Edges and moves snap to the playhead and
//! other clips' edges. The grip above the ruler sets how tall the timeline
//! is (a double-click resets it), a track's top edge its height, and
//! dragging a track's name moves the track. A swipe up or down zooms around
//! the pointer, as the wheel does in Vegas; sideways (or Shift) scrolls
//! time; over the track names, or with Option, it scrolls the tracks. Drags
//! follow the pointer anywhere in the window.
use crate::{
    video_layout::{HEADER, Hit, PANEL, PANEL_MIN, ROW_RANGE, panel_most},
    video_state::{Drag, DragKind, MenuTarget, Tool},
    workspace::Workspace,
};
use gpui::*;
use spectrum_media::Time;
use spectrum_video::{Command, Edge};

/// How near, in points, an edge snaps.
const SNAP: f32 = 8.;

impl Workspace {
    pub(crate) fn timeline_x(&self, time: Time) -> f32 {
        self.video.as_ref().expect("video").x_of(time)
    }

    /// The timeline time under a window point.
    fn timeline_time(&self, position: Point<Pixels>) -> Time {
        let video = self.video.as_ref().expect("video");
        video.time_at(video.local(position).0)
    }

    pub fn timeline_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.speed_down(event, cx) {
            self.sync_key_fields(window, cx);
            return;
        }
        let Some(video) = &self.video else {
            return;
        };
        let hit = video.hit(event.position);
        let time = self.timeline_time(event.position);
        let shift = event.modifiers.shift;
        let layout = video.layout();
        let Some(video) = &mut self.video else {
            return;
        };
        let mut clicked = None;
        let kind = match hit {
            Hit::Header { track } => DragKind::TrackOrder { track },
            Hit::Grip if event.click_count == 2 => {
                video.panel = PANEL;
                if !crate::benchmark::running() {
                    crate::prefs::set("timeline_height", PANEL.into());
                }
                cx.notify();
                return;
            }
            Hit::Grip => DragKind::Panel {
                height: layout.height,
            },
            Hit::RowEdge { track, height } => DragKind::Resize {
                track,
                height,
                scroll: video.scroll_y.min(layout.room),
            },
            Hit::Ruler | Hit::Empty { .. } => {
                if matches!(hit, Hit::Empty { .. }) && !shift {
                    video.selected.clear();
                    video.key = None;
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
                } else {
                    // One of a larger selection: it alone once no drag
                    // follows.
                    clicked = Some(clip);
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
        let mut drag = Drag::new(
            video.doc.clone(),
            kind,
            (time.seconds(), f32::from(event.position.y)),
        );
        drag.clicked = clicked;
        video.drag = Some(drag);
        if kind == DragKind::Playhead {
            self.seek_player(time, cx);
        }
        self.sync_video_controls(window, cx);
        cx.notify();
    }

    /// Starts resizing the timeline from the transport's top edge, as the
    /// grip below it does.
    pub fn start_panel_drag(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(video) = &mut self.video else {
            return;
        };
        if event.click_count == 2 {
            video.panel = PANEL;
            if !crate::benchmark::running() {
                crate::prefs::set("timeline_height", PANEL.into());
            }
        } else {
            let height = video.layout().height;
            video.drag = Some(Drag::new(
                video.doc.clone(),
                DragKind::Panel { height },
                (0., f32::from(event.position.y)),
            ));
        }
        cx.notify();
    }

    /// A right-click: notes what it was on for the menu, selecting the clip
    /// (or keyframe) if it was not.
    pub fn timeline_right_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(video) = &self.video else {
            return;
        };
        let hit = video.hit(event.position);
        let (x, y) = video.local(event.position);
        let key = self.speed_clip().and_then(|clip| {
            let track = video.doc.clip(clip.id)?.0.id;
            let row = video.layout().row(track)?;
            let (left, right) = (video.x_of(clip.start), video.x_of(clip.end()));
            let boxed = (left, row.top + 3., right - left, row.height - 6.);
            Some((clip.id, self.speed_key_at(&clip, boxed, x, y)?))
        });
        let Some(video) = &mut self.video else {
            return;
        };
        video.menu = match (key, hit) {
            (Some((clip, index)), _) => {
                video.key = Some((clip, index));
                MenuTarget::Key { clip, index }
            }
            (None, Hit::Clip { clip, .. }) => {
                if !video.selected.contains(&clip) {
                    video.selected = video.doc.linked(clip);
                }
                MenuTarget::Clip(clip)
            }
            (None, Hit::Header { track }) => MenuTarget::Track(track),
            (None, Hit::Empty { track: Some(track) }) => MenuTarget::Track(track),
            _ => MenuTarget::Timeline,
        };
        self.sync_key_fields(window, cx);
        self.sync_video_controls(window, cx);
        cx.notify();
    }

    pub fn timeline_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(video) = &mut self.video else {
            return;
        };
        let dragging = video.drag.is_some();
        if dragging || video.tool == Tool::Razor {
            // The razor marks where it would cut; a dragged keyframe shows
            // its value by the pointer.
            video.hover = Some(event.position);
        }
        let Some(drag) = video.drag.as_ref() else {
            if video.tool == Tool::Razor {
                cx.notify();
            }
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            return self.timeline_release(window, cx);
        }
        let (kind, from) = (drag.kind, drag.from);
        let y = f32::from(event.position.y);
        match kind {
            DragKind::Playhead => {
                let time = self.timeline_time(event.position);
                self.seek_player(time, cx);
                return;
            }
            DragKind::Panel { height } => {
                video.panel = (height + from.1 - y).clamp(PANEL_MIN, panel_most(window));
                cx.notify();
                return;
            }
            DragKind::Resize {
                track,
                height,
                scroll,
            } => {
                // The top edge follows the pointer: the row grows upward,
                // the tracks above scrolling up to make room.
                let grown = (height + from.1 - y).clamp(ROW_RANGE.0, ROW_RANGE.1);
                video.heights.insert(track, grown);
                video.scroll_y = (scroll + grown - height).max(0.);
                cx.notify();
                return;
            }
            DragKind::TrackOrder { .. } => {
                video.track_drop = Some(video.local(event.position).1);
                cx.notify();
                return;
            }
            _ => {}
        }
        let time = self.timeline_time(event.position);
        let by = self.snapped_by(Time::from_seconds(time.seconds() - from.0), kind);
        let rows = self.track_shift(event.position);
        let Some(video) = &self.video else {
            return;
        };
        let selected = video.selected.clone();
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
                let Some(command) = self.key_drag(clip, index, event.position) else {
                    return;
                };
                command
            }
            _ => return,
        };
        let Some(video) = &mut self.video else {
            return;
        };
        let drag = video.drag.as_mut().expect("drag");
        let mut doc = drag.base.clone();
        let applied = drag
            .prefix
            .iter()
            .chain([&command])
            .all(|c| spectrum_video::apply(&mut doc, c.clone()).is_ok());
        if applied {
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
        match drag.kind {
            DragKind::Panel { .. } if !crate::benchmark::running() => {
                crate::prefs::set("timeline_height", video.panel.into());
            }
            DragKind::TrackOrder { track } => {
                let at = video.track_drop.take();
                let from = video.doc.tracks.iter().position(|t| t.id == track);
                let to = at.map(|y| video.layout().place_at(y));
                if let (Some(from), Some(to)) = (from, to)
                    && from != to
                {
                    let command = Command::MoveTrack { track, to };
                    self.video_commands(vec![command], window, cx);
                }
                cx.notify();
                return;
            }
            _ => {}
        }
        if drag.command.is_some() {
            // The edit applies again, to the edit from before the drag.
            let commands = drag.commands();
            video.doc = drag.base;
            self.video_commands(commands, window, cx);
        } else if let Some(clip) = drag.clicked {
            video.selected = video.doc.linked(clip);
            self.sync_video_controls(window, cx);
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
            _ => return by,
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

    pub fn timeline_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let time = self.timeline_time(event.position);
        let delta = event.delta.pixel_delta(window.line_height());
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        let Some(video) = &mut self.video else {
            return;
        };
        let (x, _) = video.local(event.position);
        let layout = video.layout();
        let tracks = x < HEADER || event.modifiers.alt;
        if tracks && dy.abs() >= dx.abs() {
            // Up and down through the tracks.
            video.scroll_y = (video.scroll_y.min(layout.room) - dy).clamp(0., layout.room);
        } else if dx.abs() > dy.abs() || event.modifiers.shift {
            let sideways = if dx.abs() > dy.abs() { dx } else { dy };
            video.scroll = (video.scroll - f64::from(sideways / video.zoom)).max(0.);
        } else {
            // Up zooms in around the pointer.
            let factor = (1. + dy / 200.).clamp(0.5, 2.);
            let at = (time.seconds() - video.scroll) as f32 * video.zoom;
            video.zoom = (video.zoom * factor).clamp(2., 2000.);
            video.scroll = (time.seconds() - f64::from(at / video.zoom)).max(0.);
        }
        cx.notify();
    }

    /// How many tracks of its kind a moved selection goes: to the track of
    /// that kind nearest the pointer, counting up. Pictures lead; sound on
    /// its own moves among sound tracks.
    fn track_shift(&self, position: Point<Pixels>) -> i32 {
        let Some(video) = &self.video else {
            return 0;
        };
        let Some(drag) = &video.drag else {
            return 0;
        };
        let tracks: Vec<_> = video
            .selected
            .iter()
            .filter_map(|id| drag.base.clip(*id).map(|(t, _)| (t.id, t.kind)))
            .collect();
        let pictures = tracks
            .iter()
            .any(|(_, k)| *k == spectrum_video::TrackKind::Video);
        let kind = if pictures {
            spectrum_video::TrackKind::Video
        } else {
            spectrum_video::TrackKind::Audio
        };
        // The pressed clip's track, or the first of the kind that leads.
        let pressed = drag
            .clicked
            .and_then(|id| drag.base.clip(id))
            .filter(|(t, _)| t.kind == kind)
            .map(|(t, _)| t.id);
        let Some(source) =
            pressed.or_else(|| tracks.iter().find(|(_, k)| *k == kind).map(|(id, _)| *id))
        else {
            return 0;
        };
        let y = video.local(position).1;
        let layout = video.layout_of(&drag.base);
        let distance = |r: &crate::video_layout::Row| (r.top + r.height / 2. - y).abs();
        let nearest = layout
            .rows
            .iter()
            .filter(|r| r.kind == kind)
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
            .map(|r| r.track);
        let rank = |track: u64| {
            drag.base
                .tracks
                .iter()
                .filter(|t| t.kind == kind)
                .position(|t| t.id == track)
        };
        match (nearest.and_then(rank), rank(source)) {
            (Some(to), Some(from)) => to as i32 - from as i32,
            _ => 0,
        }
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
}
