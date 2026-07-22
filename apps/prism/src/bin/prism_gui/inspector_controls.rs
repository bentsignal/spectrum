use super::inspector::InspectorSection;
use super::*;

const INSPECTOR_TAB_GAP: f32 = 4.0;
const INSPECTOR_TAB_HEIGHT: f32 = 26.0;

fn inspector_tab_width(available_width: f32) -> f32 {
    ((available_width - INSPECTOR_TAB_GAP * 4.0) / InspectorSection::ALL.len() as f32).max(1.0)
}

pub(super) fn inspector_section_tabs(ui: &mut egui::Ui, active: &mut InspectorSection) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = INSPECTOR_TAB_GAP;
        let width = inspector_tab_width(ui.available_width());
        for section in InspectorSection::ALL {
            let selected = *active == section;
            let response = ui.add_sized(
                [width, INSPECTOR_TAB_HEIGHT],
                egui::Button::new(RichText::new(section.label()).size(9.0).color(if selected {
                    TEXT
                } else {
                    MUTED
                }))
                .fill(if selected {
                    SELECTED_SURFACE
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(Stroke::NONE),
            );
            if selected {
                ui.painter().line_segment(
                    [response.rect.left_bottom(), response.rect.right_bottom()],
                    Stroke::new(2.0, ACCENT),
                );
            }
            if response.on_hover_text(section.description()).clicked() {
                *active = section;
            }
        }
    });
}

pub(super) fn layer_kind_label(kind: &LayerKind) -> &'static str {
    match kind {
        LayerKind::Raster { .. } => "IMAGE",
        LayerKind::Text { .. } => "TEXT",
        LayerKind::Rectangle { .. } => "RECTANGLE",
        LayerKind::Ellipse { .. } => "ELLIPSE",
    }
}

pub(super) fn property_label(ui: &mut egui::Ui, label: &str) {
    ui.label(RichText::new(label).size(10.0).color(MUTED));
}

pub(super) fn section_label(ui: &mut egui::Ui, label: &str) {
    inspector_group_heading(ui, label);
}

pub(super) fn shape_size_grid(
    ui: &mut egui::Ui,
    id: u64,
    width: &mut u32,
    height: &mut u32,
    mut changed: impl FnMut(&egui::Response, u32, u32),
) {
    egui::Grid::new(("shape-size", id))
        .num_columns(4)
        .spacing(Vec2::new(6.0, 7.0))
        .show(ui, |ui| {
            property_label(ui, "W");
            let response = ui.add(
                egui::DragValue::new(width)
                    .range(1..=prism_core::MAX_CANVAS_DIMENSION)
                    .suffix(" px"),
            );
            changed(&response, *width, *height);
            property_label(ui, "H");
            let response = ui.add(
                egui::DragValue::new(height)
                    .range(1..=prism_core::MAX_CANVAS_DIMENSION)
                    .suffix(" px"),
            );
            changed(&response, *width, *height);
            ui.end_row();
        });
}

pub(super) fn color_row(ui: &mut egui::Ui, label: &str, color: &mut Color32) -> egui::Response {
    ui.horizontal(|ui| {
        property_label(ui, label);
        ui.color_edit_button_srgba(color)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspector_tabs_fit_the_minimum_sidebar_on_one_row() {
        let width = inspector_tab_width(310.0);
        let occupied = width * InspectorSection::ALL.len() as f32 + INSPECTOR_TAB_GAP * 4.0;
        assert!((occupied - 310.0).abs() < f32::EPSILON);
        assert!(width >= 58.0);
    }
}
