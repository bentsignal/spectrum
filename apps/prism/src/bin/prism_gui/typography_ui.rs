use super::*;

pub(super) struct InlineTextEdit {
    id: Option<u64>,
    position: Pos2,
    text: String,
    font_size: f32,
    color: [u8; 4],
    request_focus: bool,
}

impl PrismApp {
    pub(super) fn typography_controls(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        current: &prism_core::TextTypography,
    ) {
        ui.separator();
        ui.label(RichText::new("TYPEFACE").size(9.0).strong().color(SUBTLE));
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.font_query)
                    .hint_text("Search family or style")
                    .desired_width(154.0),
            );
            if ui.small_button("Import…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("OpenType font", &["ttf", "otf"])
                    .pick_file()
            {
                self.execute(Command::ImportFont { path });
            }
        });

        let selected_label = current
            .font_id
            .and_then(|font_id| self.workspace.document.font_asset(font_id).ok())
            .map_or_else(
                || "Spectrum Sans · Regular · 300".to_owned(),
                |font| format!("{} · {} · {}", font.family, font.style, font.weight),
            );
        let query = self.font_query.trim().to_ascii_lowercase();
        let fonts: Vec<_> = self
            .workspace
            .document
            .font_assets
            .iter()
            .filter(|font| {
                query.is_empty()
                    || font.family.to_ascii_lowercase().contains(&query)
                    || font.style.to_ascii_lowercase().contains(&query)
                    || font.weight.to_string().contains(&query)
            })
            .map(|font| {
                (
                    font.id,
                    font.family.clone(),
                    font.style.clone(),
                    font.weight,
                )
            })
            .collect();
        egui::ComboBox::from_id_salt(("text-font", id))
            .selected_text(selected_label)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(current.font_id.is_none(), "Spectrum Sans · Regular · 300")
                    .clicked()
                {
                    let mut typography = current.clone();
                    typography.font_id = None;
                    self.execute(Command::SetTextTypography { id, typography });
                }
                for (font_id, family, style, weight) in fonts {
                    if ui
                        .selectable_label(
                            current.font_id == Some(font_id),
                            format!("{family} · {style} · {weight}"),
                        )
                        .clicked()
                    {
                        let mut typography = current.clone();
                        typography.font_id = Some(font_id);
                        self.execute(Command::SetTextTypography { id, typography });
                    }
                }
            });

        ui.label(RichText::new("PARAGRAPH").size(9.0).strong().color(SUBTLE));
        ui.horizontal(|ui| {
            for (label, alignment) in [
                ("Left", prism_core::TextAlignment::Left),
                ("Center", prism_core::TextAlignment::Center),
                ("Right", prism_core::TextAlignment::Right),
            ] {
                if ui
                    .selectable_label(current.alignment == alignment, label)
                    .clicked()
                {
                    let mut typography = current.clone();
                    typography.alignment = alignment;
                    self.execute(Command::SetTextTypography { id, typography });
                }
            }
        });
        let mut line_height = current.line_height;
        let response = ui.add(
            egui::Slider::new(&mut line_height, 0.5..=4.0)
                .text("Line height")
                .fixed_decimals(2),
        );
        let mut typography = current.clone();
        typography.line_height = line_height;
        self.widget_command(&response, Command::SetTextTypography { id, typography });

        let mut tracking = current.tracking;
        let response = ui.add(
            egui::Slider::new(&mut tracking, -50.0..=200.0)
                .text("Tracking")
                .suffix(" px"),
        );
        let mut typography = current.clone();
        typography.tracking = tracking;
        self.widget_command(&response, Command::SetTextTypography { id, typography });

        let mut wrap = current.box_width.is_some();
        if ui.checkbox(&mut wrap, "Wrap in text box").changed() {
            let mut typography = current.clone();
            typography.box_width = wrap.then_some(current.box_width.unwrap_or(320.0));
            self.execute(Command::SetTextTypography { id, typography });
        }
        if let Some(mut width) = current.box_width {
            let response = ui.add(
                egui::DragValue::new(&mut width)
                    .range(1.0..=100_000.0)
                    .suffix(" px")
                    .prefix("Width "),
            );
            let mut typography = current.clone();
            typography.box_width = Some(width);
            self.widget_command(&response, Command::SetTextTypography { id, typography });
        }

        ui.label(RichText::new("EFFECTS").size(9.0).strong().color(SUBTLE));
        let mut outline_width = current.effects.outline_width;
        let response = ui.add(
            egui::Slider::new(&mut outline_width, 0.0..=32.0)
                .text("Outline")
                .suffix(" px"),
        );
        let mut typography = current.clone();
        typography.effects.outline_width = outline_width;
        self.widget_command(&response, Command::SetTextTypography { id, typography });
        let mut outline_color = color32(current.effects.outline_color);
        let response = ui.color_edit_button_srgba(&mut outline_color);
        let mut typography = current.clone();
        typography.effects.outline_color = rgba(outline_color);
        self.widget_command(&response, Command::SetTextTypography { id, typography });

        ui.horizontal(|ui| {
            let mut shadow_x = current.effects.shadow_offset_x;
            let response = ui.add(
                egui::DragValue::new(&mut shadow_x)
                    .range(-128.0..=128.0)
                    .prefix("Shadow X "),
            );
            let mut typography = current.clone();
            typography.effects.shadow_offset_x = shadow_x;
            self.widget_command(&response, Command::SetTextTypography { id, typography });
            let mut shadow_y = current.effects.shadow_offset_y;
            let response = ui.add(
                egui::DragValue::new(&mut shadow_y)
                    .range(-128.0..=128.0)
                    .prefix("Y "),
            );
            let mut typography = current.clone();
            typography.effects.shadow_offset_y = shadow_y;
            self.widget_command(&response, Command::SetTextTypography { id, typography });
        });
        let mut shadow_color = color32(current.effects.shadow_color);
        let response = ui.color_edit_button_srgba(&mut shadow_color);
        let mut typography = current.clone();
        typography.effects.shadow_color = rgba(shadow_color);
        self.widget_command(&response, Command::SetTextTypography { id, typography });
    }

    pub(super) fn start_new_inline_text_editor(&mut self, position: Pos2) {
        self.inline_text_edit = Some(InlineTextEdit {
            id: None,
            position,
            text: String::new(),
            font_size: 72.0,
            color: [245, 246, 250, 255],
            request_focus: true,
        });
    }

    pub(super) fn start_inline_text_editor(&mut self, id: u64) {
        let Ok(layer) = self.workspace.document.layer(id) else {
            return;
        };
        let LayerKind::Text {
            text,
            font_size,
            color,
            ..
        } = &layer.kind
        else {
            return;
        };
        self.inline_text_edit = Some(InlineTextEdit {
            id: Some(id),
            position: Pos2::new(layer.transform.x, layer.transform.y),
            text: text.clone(),
            font_size: *font_size,
            color: *color,
            request_focus: true,
        });
    }

    pub(super) fn inline_text_editor(&mut self, context: &egui::Context, geometry: CanvasGeometry) {
        let Some(mut edit) = self.inline_text_edit.take() else {
            return;
        };
        let position = geometry.canvas_to_screen(edit.position);
        let width = edit
            .id
            .and_then(|id| self.workspace.document.layer(id).ok())
            .and_then(|layer| match &layer.kind {
                LayerKind::Text { typography, .. } => typography.box_width,
                _ => None,
            })
            .unwrap_or(360.0)
            * geometry.pixels_per_point;
        let area = egui::Area::new(egui::Id::new(("inline-text", edit.id)))
            .order(egui::Order::Foreground)
            .fixed_pos(position)
            .show(context, |ui| {
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, ACCENT))
                    .corner_radius(4)
                    .inner_margin(6)
                    .show(ui, |ui| {
                        ui.set_width(width.clamp(180.0, 720.0));
                        ui.add(
                            egui::TextEdit::multiline(&mut edit.text)
                                .font(FontId::proportional(
                                    (edit.font_size * geometry.pixels_per_point).clamp(12.0, 96.0),
                                ))
                                .desired_rows(3)
                                .desired_width(f32::INFINITY),
                        )
                    })
                    .inner
            });
        let response = area.inner;
        if edit.request_focus {
            response.request_focus();
            edit.request_focus = false;
        }
        let cancel = context.input(|input| input.key_pressed(egui::Key::Escape));
        let commit = context
            .input(|input| input.key_pressed(egui::Key::Enter) && input.modifiers.command)
            || response.lost_focus();
        if cancel {
            return;
        }
        if commit {
            if edit.text.trim().is_empty() {
                self.status = "Text cannot be empty".into();
                self.status_error = true;
                self.inline_text_edit = Some(edit);
                return;
            }
            if let Some(id) = edit.id {
                self.execute(Command::UpdateText {
                    id,
                    text: edit.text,
                    font_size: edit.font_size,
                    color: edit.color,
                });
            } else {
                self.execute(Command::AddText {
                    text: edit.text,
                    name: None,
                    font_size: edit.font_size,
                    color: edit.color,
                    x: edit.position.x,
                    y: edit.position.y,
                });
                self.tool = Tool::Move;
            }
        } else {
            self.inline_text_edit = Some(edit);
        }
    }
}
