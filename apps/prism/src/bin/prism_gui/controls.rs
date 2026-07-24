use super::*;

pub(crate) fn contrast_text(background: Color32) -> Color32 {
    let luma = background.r() as u16 * 3 + background.g() as u16 * 6 + background.b() as u16;
    if luma > 1_400 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

pub(crate) fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(contrast_text(ACCENT)))
            .fill(ACCENT)
            .stroke(Stroke::new(1.0, ACCENT)),
    )
}

pub(crate) fn secondary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(label).size(12.0)))
}

pub(crate) fn compact_secondary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = Vec2::new(
            f32::from(COMPACT_CONTROL_HORIZONTAL_PADDING),
            f32::from(COMPACT_CONTROL_VERTICAL_PADDING),
        );
        ui.spacing_mut().interact_size.y = COMPACT_CONTROL_HEIGHT;
        ui.add(egui::Button::new(RichText::new(label).size(11.0)))
    })
    .inner
}

pub(crate) fn quiet_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(label).size(11.0)).frame(false))
}

pub(crate) fn quiet_danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(label).size(11.0).color(DANGER)).frame(false))
}

pub(crate) fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(contrast_text(DANGER)))
            .fill(DANGER)
            .stroke(Stroke::new(1.0, DANGER)),
    )
}

pub(crate) fn text_field<'text>(text: &'text mut dyn egui::TextBuffer) -> egui::TextEdit<'text> {
    egui::TextEdit::singleline(text)
        .margin(egui::Margin::symmetric(
            CONTROL_HORIZONTAL_PADDING,
            CONTROL_VERTICAL_PADDING,
        ))
        .vertical_align(egui::Align::Center)
}

pub(crate) fn compact_text_field<'text>(
    text: &'text mut dyn egui::TextBuffer,
) -> egui::TextEdit<'text> {
    egui::TextEdit::singleline(text)
        .margin(egui::Margin::symmetric(
            COMPACT_CONTROL_HORIZONTAL_PADDING,
            COMPACT_CONTROL_VERTICAL_PADDING,
        ))
        .vertical_align(egui::Align::Center)
}
