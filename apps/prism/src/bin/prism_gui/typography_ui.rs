use super::*;

const BUNDLED_FAMILY: &str = "Spectrum Sans";
const BUNDLED_STYLE: &str = "Regular";
const BUNDLED_WEIGHT: u16 = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
struct FontUsageAnalysisKey {
    tab_id: u64,
    document_identity: u64,
    document_generation: u64,
    font_id: u64,
    content_hash: String,
}

pub(super) struct CachedFontUsageAnalysis {
    key: FontUsageAnalysisKey,
    plan: prism_core::FontSubsetPlan,
}

fn font_usage_analysis_key(
    tab_id: u64,
    workspace: &Workspace,
    font: &prism_core::FontAsset,
) -> FontUsageAnalysisKey {
    FontUsageAnalysisKey {
        tab_id,
        document_identity: workspace.document_identity(),
        document_generation: workspace.document_generation(),
        font_id: font.id,
        content_hash: font.content_hash.clone(),
    }
}

fn byte_size_label(bytes: u64) -> String {
    if bytes < 1_024 {
        format!("{bytes} B")
    } else if bytes < 1_048_576 {
        format!("{:.1} KiB", bytes as f64 / 1_024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
    }
}

fn reduction_percent(source_bytes: u64, reduction_bytes: u64) -> u64 {
    if source_bytes == 0 {
        return 0;
    }
    ((u128::from(reduction_bytes) * 100 + u128::from(source_bytes) / 2) / u128::from(source_bytes))
        .min(100) as u64
}

fn candidate_size_label(source_bytes: u64, candidate: &prism_core::FontSubsetCandidate) -> String {
    format!(
        "Candidate {} · saves {} ({}%)",
        byte_size_label(candidate.bytes),
        byte_size_label(candidate.reduction_bytes),
        reduction_percent(source_bytes, candidate.reduction_bytes)
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FontFaceChoice {
    id: Option<u64>,
    family: String,
    style: String,
    weight: u16,
    slant: prism_core::FontSlant,
}

impl FontFaceChoice {
    fn bundled() -> Self {
        Self {
            id: None,
            family: BUNDLED_FAMILY.into(),
            style: BUNDLED_STYLE.into(),
            weight: BUNDLED_WEIGHT,
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

fn font_face_choices(fonts: &[prism_core::FontAsset], query: &str) -> Vec<FontFaceChoice> {
    let mut imported = fonts
        .iter()
        .map(FontFaceChoice::from_asset)
        .filter(|face| font_face_matches(face, query))
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
    let mut choices = vec![FontFaceChoice::bundled()];
    choices.extend(imported);
    choices
}

fn font_face_matches(face: &FontFaceChoice, query: &str) -> bool {
    let haystack = format!(
        "{} {} {} {}",
        face.family,
        face.style,
        face.weight,
        slant_label(face.slant)
    )
    .to_ascii_lowercase();
    query
        .split_whitespace()
        .all(|term| haystack.contains(&term.to_ascii_lowercase()))
}

fn slant_rank(slant: prism_core::FontSlant) -> u8 {
    match slant {
        prism_core::FontSlant::Normal => 0,
        prism_core::FontSlant::Italic => 1,
        prism_core::FontSlant::Oblique => 2,
    }
}

fn font_family_names(faces: &[FontFaceChoice]) -> Vec<String> {
    let mut seen = HashSet::new();
    faces
        .iter()
        .filter_map(|face| {
            let key = face.family.to_ascii_lowercase();
            seen.insert(key).then(|| face.family.clone())
        })
        .collect()
}

fn selected_face(fonts: &[prism_core::FontAsset], font_id: Option<u64>) -> FontFaceChoice {
    font_id
        .and_then(|id| fonts.iter().find(|font| font.id == id))
        .map(FontFaceChoice::from_asset)
        .unwrap_or_else(FontFaceChoice::bundled)
}

fn default_face_for_family(faces: &[FontFaceChoice], family: &str) -> Option<FontFaceChoice> {
    if family.eq_ignore_ascii_case(BUNDLED_FAMILY) {
        return faces
            .iter()
            .filter(|face| face.family.eq_ignore_ascii_case(family))
            .find(|face| face.id.is_none())
            .cloned()
            .or_else(|| {
                faces
                    .iter()
                    .find(|face| face.family.eq_ignore_ascii_case(family))
                    .cloned()
            });
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

fn with_font_id(
    current: &prism_core::TextTypography,
    font_id: Option<u64>,
) -> prism_core::TextTypography {
    let mut changed = current.clone();
    changed.font_id = font_id;
    changed
}

fn with_alignment(
    current: &prism_core::TextTypography,
    alignment: prism_core::TextAlignment,
) -> prism_core::TextTypography {
    let mut changed = current.clone();
    changed.alignment = alignment;
    changed
}

fn with_line_height(
    current: &prism_core::TextTypography,
    line_height: f32,
) -> prism_core::TextTypography {
    let mut changed = current.clone();
    changed.line_height = line_height;
    changed
}

fn with_tracking(
    current: &prism_core::TextTypography,
    tracking: f32,
) -> prism_core::TextTypography {
    let mut changed = current.clone();
    changed.tracking = tracking;
    changed
}

fn with_box_width(
    current: &prism_core::TextTypography,
    box_width: Option<f32>,
) -> prism_core::TextTypography {
    let mut changed = current.clone();
    changed.box_width = box_width;
    changed
}

fn with_outline_width(
    current: &prism_core::TextTypography,
    outline_width: f32,
) -> prism_core::TextTypography {
    let mut changed = current.clone();
    changed.effects.outline_width = outline_width;
    changed
}

fn with_outline_color(
    current: &prism_core::TextTypography,
    outline_color: [u8; 4],
) -> prism_core::TextTypography {
    let mut changed = current.clone();
    changed.effects.outline_color = outline_color;
    changed
}

fn with_shadow_offset(
    current: &prism_core::TextTypography,
    shadow_offset_x: f32,
    shadow_offset_y: f32,
) -> prism_core::TextTypography {
    let mut changed = current.clone();
    changed.effects.shadow_offset_x = shadow_offset_x;
    changed.effects.shadow_offset_y = shadow_offset_y;
    changed
}

fn with_shadow_color(
    current: &prism_core::TextTypography,
    shadow_color: [u8; 4],
) -> prism_core::TextTypography {
    let mut changed = current.clone();
    changed.effects.shadow_color = shadow_color;
    changed
}

impl PrismApp {
    pub(super) fn typeface_controls(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        current: &prism_core::TextTypography,
    ) {
        typography_section_label(ui, "TYPEFACE");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.font_query)
                    .hint_text("Search family, style, or weight")
                    .desired_width(158.0),
            );
            if ui.small_button("Import…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("OpenType font", &["ttf", "otf"])
                    .pick_file()
            {
                self.execute(Command::ImportFont { path });
            }
        });

        let current_face = selected_face(&self.workspace.document.font_assets, current.font_id);
        let all_faces = font_face_choices(&self.workspace.document.font_assets, "");
        let filtered_faces =
            font_face_choices(&self.workspace.document.font_assets, &self.font_query);
        let families = font_family_names(&filtered_faces);
        let mut chosen_family = None;
        egui::ComboBox::from_id_salt(("text-font-family", id))
            .selected_text(current_face.family.clone())
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for family in families {
                    if ui
                        .selectable_label(
                            current_face.family.eq_ignore_ascii_case(&family),
                            family.as_str(),
                        )
                        .clicked()
                    {
                        chosen_family = default_face_for_family(&all_faces, &family);
                    }
                }
            });
        if let Some(face) = chosen_family {
            if face.id != current.font_id {
                self.execute(Command::SetTextTypography {
                    id,
                    typography: with_font_id(current, face.id),
                });
            }
            return;
        }

        let family_faces = all_faces
            .iter()
            .filter(|face| face.family.eq_ignore_ascii_case(&current_face.family))
            .cloned()
            .collect::<Vec<_>>();
        let mut chosen_face = None;
        egui::ComboBox::from_id_salt(("text-font-face", id))
            .selected_text(current_face.face_label())
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for face in family_faces {
                    if ui
                        .selectable_label(face.id == current.font_id, face.face_label())
                        .clicked()
                    {
                        chosen_face = Some(face.id);
                    }
                }
            });
        if let Some(font_id) = chosen_face
            && font_id != current.font_id
        {
            self.execute(Command::SetTextTypography {
                id,
                typography: with_font_id(current, font_id),
            });
            return;
        }

        if let Some(font_id) = current.font_id {
            self.font_usage_controls(ui, font_id);
        }
    }

    fn font_usage_controls(&mut self, ui: &mut egui::Ui, font_id: u64) {
        let Some(font) = self
            .workspace
            .document
            .font_assets
            .iter()
            .find(|font| font.id == font_id)
        else {
            return;
        };
        let key = font_usage_analysis_key(self.active_tab_id, &self.workspace, font);
        ui.add_space(4.0);
        let cached_is_current = self
            .font_usage_analysis
            .as_ref()
            .is_some_and(|cached| cached.key == key);
        if !cached_is_current {
            self.font_usage_analysis = None;
        }
        if ui.small_button("Preview optimized-copy savings").clicked() {
            match prism_core::plan_font_subset(&self.workspace.document, font_id) {
                Ok(plan) => {
                    self.status = plan.candidate.as_ref().map_or_else(
                        || {
                            format!(
                                "Optimized font candidate has {} blocker(s)",
                                plan.candidate_blockers.len()
                            )
                        },
                        |candidate| {
                            format!(
                                "Previewed optimized font candidate: {} smaller",
                                byte_size_label(candidate.reduction_bytes)
                            )
                        },
                    );
                    self.status_error = false;
                    self.font_usage_analysis = Some(CachedFontUsageAnalysis { key, plan });
                }
                Err(error) => {
                    self.status = error.to_string();
                    self.status_error = true;
                }
            }
        }
        if let Some(cached) = &self.font_usage_analysis {
            let plan = &cached.plan;
            let analysis = &plan.analysis;
            ui.label(
                RichText::new(format!(
                    "{} Unicode scalars · {} variation sequences · {} text layers",
                    analysis.usage.codepoints.len(),
                    analysis.usage.variation_sequences.len(),
                    analysis.usage.layer_ids.len()
                ))
                .size(10.0)
                .color(MUTED),
            );
            let size_kib = analysis.source_bytes.div_ceil(1024);
            let missing = analysis.missing_codepoints.len()
                + analysis.missing_variation_sequences.len()
                + analysis.usage.unpaired_variation_selectors.len();
            let coverage = if missing == 0 {
                "Unicode cmap retention verified".to_owned()
            } else {
                format!("{missing} Unicode cmap mappings need attention")
            };
            ui.label(
                RichText::new(format!("{coverage} · {size_kib} KiB source"))
                    .size(10.0)
                    .color(MUTED),
            );
            let policy = if analysis.embedding_metadata_allows_subsetting {
                "Embedding metadata allows subsetting · verify the font license"
            } else {
                "Embedding metadata disallows subsetting · verify the font license"
            };
            ui.label(RichText::new(policy).size(10.0).color(MUTED));
            if let Some(candidate) = &plan.candidate {
                ui.label(
                    RichText::new(candidate_size_label(analysis.source_bytes, candidate))
                        .size(10.0)
                        .strong()
                        .color(TEXT),
                )
                .on_hover_text(format!(
                    "Deterministic candidate SHA-256: {}",
                    candidate.content_hash
                ));
                ui.label(
                    RichText::new(format!(
                        "Verified with {} per-line shaping sample(s)",
                        plan.shaping_samples.len()
                    ))
                    .size(10.0)
                    .color(MUTED),
                );
            } else {
                ui.label(
                    RichText::new("No safe optimized font candidate")
                        .size(10.0)
                        .strong()
                        .color(DANGER),
                );
                for blocker in &plan.candidate_blockers {
                    ui.label(
                        RichText::new(format!("• {blocker}"))
                            .size(9.0)
                            .color(DANGER),
                    );
                }
            }
            ui.label(
                RichText::new(format!("Source: {}", analysis.source_name))
                    .size(10.0)
                    .color(MUTED),
            )
            .on_hover_text(analysis.original_path.as_ref().map_or_else(
                || "Original path unavailable".into(),
                |path| path.display().to_string(),
            ));
            ui.label(
                RichText::new(
                    "Analysis only · editable project unchanged · no smaller copy created",
                )
                .size(9.0)
                .strong()
                .color(MUTED),
            );
            ui.label(
                RichText::new("A history-safe compact-copy export is still required to create one")
                    .size(9.0)
                    .color(MUTED),
            )
            .on_hover_text(plan.physical_replacement_blockers.join("\n"));
        }
    }

    pub(super) fn paragraph_controls(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        current: &prism_core::TextTypography,
    ) {
        typography_section_label(ui, "PARAGRAPH");
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
                    self.execute(Command::SetTextTypography {
                        id,
                        typography: with_alignment(current, alignment),
                    });
                }
            }
        });

        let mut line_height = current.line_height;
        let response = ui.add(
            egui::Slider::new(&mut line_height, 0.5..=4.0)
                .text("Line height")
                .fixed_decimals(2),
        );
        self.widget_command(
            &response,
            Command::SetTextTypography {
                id,
                typography: with_line_height(current, line_height),
            },
        );

        let mut tracking = current.tracking;
        let response = ui.add(
            egui::Slider::new(&mut tracking, -100.0..=500.0)
                .text("Tracking")
                .suffix(" px"),
        );
        self.widget_command(
            &response,
            Command::SetTextTypography {
                id,
                typography: with_tracking(current, tracking),
            },
        );

        let mut wrapped = current.box_width.is_some();
        if ui.checkbox(&mut wrapped, "Wrap in text box").changed() {
            self.execute(Command::SetTextTypography {
                id,
                typography: with_box_width(
                    current,
                    wrapped.then_some(current.box_width.unwrap_or(320.0)),
                ),
            });
        }
        if let Some(mut width) = current.box_width {
            let response = ui.add(
                egui::DragValue::new(&mut width)
                    .range(1.0..=100_000.0)
                    .prefix("Width ")
                    .suffix(" px"),
            );
            self.widget_command(
                &response,
                Command::SetTextTypography {
                    id,
                    typography: with_box_width(current, Some(width)),
                },
            );
        }
    }

    pub(super) fn text_effects_controls(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        current: &prism_core::TextTypography,
    ) {
        typography_section_label(ui, "TEXT EFFECTS");
        let mut outline_width = current.effects.outline_width;
        let response = ui.add(
            egui::Slider::new(&mut outline_width, 0.0..=128.0)
                .text("Outline")
                .suffix(" px"),
        );
        self.widget_command(
            &response,
            Command::SetTextTypography {
                id,
                typography: with_outline_width(current, outline_width),
            },
        );
        let mut outline_color = color32(current.effects.outline_color);
        let response = typography_color_row(ui, "Outline color", &mut outline_color);
        self.widget_command(
            &response,
            Command::SetTextTypography {
                id,
                typography: with_outline_color(current, rgba(outline_color)),
            },
        );

        let mut shadow_x = current.effects.shadow_offset_x;
        let mut shadow_y = current.effects.shadow_offset_y;
        ui.horizontal(|ui| {
            let x_response = ui.add(
                egui::DragValue::new(&mut shadow_x)
                    .range(-2_048.0..=2_048.0)
                    .prefix("Shadow X "),
            );
            self.widget_command(
                &x_response,
                Command::SetTextTypography {
                    id,
                    typography: with_shadow_offset(current, shadow_x, shadow_y),
                },
            );
            let y_response = ui.add(
                egui::DragValue::new(&mut shadow_y)
                    .range(-2_048.0..=2_048.0)
                    .prefix("Y "),
            );
            self.widget_command(
                &y_response,
                Command::SetTextTypography {
                    id,
                    typography: with_shadow_offset(current, shadow_x, shadow_y),
                },
            );
        });
        let mut shadow_color = color32(current.effects.shadow_color);
        let response = typography_color_row(ui, "Shadow color", &mut shadow_color);
        self.widget_command(
            &response,
            Command::SetTextTypography {
                id,
                typography: with_shadow_color(current, rgba(shadow_color)),
            },
        );
    }
}

fn typography_section_label(ui: &mut egui::Ui, label: &str) {
    inspector_group_heading(ui, label);
}

fn typography_color_row(ui: &mut egui::Ui, label: &str, color: &mut Color32) -> egui::Response {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(10.0).color(MUTED));
        ui.color_edit_button_srgba(color)
    })
    .inner
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
            subset_allowed: true,
            content_hash: format!("hash-{id}"),
            path: PathBuf::from(format!("font-{id}.otf")),
            original_path: None,
        }
    }

    fn typography() -> prism_core::TextTypography {
        prism_core::TextTypography {
            font_id: Some(9),
            alignment: prism_core::TextAlignment::Right,
            line_height: 1.6,
            tracking: 12.0,
            box_width: Some(480.0),
            effects: prism_core::TextEffects {
                outline_width: 3.0,
                outline_color: [1, 2, 3, 4],
                shadow_offset_x: 5.0,
                shadow_offset_y: 6.0,
                shadow_color: [7, 8, 9, 10],
            },
        }
    }

    #[test]
    fn font_analysis_cache_key_tracks_document_tab_revision_and_font_identity() {
        let font = font(7, "Test", "Regular", 400);
        let mut workspace = Workspace::new(Document::new("Cache", 320, 200), None);
        let initial = font_usage_analysis_key(3, &workspace, &font);
        assert_eq!(initial.document_generation, 0);
        let replacement = Workspace::new(Document::new("Cache", 320, 200), None);
        assert_ne!(initial, font_usage_analysis_key(3, &replacement, &font));

        workspace
            .execute(Command::SetCanvas {
                width: 321,
                height: 200,
                background: [24, 25, 29, 255],
            })
            .unwrap();
        assert_ne!(initial, font_usage_analysis_key(3, &workspace, &font));
        assert_ne!(initial, font_usage_analysis_key(4, &workspace, &font));

        let before_replacement = font_usage_analysis_key(3, &workspace, &font);
        let mut replaced_font = font;
        replaced_font.content_hash = "replacement-hash".into();
        assert_ne!(
            before_replacement,
            font_usage_analysis_key(3, &workspace, &replaced_font)
        );
    }

    #[test]
    fn optimized_candidate_summary_is_precise_without_promising_an_export() {
        let candidate = prism_core::FontSubsetCandidate {
            content_hash: "candidate".into(),
            bytes: 2_048,
            reduction_bytes: 6_144,
        };
        assert_eq!(
            candidate_size_label(8_192, &candidate),
            "Candidate 2.0 KiB · saves 6.0 KiB (75%)"
        );
        assert_eq!(byte_size_label(900), "900 B");
        assert_eq!(byte_size_label(2_621_440), "2.5 MiB");
        assert_eq!(reduction_percent(0, 0), 0);
    }

    #[test]
    fn bundled_face_is_first_and_imported_faces_sort_stably() {
        let fonts = vec![
            font(3, "Zed", "Bold", 700),
            font(2, "Alpha", "Italic", 400),
            font(1, "Alpha", "Regular", 400),
        ];
        let choices = font_face_choices(&fonts, "");

        assert_eq!(choices[0], FontFaceChoice::bundled());
        assert_eq!(
            choices[1..]
                .iter()
                .map(|face| face.id.unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(
            font_family_names(&choices),
            vec!["Spectrum Sans", "Alpha", "Zed"]
        );
    }

    #[test]
    fn search_matches_family_style_weight_and_slant_terms() {
        let fonts = vec![
            font(1, "Atlas Grotesk", "Regular", 400),
            font(2, "Atlas Grotesk", "Bold Italic", 700),
            font(3, "Mono", "Book", 350),
        ];

        assert_eq!(
            font_face_choices(&fonts, "atlas italic")
                .iter()
                .filter_map(|face| face.id)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(
            font_face_choices(&fonts, "350")
                .iter()
                .filter_map(|face| face.id)
                .collect::<Vec<_>>(),
            vec![3]
        );
    }

    #[test]
    fn family_selection_uses_bundled_first_and_nearest_regular_face() {
        let faces = font_face_choices(
            &[
                font(1, "Atlas", "Thin", 100),
                font(2, "Atlas", "Regular", 400),
                font(3, "Atlas", "Bold", 700),
            ],
            "",
        );

        assert_eq!(
            default_face_for_family(&faces, BUNDLED_FAMILY).unwrap().id,
            None
        );
        assert_eq!(
            default_face_for_family(&faces, "Atlas").unwrap().id,
            Some(2)
        );
    }

    #[test]
    fn typography_patches_preserve_unrelated_fields() {
        let current = typography();
        let changed = with_tracking(&current, 24.0);
        assert_eq!(changed.tracking, 24.0);
        assert_eq!(changed.font_id, current.font_id);
        assert_eq!(changed.alignment, current.alignment);
        assert_eq!(changed.line_height, current.line_height);
        assert_eq!(changed.box_width, current.box_width);
        assert_eq!(changed.effects, current.effects);

        let changed = with_outline_color(&current, [20, 21, 22, 23]);
        assert_eq!(changed.effects.outline_color, [20, 21, 22, 23]);
        assert_eq!(changed.font_id, current.font_id);
        assert_eq!(changed.alignment, current.alignment);
        assert_eq!(changed.line_height, current.line_height);
        assert_eq!(changed.tracking, current.tracking);
        assert_eq!(changed.box_width, current.box_width);
        assert_eq!(changed.effects.outline_width, current.effects.outline_width);
        assert_eq!(
            changed.effects.shadow_offset_x,
            current.effects.shadow_offset_x
        );
        assert_eq!(
            changed.effects.shadow_offset_y,
            current.effects.shadow_offset_y
        );
        assert_eq!(changed.effects.shadow_color, current.effects.shadow_color);
    }
}
