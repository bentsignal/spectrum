use crate::theme::*;
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Root, Sizable,
    button::{Button, ButtonVariants},
    color_picker::ColorPickerState,
    input::InputState,
    select::SelectState,
    slider::{SliderEvent, SliderState},
};

pub struct Gallery {
    pub page: usize,
    pub right_sidebar: bool,
    pub name: Entity<InputState>,
    pub fill: Entity<ColorPickerState>,
    pub blend: Entity<SelectState<Vec<SharedString>>>,
    pub exposure: Entity<SliderState>,
    pub contrast: Entity<SliderState>,
    pub opacity: Entity<SliderState>,
    pub enabled: bool,
    pub checked: bool,
    pub global: bool,
    pub selected_asset: usize,
    pub selected_layer: usize,
    pub visible: [bool; 3],
    pub message: SharedString,
    _subscriptions: Vec<Subscription>,
}

impl Gallery {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Give it a name")
                .default_value("Untitled project")
        });
        let fill = cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb(0xb8b8b8)));
        let blend = cx.new(|cx| {
            SelectState::new(
                vec![
                    "Normal".into(),
                    "Multiply".into(),
                    "Screen".into(),
                    "Overlay".into(),
                ],
                Some(gpui_component::IndexPath::default()),
                window,
                cx,
            )
        });
        let exposure = cx.new(|_| {
            SliderState::new()
                .min(-2.)
                .max(2.)
                .step(0.05)
                .default_value(0.)
        });
        let contrast = cx.new(|_| {
            SliderState::new()
                .min(-100.)
                .max(100.)
                .step(1.)
                .default_value(0.)
        });
        let opacity = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(100.)
        });
        let subscriptions = [&exposure, &contrast, &opacity]
            .into_iter()
            .map(|state| cx.subscribe(state, |_, _, _: &SliderEvent, cx| cx.notify()))
            .collect();
        Self {
            page: 0,
            right_sidebar: false,
            name,
            fill,
            blend,
            exposure,
            contrast,
            opacity,
            enabled: true,
            checked: true,
            global: false,
            selected_asset: 0,
            selected_layer: 0,
            visible: [true; 3],
            message: "Sample controls. Your library is untouched.".into(),
            _subscriptions: subscriptions,
        }
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let names = ["Controls", "Adjustments", "Assets", "Layers"];
        let icons = [
            IconName::Settings2,
            IconName::Palette,
            IconName::Folder,
            IconName::GalleryVerticalEnd,
        ];
        div()
            .flex()
            .flex_col()
            .w(px(232.))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(SIDEBAR))
            .p_4()
            .gap_2()
            .child(
                div()
                    .px_3()
                    .pt_5()
                    .pb_8()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Spectrum"),
            )
            .child(
                div()
                    .px_3()
                    .pb_3()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("Design preview"),
            )
            .children(
                names
                    .into_iter()
                    .zip(icons)
                    .enumerate()
                    .map(|(index, (name, icon))| {
                        div()
                            .id(("nav", index))
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_3()
                            .py_3()
                            .rounded_lg()
                            .cursor_pointer()
                            .when(self.page == index, |el| el.bg(rgb(0x383838)))
                            .hover(|el| el.bg(rgb(0x333333)))
                            .child(Icon::new(icon).small())
                            .child(name)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.page = index;
                                cx.notify();
                            }))
                    }),
            )
            .child(div().flex_1())
            .child(
                Button::new("sidebar-side")
                    .ghost()
                    .icon(if self.right_sidebar {
                        IconName::PanelLeft
                    } else {
                        IconName::PanelRight
                    })
                    .label(if self.right_sidebar {
                        "Move sidebar left"
                    } else {
                        "Move sidebar right"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.right_sidebar = !this.right_sidebar;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .px_3()
                    .py_3()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("Component study  /  01"),
            )
    }
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let titles = [
            "A quieter workspace",
            "Make room for the image",
            "Everything in its place",
            "Build it in layers",
        ];
        let descriptions = [
            "A first look at the controls for the next Spectrum.",
            "Precise adjustments, with space to focus on your work.",
            "A study of asset cards and selection. Sample content only.",
            "A study of layer selection, visibility, and editing scope.",
        ];
        let content = match self.page {
            1 => self.adjustments(cx).into_any_element(),
            2 => self.assets(cx).into_any_element(),
            3 => self.layers(cx).into_any_element(),
            _ => self.controls(cx).into_any_element(),
        };
        let sidebar = self.sidebar(cx).into_any_element();
        let body = div()
            .id(("gallery-scroll", self.page))
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .px_10()
            .py_10()
            .child(
                div()
                    .w_full()
                    .max_w(px(920.))
                    .mx_auto()
                    .flex()
                    .flex_col()
                    .gap_8()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child("SPECTRUM / DESIGN PREVIEW"),
                            )
                            .child(
                                div()
                                    .text_3xl()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(titles[self.page]),
                            )
                            .child(div().text_color(rgb(MUTED)).child(descriptions[self.page])),
                    )
                    .child(content)
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(self.message.clone()),
                    ),
            );
        let mut layout = div()
            .size_full()
            .flex()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT));
        if self.right_sidebar {
            layout = layout.child(body).child(sidebar);
        } else {
            layout = layout.child(sidebar).child(body);
        }
        layout
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}
