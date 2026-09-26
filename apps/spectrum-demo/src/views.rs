use crate::{gallery::Gallery, theme::*};
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    color_picker::ColorPicker,
    input::Input,
    select::Select,
    slider::Slider,
    switch::Switch,
};

fn section(title: &'static str, description: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_5()
        .p_6()
        .bg(rgb(PANEL))
        .rounded_2xl()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().font_weight(FontWeight::MEDIUM).child(title))
                .child(div().text_sm().text_color(rgb(MUTED)).child(description)),
        )
}

fn row() -> Div {
    div().flex().items_center().gap_3().flex_wrap()
}

fn caption(text: &'static str) -> Div {
    div().text_sm().text_color(rgb(MUTED)).child(text)
}

impl Gallery {
    pub fn controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                section(
                    "Actions",
                    "Clear priorities, without a bright accent color.",
                )
                .child(
                    row()
                        .child(
                            Button::new("create")
                                .primary()
                                .icon(IconName::Plus)
                                .label("Create project")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.message = format!(
                                        "Preview action: create \"{}\". No project was saved.",
                                        this.name.read(cx).value()
                                    )
                                    .into();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("import")
                                .label("Import assets")
                                .icon(IconName::FolderOpen)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.page = 2;
                                    cx.notify();
                                })),
                        )
                        .child(Button::new("more").ghost().label("Reset demo").on_click(
                            cx.listener(|this, _, window, cx| {
                                *this = Self::new(window, cx);
                                cx.notify();
                            }),
                        ))
                        .child(Button::new("disabled").label("Export").disabled(true)),
                ),
            )
            .child(
                section(
                    "Text & choices",
                    "Try typing, selecting, and moving between fields with Tab.",
                )
                .child(caption("Project name"))
                .child(Input::new(&self.name))
                .child(caption("Blend mode"))
                .child(Select::new(&self.blend).w_full())
                .child(
                    row()
                        .justify_between()
                        .child(
                            Checkbox::new("originals")
                                .label("Keep original dimensions")
                                .checked(self.checked)
                                .on_click(cx.listener(|this, checked, _, cx| {
                                    this.checked = *checked;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Switch::new("preview")
                                .label("Live preview")
                                .checked(self.enabled)
                                .on_click(cx.listener(|this, checked, _, cx| {
                                    this.enabled = *checked;
                                    cx.notify();
                                })),
                        ),
                ),
            )
            .child(
                section(
                    "Fine adjustments",
                    "A shared slider treatment for image and canvas controls.",
                )
                .child(
                    row()
                        .justify_between()
                        .child("Exposure")
                        .child(format!("{:+.2} EV", self.exposure.read(cx).value().start())),
                )
                .child(Slider::new(&self.exposure))
                .child(
                    row()
                        .justify_between()
                        .child("Opacity")
                        .child(format!("{:.0}%", self.opacity.read(cx).value().start())),
                )
                .child(Slider::new(&self.opacity)),
            )
    }

    pub fn adjustments(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let light = (125. + self.exposure.read(cx).value().start() * 35.).clamp(30., 220.) as u32;
        let shade = light * 0x010101;
        let backdrop = if self.enabled {
            (42. - self.contrast.read(cx).value().start() * 0.2).clamp(15., 70.) as u32 * 0x010101
        } else {
            0x2a2a2a
        };
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .h(px(260.))
                    .w_full()
                    .rounded_2xl()
                    .overflow_hidden()
                    .bg(rgb(backdrop))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size(px(155.)).rounded_full().bg(rgb(if self.enabled {
                        shade
                    } else {
                        0x7d7d7d
                    }))),
            )
            .child(caption(
                "Sample preview. Exposure and contrast change the gray study above.",
            ))
            .child(
                section(
                    "Color & tone",
                    "Try the full slider range, then reset the adjustments.",
                )
                .child(
                    row()
                        .justify_between()
                        .child("Exposure")
                        .child(format!("{:+.2} EV", self.exposure.read(cx).value().start())),
                )
                .child(Slider::new(&self.exposure))
                .child(
                    row()
                        .justify_between()
                        .child("Contrast")
                        .child(format!("{:+.0}", self.contrast.read(cx).value().start())),
                )
                .child(Slider::new(&self.contrast))
                .child(
                    row()
                        .justify_between()
                        .child(
                            Switch::new("adjustment-preview")
                                .label("Live preview")
                                .checked(self.enabled)
                                .on_click(cx.listener(|this, value, _, cx| {
                                    this.enabled = *value;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("reset-adjustments")
                                .ghost()
                                .label("Reset adjustments")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.exposure
                                        .update(cx, |state, cx| state.set_value(0., window, cx));
                                    this.contrast
                                        .update(cx, |state, cx| state.set_value(0., window, cx));
                                    cx.notify();
                                })),
                        ),
                ),
            )
    }

    pub fn assets(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let names = ["Light study", "Paper forms", "Untitled canvas"];
        div().flex().flex_col().gap_5()
            .child(row().justify_between()
                .child(caption("Sample collection · 3 assets"))
                .child(Button::new("choose-asset").label("Use selected asset").primary()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.message = format!("Selected {} for this preview. No library changes.", names[this.selected_asset]).into(); cx.notify();
                    }))))
            .child(div().flex().gap_4().flex_wrap().children(names.into_iter().enumerate().map(|(index, name)| {
                div().id(("asset", index)).w(px(210.)).rounded_xl().overflow_hidden()
                    .border_1().border_color(rgb(if self.selected_asset == index { 0xb0b0b0 } else { BORDER }))
                    .bg(rgb(PANEL)).cursor_pointer().hover(|el| el.bg(rgb(0x303030)))
                    .child(div().h(px(175.))
                                    .rounded_t_xl().flex().items_center().justify_center().bg(rgb(0x343434 + index as u32 * 0x080808))
                        .child(div().size(px(90.)).bg(rgb(0xa5a5a5)).when(index != 1, |el| el.rounded_full())
                            .when(index == 2, |el| el.child(div().m_4().size(px(58.)).rounded_full().bg(rgb(0x454545))))))
                    .child(div().p_4().flex().flex_col().gap_2().child(name)
                        .child(caption(if index == 2 { "Canvas · 1920 × 1080" } else { "Image · Sample asset" })))
                    .on_click(cx.listener(move |this, _, _, cx| { this.selected_asset = index; cx.notify(); }))
            })))
            .child(section("An empty collection", "The same visual language when there is nothing here yet.")
                .child(div().py_8().flex().flex_col().items_center().gap_4()
                    .child(Icon::new(IconName::FolderOpen).text_color(rgb(MUTED)))
                    .child("A place for your next idea")
                    .child(caption("Import images and other assets into your project."))
                    .child(Button::new("sample-import").label("Try import action").on_click(cx.listener(|this, _, _, cx| {
                        this.message = "Import is a preview action here. Real imports come after design review.".into(); cx.notify();
                    })))))
    }

    pub fn layers(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                section("Layers", "Select a layer or toggle its visibility.").children(
                    ["Title", "Image", "Background"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, name)| {
                            div()
                                .id(("layer", index))
                                .flex()
                                .items_center()
                                .gap_3()
                                .p_3()
                                .rounded_lg()
                                .bg(rgb(if self.selected_layer == index {
                                    0x383838
                                } else {
                                    PANEL
                                }))
                                .cursor_pointer()
                                .hover(|el| el.bg(rgb(0x333333)))
                                .child(
                                    Icon::new(if index == 0 {
                                        IconName::ALargeSmall
                                    } else {
                                        IconName::Frame
                                    })
                                    .small(),
                                )
                                .child(div().flex_1().child(name))
                                .child(
                                    Button::new(("visibility", index))
                                        .ghost()
                                        .icon(if self.visible[index] {
                                            IconName::Eye
                                        } else {
                                            IconName::EyeOff
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.visible[index] = !this.visible[index];
                                            cx.stop_propagation();
                                            cx.notify();
                                        })),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.selected_layer = index;
                                    cx.notify();
                                }))
                        }),
                ),
            )
            .child(
                section(
                    "Editing scope",
                    "Local edits are the default. Global edits will affect shared uses.",
                )
                .child(
                    row()
                        .child(
                            Button::new("local")
                                .label("Edit locally")
                                .when(!self.global, |button| button.primary())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.global = false;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("global")
                                .label("Edit globally")
                                .when(self.global, |button| button.primary())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.global = true;
                                    cx.notify();
                                })),
                        ),
                )
                .child(caption(if self.global {
                    "Preview scope: all uses of the shared asset."
                } else {
                    "Preview scope: only this use in the canvas."
                })),
            )
            .child(
                section(
                    "Layer properties",
                    "Dropdown and slider controls in an inspector context.",
                )
                .child(Select::new(&self.blend).w_full())
                .child(ColorPicker::new(&self.fill).label("Fill color"))
                .child(
                    row()
                        .justify_between()
                        .child("Opacity")
                        .child(format!("{:.0}%", self.opacity.read(cx).value().start())),
                )
                .child(Slider::new(&self.opacity)),
            )
    }
}
