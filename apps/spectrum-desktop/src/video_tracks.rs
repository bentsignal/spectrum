//! Drawing the timeline: the grip, the ruler, the tracks (clipped, so
//! clips and their lines never draw over the track names or the ruler), the
//! playhead, the razor's mark, and where a moved track would go.
use crate::{
    theme::*,
    video_layout::{GRIP, HEADER, Layout, PANEL_MIN, RULER, TOP, layout, panel_most},
    video_state::{DragKind, Tool},
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use gpui_component::menu::ContextMenuExt;
use spectrum_media::Time;
use spectrum_video::{Command, TrackKind};

impl Workspace {
    pub fn timeline(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(video) = &self.video else {
            return div().into_any_element();
        };
        let height = video.panel.clamp(PANEL_MIN, panel_most(window));
        let layout = layout(&video.doc, video, height);
        let bounds = video.timeline.clone();
        let width = f32::from(bounds.borrow().size.width).max(400.);
        let visible = (
            video.scroll,
            video.scroll + f64::from((width - HEADER) / video.zoom),
        );
        let razor = video.tool == Tool::Razor;
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
            .child(
                // The grip: drag to give the timeline more or less room;
                // a double-click resets it.
                div()
                    .id("timeline-grip")
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(GRIP + 3.))
                    .bg(rgb(SIDEBAR))
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .flex()
                    .justify_center()
                    .pt(px(3.))
                    .cursor(CursorStyle::ResizeRow)
                    .child(div().w(px(36.)).h(px(3.)).rounded_full().bg(rgb(FAINT))),
            )
            .child(self.ruler(width, visible))
            .child(self.tracks(&layout, width, visible, cx));
        let playhead = self.timeline_x(video.monitor.now());
        if playhead >= HEADER {
            layer = layer.child(
                div()
                    .absolute()
                    .top(px(GRIP))
                    .bottom_0()
                    .left(px(playhead))
                    .w(px(1.5))
                    .bg(rgb(0xff5a5a)),
            );
        }
        // The razor in place of the pointer over the tracks.
        if let Some((x, y)) = video.hover.filter(|_| razor).map(|p| video.local(p))
            && x >= HEADER
            && y >= TOP
        {
            layer = layer.child(
                div()
                    .absolute()
                    .left(px(x - 7.))
                    .top(px(y - 7.))
                    .size(px(14.))
                    .text_color(rgb(TEXT))
                    .child(crate::icons::icon("scissors").size(px(14.))),
            );
        }
        // Where a moved track would go.
        if let (Some(y), Some(DragKind::TrackOrder { .. })) =
            (video.track_drop, video.drag.as_ref().map(|d| d.kind))
        {
            let place = layout.place_at(y);
            let index = layout.rows.len().saturating_sub(place + 1);
            let line = layout.rows.get(index).map_or(TOP, |r| {
                if y < r.top + r.height / 2. {
                    r.top
                } else {
                    r.top + r.height
                }
            });
            layer = layer.child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(line - 1.))
                    .w_full()
                    .h(px(2.))
                    .bg(rgb(0x5b8def)),
            );
        }
        // While a drag runs, it follows the pointer anywhere in the window,
        // however the timeline moves under it.
        if video.drag.is_some() {
            let view = cx.entity().downgrade();
            layer = layer.child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        let moved = view.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase.bubble() {
                                moved
                                    .update(cx, |this, cx| this.timeline_move(event, window, cx))
                                    .ok();
                            }
                        });
                        let released = view.clone();
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                            if phase.bubble() {
                                released
                                    .update(cx, |this, cx| this.timeline_release(window, cx))
                                    .ok();
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            );
        }
        let menu = self.timeline_menu(cx);
        layer
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.timeline_down(event, window, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.timeline_right_down(event, window, cx)
                }),
            )
            // Drags are followed window-wide (above); here only the
            // razor's mark follows the pointer.
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if this.video.as_ref().is_some_and(|v| v.drag.is_none()) {
                    this.timeline_move(event, window, cx)
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.timeline_release(window, cx)),
            )
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered && let Some(video) = &mut this.video {
                    video.hover = None;
                    cx.notify();
                }
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                this.timeline_wheel(event, window, cx)
            }))
            .on_drop(cx.listener(
                |this, media: &crate::video_sidebar::DraggedMedia, window, cx| {
                    this.drop_media(media.0, window.mouse_position(), window, cx)
                },
            ))
            .context_menu(menu)
            .into_any_element()
    }

    /// The tracks: names on the left, clips to the right, clipped to the
    /// tracks' area.
    fn tracks(
        &self,
        layout: &Layout,
        width: f32,
        visible: (f64, f64),
        cx: &mut Context<Self>,
    ) -> Div {
        let video = self.video.as_ref().expect("video");
        let doc = &video.doc;
        let (top, bottom) = layout.area();
        let razor = video.tool == Tool::Razor;
        let hover = video.hover.filter(|_| razor).map(|p| video.local(p));
        let mut headers = div().absolute().left_0().top_0().w(px(HEADER)).h_full();
        let mut content = div()
            .absolute()
            .left(px(HEADER))
            .right_0()
            .top_0()
            .bottom_0()
            .overflow_hidden()
            .when(razor, |el| el.cursor(CursorStyle::None));
        // Lines and waveforms paint in timeline points, offset back here.
        let mut overlays = div()
            .absolute()
            .left(px(-HEADER))
            .top(px(-top))
            .w(px(width))
            .h(px(layout.height));
        for row in &layout.rows {
            let kind = row.kind;
            let Some(track) = doc.track(row.track) else {
                continue;
            };
            let y = row.top - top;
            content = content.child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(y))
                    .w_full()
                    .h(px(row.height))
                    .border_b_1()
                    .border_color(rgb(BORDER)),
            );
            for clip in track
                .clips
                .iter()
                .filter(|c| c.end().seconds() >= visible.0 && c.start.seconds() <= visible.1)
            {
                let left = self.timeline_x(clip.start);
                let right = self.timeline_x(clip.end());
                let shown_left = left.max(HEADER - 4.);
                let selected = video.selected.contains(&clip.id);
                let fill = match (kind, clip.enabled) {
                    (_, false) => 0x262626,
                    (TrackKind::Video, _) => 0x36404f,
                    (TrackKind::Audio, _) => 0x34473d,
                };
                content = content.child(
                    div()
                        .absolute()
                        .left(px(shown_left - HEADER))
                        .top(px(y + 3.))
                        .w(px((right - shown_left).max(2.)))
                        .h(px(row.height - 6.))
                        .rounded(px(4.))
                        .bg(rgb(fill))
                        .border_1()
                        .border_color(rgb(if selected { 0xececec } else { 0x4a4a4a }))
                        .px_1p5()
                        .py_1()
                        .overflow_hidden()
                        .text_xs()
                        .text_color(rgb(TEXT))
                        .flex()
                        .gap_1p5()
                        .children(self.clip_picture(clip).map(|path| {
                            img(path)
                                .h_full()
                                .flex_none()
                                .rounded_sm()
                                .object_fit(ObjectFit::Cover)
                                .w(px(((row.height - 14.) * 16. / 9.)
                                    .min(right - shown_left - 12.)
                                    .max(0.)))
                        }))
                        .child(div().truncate().child(clip.name.clone())),
                );
                let boxed = (left, row.top + 3., right, row.height - 6.);
                if let Some(waveform) = self.clip_waveform(clip, boxed) {
                    overlays = overlays.child(waveform);
                }
                let is_speed_clip = self.speed_clip().is_some_and(|c| c.id == clip.id);
                if is_speed_clip {
                    let key = video.key.filter(|(id, _)| *id == clip.id).map(|(_, i)| i);
                    let boxed = (left, row.top + 3., right - left, row.height - 6.);
                    if let Some(line) = self.speed_line(clip, boxed, key) {
                        overlays = overlays.child(line);
                    }
                }
                // Where the razor would cut this clip.
                if let Some((x, hy)) = hover
                    && hy >= row.top
                    && hy < row.top + row.height
                    && x >= left
                    && x <= right
                {
                    let at = doc.snap(video.time_at(x));
                    let cut = self.timeline_x(at);
                    content = content.child(
                        div()
                            .absolute()
                            .left(px(cut - HEADER))
                            .top(px(y + 1.))
                            .w(px(1.))
                            .h(px(row.height - 2.))
                            .bg(rgb(0xffffff)),
                    );
                }
            }
            headers = headers.child(self.track_header(track, kind, y, row.height, cx));
            // The top edge drags the row taller or shorter.
            let edge = y - 4.;
            headers = headers.child(
                div()
                    .id(("row-edge", row.track))
                    .absolute()
                    .left_0()
                    .top(px(edge))
                    .w(px(HEADER))
                    .h(px(8.))
                    .cursor(CursorStyle::ResizeRow),
            );
        }
        div()
            .absolute()
            .left_0()
            .top(px(top))
            .w_full()
            .h(px((bottom - top).max(0.)))
            .overflow_hidden()
            .child(content.child(overlays))
            .child(headers)
    }

    fn track_header(
        &self,
        track: &spectrum_video::Track,
        kind: TrackKind,
        y: f32,
        height: f32,
        cx: &mut Context<Self>,
    ) -> Div {
        let (muted, locked, id) = (track.muted, track.locked, track.id);
        div()
            .absolute()
            .left_0()
            .top(px(y))
            .w(px(HEADER))
            .h(px(height))
            .bg(rgb(SIDEBAR))
            .border_r_1()
            .border_b_1()
            .border_color(rgb(BORDER))
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .text_xs()
            .child(
                div()
                    .w(px(22.))
                    .text_color(rgb(MUTED))
                    .child(track.name.clone()),
            )
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
            ))
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
            .top(px(GRIP))
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
        // In to Out, shaded.
        if video.in_point.is_some() || video.out_point.is_some() {
            let left = self
                .timeline_x(video.in_point.unwrap_or(Time::ZERO))
                .max(HEADER);
            let right = self
                .timeline_x(video.out_point.unwrap_or(video.doc.duration()))
                .min(width);
            if right > left {
                ruler = ruler.child(
                    div()
                        .absolute()
                        .left(px(left))
                        .top_0()
                        .w(px(right - left))
                        .h(px(RULER - 1.))
                        .bg(rgba(0x5b8def30))
                        .border_l_2()
                        .border_r_2()
                        .border_color(rgb(0x5b8def)),
                );
            }
        }
        // What is rendered ahead, in green along the bottom.
        for (left, right) in video.rendered_strip(width) {
            ruler = ruler.child(
                div()
                    .absolute()
                    .left(px(left))
                    .bottom(px(1.))
                    .w(px(right - left))
                    .h(px(3.))
                    .bg(rgb(0x4caf6a)),
            );
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
