use super::*;

const FONT_PICKER_MIN_HEIGHT: f32 = 220.0;
const FONT_PICKER_MAX_HEIGHT: f32 = 420.0;
const MAX_FONT_PICKER_RESULTS: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FontHoverPreview {
    tab_id: u64,
    layer_id: u64,
    committed_font_id: Option<u64>,
    preview_font_id: Option<u64>,
    document_identity: u64,
    document_generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FontFaceChoice {
    id: Option<u64>,
    family: String,
    style: String,
    weight: u16,
    slant: prism_core::FontSlant,
}

struct FontFaceChoices {
    faces: Vec<FontFaceChoice>,
    matching_count: usize,
}

impl FontFaceChoice {
    fn bundled() -> Self {
        Self {
            id: None,
            family: prism_core::BUNDLED_FONT.family.into(),
            style: prism_core::BUNDLED_FONT.style.into(),
            weight: prism_core::BUNDLED_FONT.weight,
            slant: prism_core::FontSlant::Normal,
        }
    }

    fn from_asset(font: &prism_core::FontAsset) -> Self {
        Self {
            id: Some(font.id),
            family: font.family.clone(),
            style: font.style.clone(),
            weight: font.weight,
            slant: font.slant,
        }
    }

    fn face_label(&self) -> String {
        format!(
            "{} · {} · {}",
            self.style,
            self.weight,
            slant_label(self.slant)
        )
    }
}

fn slant_label(slant: prism_core::FontSlant) -> &'static str {
    match slant {
        prism_core::FontSlant::Normal => "Normal",
        prism_core::FontSlant::Italic => "Italic",
        prism_core::FontSlant::Oblique => "Oblique",
    }
}

fn font_face_choices(
    fonts: &[prism_core::FontAsset],
    query: &str,
    selected_font_id: Option<u64>,
) -> FontFaceChoices {
    let mut imported = fonts
        .iter()
        .filter(|font| {
            prism_core::font_metadata_matches_query(
                &font.family,
                &font.style,
                font.weight,
                slant_label(font.slant),
                query,
            )
        })
        .collect::<Vec<_>>();
    imported.sort_by(|left, right| {
        left.family
            .to_ascii_lowercase()
            .cmp(&right.family.to_ascii_lowercase())
            .then_with(|| left.weight.cmp(&right.weight))
            .then_with(|| slant_rank(left.slant).cmp(&slant_rank(right.slant)))
            .then_with(|| {
                left.style
                    .to_ascii_lowercase()
                    .cmp(&right.style.to_ascii_lowercase())
            })
            .then_with(|| left.id.cmp(&right.id))
    });
    let bundled = FontFaceChoice::bundled();
    let bundled_matches = font_face_matches(&bundled, query);
    let matching_count = imported.len() + usize::from(bundled_matches);
    let mut faces = bundled_matches
        .then_some(bundled)
        .into_iter()
        .collect::<Vec<_>>();
    faces.extend(
        imported
            .into_iter()
            .take(MAX_FONT_PICKER_RESULTS.saturating_sub(faces.len()))
            .map(FontFaceChoice::from_asset),
    );
    if let Some(selected) = selected_font_id
        .and_then(|id| fonts.iter().find(|font| font.id == id))
        .map(FontFaceChoice::from_asset)
        .filter(|face| font_face_matches(face, query))
        .filter(|selected| !faces.iter().any(|face| face.id == selected.id))
    {
        if faces.len() == MAX_FONT_PICKER_RESULTS {
            faces.pop();
        }
        faces.push(selected);
    }
    FontFaceChoices {
        faces,
        matching_count,
    }
}

fn font_face_matches(face: &FontFaceChoice, query: &str) -> bool {
    prism_core::font_metadata_matches_query(
        &face.family,
        &face.style,
        face.weight,
        slant_label(face.slant),
        query,
    )
}

fn slant_rank(slant: prism_core::FontSlant) -> u8 {
    match slant {
        prism_core::FontSlant::Normal => 0,
        prism_core::FontSlant::Italic => 1,
        prism_core::FontSlant::Oblique => 2,
    }
}

fn selected_face(fonts: &[prism_core::FontAsset], font_id: Option<u64>) -> FontFaceChoice {
    font_id
        .and_then(|id| fonts.iter().find(|font| font.id == id))
        .map(FontFaceChoice::from_asset)
        .unwrap_or_else(FontFaceChoice::bundled)
}

fn with_font_id(
    current: &prism_core::TextTypography,
    font_id: Option<u64>,
) -> prism_core::TextTypography {
    let mut changed = current.clone();
    changed.font_id = font_id;
    changed
}

fn font_picker_available_space(viewport: egui::Rect, anchor: egui::Rect) -> (f32, f32) {
    let above = (anchor.top() - viewport.top() - 16.0).max(0.0);
    let below = (viewport.bottom() - anchor.bottom() - 16.0).max(0.0);
    (above, below)
}

fn font_picker_popup_height(viewport: egui::Rect, anchor: egui::Rect) -> f32 {
    let (above, below) = font_picker_available_space(viewport, anchor);
    let available = above.max(below);
    available
        .min(FONT_PICKER_MAX_HEIGHT)
        .max(FONT_PICKER_MIN_HEIGHT.min(available))
}

fn font_picker_opens_above(viewport: egui::Rect, anchor: egui::Rect) -> bool {
    let (above, below) = font_picker_available_space(viewport, anchor);
    above > below
}

fn move_font_picker_index(current: usize, len: usize, direction: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (current as isize + direction).rem_euclid(len as isize) as usize
}

#[derive(Clone, Copy)]
struct FontHoverScope {
    active_tab_id: u64,
    selected_layer_id: Option<u64>,
    document_identity: u64,
    document_generation: u64,
}

fn hover_preview_matches(
    preview: &FontHoverPreview,
    scope: FontHoverScope,
    layer_id: u64,
    committed_font_id: Option<u64>,
    preview_font_exists: bool,
) -> bool {
    preview.tab_id == scope.active_tab_id
        && preview.layer_id == layer_id
        && scope.selected_layer_id == Some(layer_id)
        && preview.document_identity == scope.document_identity
        && preview.document_generation == scope.document_generation
        && preview.committed_font_id == committed_font_id
        && preview_font_exists
}

fn preview_layer_with_font(layer: &Layer, preview_font_id: Option<Option<u64>>) -> Layer {
    let mut preview_layer = layer.clone();
    if let Some(font_id) = preview_font_id
        && let LayerKind::Text { typography, .. } = &mut preview_layer.kind
    {
        typography.font_id = font_id;
    }
    preview_layer
}

impl PrismApp {
    pub(crate) fn clear_font_hover_preview(&mut self) {
        if let Some(preview) = self.font_hover_preview.take() {
            self.layer_visual_dirty.insert(preview.layer_id);
            self.text_source_geometries.remove(preview.layer_id);
        }
    }

    fn set_font_hover_preview(
        &mut self,
        layer_id: u64,
        committed_font_id: Option<u64>,
        preview_font_id: Option<u64>,
    ) {
        if committed_font_id == preview_font_id {
            self.clear_font_hover_preview();
            return;
        }
        let preview = FontHoverPreview {
            tab_id: self.active_tab_id,
            layer_id,
            committed_font_id,
            preview_font_id,
            document_identity: self.workspace.document_identity(),
            document_generation: self.workspace.document_generation(),
        };
        if self.font_hover_preview.as_ref() != Some(&preview) {
            self.clear_font_hover_preview();
            self.layer_visual_dirty.insert(layer_id);
            self.text_source_geometries.remove(layer_id);
            self.font_hover_preview = Some(preview);
        }
    }

    pub(crate) fn font_hover_preview_id(&self, layer: &Layer) -> Option<Option<u64>> {
        let preview = self.font_hover_preview.as_ref()?;
        let LayerKind::Text { typography, .. } = &layer.kind else {
            return None;
        };
        let preview_font_exists = preview.preview_font_id.is_none_or(|id| {
            self.workspace
                .document
                .font_assets
                .iter()
                .any(|font| font.id == id)
        });
        hover_preview_matches(
            preview,
            FontHoverScope {
                active_tab_id: self.active_tab_id,
                selected_layer_id: self.workspace.document.selected,
                document_identity: self.workspace.document_identity(),
                document_generation: self.workspace.document_generation(),
            },
            layer.id,
            typography.font_id,
            preview_font_exists,
        )
        .then_some(preview.preview_font_id)
    }

    pub(crate) fn font_preview_layer_override(&self, layer: &Layer) -> Option<Layer> {
        self.font_hover_preview_id(layer)
            .map(|font_id| preview_layer_with_font(layer, Some(font_id)))
    }

    pub(crate) fn typeface_controls(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        current: &prism_core::TextTypography,
    ) {
        let current_face = selected_face(&self.workspace.document.font_assets, current.font_id);
        let choices = font_face_choices(
            &self.workspace.document.font_assets,
            &self.font_query,
            current.font_id,
        );
        let mut chosen_face = None;
        let mut hovered_face = None;
        let popup_id = ui.make_persistent_id(("text-font-picker-popup", id));
        let keyboard_index_id = popup_id.with("keyboard-index");
        let picker_is_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
        let committed_index = choices
            .faces
            .iter()
            .position(|face| face.id == current.font_id)
            .unwrap_or(0);
        let mut keyboard_index = ui
            .data(|data| data.get_temp::<usize>(keyboard_index_id))
            .unwrap_or(committed_index)
            .min(choices.faces.len().saturating_sub(1));
        // Read navigation before the search TextEdit has a chance to consume it.
        if picker_is_open && !choices.faces.is_empty() {
            let (move_up, move_down, activate) = ui.input(|input| {
                (
                    input.key_pressed(egui::Key::ArrowUp),
                    input.key_pressed(egui::Key::ArrowDown),
                    input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space),
                )
            });
            if move_up {
                keyboard_index = move_font_picker_index(keyboard_index, choices.faces.len(), -1);
            } else if move_down {
                keyboard_index = move_font_picker_index(keyboard_index, choices.faces.len(), 1);
            }
            if activate {
                chosen_face = choices.faces.get(keyboard_index).map(|face| face.id);
                egui::Popup::close_id(ui.ctx(), popup_id);
            }
        }

        typography_section_label(ui, "TYPEFACE");
        ui.horizontal(|ui| {
            ui.add(
                compact_text_field(&mut self.font_query)
                    .hint_text("Search family, style, or weight")
                    .desired_width(158.0),
            );
            if compact_secondary_button(ui, "Import…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("OpenType font", &["ttf", "otf"])
                    .pick_file()
            {
                self.execute(Command::ImportFont {
                    path,
                    source_name: None,
                });
            }
        });

        let picker_button = ui.add_sized(
            [ui.available_width(), ui.spacing().interact_size.y],
            egui::Button::new(format!(
                "{} · {}",
                current_face.family,
                current_face.face_label()
            ))
            .truncate()
            .selected(picker_is_open),
        );
        picker_button.widget_info(|| {
            let mut info = egui::WidgetInfo::new(egui::WidgetType::ComboBox);
            info.current_text_value = Some(format!(
                "{} · {}",
                current_face.family,
                current_face.face_label()
            ));
            info
        });
        if picker_button.clicked() && !picker_is_open {
            picker_button.request_focus();
            keyboard_index = committed_index;
        }
        ui.data_mut(|data| data.insert_temp(keyboard_index_id, keyboard_index));
        let viewport = ui.ctx().content_rect();
        let popup_height = font_picker_popup_height(viewport, picker_button.rect);
        let popup_alignment = if font_picker_opens_above(viewport, picker_button.rect) {
            egui::RectAlign::TOP_START
        } else {
            egui::RectAlign::BOTTOM_START
        };
        let content_height = ui.spacing().interact_size.y
            * (choices.faces.len() + usize::from(choices.matching_count > choices.faces.len()))
                as f32;
        let scrolled_height = popup_height.min(content_height);
        let picker = egui::Popup::menu(&picker_button)
            .id(popup_id)
            .width(picker_button.rect.width())
            .align(popup_alignment)
            .align_alternatives(&[])
            .show(|ui| {
                ui.set_min_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .max_height(popup_height)
                    .min_scrolled_height(scrolled_height)
                    .show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        if choices.matching_count > choices.faces.len() {
                            ui.label(
                                RichText::new(format!(
                                    "Showing {} of {} matches · refine search",
                                    choices.faces.len(),
                                    choices.matching_count
                                ))
                                .size(9.0)
                                .color(MUTED),
                            );
                        }
                        for (index, face) in choices.faces.iter().enumerate() {
                            let response = ui.selectable_label(
                                face.id == current.font_id || index == keyboard_index,
                                format!("{} · {}", face.family, face.face_label()),
                            );
                            if response.hovered() {
                                hovered_face = Some(face.id);
                            }
                            if response.clicked() {
                                chosen_face = Some(face.id);
                            }
                        }
                    });
            });
        if picker.is_some() {
            if let Some(font_id) = hovered_face {
                self.set_font_hover_preview(id, current.font_id, font_id);
            } else {
                self.clear_font_hover_preview();
            }
        } else {
            ui.data_mut(|data| data.remove_temp::<usize>(keyboard_index_id));
            self.clear_font_hover_preview();
        }
        if picker.is_some() && hovered_face.is_none() {
            hovered_face = choices.faces.get(keyboard_index).map(|face| face.id);
            if let Some(font_id) = hovered_face {
                self.set_font_hover_preview(id, current.font_id, font_id);
            }
        }
        if let Some(font_id) = chosen_face
            && font_id != current.font_id
        {
            self.clear_font_hover_preview();
            self.execute(Command::SetTextTypography {
                id,
                typography: with_font_id(current, font_id),
            });
            return;
        }
        if current.font_id.is_none() {
            let bundled = prism_core::bundled_font_provenance();
            ui.label(
                RichText::new(format!(
                    "Bundled · {} {} · {}",
                    bundled.family, bundled.style, bundled.license_name
                ))
                .size(10.0)
                .color(MUTED),
            )
            .on_hover_text(format!(
                "Designed by {} · source: {} via {} · packaged as {}",
                bundled.designed_by,
                bundled.source_file,
                bundled.distributed_by,
                bundled.packaged_license_file
            ));
        }

        if let Some(font_id) = current.font_id {
            self.font_usage_controls(ui, font_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font(id: u64, family: &str, style: &str, weight: u16) -> prism_core::FontAsset {
        prism_core::FontAsset {
            id,
            family: family.into(),
            style: style.into(),
            weight,
            slant: if style.contains("Italic") {
                prism_core::FontSlant::Italic
            } else {
                prism_core::FontSlant::Normal
            },
            source_name: format!("{family}-{style}.otf"),
            embedding_permission: prism_core::FontEmbeddingPermission::Installable,
            subset_allowed: true,
            content_hash: format!("hash-{id}"),
            path: PathBuf::from(format!("font-{id}.otf")),
            original_path: None,
        }
    }

    fn family_names(faces: &[FontFaceChoice]) -> Vec<String> {
        let mut seen = HashSet::new();
        faces
            .iter()
            .filter_map(|face| {
                let key = face.family.to_ascii_lowercase();
                seen.insert(key).then(|| face.family.clone())
            })
            .collect()
    }

    fn default_face_for_family(faces: &[FontFaceChoice], family: &str) -> Option<FontFaceChoice> {
        if prism_core::is_bundled_font_family(family) {
            return faces.iter().find(|face| face.id.is_none()).cloned();
        }
        faces
            .iter()
            .filter(|face| face.family.eq_ignore_ascii_case(family))
            .min_by_key(|face| {
                (
                    face.weight.abs_diff(400),
                    slant_rank(face.slant),
                    face.style.to_ascii_lowercase(),
                    face.id,
                )
            })
            .cloned()
    }

    #[test]
    fn bundled_face_is_truthful_first_and_imported_faces_sort_stably() {
        let choices = font_face_choices(
            &[
                font(3, "Zed", "Bold", 700),
                font(2, "Alpha", "Italic", 400),
                font(1, "Alpha", "Regular", 400),
            ],
            "",
            None,
        );
        assert_eq!(choices.faces[0], FontFaceChoice::bundled());
        assert_eq!(
            choices.faces[1..]
                .iter()
                .map(|face| face.id.unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(family_names(&choices.faces), vec!["Ubuntu", "Alpha", "Zed"]);
    }

    #[test]
    fn search_matches_family_style_weight_and_slant_terms() {
        let fonts = vec![
            font(1, "Atlas Grotesk", "Regular", 400),
            font(2, "Atlas Grotesk", "Bold Italic", 700),
            font(3, "Mono", "Book", 350),
        ];
        assert_eq!(
            font_face_choices(&fonts, "atlas italic", None)
                .faces
                .iter()
                .filter_map(|face| face.id)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(
            font_face_choices(&fonts, "350", None)
                .faces
                .iter()
                .filter_map(|face| face.id)
                .collect::<Vec<_>>(),
            vec![3]
        );
        assert!(
            font_face_choices(&fonts, "atlas", None)
                .faces
                .iter()
                .all(|face| face.family == "Atlas Grotesk")
        );
    }

    #[test]
    fn family_selection_handles_bundled_alias_and_nearest_regular_face() {
        let choices = font_face_choices(
            &[
                font(1, "Atlas", "Thin", 100),
                font(2, "Atlas", "Regular", 400),
                font(3, "Atlas", "Bold", 700),
            ],
            "",
            None,
        );
        assert_eq!(
            default_face_for_family(&choices.faces, prism_core::LEGACY_BUNDLED_FONT_ALIAS,)
                .unwrap()
                .id,
            None
        );
        assert_eq!(
            default_face_for_family(&choices.faces, "Atlas").unwrap().id,
            Some(2)
        );
    }

    #[test]
    fn large_result_sets_are_bounded_and_report_the_full_match_count() {
        let fonts = (0..600)
            .map(|id| font(id, &format!("Family {id:03}"), "Regular", 400))
            .collect::<Vec<_>>();
        let choices = font_face_choices(&fonts, "", None);
        assert_eq!(choices.matching_count, 601);
        assert_eq!(choices.faces.len(), MAX_FONT_PICKER_RESULTS);
        let selected = font_face_choices(&fonts, "", Some(599));
        assert!(selected.faces.iter().any(|face| face.id == Some(599)));
    }

    #[test]
    fn popup_uses_large_available_side_and_hover_scope_fails_closed() {
        let viewport = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(800.0, 700.0));
        let near_bottom =
            egui::Rect::from_min_max(egui::pos2(600.0, 640.0), egui::pos2(780.0, 660.0));
        assert_eq!(font_picker_popup_height(viewport, near_bottom), 420.0);
        assert!(font_picker_opens_above(viewport, near_bottom));
        let near_top = egui::Rect::from_min_max(egui::pos2(600.0, 40.0), egui::pos2(780.0, 60.0));
        assert!(!font_picker_opens_above(viewport, near_top));
        let short = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(800.0, 180.0));
        let centered = egui::Rect::from_min_max(egui::pos2(600.0, 80.0), egui::pos2(780.0, 100.0));
        assert_eq!(font_picker_popup_height(short, centered), 64.0);
        assert_eq!(move_font_picker_index(0, 4, -1), 3);
        assert_eq!(move_font_picker_index(3, 4, 1), 0);
        assert_eq!(move_font_picker_index(0, 0, 1), 0);
        let preview = FontHoverPreview {
            tab_id: 2,
            layer_id: 9,
            committed_font_id: None,
            preview_font_id: Some(4),
            document_identity: 11,
            document_generation: 3,
        };
        let scope = FontHoverScope {
            active_tab_id: 2,
            selected_layer_id: Some(9),
            document_identity: 11,
            document_generation: 3,
        };
        assert!(hover_preview_matches(&preview, scope, 9, None, true));
        assert!(!hover_preview_matches(
            &preview,
            FontHoverScope {
                active_tab_id: 3,
                ..scope
            },
            9,
            None,
            true
        ));
        assert!(!hover_preview_matches(
            &preview,
            FontHoverScope {
                selected_layer_id: Some(8),
                ..scope
            },
            9,
            None,
            true
        ));
        assert!(!hover_preview_matches(
            &preview,
            FontHoverScope {
                document_generation: 4,
                ..scope
            },
            9,
            None,
            true
        ));
        assert!(!hover_preview_matches(&preview, scope, 9, None, false));
    }

    #[test]
    fn hover_is_reversible_and_click_is_one_normal_typography_revision() {
        let mut document = Document::new("Hover", 320, 200);
        document.font_assets.push(font(4, "Atlas", "Regular", 400));
        let mut workspace = Workspace::new(document, None);
        workspace
            .execute(Command::AddText {
                text: "Preview me".into(),
                name: None,
                font_size: 32.0,
                color: [255; 4],
                x: 0.0,
                y: 0.0,
            })
            .unwrap();
        let layer_id = workspace.document.selected.unwrap();
        let committed = workspace.document.layer(layer_id).unwrap().clone();
        let committed_generation = workspace.document_generation();

        let hovered = preview_layer_with_font(&committed, Some(Some(4)));
        let LayerKind::Text { typography, .. } = &hovered.kind else {
            panic!("hover target should remain text");
        };
        assert_eq!(typography.font_id, Some(4));
        assert_eq!(workspace.document.layer(layer_id).unwrap(), &committed);
        assert_eq!(workspace.document_generation(), committed_generation);
        assert_eq!(preview_layer_with_font(&committed, None), committed);

        workspace
            .execute(Command::SetTextTypography {
                id: layer_id,
                typography: prism_core::TextTypography {
                    font_id: Some(4),
                    ..Default::default()
                },
            })
            .unwrap();
        assert_eq!(
            workspace.document_generation(),
            committed_generation + 1,
            "one click must execute exactly one normal typography revision"
        );
        workspace.execute(Command::Undo).unwrap();
        assert_eq!(workspace.document.layer(layer_id).unwrap(), &committed);
    }
}
