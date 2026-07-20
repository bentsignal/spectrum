use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ShapeKind {
    #[default]
    Rectangle,
    Ellipse,
}

impl ShapeKind {
    const ALL: [Self; 2] = [Self::Rectangle, Self::Ellipse];

    fn label(self) -> &'static str {
        match self {
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Rectangle => "Draw a rectangle with editable corners, fill, and stroke.",
            Self::Ellipse => "Draw an ellipse or a standard circle.",
        }
    }

    fn matches(self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        query.is_empty()
            || self.label().to_ascii_lowercase().contains(&query)
            || self.description().to_ascii_lowercase().contains(&query)
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct PaletteState {
    query: String,
    active_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaletteNavigation {
    Next,
    Previous,
    Confirm,
    Cancel,
}

fn palette_navigation(ui: &egui::Ui) -> Option<PaletteNavigation> {
    ui.input(|input| {
        if input.key_pressed(egui::Key::ArrowDown) {
            Some(PaletteNavigation::Next)
        } else if input.key_pressed(egui::Key::ArrowUp) {
            Some(PaletteNavigation::Previous)
        } else if input.key_pressed(egui::Key::Enter) {
            Some(PaletteNavigation::Confirm)
        } else if input.key_pressed(egui::Key::Escape) {
            Some(PaletteNavigation::Cancel)
        } else {
            None
        }
    })
}

fn normalized_active_index(
    active_index: usize,
    result_count: usize,
    enabled: impl Fn(usize) -> bool,
) -> usize {
    if active_index < result_count && enabled(active_index) {
        active_index
    } else {
        (0..result_count).find(|index| enabled(*index)).unwrap_or(0)
    }
}

fn step_active_index(
    active_index: usize,
    result_count: usize,
    forward: bool,
    enabled: impl Fn(usize) -> bool,
) -> usize {
    if result_count == 0 {
        return 0;
    }
    for offset in 1..=result_count {
        let index = if forward {
            (active_index + offset) % result_count
        } else {
            (active_index + result_count - (offset % result_count)) % result_count
        };
        if enabled(index) {
            return index;
        }
    }
    0
}

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
                                let label = if self.tool == Tool::Shape {
                                    format!("Shape · {}", self.shape_kind.label())
                                } else {
                                    self.tool.label().into()
                                };
                                ui.label(RichText::new(label).strong().color(ACCENT));
                                shortcut_key(ui, self.tool.shortcut());
                            });
                        });
                    if workbench_action_button(
                        ui,
                        "Tools & Actions",
                        shortcuts::GlobalShortcut::ToolsAndActions.label(),
                    )
                    .on_hover_text("Search every canvas tool and one-step action")
                    .clicked()
                    {
                        self.tool_palette = Some(PaletteState::default());
                    }
                    ui.separator();
                    ui.label(
                        RichText::new(if self.tool == Tool::Shape {
                            self.shape_kind.description()
                        } else {
                            self.tool.description()
                        })
                        .size(11.0)
                        .color(MUTED),
                    );
                });
            });
    }

    pub(super) fn choose_tool(&mut self, tool: Tool) {
        if tool.activation() == ToolActivation::ChoiceDialog {
            self.shape_palette = Some(PaletteState::default());
            return;
        }
        self.tool = tool;
        self.drag = None;
        self.status = tool.description().into();
        self.status_error = false;
        if tool.activation() == ToolActivation::ImmediateDialog {
            self.open_new_text_dialog();
        }
    }

    fn choose_shape(&mut self, shape: ShapeKind) {
        self.shape_kind = shape;
        self.tool = Tool::Shape;
        self.drag = None;
        self.status = format!("{} ready · {}", shape.label(), shape.description());
        self.status_error = false;
    }

    pub(super) fn tool_palette_dialog(&mut self, context: &egui::Context) {
        if self.shape_palette.is_some() {
            self.shape_palette_dialog(context);
            return;
        }
        let Some(mut state) = self.tool_palette.take() else {
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
                    egui::TextEdit::singleline(&mut state.query)
                        .hint_text("Search tools and actions…")
                        .desired_width(f32::INFINITY),
                );
                search.request_focus();
                let results = palette_results(&state.query);
                if search.changed() {
                    state.active_index = 0;
                }
                state.active_index =
                    normalized_active_index(state.active_index, results.len(), |index| {
                        results[index].enabled(has_selection)
                    });
                let mut scroll_to_active = false;
                match palette_navigation(ui) {
                    Some(PaletteNavigation::Next) => {
                        state.active_index =
                            step_active_index(state.active_index, results.len(), true, |index| {
                                results[index].enabled(has_selection)
                            });
                        scroll_to_active = true;
                    }
                    Some(PaletteNavigation::Previous) => {
                        state.active_index =
                            step_active_index(state.active_index, results.len(), false, |index| {
                                results[index].enabled(has_selection)
                            });
                        scroll_to_active = true;
                    }
                    Some(PaletteNavigation::Confirm) => {
                        chosen = results
                            .get(state.active_index)
                            .copied()
                            .filter(|item| item.enabled(has_selection));
                    }
                    Some(PaletteNavigation::Cancel) => keep_open = false,
                    None => {}
                }
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .id_salt("action-results")
                    .max_height(390.0)
                    .show(ui, |ui| {
                        let tools: Vec<_> = results
                            .iter()
                            .enumerate()
                            .filter(|(_, item)| matches!(item, PaletteItem::Tool(_)))
                            .map(|(index, item)| (index, *item))
                            .collect();
                        let actions: Vec<_> = results
                            .iter()
                            .enumerate()
                            .filter(|(_, item)| matches!(item, PaletteItem::PlaceImage))
                            .map(|(index, item)| (index, *item))
                            .collect();
                        if !tools.is_empty() {
                            palette_group(
                                ui,
                                "TOOLS · change how the canvas responds",
                                &tools,
                                has_selection,
                                &mut state.active_index,
                                scroll_to_active,
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
                                &mut state.active_index,
                                scroll_to_active,
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
            });
        if let Some(item) = chosen {
            match item {
                PaletteItem::Tool(tool) => self.choose_tool(tool),
                PaletteItem::PlaceImage => self.add_raster(),
            }
        } else if keep_open {
            self.tool_palette = Some(state);
        }
    }

    fn shape_palette_dialog(&mut self, context: &egui::Context) {
        let Some(mut state) = self.shape_palette.take() else {
            return;
        };
        let mut keep_open = true;
        let mut chosen = None;
        egui::Window::new("Choose a shape")
            .id(egui::Id::new("prism-shape-palette"))
            .collapsible(false)
            .resizable(false)
            .fixed_size(Vec2::new(480.0, 300.0))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 150.0))
            .show(context, |ui| {
                ui.label(
                    RichText::new("What do you want to draw?")
                        .size(17.0)
                        .strong(),
                );
                ui.label(
                    RichText::new("Search shapes now; new shape types will appear here.")
                        .size(11.0)
                        .color(MUTED),
                );
                ui.add_space(6.0);
                let search = ui.add(
                    egui::TextEdit::singleline(&mut state.query)
                        .hint_text("Search shapes…")
                        .desired_width(f32::INFINITY),
                );
                search.request_focus();
                let results: Vec<_> = ShapeKind::ALL
                    .into_iter()
                    .filter(|shape| shape.matches(&state.query))
                    .collect();
                if search.changed() {
                    state.active_index = 0;
                }
                state.active_index =
                    normalized_active_index(state.active_index, results.len(), |_| true);
                let mut scroll_to_active = false;
                match palette_navigation(ui) {
                    Some(PaletteNavigation::Next) => {
                        state.active_index =
                            step_active_index(state.active_index, results.len(), true, |_| true);
                        scroll_to_active = true;
                    }
                    Some(PaletteNavigation::Previous) => {
                        state.active_index =
                            step_active_index(state.active_index, results.len(), false, |_| true);
                        scroll_to_active = true;
                    }
                    Some(PaletteNavigation::Confirm) => {
                        chosen = results.get(state.active_index).copied();
                    }
                    Some(PaletteNavigation::Cancel) => keep_open = false,
                    None => {}
                }
                ui.add_space(8.0);
                for (index, shape) in results.iter().copied().enumerate() {
                    let active = index == state.active_index;
                    let response = shape_palette_row(ui, shape, active);
                    if active && scroll_to_active {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.hovered() {
                        state.active_index = index;
                    }
                    if response.clicked() {
                        chosen = Some(shape);
                    }
                }
                if results.is_empty() {
                    ui.add_space(16.0);
                    ui.label(RichText::new("No matching shape").size(12.0).color(MUTED));
                }
            });
        if let Some(shape) = chosen {
            self.choose_shape(shape);
        } else if keep_open {
            self.shape_palette = Some(state);
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
        PaletteItem::Tool(Tool::Shape),
        PaletteItem::PlaceImage,
    ]
    .into_iter()
    .filter(|item| item.matches(query))
    .collect()
}

fn palette_group(
    ui: &mut egui::Ui,
    heading: &str,
    items: &[(usize, PaletteItem)],
    has_selection: bool,
    active_index: &mut usize,
    scroll_to_active: bool,
    chosen: &mut Option<PaletteItem>,
) {
    ui.label(RichText::new(heading).size(9.0).color(MUTED));
    for (index, item) in items {
        let enabled = item.enabled(has_selection);
        let active = enabled && *active_index == *index;
        let response = palette_row(ui, *item, enabled, active)
            .on_disabled_hover_text("Focus an element before drawing its mask.");
        if active && scroll_to_active {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        if enabled && response.hovered() {
            *active_index = *index;
        }
        if response.clicked() {
            *chosen = Some(*item);
        }
    }
}

fn palette_row(
    ui: &mut egui::Ui,
    item: PaletteItem,
    enabled: bool,
    active: bool,
) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        let response = ui.add_sized(
            [ui.available_width(), 48.0],
            egui::Button::new("")
                .frame(true)
                .fill(if active { SELECTED_SURFACE } else { RAISED })
                .stroke(Stroke::new(
                    if active { 1.5 } else { 1.0 },
                    if active { ACCENT } else { BORDER },
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
                if !item.shortcut().is_empty() || active {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let suffix = if active { "   Return" } else { "" };
                        ui.label(
                            RichText::new(format!("{}{suffix}", item.shortcut()))
                                .monospace()
                                .color(if active { ACCENT } else { MUTED }),
                        );
                    });
                }
            },
        );
        response
    })
    .inner
}

fn shape_palette_row(ui: &mut egui::Ui, shape: ShapeKind, active: bool) -> egui::Response {
    let response = ui.add_sized(
        [ui.available_width(), 56.0],
        egui::Button::new("")
            .fill(if active { SELECTED_SURFACE } else { RAISED })
            .stroke(Stroke::new(
                if active { 1.5 } else { 1.0 },
                if active { ACCENT } else { BORDER },
            )),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(response.rect.shrink2(Vec2::new(12.0, 6.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(shape.label()).size(12.0).strong());
                ui.label(RichText::new(shape.description()).size(10.0).color(MUTED));
            });
            if active {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("Return").monospace().color(ACCENT));
                });
            }
        },
    );
    response
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
    fn shape_is_one_searchable_tool_with_extensible_choices() {
        assert_eq!(
            palette_results("rectangle"),
            vec![PaletteItem::Tool(Tool::Shape)]
        );
        assert_eq!(
            palette_results("ellipse"),
            vec![PaletteItem::Tool(Tool::Shape)]
        );
        assert_eq!(
            palette_results("")
                .into_iter()
                .filter(|item| matches!(item, PaletteItem::Tool(Tool::Shape)))
                .count(),
            1
        );
        assert_eq!(ShapeKind::ALL, [ShapeKind::Rectangle, ShapeKind::Ellipse]);
        assert!(ShapeKind::Ellipse.matches("circle"));
    }

    #[test]
    fn keyboard_navigation_wraps_and_skips_disabled_rows() {
        let enabled = |index| index != 1;
        assert_eq!(normalized_active_index(1, 4, enabled), 0);
        assert_eq!(step_active_index(0, 4, true, enabled), 2);
        assert_eq!(step_active_index(0, 4, false, enabled), 3);
        assert_eq!(step_active_index(3, 4, true, enabled), 0);
        assert_eq!(step_active_index(0, 0, true, |_| true), 0);
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
