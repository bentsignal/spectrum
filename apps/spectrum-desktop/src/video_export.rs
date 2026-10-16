//! The Video tab (the video's own size, frame rate, and background) and the
//! Export tab (format, size, frame rate, bitrate, sound, and range), with
//! searchable lists for anything longer than a few choices and every number
//! typeable.
use crate::{
    controls::{chip, group, toggle},
    theme::*,
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use gpui_component::{
    IndexPath, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
};
use spectrum_media::{ExportSettings, FrameRate, VideoCodec};
use spectrum_video::Command;

type List = Entity<SelectState<SearchableVec<SharedString>>>;

const SAME: &str = "Same as the video";

const CODECS: [(VideoCodec, &str); 3] = [
    (VideoCodec::H264, "H.264 · MP4 (plays everywhere)"),
    (VideoCodec::Hevc, "HEVC (H.265) · MP4 (smaller)"),
    (VideoCodec::ProRes, "ProRes 422 HQ · MOV (for more editing)"),
];

const SIZES: [(&str, u32, u32); 10] = [
    ("3840 × 2160 · 4K UHD", 3840, 2160),
    ("4096 × 2160 · DCI 4K", 4096, 2160),
    ("2560 × 1440 · QHD", 2560, 1440),
    ("1920 × 1080 · Full HD", 1920, 1080),
    ("1280 × 720 · HD", 1280, 720),
    ("854 × 480 · SD", 854, 480),
    ("1080 × 1920 · Vertical", 1080, 1920),
    ("2160 × 3840 · Vertical 4K", 2160, 3840),
    ("1080 × 1080 · Square", 1080, 1080),
    ("1080 × 1350 · Portrait 4:5", 1080, 1350),
];

const RATES: [&str; 10] = [
    "23.976", "24", "25", "29.97", "30", "48", "50", "59.94", "60", "120",
];

/// The Video and Export tabs' fields, made with the editor.
pub struct Forms {
    codec: List,
    size: List,
    rate: List,
    width: Entity<InputState>,
    height: Entity<InputState>,
    bitrate: Entity<InputState>,
    /// Frames a second, typed, over the list.
    fps: Entity<InputState>,
    pub sound: bool,
    pub in_to_out: bool,
    /// Sizes keep the video's shape as either side is typed.
    pub export_locked: bool,
    pub video_locked: bool,
    video_fps: Entity<InputState>,
    /// The Video tab's.
    video_size: List,
    video_rate: List,
    video_width: Entity<InputState>,
    video_height: Entity<InputState>,
    /// The canvas's color picker, for the background.
    pub(crate) background: Entity<crate::color_picker::ColorPicker>,
    _subscriptions: Vec<Subscription>,
}

fn list(
    items: Vec<SharedString>,
    selected: usize,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> List {
    cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(items),
            Some(IndexPath::new(selected)),
            window,
            cx,
        )
        .searchable(true)
    })
}

fn field(
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

/// A typed size: an even number of pixels, 2 to 16384.
fn pixels(text: &str) -> Option<u32> {
    text.trim()
        .parse::<u32>()
        .ok()
        .filter(|n| (2..=16384).contains(n))
        .map(|n| n & !1)
}

impl Forms {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let codec = list(CODECS.map(|(_, name)| name.into()).to_vec(), 0, window, cx);
        let mut sizes: Vec<SharedString> = vec![SAME.into()];
        sizes.extend(SIZES.map(|(name, ..)| SharedString::from(name)));
        let size = list(sizes, 0, window, cx);
        let mut rates: Vec<SharedString> = vec![SAME.into()];
        rates.extend(RATES.map(|r| SharedString::from(format!("{r} fps"))));
        let rate = list(rates, 0, window, cx);
        let video_size = list(SIZES.map(|(name, ..)| name.into()).to_vec(), 3, window, cx);
        let video_rate = list(
            RATES
                .map(|r| SharedString::from(format!("{r} fps")))
                .to_vec(),
            4,
            window,
            cx,
        );
        let (width, height) = (field("Width", window, cx), field("Height", window, cx));
        let bitrate = field("Automatic", window, cx);
        let fps = field("Or type: 47.952, 30000/1001", window, cx);
        let video_fps = field("Or type: 47.952, 30000/1001", window, cx);
        let (video_width, video_height) = (field("Width", window, cx), field("Height", window, cx));
        let background = crate::color_picker::ColorPicker::new([0, 0, 0, 255], window, cx);
        let subscriptions = vec![
            // A size from the list fills the fields.
            cx.subscribe_in(
                &size,
                window,
                |this, _, event: &SelectEvent<_>, window, cx| {
                    if let SelectEvent::Confirm(Some(name)) = event {
                        this.export_size_chosen(name, window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &video_size,
                window,
                |this, _, event: &SelectEvent<_>, window, cx| {
                    if let SelectEvent::Confirm(Some(name)) = event
                        && let Some((_, w, h)) = SIZES.iter().find(|(n, ..)| *n == name.as_ref())
                    {
                        this.set_video_format(Some((*w, *h)), None, window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &video_rate,
                window,
                |this, _, event: &SelectEvent<_>, window, cx| {
                    if let SelectEvent::Confirm(Some(name)) = event
                        && let Ok(rate) = FrameRate::from_text(name)
                    {
                        this.set_video_format(None, Some(rate), window, cx);
                    }
                },
            ),
            cx.subscribe_in(&video_width, window, |this, _, event, window, cx| {
                Self::typed_size(this, true, true, event, window, cx)
            }),
            cx.subscribe_in(&video_height, window, |this, _, event, window, cx| {
                Self::typed_size(this, true, false, event, window, cx)
            }),
            cx.subscribe_in(&width, window, |this, _, event, window, cx| {
                Self::typed_size(this, false, true, event, window, cx)
            }),
            cx.subscribe_in(&height, window, |this, _, event, window, cx| {
                Self::typed_size(this, false, false, event, window, cx)
            }),
            cx.subscribe_in(
                &video_fps,
                window,
                |this, state, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.focus_handle.focus(window);
                        let text = state.read(cx).value().to_string();
                        if let Ok(rate) = FrameRate::from_text(&text) {
                            this.set_video_format(None, Some(rate), window, cx);
                        }
                    }
                },
            ),
            cx.subscribe_in(
                &background,
                window,
                |this, _, crate::color_picker::Picked([r, g, b, _]), window, cx| {
                    let color = [*r, *g, *b];
                    if this
                        .video
                        .as_ref()
                        .is_some_and(|v| v.doc.background != color)
                    {
                        this.video_commands(vec![Command::SetBackground { color }], window, cx);
                    }
                },
            ),
        ];
        Self {
            codec,
            size,
            rate,
            width,
            height,
            bitrate,
            fps,
            video_fps,
            sound: true,
            in_to_out: false,
            export_locked: true,
            video_locked: true,
            video_size,
            video_rate,
            video_width,
            video_height,
            background,
            _subscriptions: subscriptions,
        }
    }

    /// A width or height typed in the Video tab (`video`) or the Export
    /// tab: with the shape kept, the other side follows; the Video tab then
    /// applies the size.
    fn typed_size(
        this: &mut Workspace,
        video: bool,
        width_typed: bool,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        if !matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
            return;
        }
        if matches!(event, InputEvent::PressEnter { .. }) {
            this.focus_handle.focus(window);
        }
        let Some(editor) = this.video.as_ref() else {
            return;
        };
        let forms = &editor.forms;
        let ratio = f64::from(editor.doc.width) / f64::from(editor.doc.height.max(1));
        let (width_field, height_field, locked) = if video {
            (&forms.video_width, &forms.video_height, forms.video_locked)
        } else {
            (&forms.width, &forms.height, forms.export_locked)
        };
        let mut width = pixels(&width_field.read(cx).value());
        let mut height = pixels(&height_field.read(cx).value());
        if locked {
            if width_typed {
                height = width.map(|w| ((f64::from(w) / ratio).round() as u32).max(2) & !1);
            } else {
                width = height.map(|h| ((f64::from(h) * ratio).round() as u32).max(2) & !1);
            }
            let (width_field, height_field) = (width_field.clone(), height_field.clone());
            if let (Some(w), Some(h)) = (width, height) {
                width_field.update(cx, |s, cx| s.set_value(w.to_string(), window, cx));
                height_field.update(cx, |s, cx| s.set_value(h.to_string(), window, cx));
            }
        }
        if video {
            match width.zip(height) {
                Some(size) => this.set_video_format(Some(size), None, window, cx),
                None => this.sync_video_settings(window, cx),
            }
        }
    }
}

/// A size's shape, as people say it: 16:9, 4:5, or 1.90:1.
pub fn aspect(width: u32, height: u32) -> String {
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let d = gcd(width, height).max(1);
    let (w, h) = (width / d, height / d);
    if w <= 32 && h <= 32 {
        format!("{w}:{h}")
    } else {
        format!("{:.2}:1", f64::from(width) / f64::from(height.max(1)))
    }
}

impl Workspace {
    fn export_size_chosen(
        &mut self,
        name: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(video) = &self.video else {
            return;
        };
        let (w, h) = SIZES
            .iter()
            .find(|(n, ..)| *n == name.as_ref())
            .map_or((video.doc.width, video.doc.height), |(_, w, h)| (*w, *h));
        let forms = &video.forms;
        forms
            .width
            .update(cx, |s, cx| s.set_value(w.to_string(), window, cx));
        forms
            .height
            .update(cx, |s, cx| s.set_value(h.to_string(), window, cx));
    }

    /// Changes the video's size or frame rate, keeping the rest.
    fn set_video_format(
        &mut self,
        size: Option<(u32, u32)>,
        rate: Option<FrameRate>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(video) = &self.video else {
            return;
        };
        let (width, height) = size.unwrap_or((video.doc.width, video.doc.height));
        let command = Command::SetFormat {
            width,
            height,
            rate: rate.unwrap_or(video.doc.rate),
        };
        self.video_commands(vec![command], window, cx);
        self.sync_video_settings(window, cx);
    }

    /// Shows the video's own settings in the Video and Export tabs' fields.
    pub fn sync_video_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(video) = &self.video else {
            return;
        };
        let doc = &video.doc;
        let forms = &video.forms;
        let (w, h, [r, g, b]) = (doc.width, doc.height, doc.background);
        for (field, value) in [
            (&forms.video_width, w.to_string()),
            (&forms.video_height, h.to_string()),
        ] {
            field.update(cx, |s, cx| s.set_value(value, window, cx));
        }
        forms
            .background
            .update(cx, |picker, cx| picker.set([r, g, b, 255], window, cx));
        // Export starts at the video's size until a size is chosen.
        for (field, value) in [(&forms.width, w), (&forms.height, h)] {
            if field.read(cx).value().is_empty() {
                field.update(cx, |s, cx| s.set_value(value.to_string(), window, cx));
            }
        }
    }

    /// The Video tab: the video's own size, frame rate, and background.
    pub fn video_settings_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let Some(video) = &self.video else {
            return div();
        };
        let forms = &video.forms;
        let doc = &video.doc;
        let [r, g, b] = doc.background;
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(
                group("Size", None)
                    .gap_2()
                    .child(
                        Select::new(&forms.video_size)
                            .placeholder(format!("{} × {}", doc.width, doc.height)),
                    )
                    .child(pair(&forms.video_width, "×", &forms.video_height))
                    .child(
                        toggle("video-aspect", "Keep the shape", forms.video_locked).on_click(
                            cx.listener(|this, _, _, cx| {
                                if let Some(video) = &mut this.video {
                                    video.forms.video_locked = !video.forms.video_locked;
                                }
                                cx.notify();
                            }),
                        ),
                    )
                    .child(div().text_xs().text_color(rgb(FAINT)).child(format!(
                        "{} × {} is {}",
                        doc.width,
                        doc.height,
                        aspect(doc.width, doc.height)
                    ))),
            )
            .child(
                group("Frame rate", None)
                    .gap_2()
                    .child(Select::new(&forms.video_rate).placeholder(doc.rate.to_string()))
                    .child(Input::new(&forms.video_fps).small()),
            )
            .child(crate::colors::color_row(
                "Background",
                crate::color_picker::color_well(
                    "video-background",
                    [r, g, b, 255],
                    &forms.background,
                ),
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(FAINT))
                    .child("The background shows wherever no clip covers the frame."),
            )
    }

    /// The Export tab.
    pub fn video_export_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let Some(video) = &self.video else {
            return div();
        };
        let forms = &video.forms;
        let exporting = video.exporting;
        let marked = video.in_point.is_some() || video.out_point.is_some();
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(group("Format", None).child(Select::new(&forms.codec)))
            .child(
                group("Size", None)
                    .gap_2()
                    .child(Select::new(&forms.size))
                    .child(pair(&forms.width, "×", &forms.height))
                    .child(
                        toggle(
                            "export-aspect",
                            "Keep the video's shape",
                            forms.export_locked,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(video) = &mut this.video {
                                video.forms.export_locked = !video.forms.export_locked;
                            }
                            cx.notify();
                        })),
                    )
                    .child(div().text_xs().text_color(rgb(FAINT)).child(format!(
                        "The video is {}",
                        aspect(video.doc.width, video.doc.height)
                    ))),
            )
            .child(
                group("Frame rate", None)
                    .gap_2()
                    .child(Select::new(&forms.rate))
                    .child(Input::new(&forms.fps).small()),
            )
            .child(
                group("Bitrate (Mb/s)", None)
                    .gap_2()
                    .child(Input::new(&forms.bitrate).small())
                    .child(
                        div().text_xs().text_color(rgb(FAINT)).child(
                            "Empty picks one from the size and frame rate. ProRes ignores it.",
                        ),
                    ),
            )
            .child(
                group("Range", None)
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                chip("export-all", "Whole video", !forms.in_to_out).on_click(
                                    cx.listener(|this, _, _, cx| this.set_export_range(false, cx)),
                                ),
                            )
                            .child(
                                chip("export-marked", "In to Out", forms.in_to_out).on_click(
                                    cx.listener(|this, _, _, cx| this.set_export_range(true, cx)),
                                ),
                            ),
                    )
                    .when(forms.in_to_out && !marked, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(rgb(FAINT))
                                .child("Press I and O on the timeline to mark In and Out."),
                        )
                    }),
            )
            .child(
                toggle("export-sound", "Include sound", forms.sound).on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(video) = &mut this.video {
                            video.forms.sound = !video.forms.sound;
                        }
                        cx.notify();
                    },
                )),
            )
            .child(
                Button::new("export-video")
                    .primary()
                    .w_full()
                    .label(match exporting {
                        Some(share) => format!("Exporting {:.0}%…", share * 100.),
                        None => "Export…".into(),
                    })
                    .loading(exporting.is_some())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.choose_video_export(window, cx)),
                    ),
            )
    }

    fn set_export_range(&mut self, in_to_out: bool, cx: &mut Context<Self>) {
        if let Some(video) = &mut self.video {
            video.forms.in_to_out = in_to_out;
        }
        cx.notify();
    }

    /// The export as chosen, and the range to write.
    pub fn export_choice(
        &self,
        cx: &App,
    ) -> Option<(
        ExportSettings,
        Option<(spectrum_media::Time, spectrum_media::Time)>,
    )> {
        let video = self.video.as_ref()?;
        let forms = &video.forms;
        let doc = &video.doc;
        let codec = forms
            .codec
            .read(cx)
            .selected_value()
            .and_then(|name| CODECS.iter().find(|(_, n)| *n == name.as_ref()))
            .map_or(VideoCodec::H264, |(codec, _)| *codec);
        let width = pixels(&forms.width.read(cx).value()).unwrap_or(doc.width & !1);
        let height = pixels(&forms.height.read(cx).value()).unwrap_or(doc.height & !1);
        // A typed rate wins over the list.
        let typed = FrameRate::from_text(&forms.fps.read(cx).value()).ok();
        let rate = typed
            .or_else(|| {
                forms
                    .rate
                    .read(cx)
                    .selected_value()
                    .filter(|name| name.as_ref() != SAME)
                    .and_then(|name| FrameRate::from_text(name).ok())
            })
            .unwrap_or(doc.rate);
        let bitrate = forms
            .bitrate
            .read(cx)
            .value()
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|mbps| *mbps > 0.)
            .map(|mbps| (mbps * 1e6) as u64);
        let range = forms.in_to_out.then(|| {
            (
                video.in_point.unwrap_or(spectrum_media::Time::ZERO),
                video.out_point.unwrap_or(doc.duration()),
            )
        });
        Some((
            ExportSettings {
                codec,
                width,
                height,
                rate,
                bitrate,
                sound: forms.sound,
            },
            range,
        ))
    }
}

/// Two fields with a mark between, such as width × height.
fn pair(left: &Entity<InputState>, mark: &'static str, right: &Entity<InputState>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().flex_1().child(Input::new(left).small()))
        .child(div().text_sm().text_color(rgb(FAINT)).child(mark))
        .child(div().flex_1().child(Input::new(right).small()))
}
