//! The Speed tab: which view the timing shows in (velocity or time remap,
//! the same timing either way), the selected clip's keyframes with the
//! chosen one's time, value, and curve typeable, its slow-motion frames, and
//! whether its sound follows. Actions (freeze, reverse, reset) are in the
//! command list and the clip's right-click menu.
use crate::{
    controls::{chip, group, toggle},
    theme::*,
    video_speed::{key_value, keys},
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use gpui_component::Sizable;
use gpui_component::input::{Input, InputEvent, InputState};
use spectrum_media::Time;
use spectrum_video::{Command, Curve, Interpolation, SpeedForm};

/// The curves a keyframe can take, as Vegas names them.
pub const CURVES: [(Curve, &str, &str); 6] = [
    (Curve::Smooth, "Smooth", "curve-smooth"),
    (Curve::Linear, "Linear", "curve-linear"),
    (Curve::Fast, "Fast", "curve-fast"),
    (Curve::Slow, "Slow", "curve-slow"),
    (Curve::Sharp, "Sharp", "curve-sharp"),
    (Curve::Hold, "Hold", "curve-hold"),
];

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

impl Workspace {
    pub fn video_speed_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let Some(video) = &self.video else {
            return div();
        };
        let view = video.speed_view;
        let views = div().flex().gap_2().children(
            [
                (SpeedForm::Velocity, "speed-velocity", "Velocity"),
                (SpeedForm::Remap, "speed-remap", "Time remap"),
            ]
            .map(|(form, id, label)| {
                chip(id, label, view == form).on_click(
                    cx.listener(move |this, _, window, cx| this.set_speed_view(form, window, cx)),
                )
            }),
        );
        let shown = group("View", None).gap_2().child(views).child(note(
            "The same timing either way; switching changes nothing.",
        ));
        let Some(clip) = self.speed_clip() else {
            return div()
                .flex()
                .flex_col()
                .gap_7()
                .child(shown)
                .child(note("Select a clip to see its speed."));
        };
        let rate = video.doc.rate;
        let selected = video
            .key
            .filter(|(id, i)| *id == clip.id && *i < keys(&clip.speed).len())
            .map(|(_, i)| i);
        let list = keys(&clip.speed);
        let rows = list.iter().enumerate().map(|(index, (at, _, curve))| {
            let at = (*at - clip.speed.origin()).timecode(rate);
            let value = match (view, key_value(&clip, view, index)) {
                (SpeedForm::Remap, Some(source)) => Time::from_seconds(source).timecode(rate),
                (SpeedForm::Velocity, Some(percent)) => format!("{percent:.0}%"),
                (_, None) => String::new(),
            };
            let curve = CURVES
                .iter()
                .find(|(c, ..)| c == curve)
                .map_or("", |(_, name, _)| name);
            let id = clip.id;
            div()
                .id(("key", index))
                .px_2()
                .py_1()
                .rounded_md()
                .flex()
                .gap_2()
                .text_sm()
                .when(selected == Some(index), |el| el.bg(rgb(SELECTED)))
                .hover(|el| el.bg(rgb(HOVER)))
                .child(div().flex_1().child(at))
                .child(div().w(px(84.)).text_color(rgb(MUTED)).child(value))
                .child(
                    div()
                        .w(px(52.))
                        .text_xs()
                        .text_color(rgb(FAINT))
                        .child(curve),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(video) = &mut this.video {
                        video.key = Some((id, index));
                    }
                    this.sync_key_fields(window, cx);
                    cx.notify();
                }))
        });
        let keyframes = if list.is_empty() {
            group("Keyframes", None).child(note(
                "Plays at the source's speed. Double-click the line on the clip to add a keyframe.",
            ))
        } else {
            group("Keyframes", None)
                .gap_2()
                .child(div().flex().flex_col().gap_0p5().children(rows))
                .when_some(selected, |el, index| {
                    let current = list[index].2;
                    el.child(
                        div()
                            .flex()
                            .gap_2()
                            .child(div().flex_1().child(Input::new(&video.keys.time).small()))
                            .child(div().flex_1().child(Input::new(&video.keys.value).small())),
                    )
                    .child(div().flex().flex_wrap().gap_2().children(CURVES.map(
                        |(curve, label, id)| {
                            chip(id, label, current == curve).on_click(cx.listener(
                                move |this, _, window, cx| this.set_key_curve(curve, window, cx),
                            ))
                        },
                    )))
                })
                .child(note(
                    "Right-click a keyframe on the clip for its curve, or to delete it.",
                ))
        };
        let method = clip.interpolation;
        let interpolation = group("Slow motion", None).gap_2().child(
            div().flex().gap_2().children(
                [
                    (Interpolation::Off, "interp-off", "Off"),
                    (Interpolation::Blend, "interp-blend", "Blend"),
                    (Interpolation::Motion, "interp-motion", "Motion"),
                ]
                .map(|(choice, id, label)| {
                    let clip = clip.id;
                    chip(id, label, method == choice).on_click(cx.listener(
                        move |this, _, window, cx| {
                            this.set_speed_options(clip, Some(choice), None, window, cx)
                        },
                    ))
                }),
            ),
        );
        let follows = clip.sound_follows_speed;
        let id = clip.id;
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(shown)
            .child(keyframes)
            .child(interpolation)
            .child(
                toggle("sound-follows", "Sound follows the speed", follows).on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.set_speed_options(id, None, Some(!follows), window, cx)
                    },
                )),
            )
    }

    /// Sets a clip's slow-motion frames or whether its sound follows, on it
    /// and the clips linked to it.
    fn set_speed_options(
        &mut self,
        clip: u64,
        interpolation: Option<Interpolation>,
        follows: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let linked = self
            .video
            .as_ref()
            .map_or(vec![clip], |v| v.doc.linked(clip));
        let commands = linked
            .into_iter()
            .map(|clip| Command::SetClip {
                clip,
                name: None,
                enabled: None,
                opacity: None,
                gain_db: None,
                interpolation,
                sound_follows_speed: follows,
            })
            .collect();
        self.video_commands(commands, window, cx)
    }
}

fn note(text: &'static str) -> Div {
    div().text_xs().text_color(rgb(FAINT)).child(text)
}
