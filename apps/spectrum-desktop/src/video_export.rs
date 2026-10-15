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
    pub sound: bool,
    pub in_to_out: bool,
    /// The Video tab's.
    video_size: List,
    video_rate: List,
    video_width: Entity<InputState>,
    video_height: Entity<InputState>,
    pub(crate) background: Entity<InputState>,
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

fn color_text([r, g, b]: [u8; 3]) -> String {
    format!("{r:02x}{g:02x}{b:02x}")
}

fn parse_color(text: &str) -> Option<[u8; 3]> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
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
        let (video_width, video_height) = (field("Width", window, cx), field("Height", window, cx));
        let background = field("000000", window, cx);
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
            cx.subscribe_in(&video_width, window, Self::typed_size),
            cx.subscribe_in(&video_height, window, Self::typed_size),
            cx.subscribe_in(
                &background,
                window,
                |this, state, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        if matches!(event, InputEvent::PressEnter { .. }) {
                            // Done typing: the keyboard goes back to the editor.
                            this.focus_handle.focus(window);
                        }
                        let text = state.read(cx).value().to_string();
                        match parse_color(&text) {
                            Some(color) => this.video_commands(
                                vec![Command::SetBackground { color }],
                                window,
                                cx,
                            ),
                            None => this.sync_video_settings(window, cx),
                        }
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
            sound: true,
            in_to_out: false,
            video_size,
            video_rate,
            video_width,
            video_height,
            background,
            _subscriptions: subscriptions,
        }
    }

    fn typed_size(
        this: &mut Workspace,
        _: &Entity<InputState>,
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
        let Some(forms) = this.video.as_ref().map(|v| &v.forms) else {
            return;
        };
        let width = pixels(&forms.video_width.read(cx).value());
        let height = pixels(&forms.video_height.read(cx).value());
        match width.zip(height) {
            Some(size) => this.set_video_format(Some(size), None, window, cx),
            None => this.sync_video_settings(window, cx),
        }
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
        let (w, h, background) = (doc.width, doc.height, color_text(doc.background));
        for (field, value) in [
            (&forms.video_width, w.to_string()),
            (&forms.video_height, h.to_string()),
            (&forms.background, background),
        ] {
            field.update(cx, |s, cx| s.set_value(value, window, cx));
        }
        // Export starts at the video's size until a size is chosen.
        for (field, value) in [(&forms.width, w), (&forms.height, h)] {
            if field.read(cx).value().is_empty() {
                field.update(cx, |s, cx| s.set_value(value.to_string(), window, cx));
            }
        }
    }

    /// The Video tab: the video's own size, frame rate, and background.
    pub fn video_settings_sidebar(&self, _: &mut Context<Self>) -> Div {
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
                    .child(pair(&forms.video_width, "×", &forms.video_height)),
            )
            .child(
                group("Frame rate", None)
                    .gap_2()
                    .child(Select::new(&forms.video_rate).placeholder(doc.rate.to_string())),
            )
            .child(
                group("Background", None).gap_2().child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .size(px(28.))
                                .rounded_md()
                                .border_1()
                                .border_color(rgb(BORDER))
                                .bg(rgb(u32::from_be_bytes([0, r, g, b]))),
                        )
                        .child(div().flex_1().child(Input::new(&forms.background).small())),
                ),
            )
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
                    .child(pair(&forms.width, "×", &forms.height)),
            )
            .child(group("Frame rate", None).child(Select::new(&forms.rate)))
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
        let rate = forms
            .rate
            .read(cx)
            .selected_value()
            .filter(|name| name.as_ref() != SAME)
            .and_then(|name| FrameRate::from_text(name).ok())
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
