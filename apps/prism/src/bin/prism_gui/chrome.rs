use super::*;

impl PrismApp {
    pub(super) fn top_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("prism-top")
            .exact_size(86.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(8)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(3.0);
                    ui.label(RichText::new("PRISM").size(15.0).strong().color(ACCENT));
                    ui.separator();
                    if ui.button("New").clicked() {
                        self.new_dialog = Some(NewDocumentDialog::default());
                    }
                    if ui.button("Open").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Prism project", &["prism", "mica"])
                            .pick_file()
                    {
                        self.open_path(&path);
                    }
                    if ui.button("Save").clicked() {
                        self.save(false);
                    }
                    if ui.button("Save As").clicked() {
                        self.save(true);
                    }
                    if ui.button(RichText::new("Export").color(ACCENT)).clicked() {
                        self.export();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(self.workspace.can_undo(), egui::Button::new("Back"))
                        .clicked()
                    {
                        self.execute(Command::Undo);
                    }
                    if ui
                        .add_enabled(self.workspace.can_redo(), egui::Button::new("Forward"))
                        .clicked()
                    {
                        self.execute(Command::Redo);
                    }
                });
                ui.add_space(5.0);
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
                                        "Save before closing"
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
            });
    }

    pub(super) fn tools(&mut self, root: &mut egui::Ui) {
        egui::Panel::left("prism-tools")
            .exact_size(58.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(7)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("TOOLS").size(9.0).color(MUTED));
                    ui.add_space(8.0);
                    for (tool, key, hint) in Tool::ALL {
                        let selected = self.tool == tool;
                        let response = tool_button(ui, tool, selected)
                            .on_hover_text(format!("{hint} ({key})"));
                        if response.clicked() {
                            self.tool = tool;
                            self.drag = None;
                        }
                        ui.add_space(3.0);
                    }
                });
            });
    }

    pub(super) fn right_panel(&mut self, root: &mut egui::Ui) {
        egui::Panel::right("prism-inspector")
            .default_size(300.0)
            .min_size(260.0)
            .max_size(380.0)
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
