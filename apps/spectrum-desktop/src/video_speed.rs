//! Speed on the timeline. A clip's timing shows as a line across the
//! selected clip, in either view: velocity (Sony Vegas, how fast the source
//! plays) or time remap (After Effects, which source moment shows). Both
//! views show the same timing and the same keyframes; switching changes
//! nothing. Editing in the view the keyframes are not kept as converts them
//! first, keeping every frame, in the same undo step. Double-click the line
//! to add a keyframe (and keep holding to drag it); right-click one for its
//! curve.
use crate::{video_layout::HEADER, video_state::Drag, workspace::Workspace};
use gpui::*;
use spectrum_media::Time;
use spectrum_video::{
    Clip, Command, Curve, MAX_PERCENT, RemapKey, Speed, SpeedForm, VelocityKey, Video,
};

/// Velocity across the line's height, from -1 at the bottom to 1 at the
/// top. Most of the height goes to what speed ramps live in, 0% to 300%;
/// past -100% and 300% the scale tightens toward -1000% and 1000%.
const SCALE: [(f64, f64); 7] = [
    (-MAX_PERCENT, -1.),
    (-100., -0.7),
    (0., -0.45),
    (100., 0.1),
    (200., 0.45),
    (300., 0.7),
    (MAX_PERCENT, 1.),
];

pub fn velocity_height(percent: f64) -> f64 {
    let percent = percent.clamp(-MAX_PERCENT, MAX_PERCENT);
    for pair in SCALE.windows(2) {
        let ((p0, h0), (p1, h1)) = (pair[0], pair[1]);
        if percent <= p1 {
            return h0 + (h1 - h0) * (percent - p0) / (p1 - p0);
        }
    }
    1.
}

pub fn height_velocity(height: f64) -> f64 {
    let height = height.clamp(-1., 1.);
    for pair in SCALE.windows(2) {
        let ((p0, h0), (p1, h1)) = (pair[0], pair[1]);
        if height <= h1 {
            return p0 + (p1 - p0) * (height - h0) / (h1 - h0);
        }
    }
    MAX_PERCENT
}

/// Keyframe times and values for either kind of speed.
pub fn keys(speed: &Speed) -> Vec<(Time, f64, Curve)> {
    match speed {
        Speed::Normal => Vec::new(),
        Speed::Velocity { keys, .. } => keys.iter().map(|k| (k.at, k.percent, k.curve)).collect(),
        Speed::Remap { keys, .. } => keys
            .iter()
            .map(|k| (k.at, k.source.seconds(), k.curve))
            .collect(),
    }
}

pub fn with_keys(speed: &Speed, keys: &[(Time, f64, Curve)]) -> Speed {
    let mut keys = keys.to_vec();
    keys.sort_by_key(|k| k.0);
    match speed {
        Speed::Velocity { origin, .. } => Speed::Velocity {
            keys: keys
                .iter()
                .map(|&(at, percent, curve)| VelocityKey {
                    at,
                    percent: percent.clamp(-MAX_PERCENT, MAX_PERCENT),
                    curve,
                })
                .collect(),
            origin: *origin,
        },
        Speed::Remap { origin, .. } => Speed::Remap {
            keys: keys
                .iter()
                .map(|&(at, source, curve)| RemapKey {
                    at,
                    source: Time::from_seconds(source.max(0.)),
                    curve,
                })
                .collect(),
            origin: *origin,
        },
        Speed::Normal => Speed::Normal,
    }
}

/// Whether `speed` is kept as the keyframes `view` shows.
pub fn kept_as(speed: &Speed, view: SpeedForm) -> bool {
    matches!(
        (speed, view),
        (Speed::Velocity { .. }, SpeedForm::Velocity) | (Speed::Remap { .. }, SpeedForm::Remap)
    )
}

/// The source's length, for a remap's height.
fn source_length(clip: &Clip) -> f64 {
    clip.source_duration
        .map_or(clip.duration.seconds(), |d| d.seconds())
        .max(1e-3)
}

/// The line's height at `offset` into the clip, from -1 to 1, in `view`.
fn height_at(clip: &Clip, view: SpeedForm, offset: f64) -> f64 {
    let offset = Time::from_seconds(offset);
    match view {
        SpeedForm::Velocity => velocity_height(clip.speed.rate(offset) * 100.),
        SpeedForm::Remap => {
            clip.source_seconds(clip.start + offset) / source_length(clip) * 2. - 1.
        }
    }
}

/// What a keyframe reads in `view`: its own value when kept as that view,
/// otherwise the timing there. Velocity in percent, remap in source seconds.
pub fn key_value(clip: &Clip, view: SpeedForm, index: usize) -> Option<f64> {
    let (at, value, _) = *keys(&clip.speed).get(index)?;
    if kept_as(&clip.speed, view) {
        return Some(match view {
            SpeedForm::Velocity => value,
            SpeedForm::Remap => clip.wrap(value),
        });
    }
    let offset = at - clip.speed.origin();
    Some(match view {
        SpeedForm::Velocity => clip.speed.rate(offset) * 100.,
        SpeedForm::Remap => clip.source_seconds(clip.start + offset),
    })
}

/// The keyframe nearest `at` in keyframe time.
fn nearest(speed: &Speed, at: Time) -> Option<usize> {
    keys(speed)
        .iter()
        .enumerate()
        .min_by_key(|(_, k)| (k.0 - at).0.abs())
        .map(|(i, _)| i)
}

/// A speed line's points and keyframe dots on the timeline, and guides at
/// 0% and 100% in the velocity view.
pub struct Geometry {
    pub points: Vec<(f32, f32)>,
    pub dots: Vec<(f32, f32, bool)>,
    /// Each dot's keyframe.
    pub indices: Vec<usize>,
    pub guides: Vec<(f32, bool)>,
}

impl Workspace {
    /// The clip whose timing the line and the Speed tab show: the one
    /// selected picture, or the one selected sound clip with no picture.
    pub fn speed_clip(&self) -> Option<Clip> {
        let video = self.video.as_ref()?;
        let selected: Vec<_> = video
            .selected
            .iter()
            .filter_map(|id| video.doc.clip(*id))
            .collect();
        let pictures: Vec<_> = selected
            .iter()
            .filter(|(t, _)| t.kind == spectrum_video::TrackKind::Video)
            .collect();
        match pictures.as_slice() {
            [(_, clip)] => Some((*clip).clone()),
            [] if selected.len() == 1 => Some(selected[0].1.clone()),
            _ => None,
        }
    }

    /// `clip` in `doc` as the keyframes the view shows: the commands that
    /// convert it (none when it is kept that way), and the clip after.
    pub fn in_view(&self, doc: &Video, clip: u64) -> Option<(Vec<Command>, Clip)> {
        let video = self.video.as_ref()?;
        let found = doc.clip(clip)?.1.clone();
        if kept_as(&found.speed, video.speed_view) {
            return Some((Vec::new(), found));
        }
        let command = Command::ConvertSpeed {
            clip,
            to: video.speed_view,
        };
        let mut converted = doc.clone();
        spectrum_video::apply(&mut converted, command.clone()).ok()?;
        Some((vec![command], converted.clip(clip)?.1.clone()))
    }

    /// Changes the chosen keyframe's list in the view's form, converting
    /// the clip first if needed, as one edit. `change` gets the list and
    /// the keyframe's index, and returns the index it ends at (or `None`
    /// once it is gone).
    fn edit_key(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Vec<(Time, f64, Curve)>, usize) -> Option<usize>,
    ) {
        let Some((clip, index)) = self.selected_key() else {
            return;
        };
        let Some(doc) = self.video.as_ref().map(|v| v.doc.clone()) else {
            return;
        };
        let Some((mut commands, converted)) = self.in_view(&doc, clip.id) else {
            return;
        };
        let index = if commands.is_empty() {
            index
        } else {
            let at = keys(&clip.speed)[index].0;
            let Some(index) = nearest(&converted.speed, at) else {
                return;
            };
            index
        };
        let mut list = keys(&converted.speed);
        let kept = change(&mut list, index);
        let at = kept.map(|i| list[i].0);
        let speed = with_keys(&converted.speed, &list);
        let index = at.and_then(|at| keys(&speed).iter().position(|k| k.0 == at));
        commands.push(Command::SetSpeed {
            clip: clip.id,
            speed,
        });
        if let Some(video) = &mut self.video {
            video.key = index.map(|i| (clip.id, i));
        }
        self.video_commands(commands, window, cx);
        self.sync_key_fields(window, cx);
    }

    /// The chosen keyframe gets `curve` to the next.
    pub fn set_key_curve(&mut self, curve: Curve, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_key(window, cx, |list, index| {
            list[index].2 = curve;
            Some(index)
        });
    }

    pub fn delete_speed_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_key(window, cx, |list, index| {
            if list.len() > 1 {
                list.remove(index);
            }
            None
        });
    }

    /// A typed keyframe time (from the clip's start) or value (a percent,
    /// or a source time), in the view shown.
    pub fn type_key(
        &mut self,
        time: bool,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(video) = &self.video else {
            return;
        };
        let (rate, view) = (video.doc.rate, video.speed_view);
        let origin = self.selected_key().map(|(c, _)| c.speed.origin());
        let parsed = if time {
            Time::parse(text, Some(rate)).ok().map(|t| t.seconds())
        } else {
            match view {
                SpeedForm::Remap => Time::parse(text, Some(rate)).ok().map(|t| t.seconds()),
                SpeedForm::Velocity => text.trim().trim_end_matches('%').trim().parse().ok(),
            }
        };
        let (Some(value), Some(origin)) = (parsed, origin) else {
            return self.sync_key_fields(window, cx);
        };
        self.edit_key(window, cx, |list, index| {
            if time {
                list[index].0 = (Time::from_seconds(value) + origin).max(Time::ZERO);
            } else {
                list[index].1 = value;
            }
            Some(index)
        });
    }

    /// The chosen keyframe's clip and place in its list.
    pub fn selected_key(&self) -> Option<(Clip, usize)> {
        let video = self.video.as_ref()?;
        let (id, index) = video.key?;
        let clip = video.doc.clip(id)?.1.clone();
        (index < keys(&clip.speed).len()).then_some((clip, index))
    }

    /// Shows the chosen keyframe's time and value, in the view, in the
    /// Speed tab's fields.
    pub fn sync_key_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.selected_key();
        let Some(video) = &self.video else {
            return;
        };
        let (rate, view) = (video.doc.rate, video.speed_view);
        let (time, value) = match &selected {
            Some((clip, index)) => {
                let at = keys(&clip.speed)[*index].0 - clip.speed.origin();
                let value = key_value(clip, view, *index).unwrap_or(0.);
                let value = match view {
                    SpeedForm::Remap => Time::from_seconds(value).timecode(rate),
                    SpeedForm::Velocity => format!("{value:.0}%"),
                };
                (at.timecode(rate), value)
            }
            None => (String::new(), String::new()),
        };
        video
            .keys
            .time
            .update(cx, |s, cx| s.set_value(time, window, cx));
        video
            .keys
            .value
            .update(cx, |s, cx| s.set_value(value, window, cx));
    }

    /// Shows speed as velocity or as a time remap; nothing changes.
    pub fn set_speed_view(&mut self, view: SpeedForm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(video) = &mut self.video {
            video.speed_view = view;
        }
        self.sync_key_fields(window, cx);
        cx.notify();
    }

    /// Holds the frame at the playhead for a second, then plays on; the
    /// keyframe where it plays on is chosen, so its time can be typed.
    pub fn freeze_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(clip) = self.speed_clip() else {
            return;
        };
        let Some(now) = self.video.as_ref().map(|v| v.monitor.now()) else {
            return;
        };
        if !clip.covers(now) {
            self.notify_error(
                anyhow::anyhow!("Move the playhead onto the clip to freeze a frame"),
                window,
                cx,
            );
            return;
        }
        let length = Time::from_seconds(1.);
        let command = Command::Freeze {
            clip: clip.id,
            at: now,
            length: Some(length),
        };
        self.video_commands(vec![command], window, cx);
        if let Some(frozen) = self.speed_clip() {
            let at = now - clip.start + length + frozen.speed.origin();
            let index = keys(&frozen.speed).iter().position(|k| k.0 == at);
            if let Some(video) = &mut self.video {
                video.key = index.map(|i| (frozen.id, i));
            }
        }
        self.sync_key_fields(window, cx);
    }

    /// Plays the clip's frames backward.
    pub fn reverse_clip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(clip) = self.speed_clip() else {
            return;
        };
        if let Some(video) = &mut self.video {
            video.key = None;
        }
        self.video_commands(vec![Command::Reverse { clip: clip.id }], window, cx);
    }

    /// Back to the source's own speed.
    pub fn reset_speed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(clip) = self.speed_clip() else {
            return;
        };
        if let Some(video) = &mut self.video {
            video.key = None;
        }
        let command = Command::SetSpeed {
            clip: clip.id,
            speed: Speed::Normal,
        };
        self.video_commands(vec![command], window, cx);
    }

    /// Adds a keyframe at the playhead, keeping the timing there.
    pub fn add_speed_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(clip), Some(video)) = (self.speed_clip(), &self.video) else {
            return;
        };
        let (now, doc) = (video.monitor.now(), video.doc.clone());
        if !clip.covers(now) {
            return;
        }
        let Some((mut commands, converted)) = self.in_view(&doc, clip.id) else {
            return;
        };
        let (speed, index) = with_key_at(&converted, now);
        commands.push(Command::SetSpeed {
            clip: clip.id,
            speed,
        });
        if let Some(video) = &mut self.video {
            video.key = Some((clip.id, index));
        }
        self.video_commands(commands, window, cx);
        self.sync_key_fields(window, cx);
    }

    /// The selected clip's box on the timeline, as drawn: left, top, width,
    /// and height.
    fn speed_box(&self, clip: &Clip) -> Option<(f32, f32, f32, f32)> {
        let video = self.video.as_ref()?;
        let track = video.doc.clip(clip.id)?.0.id;
        let row = video.layout().row(track)?;
        let (left, right) = (video.x_of(clip.start), video.x_of(clip.end()));
        Some((left, row.top + 3., right - left, row.height - 6.))
    }

    /// Presses on the speed line: a keyframe starts its drag; a double-click
    /// adds one there and drags it while held. Returns whether the line
    /// took the press.
    pub fn speed_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) -> bool {
        // A keyframe on another clip's line selects that clip first.
        if let Some(other) = self.key_owner(event.position)
            && self.speed_clip().is_none_or(|c| c.id != other)
            && let Some(video) = &mut self.video
        {
            video.selected = video.doc.linked(other);
        }
        let Some(clip) = self.speed_clip() else {
            return false;
        };
        let Some(boxed) = self.speed_box(&clip) else {
            return false;
        };
        let Some(video) = &self.video else {
            return false;
        };
        let (x, y) = video.local(event.position);
        let time = video.time_at(x);
        let doc = video.doc.clone();
        // On the line itself, not just inside the clip.
        let on_line = x >= boxed.0.max(HEADER)
            && x <= boxed.0 + boxed.2
            && self.speed_geometry(&clip, boxed, None).is_some_and(|g| {
                g.points
                    .iter()
                    .min_by(|a, b| (a.0 - x).abs().total_cmp(&(b.0 - x).abs()))
                    .is_some_and(|(_, line)| (line - y).abs() <= 8.)
            });
        let hit = self.speed_key_at(&clip, boxed, x, y);
        if hit.is_none() && !(event.click_count == 2 && on_line) {
            return false;
        }
        let Some((prefix, converted)) = self.in_view(&doc, clip.id) else {
            return false;
        };
        let mut drag = Drag::new(
            doc,
            crate::video_state::DragKind::Key {
                clip: clip.id,
                index: 0,
            },
            (time.seconds(), f32::from(event.position.y)),
        );
        let index = match hit {
            Some(index) if prefix.is_empty() => index,
            Some(index) => {
                let at = keys(&clip.speed)[index].0;
                let Some(index) = nearest(&converted.speed, at) else {
                    return false;
                };
                index
            }
            None => {
                // Added here, and dragged from here while the button holds.
                let (speed, index) = with_key_at(&converted, time);
                drag.command = Some(Command::SetSpeed {
                    clip: clip.id,
                    speed: speed.clone(),
                });
                drag.start = Some(speed);
                index
            }
        };
        if drag.start.is_none() {
            drag.start = Some(converted.speed.clone());
        }
        drag.kind = crate::video_state::DragKind::Key {
            clip: clip.id,
            index,
        };
        drag.prefix = prefix;
        let preview = drag.command.is_some().then(|| {
            let mut doc = drag.base.clone();
            for command in drag.commands() {
                let _ = spectrum_video::apply(&mut doc, command);
            }
            doc
        });
        let Some(video) = &mut self.video else {
            return false;
        };
        video.key = Some((clip.id, index));
        video.drag = Some(drag);
        if let Some(doc) = preview {
            self.show_video_doc(doc);
        }
        cx.notify();
        true
    }

    /// The clip whose speed keyframe is under `position`, if any.
    fn key_owner(&self, position: Point<Pixels>) -> Option<u64> {
        let video = self.video.as_ref()?;
        let (x, y) = video.local(position);
        let layout = video.layout();
        let row = layout
            .rows
            .iter()
            .find(|r| y >= r.top && y < r.top + r.height)?;
        let track = video.doc.track(row.track)?;
        track
            .clips
            .iter()
            .filter(|c| !c.speed.is_normal())
            .find(|c| {
                let (left, right) = (video.x_of(c.start), video.x_of(c.end()));
                let boxed = (left, row.top + 3., right - left, row.height - 6.);
                self.speed_key_at(c, boxed, x, y).is_some()
            })
            .map(|c| c.id)
    }

    /// The speed command a keyframe drag to `position` makes.
    pub fn key_drag(&self, clip: u64, index: usize, position: Point<Pixels>) -> Option<Command> {
        let video = self.video.as_ref()?;
        let drag = video.drag.as_ref()?;
        let mut base = drag.base.clip(clip)?.1.clone();
        base.speed = drag.start.clone()?;
        let track = drag.base.clip(clip)?.0.id;
        let row = video.layout_of(&drag.base).row(track)?;
        let (x, y) = video.local(position);
        let at = drag.base.snap(video.time_at(x));
        let speed = dragged_speed(
            &base,
            video.speed_view,
            index,
            at,
            y,
            (row.top + 3., row.height - 6.),
        );
        Some(Command::SetSpeed { clip, speed })
    }

    /// Where the selected clip's speed line is, in the view.
    pub fn speed_geometry(
        &self,
        clip: &Clip,
        bounds: (f32, f32, f32, f32),
        selected: Option<usize>,
    ) -> Option<Geometry> {
        let video = self.video.as_ref()?;
        let view = video.speed_view;
        let (_, top, _, height) = bounds;
        let y_of = |unit: f64| top + height / 2. - unit as f32 * (height / 2. - 4.);
        let x_of = |offset: f64| video.x_of(clip.start) + offset as f32 * video.zoom;
        // Every 3 points across the part of the clip on screen.
        let width = f32::from(video.timeline.borrow().size.width);
        let offset_at = |x: f32| f64::from((x - x_of(0.)) / video.zoom);
        let from = offset_at(HEADER).max(0.);
        let to = offset_at(width).min(clip.duration.seconds());
        let samples = ((x_of(to) - x_of(from)) / 3.).max(2.) as usize;
        let points = (0..=samples)
            .map(|i| {
                let offset = from + (to - from) * i as f64 / samples as f64;
                (x_of(offset), y_of(height_at(clip, view, offset)))
            })
            .collect();
        let origin = clip.speed.origin().seconds();
        let mut dots = Vec::new();
        let mut indices = Vec::new();
        for (index, (at, ..)) in keys(&clip.speed).iter().enumerate() {
            let offset = at.seconds() - origin;
            if offset < 0. || offset > clip.duration.seconds() {
                continue;
            }
            let unit = match (view, key_value(clip, view, index)) {
                (SpeedForm::Velocity, Some(percent)) => velocity_height(percent),
                (SpeedForm::Remap, Some(source)) => source / source_length(clip) * 2. - 1.,
                (_, None) => continue,
            };
            dots.push((x_of(offset), y_of(unit), selected == Some(index)));
            indices.push(index);
        }
        let guides = match view {
            SpeedForm::Velocity => vec![
                (y_of(velocity_height(0.)), false),
                (y_of(velocity_height(100.)), true),
            ],
            SpeedForm::Remap => Vec::new(),
        };
        Some(Geometry {
            points,
            dots,
            indices,
            guides,
        })
    }

    /// The keyframe of a clip's speed line under a timeline point, if any.
    pub fn speed_key_at(
        &self,
        clip: &Clip,
        bounds: (f32, f32, f32, f32),
        x: f32,
        y: f32,
    ) -> Option<usize> {
        let geometry = self.speed_geometry(clip, bounds, None)?;
        geometry
            .dots
            .iter()
            .zip(geometry.indices)
            .filter(|((dx, ..), _)| *dx >= HEADER)
            .find(|((dx, dy, _), _)| (dx - x).abs() <= 6. && (dy - y).abs() <= 6.)
            .map(|(_, index)| index)
    }

    /// Paints the selected clip's speed line, in the view, with its
    /// keyframes as dots.
    pub fn speed_line(
        &self,
        clip: &Clip,
        bounds: (f32, f32, f32, f32),
        selected: Option<usize>,
        dim: bool,
    ) -> Option<AnyElement> {
        let geometry = self.speed_geometry(clip, bounds, selected)?;
        let (line_color, dot_color) = if dim {
            (rgba(0xf2c14e80), rgba(0xf2c14e99))
        } else {
            (rgba(0xf2c14eff), rgba(0xf2c14eff))
        };
        let (left, width) = (bounds.0.max(HEADER), bounds.2);
        let right = bounds.0 + width;
        Some(
            canvas(
                |_, _, _| {},
                move |area, _, window, _| {
                    let at = |x: f32, y: f32| area.origin + point(px(x), px(y));
                    for (y, full) in &geometry.guides {
                        window.paint_quad(fill(
                            Bounds::new(at(left, *y), size(px((right - left).max(0.)), px(1.))),
                            rgba(if *full { 0xffffff38 } else { 0xffffff1c }),
                        ));
                    }
                    let mut line = PathBuilder::stroke(px(1.5));
                    for (i, (x, y)) in geometry.points.iter().enumerate() {
                        if i == 0 {
                            line.move_to(at(*x, *y));
                        } else {
                            line.line_to(at(*x, *y));
                        }
                    }
                    if let Ok(path) = line.build() {
                        window.paint_path(path, line_color);
                    }
                    for (x, y, on) in &geometry.dots {
                        let r = if *on { 5. } else { 4. };
                        window.paint_quad(fill(
                            Bounds::new(at(x - r, y - r), size(px(r * 2.), px(r * 2.))),
                            if *on { rgba(0xffffffff) } else { dot_color },
                        ));
                    }
                },
            )
            .absolute()
            .size_full()
            .into_any_element(),
        )
    }
}

/// `clip`'s speed (already in the view's form) with a keyframe at timeline
/// `now` that keeps the timing there, and that keyframe's index.
fn with_key_at(clip: &Clip, now: Time) -> (Speed, usize) {
    let offset = (now - clip.start).max(Time::ZERO);
    let at = offset + clip.speed.origin();
    let value = match &clip.speed {
        Speed::Remap { .. } => clip.unwrapped_seconds(now),
        _ => clip.speed.rate(offset) * 100.,
    };
    let mut list = keys(&clip.speed);
    list.retain(|k| k.0 != at);
    let curve = list
        .iter()
        .rev()
        .find(|k| k.0 < at)
        .map_or(Curve::Smooth, |k| k.2);
    list.push((at, value, curve));
    let speed = with_keys(&clip.speed, &list);
    let index = keys(&speed)
        .iter()
        .position(|k| k.0 == at)
        .unwrap_or_default();
    (speed, index)
}

/// The speed a keyframe drag makes: keyframe `index` of `clip` (kept as the
/// view's form) moved to timeline `at` and height `y` within the clip's box,
/// kept between its neighbors.
fn dragged_speed(
    clip: &Clip,
    view: SpeedForm,
    index: usize,
    at: Time,
    y: f32,
    (top, height): (f32, f32),
) -> Speed {
    let mut list = keys(&clip.speed);
    if index >= list.len() {
        return clip.speed.clone();
    }
    let unit = ((top + height / 2. - y) / (height / 2. - 4.)).clamp(-1., 1.) as f64;
    let value = match view {
        SpeedForm::Remap => (unit + 1.) / 2. * source_length(clip),
        SpeedForm::Velocity => height_velocity(unit),
    };
    let origin = clip.speed.origin();
    // Up to its neighbors, which a freeze's keyframes share times with.
    let low = index.checked_sub(1).map_or(Time::ZERO, |i| list[i].0);
    let high = list.get(index + 1).map_or(Time(i64::MAX), |k| k.0);
    let at = (at - clip.start + origin).clamp(low, high);
    list[index] = (at, value, list[index].2);
    with_keys(&clip.speed, &list)
}

#[cfg(test)]
mod tests {
    use super::{height_velocity, velocity_height};
    use spectrum_video::MAX_PERCENT;

    #[test]
    fn the_velocity_scale_gives_most_room_to_ramps_and_inverts() {
        assert!((velocity_height(-MAX_PERCENT) + 1.).abs() < 1e-9);
        assert!((velocity_height(MAX_PERCENT) - 1.).abs() < 1e-9);
        let ramps = velocity_height(300.) - velocity_height(0.);
        assert!(
            ramps > 1.0,
            "0% to 300% takes over half the height: {ramps}"
        );
        for percent in [
            -1000., -250., -100., -20., 0., 37., 100., 180., 300., 640., 1000.,
        ] {
            let back = height_velocity(velocity_height(percent));
            assert!(
                (back - percent).abs() < 1e-6,
                "{percent} came back as {back}"
            );
        }
    }
}
