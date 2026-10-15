//! The Speed tab: which view the timing shows in (velocity or time remap,
//! the same timing either way), the selected clip's keyframes with the
//! chosen one's time, value, and curve typeable, its slow-motion frames, and
//! whether its sound follows. Actions (freeze, reverse, reset) are in the
//! command list and the clip's right-click menu.
use crate::{
    controls::{chip, group, toggle},
    theme::*,
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use gpui_component::input::{InputEvent, InputState};
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
                    let open = this.video.as_ref().and_then(|v| v.key_editor);
                    if open.is_some_and(|(time, _)| time == field) {
                        let text = state.read(cx).value().to_string();
                        this.type_key(field, &text, window, cx);
                        this.close_key_field(window, cx);
                    }
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
            .child(interpolation)
            .child(
                toggle("sound-follows", "Sound follows the speed", follows).on_click(cx.listener(
                    move |this, _, window, cx| this.set_speed_options(id, None, Some(!follows), window, cx),
                )),
            )
            .child(note(
                "Double-click the line on the clip to add a keyframe; right-click one for its curve, or to type its value or time.",
            ))
    }

    /// Opens a field at the chosen keyframe for its value (or its time),
    /// from its right-click menu.
    pub fn edit_key_field(&mut self, time: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((clip, index)) = self.selected_key() else {
            return;
        };
        let Some(video) = &self.video else {
            return;
        };
        let track = video.doc.clip(clip.id).map(|(t, _)| t.id);
        let row = track.and_then(|t| video.layout().row(t));
        let (left, right) = (video.x_of(clip.start), video.x_of(clip.end()));
        let place = row.and_then(|row| {
            let boxed = (left, row.top + 3., right - left, row.height - 6.);
            let geometry = self.speed_geometry(&clip, boxed, None)?;
            let at = geometry.indices.iter().position(|i| *i == index)?;
            let (x, y, _) = geometry.dots[at];
            Some((x, y))
        });
        let Some(place) = place else {
            return;
        };
        self.sync_key_fields(window, cx);
        let Some(video) = &mut self.video else {
            return;
        };
        video.key_editor = Some((time, place));
        let field = if time {
            &video.keys.time
        } else {
            &video.keys.value
        };
        field.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// Closes the keyframe field, back to the workspace's keys.
    pub fn close_key_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(video) = &mut self.video
            && video.key_editor.take().is_some()
        {
            self.focus_handle.focus(window);
            cx.notify();
        }
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
