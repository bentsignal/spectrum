//! Speed on the timeline and in the sidebar. A clip plays at normal speed,
//! by velocity keyframes (a line across the clip, as in Sony Vegas: drag a
//! point, double-click the line to add one), or by a time remap (After
//! Effects: which source time shows when). The selected keyframe's time and
//! value can be typed; its curve, the clip's slow-motion interpolation, and
//! whether its sound follows the speed are a click away.
use crate::{
    controls::{chip, group, toggle},
    theme::*,
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use spectrum_media::Time;
use spectrum_video::{Clip, Command, Curve, Interpolation, RemapKey, Speed, VelocityKey};

/// Velocity on the line's height: 0% in the middle, 100% about a third of
/// the way up, 1000% at the top, and backward below.
pub fn velocity_height(percent: f64) -> f64 {
    percent.signum() * (1. + percent.abs() / 100.).ln() / 11f64.ln()
}

pub fn height_velocity(height: f64) -> f64 {
    let height = height.clamp(-1., 1.);
    height.signum() * 100. * (11f64.powf(height.abs()) - 1.)
}

/// The keyframe editor's fields, made once with the editor.
pub struct KeyFields {
    pub time: Entity<InputState>,
    pub value: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl KeyFields {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let time = cx.new(|cx| InputState::new(window, cx).placeholder("Time"));
        let value = cx.new(|cx| InputState::new(window, cx).placeholder("Value"));
        let typed = |field: bool| {
            move |this: &mut Workspace,
                  state: &Entity<InputState>,
                  event: &InputEvent,
                  window: &mut Window,
                  cx: &mut Context<Workspace>| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let text = state.read(cx).value().to_string();
                    this.type_key(field, &text, window, cx);
                }
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&time, window, typed(true)),
            cx.subscribe_in(&value, window, typed(false)),
        ];
        Self {
            time,
            value,
            _subscriptions: subscriptions,
        }
    }
}

/// Keyframe times and values for either kind of speed.
fn keys(speed: &Speed) -> Vec<(Time, f64, Curve)> {
    match speed {
        Speed::Normal => Vec::new(),
        Speed::Velocity { keys, .. } => keys.iter().map(|k| (k.at, k.percent, k.curve)).collect(),
        Speed::Remap { keys, .. } => keys
            .iter()
            .map(|k| (k.at, k.source.seconds(), k.curve))
            .collect(),
    }
}

fn with_keys(speed: &Speed, keys: &[(Time, f64, Curve)]) -> Speed {
    let mut keys = keys.to_vec();
    keys.sort_by_key(|k| k.0);
    match speed {
        Speed::Velocity { origin, .. } => Speed::Velocity {
            keys: keys
                .iter()
                .map(|&(at, percent, curve)| VelocityKey {
                    at,
                    percent: percent
                        .clamp(-spectrum_video::MAX_PERCENT, spectrum_video::MAX_PERCENT),
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

impl Workspace {
    /// The one selected picture clip, whose speed the sidebar shows.
    pub fn speed_clip(&self) -> Option<Clip> {
        let video = self.video.as_ref()?;
        let mut pictures = video
            .selected
            .iter()
            .filter_map(|id| video.doc.clip(*id))
            .filter(|(t, _)| t.kind == spectrum_video::TrackKind::Video);
        let (_, clip) = pictures.next()?;
        pictures.next().is_none().then(|| clip.clone())
    }

    fn set_speed(&mut self, clip: u64, speed: Speed, window: &mut Window, cx: &mut Context<Self>) {
        self.video_commands(vec![Command::SetSpeed { clip, speed }], window, cx);
    }

    fn speed_mode(&mut self, mode: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(clip) = self.speed_clip() else {
            return;
        };
        let origin = Time::ZERO;
        let speed = match mode {
            1 => Speed::Velocity {
                keys: vec![VelocityKey {
                    at: Time::ZERO,
                    percent: 100.,
                    curve: Curve::Smooth,
                }],
                origin,
            },
            2 => Speed::Remap {
                keys: vec![
                    RemapKey {
                        at: Time::ZERO,
                        source: clip.source_time(clip.start),
                        curve: Curve::Linear,
                    },
                    RemapKey {
                        at: clip.duration,
                        source: clip.source_time(clip.start) + clip.duration,
                        curve: Curve::Linear,
                    },
                ],
                origin,
            },
            _ => Speed::Normal,
        };
        if let Some(video) = &mut self.video {
            video.key = None;
        }
        self.set_speed(clip.id, speed, window, cx);
    }

    /// Adds a keyframe at the playhead, keeping the speed there as it is.
    fn add_speed_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(now) = self.video.as_ref().map(|v| v.monitor.now()) else {
            return;
        };
        self.add_speed_key_at(now, window, cx);
    }

    /// Adds a keyframe at timeline `now` on the selected clip's line.
    pub fn add_speed_key_at(&mut self, now: Time, window: &mut Window, cx: &mut Context<Self>) {
        let Some(clip) = self.speed_clip() else {
            return;
        };
        if !clip.covers(now) {
            return;
        }
        let offset = now - clip.start;
        let at = offset + clip.speed.origin();
        let value = match &clip.speed {
            Speed::Velocity { .. } => clip.speed.rate(offset) * 100.,
            Speed::Remap { .. } => clip.source_seconds(now),
            Speed::Normal => return,
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
        let index = keys(&speed).iter().position(|k| k.0 == at);
        if let Some(video) = &mut self.video {
            video.key = index.map(|i| (clip.id, i));
        }
        self.set_speed(clip.id, speed, window, cx);
        self.sync_key_fields(window, cx);
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
        let resume = now - clip.start + length;
        if let Some(frozen) = self.speed_clip() {
            let at = resume + frozen.speed.origin();
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

    fn delete_speed_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((clip, index)) = self.selected_key() else {
            return;
        };
        let mut list = keys(&clip.speed);
        if list.len() <= 1 {
            return;
        }
        list.remove(index);
        if let Some(video) = &mut self.video {
            video.key = None;
        }
        self.set_speed(clip.id, with_keys(&clip.speed, &list), window, cx);
    }

    fn set_key_curve(&mut self, curve: Curve, window: &mut Window, cx: &mut Context<Self>) {
        let Some((clip, index)) = self.selected_key() else {
            return;
        };
        let mut list = keys(&clip.speed);
        list[index].2 = curve;
        self.set_speed(clip.id, with_keys(&clip.speed, &list), window, cx);
    }

    /// The selected keyframe's clip and place in its list.
    pub fn selected_key(&self) -> Option<(Clip, usize)> {
        let video = self.video.as_ref()?;
        let (id, index) = video.key?;
        let clip = video.doc.clip(id)?.1.clone();
        (index < keys(&clip.speed).len()).then_some((clip, index))
    }

    /// A typed keyframe time (from the clip's start) or value (a percent,
    /// or a source time).
    fn type_key(&mut self, time: bool, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some((clip, index)) = self.selected_key() else {
            return;
        };
        let rate = self.video.as_ref().map(|v| v.doc.rate);
        let mut list = keys(&clip.speed);
        let origin = clip.speed.origin();
        if time {
            let Ok(at) = Time::parse(text, rate) else {
                return self.sync_key_fields(window, cx);
            };
            list[index].0 = (at + origin).max(Time::ZERO);
        } else {
            let value = match &clip.speed {
                Speed::Remap { .. } => Time::parse(text, rate).map(|t| t.seconds()).ok(),
                _ => text.trim().trim_end_matches('%').trim().parse::<f64>().ok(),
            };
            let Some(value) = value else {
                return self.sync_key_fields(window, cx);
            };
            list[index].1 = value;
        }
        let at = list[index].0;
        let speed = with_keys(&clip.speed, &list);
        let index = keys(&speed).iter().position(|k| k.0 == at);
        if let Some(video) = &mut self.video {
            video.key = index.map(|i| (clip.id, i));
        }
        self.set_speed(clip.id, speed, window, cx);
        self.sync_key_fields(window, cx);
    }

    /// Shows the selected keyframe's time and value in the fields.
    pub fn sync_key_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.selected_key();
        let Some(video) = &self.video else {
            return;
        };
        let rate = video.doc.rate;
        let (time, value) = match &selected {
            Some((clip, index)) => {
                let (at, value, _) = keys(&clip.speed)[*index];
                let at = at - clip.speed.origin();
                let value = match clip.speed {
                    Speed::Remap { .. } => Time::from_seconds(value).timecode(rate),
                    _ => format!("{value:.0}%"),
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

    pub fn speed_section(&self, cx: &mut Context<Self>) -> Option<Div> {
        let clip = self.speed_clip()?;
        let video = self.video.as_ref()?;
        let mode = match clip.speed {
            Speed::Normal => 0,
            Speed::Velocity { .. } => 1,
            Speed::Remap { .. } => 2,
        };
        let modes = div().flex().gap_2().children(
            [
                (0, "speed-normal", "Normal"),
                (1, "speed-velocity", "Velocity"),
                (2, "speed-remap", "Time remap"),
            ]
            .map(|(index, id, label)| {
                chip(id, label, mode == index).on_click(
                    cx.listener(move |this, _, window, cx| this.speed_mode(index, window, cx)),
                )
            }),
        );
        // A keyframe chosen before an undo may be gone.
        let selected = video
            .key
            .filter(|(id, i)| *id == clip.id && *i < keys(&clip.speed).len())
            .map(|(_, i)| i);
        let rows = keys(&clip.speed)
            .into_iter()
            .enumerate()
            .map(|(index, (at, value, _))| {
                let at = (at - clip.speed.origin()).timecode(video.doc.rate);
                let value = match clip.speed {
                    Speed::Remap { .. } => Time::from_seconds(value).timecode(video.doc.rate),
                    _ => format!("{value:.0}%"),
                };
                let id = clip.id;
                div()
                    .id(SharedString::from(format!("key-{index}")))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .flex()
                    .justify_between()
                    .text_sm()
                    .when(selected == Some(index), |el| el.bg(rgb(SELECTED)))
                    .hover(|el| el.bg(rgb(HOVER)))
                    .child(at)
                    .child(div().text_color(rgb(MUTED)).child(value))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(video) = &mut this.video {
                            video.key = Some((id, index));
                        }
                        this.sync_key_fields(window, cx);
                        cx.notify();
                    }))
            });
        let curves =
            selected.map(|index| {
                let current = keys(&clip.speed)[index].2;
                div().flex().flex_wrap().gap_2().children(
                    [
                        (Curve::Smooth, "curve-smooth", "Smooth"),
                        (Curve::Linear, "curve-linear", "Linear"),
                        (Curve::Fast, "curve-fast", "Fast"),
                        (Curve::Slow, "curve-slow", "Slow"),
                        (Curve::Sharp, "curve-sharp", "Sharp"),
                        (Curve::Hold, "curve-hold", "Hold"),
                    ]
                    .map(|(curve, id, label)| {
                        chip(id, label, current == curve).on_click(cx.listener(
                            move |this, _, window, cx| this.set_key_curve(curve, window, cx),
                        ))
                    }),
                )
            });
        let interpolation = div().flex().gap_2().children(
            [
                (Interpolation::Off, "interp-off", "Off"),
                (Interpolation::Blend, "interp-blend", "Blend"),
                (Interpolation::Motion, "interp-motion", "Motion"),
            ]
            .map(|(method, id, label)| {
                let clip = clip.id;
                chip(id, label, clip_interpolation(&video.doc, clip) == method).on_click(
                    cx.listener(move |this, _, window, cx| {
                        this.video_commands(vec![set_clip(clip, Some(method), None)], window, cx)
                    }),
                )
            }),
        );
        let follows = clip.sound_follows_speed;
        let id = clip.id;
        let actions = div()
            .flex()
            .gap_2()
            .child(
                Button::new("freeze-frame")
                    .small()
                    .ghost()
                    .label("Freeze frame")
                    .on_click(cx.listener(|this, _, window, cx| this.freeze_frame(window, cx))),
            )
            .child(
                Button::new("reverse-clip")
                    .small()
                    .ghost()
                    .label("Reverse")
                    .on_click(cx.listener(|this, _, window, cx| this.reverse_clip(window, cx))),
            );
        Some(
            group("Speed", None)
                .gap_3()
                .child(modes)
                .child(actions)
                .when(mode != 0, |el| {
                    el.child(div().flex().flex_col().gap_0p5().children(rows))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    Button::new("add-key")
                                        .small()
                                        .label("Add at playhead")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.add_speed_key(window, cx)
                                        })),
                                )
                                .when(selected.is_some(), |el| {
                                    el.child(
                                        Button::new("delete-key")
                                            .small()
                                            .ghost()
                                            .label("Delete")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.delete_speed_key(window, cx)
                                            })),
                                    )
                                }),
                        )
                        .when(selected.is_some(), |el| {
                            el.child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        div().flex_1().child(Input::new(&video.keys.time).small()),
                                    )
                                    .child(
                                        div().flex_1().child(Input::new(&video.keys.value).small()),
                                    ),
                            )
                        })
                        .children(curves)
                        .child(div().text_xs().text_color(rgb(FAINT)).child("Slow motion"))
                        .child(interpolation)
                        .child(
                            toggle("sound-follows", "Sound follows the speed", follows).on_click(
                                cx.listener(move |this, _, window, cx| {
                                    // The picture's sound clips follow it.
                                    let linked =
                                        this.video.as_ref().map_or(vec![id], |v| v.doc.linked(id));
                                    let commands = linked
                                        .into_iter()
                                        .map(|clip| set_clip(clip, None, Some(!follows)))
                                        .collect();
                                    this.video_commands(commands, window, cx)
                                }),
                            ),
                        )
                }),
        )
    }

    /// Paints a clip's velocity or remap line inside its box on the
    /// timeline, with its keyframes as dots.
    pub fn speed_line(
        &self,
        clip: &Clip,
        bounds: (f32, f32, f32, f32),
        selected: Option<usize>,
    ) -> Option<AnyElement> {
        let (dots, points) = self.speed_geometry(clip, bounds, selected)?;
        Some(paint_line(points, dots))
    }

    /// Where a speed line's points and keyframe dots are on the timeline.
    #[allow(clippy::type_complexity)]
    pub fn speed_geometry(
        &self,
        clip: &Clip,
        bounds: (f32, f32, f32, f32),
        selected: Option<usize>,
    ) -> Option<(Vec<(f32, f32, bool)>, Vec<(f32, f32)>)> {
        if clip.speed.is_normal() {
            return None;
        }
        let video = self.video.as_ref()?;
        let (left, top, width, height) = bounds;
        let zoom = video.zoom;
        let start = clip.start.seconds() - video.scroll;
        let origin = clip.speed.origin().seconds();
        let length = clip
            .source_duration
            .map_or(clip.duration.seconds(), |d| d.seconds())
            .max(1e-3);
        let remap = matches!(clip.speed, Speed::Remap { .. });
        let y_of = |offset: f64| -> f32 {
            let unit = if remap {
                clip.source_seconds(clip.start + Time::from_seconds(offset)) / length * 2. - 1.
            } else {
                velocity_height(clip.speed.rate(Time::from_seconds(offset)) * 100.)
            };
            top + height / 2. - unit as f32 * (height / 2. - 4.)
        };
        let x_of = |offset: f64| crate::video_timeline::HEADER + (start + offset) as f32 * zoom;
        let samples = ((width / 3.).max(2.)) as usize;
        let mut points = Vec::with_capacity(samples + 1);
        for i in 0..=samples {
            let offset = clip.duration.seconds() * i as f64 / samples as f64;
            points.push((x_of(offset), y_of(offset)));
        }
        let dots: Vec<(f32, f32, bool)> = keys(&clip.speed)
            .iter()
            .map(|(at, _, _)| at.seconds() - origin)
            .zip(0..)
            .filter(|(offset, _)| *offset >= 0. && *offset <= clip.duration.seconds())
            .map(|(offset, i)| (x_of(offset), y_of(offset), selected == Some(i)))
            .collect();
        let _ = left;
        Some((dots, points))
    }

    /// The keyframe of a clip's speed line under a timeline point, if any.
    pub fn speed_key_at(
        &self,
        clip: &Clip,
        bounds: (f32, f32, f32, f32),
        x: f32,
        y: f32,
    ) -> Option<usize> {
        let (dots, _) = self.speed_geometry(clip, bounds, None)?;
        let offsets: Vec<_> = keys(&clip.speed)
            .iter()
            .enumerate()
            .filter(|(_, k)| {
                let offset = k.0.seconds() - clip.speed.origin().seconds();
                offset >= 0. && offset <= clip.duration.seconds()
            })
            .map(|(i, _)| i)
            .collect();
        dots.iter()
            .zip(offsets)
            .find(|((dx, dy, _), _)| (dx - x).abs() <= 6. && (dy - y).abs() <= 6.)
            .map(|(_, index)| index)
    }

    /// The speed a keyframe drag makes: the key moved to timeline `at` and
    /// height `y` within the clip's box, kept between its neighbors.
    pub fn dragged_speed(
        &self,
        clip: &Clip,
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
        let value = match clip.speed {
            Speed::Remap { .. } => {
                let length = clip
                    .source_duration
                    .map_or(clip.duration.seconds(), |d| d.seconds());
                (unit + 1.) / 2. * length
            }
            _ => height_velocity(unit),
        };
        let origin = clip.speed.origin();
        // Up to its neighbors, which a freeze's keyframes share times with.
        let low = index.checked_sub(1).map_or(Time::ZERO, |i| list[i].0);
        let high = list.get(index + 1).map_or(Time(i64::MAX), |k| k.0);
        let at = (at - clip.start + origin).clamp(low, high);
        list[index] = (at, value, list[index].2);
        with_keys(&clip.speed, &list)
    }
}

fn paint_line(points: Vec<(f32, f32)>, dots: Vec<(f32, f32, bool)>) -> AnyElement {
    {
        {
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let at = |x: f32, y: f32| bounds.origin + point(px(x), px(y));
                    let mut line = PathBuilder::stroke(px(1.5));
                    for (i, (x, y)) in points.iter().enumerate() {
                        if i == 0 {
                            line.move_to(at(*x, *y));
                        } else {
                            line.line_to(at(*x, *y));
                        }
                    }
                    if let Ok(path) = line.build() {
                        window.paint_path(path, rgb(0xf2c14e));
                    }
                    for (x, y, on) in &dots {
                        let r = if *on { 5. } else { 4. };
                        window.paint_quad(fill(
                            Bounds::new(at(x - r, y - r), size(px(r * 2.), px(r * 2.))),
                            rgb(if *on { 0xffffff } else { 0xf2c14e }),
                        ));
                    }
                },
            )
            .absolute()
            .size_full()
            .into_any_element()
        }
    }
}

fn clip_interpolation(video: &spectrum_video::Video, clip: u64) -> Interpolation {
    video
        .clip(clip)
        .map_or(Interpolation::Motion, |(_, c)| c.interpolation)
}

fn set_clip(clip: u64, interpolation: Option<Interpolation>, follows: Option<bool>) -> Command {
    Command::SetClip {
        clip,
        name: None,
        enabled: None,
        opacity: None,
        gain_db: None,
        interpolation,
        sound_follows_speed: follows,
    }
}
