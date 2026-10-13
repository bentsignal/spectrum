//! The player on screen: the picture (or the sound's waveform) filling the
//! main area, and below it the play button, a typeable timecode, and a scrub
//! bar. Info mode shows what the file holds and which sound track plays.
use crate::{controls::group, info::row, theme::*, workspace::Workspace};
use gpui::{prelude::*, *};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use spectrum_media::Time;
use std::{cell::RefCell, rc::Rc};

const TRANSPORT_HEIGHT: f32 = 56.;

/// The player's controls that live on screen.
pub struct PlayerControls {
    pub timecode: Entity<InputState>,
    pub scrub: Rc<RefCell<Bounds<Pixels>>>,
    pub scrubbing: bool,
    /// The waveform of a sound without picture, once drawn.
    pub waveform: Option<std::sync::Arc<RenderImage>>,
    /// The timecode last put in the field, so it is set only on change.
    pub shown: Option<String>,
    _typed: Subscription,
}

impl PlayerControls {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let timecode = cx.new(|cx| InputState::new(window, cx).placeholder("00:00:00:00"));
        let typed = cx.subscribe_in(
            &timecode,
            window,
            |this: &mut Workspace, state, event: &InputEvent, window, cx| {
                if !matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    return;
                }
                if matches!(event, InputEvent::PressEnter { .. }) {
                    // Back to the player, so Space plays again.
                    this.focus_handle.focus(window);
                }
                let text = state.read(cx).value().to_string();
                let rate = this
                    .player
                    .as_ref()
                    .and_then(|p| p.media.info.video.as_ref().map(|v| v.rate));
                if let Ok(time) = Time::parse(&text, rate) {
                    this.seek_player(time, cx);
                }
                if let Some(controls) = &mut this.player_controls {
                    controls.shown = None;
                }
            },
        );
        Self {
            timecode,
            scrub: Default::default(),
            scrubbing: false,
            waveform: None,
            shown: None,
            _typed: typed,
        }
    }
}

impl Workspace {
    pub fn player_main(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(player) = &self.player else {
            return div().into_any_element();
        };
        // The timecode field follows the playhead unless someone is typing.
        let now = player.now();
        let text = player.timecode(now);
        let controls = self
            .player_controls
            .get_or_insert_with(|| crate::player_view::PlayerControls::new(window, cx));
        let typing = controls
            .timecode
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if !typing && controls.shown.as_deref() != Some(text.as_str()) {
            controls
                .timecode
                .update(cx, |state, cx| state.set_value(text.clone(), window, cx));
            controls.shown = Some(text);
        }
        let Some(player) = &self.player else {
            return div().into_any_element();
        };
        let controls = self.player_controls.as_ref().expect("player controls");
        let duration = player.duration();
        let share = (now.seconds() / duration.seconds().max(1e-9)).clamp(0., 1.) as f32;
        let picture = if player.has_picture() {
            div()
                .size_full()
                .children(player.shown.as_ref().map(|(image, _)| {
                    img(image.clone())
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                }))
        } else {
            self.waveform_view(share)
        };
        let bounds = controls.scrub.clone();
        let scrub = div()
            .id("scrub")
            .flex_1()
            .h(px(28.))
            .flex()
            .items_center()
            .cursor_pointer()
            .child(
                canvas(move |b, _, _| *bounds.borrow_mut() = b, |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(4.))
                    .rounded_full()
                    .bg(rgb(BORDER))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .h_full()
                            .w(relative(share))
                            .rounded_full()
                            .bg(rgb(MUTED)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(-5.))
                            .left(relative(share))
                            .ml(px(-7.))
                            .size(px(14.))
                            .rounded_full()
                            .bg(rgb(TEXT)),
                    ),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    if let Some(controls) = &mut this.player_controls {
                        controls.scrubbing = true;
                    }
                    this.scrub_to(event.position, cx);
                }),
            );
        let playing = player.playing();
        let transport = div()
            .h(px(TRANSPORT_HEIGHT))
            .flex_shrink_0()
            .px_6()
            .flex()
            .items_center()
            .gap_4()
            .border_t_1()
            .border_color(rgb(BORDER))
            .child(
                Button::new("play")
                    .ghost()
                    .icon(if playing {
                        crate::icons::icon("pause")
                    } else {
                        crate::icons::icon("play")
                    })
                    .tooltip(if playing {
                        "Pause (Space)"
                    } else {
                        "Play (Space)"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_play(cx);
                    })),
            )
            .child(
                div()
                    .w(px(120.))
                    .child(Input::new(&controls.timecode).small()),
            )
            .child(scrub)
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(FAINT))
                    .child(player.timecode(duration)),
            )
            .when(!player.media.info.audio.is_empty(), |el| {
                el.child(
                    Button::new("mute")
                        .ghost()
                        .small()
                        .icon(crate::icons::icon(if player.muted {
                            "volume-off"
                        } else {
                            "volume"
                        }))
                        .tooltip(if player.muted { "Unmute" } else { "Mute" })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_mute(cx))),
                )
            });
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().bg(rgb(0x000000)).child(picture))
            .child(transport)
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.player_controls.as_ref().is_some_and(|c| c.scrubbing) {
                    if event.pressed_button == Some(MouseButton::Left) {
                        this.scrub_to(event.position, cx);
                    } else if let Some(controls) = &mut this.player_controls {
                        controls.scrubbing = false;
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    if let Some(controls) = &mut this.player_controls {
                        controls.scrubbing = false;
                    }
                }),
            )
            .into_any_element()
    }

    /// Moves the playhead to the scrub bar's point under `position`.
    fn scrub_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let (Some(controls), Some(player)) = (&self.player_controls, &self.player) else {
            return;
        };
        let bounds = *controls.scrub.borrow();
        let width = f32::from(bounds.size.width).max(1.);
        let share = (f32::from(position.x - bounds.origin.x) / width).clamp(0., 1.);
        let time = Time((player.duration().0 as f64 * f64::from(share)) as i64);
        self.seek_player(time, cx);
    }

    fn waveform_view(&self, share: f32) -> Div {
        let waveform = self
            .player_controls
            .as_ref()
            .and_then(|controls| controls.waveform.clone());
        div().size_full().flex().items_center().p_8().child(
            div()
                .relative()
                .w_full()
                .h(px(320.))
                .children(waveform.map(|image| img(image).size_full().object_fit(ObjectFit::Fill)))
                .child(
                    div()
                        .absolute()
                        .top(px(-24.))
                        .bottom(px(-24.))
                        .left(relative(share))
                        .w(px(2.))
                        .bg(rgb(TEXT)),
                ),
        )
    }

    /// The header's detail for open footage or audio.
    pub fn media_detail(&self) -> SharedString {
        let Some(player) = &self.player else {
            return SharedString::default();
        };
        match &player.media.info.video {
            Some(video) => format!(
                "Footage · {} × {} · {}",
                video.width, video.height, video.rate
            )
            .into(),
            None => "Audio".into(),
        }
    }

    /// Info mode for footage and audio: what the file holds.
    pub fn media_info_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let Some(player) = &self.player else {
            return div();
        };
        let info = &player.media.info;
        let megabytes = player.media.bytes as f64 / 1_048_576.;
        let size = if megabytes >= 1024. {
            format!("{:.1} GB", megabytes / 1024.)
        } else if megabytes >= 1. {
            format!("{megabytes:.0} MB")
        } else {
            format!("{:.0} KB", megabytes * 1024.)
        };
        let file = group("File", None)
            .gap_2()
            .child(row("Name", player.media.original_name.clone()))
            .child(row("Duration", player.timecode(info.duration)))
            .child(row("Size", size))
            .child(row(
                "Format",
                format_name(&player.media.original_name, &info.container),
            ));
        let playback = match self
            .store
            .as_ref()
            .ok()
            .and_then(|s| s.copies.progress(player.id))
        {
            Some(share) => format!("Preparing {:.0}%", share * 100.),
            None if player.uses_copy() => "Playback copy".to_string(),
            None => "Original".to_string(),
        };
        let picture = info.video.as_ref().map(|video| {
            let dynamic = match video.transfer.as_str() {
                "arib-std-b67" => "HLG",
                "smpte2084" => "PQ (HDR10)",
                _ => "Standard",
            };
            group("Picture", None)
                .gap_2()
                .child(row("Size", format!("{} × {}", video.width, video.height)))
                .child(row("Frame rate", video.rate.to_string()))
                .child(row("Frames", video.frames.to_string()))
                .child(row("Codec", codec_name(&video.codec)))
                .child(row(
                    "Color",
                    format!("{}-bit {}", video.bit_depth, chroma(&video.pixel_format)),
                ))
                .child(row("Range", dynamic.to_string()))
                .child(row("Playback", playback.clone()))
        });
        let tracks = info.audio.iter().map(|track| {
            let label = track
                .title
                .clone()
                .unwrap_or_else(|| format!("Track {}", track.track + 1));
            let detail = format!(
                "{} · {} kHz · {}",
                codec_name(&track.codec),
                track.sample_rate as f32 / 1000.,
                match track.channels {
                    1 => "Mono".to_string(),
                    2 => "Stereo".to_string(),
                    n => format!("{n} channels"),
                }
            );
            let index = track.track;
            let chosen = player.track == index;
            div()
                .id(SharedString::from(format!("track-{index}")))
                .px_2()
                .py_1p5()
                .rounded_md()
                .flex()
                .flex_col()
                .gap_0p5()
                .when(chosen, |el| el.bg(rgb(HOVER)))
                .hover(|el| el.bg(rgb(HOVER)))
                .child(div().text_sm().child(label))
                .child(div().text_xs().text_color(rgb(FAINT)).child(detail))
                .on_click(cx.listener(move |this, _, _, cx| this.choose_track(index, cx)))
        });
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(file)
            .children(picture)
            .when(!info.audio.is_empty(), |el| {
                el.child(group("Sound", None).gap_1().children(tracks))
            })
    }
}

/// A codec as people know it: "PCM 24-bit" for "pcm_s24le".
fn codec_name(codec: &str) -> String {
    if let Some(rest) = codec.strip_prefix("pcm_") {
        let bits: String = rest.chars().filter(char::is_ascii_digit).collect();
        let float = if rest.starts_with('f') { " float" } else { "" };
        return format!("PCM {bits}-bit{float}");
    }
    match codec {
        "h264" => "H.264".into(),
        "hevc" => "HEVC (H.265)".into(),
        "prores" => "ProRes".into(),
        "mp3" => "MP3".into(),
        "vorbis" => "Vorbis".into(),
        "opus" => "Opus".into(),
        "flac" => "FLAC".into(),
        "alac" => "Apple Lossless".into(),
        other => other.to_uppercase(),
    }
}

/// The file's format: its extension, or the container when it has none.
fn format_name(original: &str, container: &str) -> String {
    std::path::Path::new(original)
        .extension()
        .map(|extension| extension.to_string_lossy().to_uppercase())
        .unwrap_or_else(|| container.split(',').next().unwrap_or("").to_uppercase())
}

/// "4:2:2" from a pixel format such as "yuv422p10le".
fn chroma(format: &str) -> &'static str {
    if format.contains("444") || format.starts_with("rgb") || format.starts_with("gbr") {
        "4:4:4"
    } else if format.contains("422") {
        "4:2:2"
    } else {
        "4:2:0"
    }
}

/// Draws a sound's waveform for the player, on a background thread.
pub fn draw_waveform(
    file: &std::path::Path,
    track: usize,
    duration: Time,
) -> anyhow::Result<std::sync::Arc<RenderImage>> {
    let (width, height) = (2048u32, 320u32);
    let peaks = spectrum_media::waveform(file, track, duration, width as usize / 2)?;
    // Already blue, green, red, alpha, as the screen takes it.
    let mut picture = image::RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
    let middle = height as f32 / 2.;
    for (bar, peak) in peaks.iter().enumerate() {
        let reach = (peak.sqrt() * middle * 0.95).max(1.);
        for y in (middle - reach) as u32..((middle + reach) as u32).min(height) {
            for x in [bar as u32 * 2, bar as u32 * 2 + 1] {
                picture.put_pixel(x, y, image::Rgba([150, 150, 150, 255]));
            }
        }
    }
    Ok(std::sync::Arc::new(RenderImage::new(vec![
        image::Frame::new(picture),
    ])))
}
