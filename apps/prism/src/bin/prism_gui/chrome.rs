use super::*;

impl PrismApp {
    pub(super) fn top_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("prism-top")
            .exact_size(82.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(8)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(3.0);
                    ui.label(RichText::new("PRISM").size(15.0).strong().color(ACCENT));
                    ui.separator();
                    ui.menu_button("Project", |ui| {
                        if ui.button("New document").clicked() {
                            self.new_dialog = Some(NewDocumentDialog::default());
                            ui.close();
                        }
                        if ui.button("Open…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Prism project", &["prism", "mica"])
                                .pick_file()
                            {
                                self.open_path(&path);
                            }
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Move project…").clicked() {
                            self.begin_move_project();
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .button(if self.history.visible {
                                "Close history  ⌘H"
                            } else {
                                "History  ⌘H"
                            })
                            .clicked()
                        {
                            self.toggle_history();
                            ui.close();
                        }
                    });
                    if ui
                        .add_enabled(self.workspace.can_undo(), egui::Button::new("Back"))
                        .on_hover_text("Undo · ⌘Z")
                        .clicked()
                    {
                        self.execute(Command::Undo);
                    }
                    if ui
                        .add_enabled(self.workspace.can_redo(), egui::Button::new("Forward"))
                        .on_hover_text("Redo · ⇧⌘Z")
                        .clicked()
                    {
                        self.execute(Command::Redo);
                    }
                    if ui
                        .selectable_label(self.history.visible, "History")
                        .on_hover_text("Revision history · ⌘H")
                        .clicked()
                    {
                        self.toggle_history();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(RichText::new("Export").strong().color(ACCENT))
                            .clicked()
                        {
                            self.export();
                        }
                        ui.label(
                            RichText::new(&self.workspace.document.name)
                                .size(12.0)
                                .color(MUTED),
                        );
                    });
                });
                ui.add_space(4.0);
                self.document_tabs(ui);
            });
    }

    fn document_tabs(&mut self, ui: &mut egui::Ui) {
        let tabs: Vec<_> = self
            .tab_ids
            .iter()
            .filter_map(|id| {
                let workspace = if *id == self.active_tab_id {
                    Some(&self.workspace)
                } else {
                    self.inactive_workspaces.get(id)
                }?;
                Some((*id, workspace.document.name.clone(), workspace.is_dirty()))
            })
            .collect();
        egui::ScrollArea::horizontal()
            .id_salt("document-tabs")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut close = None;
                    for (id, name, dirty) in tabs {
                        let label = if dirty { format!("{name}  •") } else { name };
                        let response = ui
                            .selectable_label(
                                id == self.active_tab_id,
                                RichText::new(label).size(11.0).color(if dirty {
                                    ACCENT_WARM
                                } else {
                                    TEXT
                                }),
                            )
                            .on_hover_text("Switch document");
                        if response.clicked() {
                            self.activate_tab(id);
                        }
                        if ui
                            .small_button("×")
                            .on_hover_text(if dirty {
                                "Project has not finished persisting"
                            } else {
                                "Close document"
                            })
                            .clicked()
                        {
                            close = Some(id);
                        }
                        ui.separator();
                    }
                    if let Some(id) = close {
                        self.close_tab(id);
                    }
                });
            });
    }

    pub(super) fn workbench_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("prism-workbench")
            .exact_size(52.0)
            .frame(
                egui::Frame::new()
                    .fill(SURFACE)
                    .inner_margin(8)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("ACTIVE TOOL").size(9.0).strong().color(MUTED));
                    egui::Frame::new()
                        .fill(RAISED)
                        .corner_radius(5.0)
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(self.tool.label()).strong().color(ACCENT));
                                shortcut_key(ui, self.tool.shortcut());
                            });
                        });
                    if workbench_action_button(ui, "Tools & Actions", "K")
                        .on_hover_text("Search every canvas tool and one-step action")
                        .clicked()
                    {
                        self.tool_palette = Some(String::new());
                    }
                    ui.separator();
                    ui.label(
                        RichText::new(self.tool.description())
                            .size(11.0)
                            .color(MUTED),
                    );
                });
            });
    }

    pub(super) fn choose_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.drag = None;
        self.status = tool.description().into();
        self.status_error = false;
        if tool.activation() == ToolActivation::ImmediateDialog {
            self.open_new_text_dialog();
        }
    }

    pub(super) fn tool_palette_dialog(&mut self, context: &egui::Context) {
        let Some(mut query) = self.tool_palette.take() else {
            return;
        };
        let mut keep_open = true;
        let mut chosen = None;
        let has_selection = self.workspace.document.selected.is_some();
        egui::Window::new("Tools & Actions")
            .id(egui::Id::new("prism-action-palette"))
            .collapsible(false)
            .resizable(false)
            .fixed_size(Vec2::new(520.0, 520.0))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 126.0))
            .show(context, |ui| {
                ui.label(
                    RichText::new("Find a tool or run an action")
                        .size(17.0)
                        .strong(),
                );
                ui.label(
                    RichText::new(
                        "Tools change how the canvas responds. Actions happen immediately.",
                    )
                    .size(11.0)
                    .color(MUTED),
                );
                ui.add_space(6.0);
                let search = ui.add(
                    egui::TextEdit::singleline(&mut query)
                        .hint_text("Search tools and actions…")
                        .desired_width(f32::INFINITY),
                );
                search.request_focus();
                let results = palette_results(&query);
                let default = results
                    .iter()
                    .copied()
                    .find(|item| item.enabled(has_selection));
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .id_salt("action-results")
                    .max_height(390.0)
                    .show(ui, |ui| {
                        let tools: Vec<_> = results
                            .iter()
                            .copied()
                            .filter(|item| matches!(item, PaletteItem::Tool(_)))
                            .collect();
                        let actions: Vec<_> = results
                            .iter()
                            .copied()
                            .filter(|item| matches!(item, PaletteItem::PlaceImage))
                            .collect();
                        if !tools.is_empty() {
                            palette_group(
                                ui,
                                "TOOLS · change how the canvas responds",
                                &tools,
                                has_selection,
                                default,
                                &mut chosen,
                            );
                        }
                        if !tools.is_empty() && !actions.is_empty() {
                            ui.add_space(8.0);
                        }
                        if !actions.is_empty() {
                            palette_group(
                                ui,
                                "ACTIONS · happen immediately",
                                &actions,
                                has_selection,
                                default,
                                &mut chosen,
                            );
                        }
                        if results.is_empty() {
                            ui.add_space(16.0);
                            ui.label(
                                RichText::new("No matching tool or action")
                                    .size(12.0)
                                    .color(MUTED),
                            );
                        }
                    });
                match modal_action(ui) {
                    ModalAction::Cancel => keep_open = false,
                    ModalAction::Confirm => chosen = default,
                    ModalAction::None => {}
                }
            });
        if let Some(item) = chosen {
            match item {
                PaletteItem::Tool(tool) => self.choose_tool(tool),
                PaletteItem::PlaceImage => self.add_raster(),
            }
        } else if keep_open {
            self.tool_palette = Some(query);
        }
    }

    pub(super) fn right_panel(&mut self, root: &mut egui::Ui) {
        egui::Panel::right("prism-inspector")
            .default_size(370.0)
            .min_size(330.0)
            .max_size(460.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(10)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                self.layers_panel(ui);
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                self.inspector(ui);
            });
    }
}

const WORKBENCH_ACTION_SIZE: Vec2 = Vec2::new(154.0, 32.0);

fn workbench_shortcut_rect(rect: Rect) -> Rect {
    Rect::from_center_size(
        Pos2::new(rect.right() - 31.5, rect.center().y),
        Vec2::new(43.0, 20.0),
    )
}

fn workbench_action_button(ui: &mut egui::Ui, label: &str, key: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(WORKBENCH_ACTION_SIZE, Sense::click());
    let visuals = if response.is_pointer_button_down_on() {
        &ui.style().visuals.widgets.active
    } else if response.hovered() {
        &ui.style().visuals.widgets.hovered
    } else {
        &ui.style().visuals.widgets.inactive
    };
    ui.painter().rect(
        rect,
        5.0,
        visuals.bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.0),
        TEXT,
    );
    paint_command_shortcut(ui, workbench_shortcut_rect(rect), key);
    response
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaletteItem {
    Tool(Tool),
    PlaceImage,
}

impl PaletteItem {
    fn label(self) -> &'static str {
        match self {
            Self::Tool(tool) => tool.label(),
            Self::PlaceImage => "Place image",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Tool(tool) => tool.description(),
            Self::PlaceImage => "Choose a linked image and center it on the canvas.",
        }
    }

    fn shortcut(self) -> &'static str {
        match self {
            Self::Tool(tool) => tool.shortcut(),
            Self::PlaceImage => "",
        }
    }

    fn kind(self) -> &'static str {
        match self {
            Self::Tool(_) => "TOOL",
            Self::PlaceImage => "ACTION",
        }
    }

    fn enabled(self, has_selection: bool) -> bool {
        !matches!(self, Self::Tool(Tool::Mask)) || has_selection
    }

    fn matches(self, query: &str) -> bool {
        match self {
            Self::Tool(tool) => tool.matches(query),
            Self::PlaceImage => {
                let query = query.trim().to_ascii_lowercase();
                query.is_empty() || "place image import photo raster".contains(query.as_str())
            }
        }
    }
}

fn palette_results(query: &str) -> Vec<PaletteItem> {
    [
        PaletteItem::Tool(Tool::Move),
        PaletteItem::Tool(Tool::Crop),
        PaletteItem::Tool(Tool::Mask),
        PaletteItem::Tool(Tool::Text),
        PaletteItem::Tool(Tool::Rectangle),
        PaletteItem::Tool(Tool::Ellipse),
        PaletteItem::PlaceImage,
    ]
    .into_iter()
    .filter(|item| item.matches(query))
    .collect()
}

fn palette_group(
    ui: &mut egui::Ui,
    heading: &str,
    items: &[PaletteItem],
    has_selection: bool,
    default: Option<PaletteItem>,
    chosen: &mut Option<PaletteItem>,
) {
    ui.label(RichText::new(heading).size(9.0).color(MUTED));
    for item in items {
        let enabled = item.enabled(has_selection);
        if palette_row(ui, *item, enabled, default == Some(*item))
            .on_disabled_hover_text("Focus an element before drawing its mask.")
            .clicked()
        {
            *chosen = Some(*item);
        }
    }
}

fn palette_row(
    ui: &mut egui::Ui,
    item: PaletteItem,
    enabled: bool,
    default: bool,
) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        let response = ui.add_sized(
            [ui.available_width(), 48.0],
            egui::Button::new("").frame(true).stroke(Stroke::new(
                if default { 1.5 } else { 1.0 },
                if default { ACCENT } else { BORDER },
            )),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(response.rect.shrink2(Vec2::new(10.0, 5.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.label(RichText::new(item.kind()).monospace().size(9.0).color(
                    if matches!(item, PaletteItem::Tool(_)) {
                        ACCENT
                    } else {
                        ACCENT_WARM
                    },
                ));
                ui.vertical(|ui| {
                    ui.label(RichText::new(item.label()).size(12.0).strong());
                    ui.label(RichText::new(item.description()).size(10.0).color(MUTED));
                });
                if !item.shortcut().is_empty() || default {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let suffix = if default { "   Enter" } else { "" };
                        ui.label(
                            RichText::new(format!("{}{suffix}", item.shortcut()))
                                .monospace()
                                .color(if default { ACCENT } else { MUTED }),
                        );
                    });
                }
            },
        );
        response
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_search_orders_the_default_result_as_displayed() {
        assert_eq!(
            palette_results("crop").first(),
            Some(&PaletteItem::Tool(Tool::Crop))
        );
        assert_eq!(
            palette_results("image").first(),
            Some(&PaletteItem::PlaceImage)
        );
        assert!(palette_results("not a real command").is_empty());
    }

    #[test]
    fn workbench_shortcut_is_centered_inside_the_complete_control() {
        let control = Rect::from_min_size(Pos2::new(10.0, 20.0), WORKBENCH_ACTION_SIZE);
        let shortcut = workbench_shortcut_rect(control);
        assert_eq!(shortcut.center().y, control.center().y);
        assert_eq!(shortcut.height(), 20.0);
        assert_eq!(control.height(), 32.0);
    }
}
