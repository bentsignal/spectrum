use super::*;

impl PrismApp {
    pub(super) fn canvas(&mut self, root: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(INK).inner_margin(12))
            .show(root, |ui| {
                let available = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(available, Sense::click_and_drag());
                let geometry = canvas_geometry(
                    available,
                    self.workspace.document.width,
                    self.workspace.document.height,
                    self.zoom,
                    self.pan,
                );
                if self.fit_requested {
                    self.zoom = 1.0;
                    self.pan = Vec2::ZERO;
                    self.fit_requested = false;
                }
                self.ensure_layer_visuals(
                    ui.ctx(),
                    geometry.pixels_per_point * ui.ctx().pixels_per_point(),
                );
                self.canvas_interaction(ui, &response, geometry);
                let direct_manipulation = direct_manipulation_preview(self.drag);
                if document_requires_composite_preview(&self.workspace.document)
                    && !direct_manipulation
                {
                    let frame = match self.composite_preview.ensure(
                        ui.ctx(),
                        self.active_tab_id,
                        &self.workspace.document,
                        geometry,
                        ui.ctx().pixels_per_point(),
                    ) {
                        Ok(frame) => {
                            self.preview_error = None;
                            frame
                        }
                        Err(error) => {
                            self.preview_error = Some(error);
                            None
                        }
                    };
                    if let Some(frame) = frame.as_ref() {
                        paint_composite_preview(ui, geometry, Some(frame));
                    } else {
                        // Never put a stale composited surface behind a current transform. The
                        // cached per-layer visuals are an immediate GPU-transformed preview while
                        // the exact blend/mask compositor catches up after the gesture.
                        paint_interactive_document(
                            ui,
                            geometry,
                            &self.workspace.document,
                            &self.layer_visuals,
                        );
                    }
                } else {
                    paint_interactive_document(
                        ui,
                        geometry,
                        &self.workspace.document,
                        &self.layer_visuals,
                    );
                }
                if let Some(layer) = self.selected_layer() {
                    if selection_outline_has_resize_handles(self.tool) {
                        paint_layer_outline(
                            ui,
                            geometry,
                            layer,
                            self.layer_source_geometry(layer),
                            Vec2::ZERO,
                        );
                    } else {
                        paint_rotation_outline(
                            ui,
                            geometry,
                            layer,
                            self.layer_source_geometry(layer),
                        );
                    }
                }
                if let Some(drag) = self.drag {
                    self.paint_drag(ui, geometry, drag);
                }
                if let Some(error) = &self.preview_error {
                    ui.painter().text(
                        available.center(),
                        Align2::CENTER_CENTER,
                        error,
                        FontId::proportional(13.0),
                        DANGER,
                    );
                }
            });
    }

    pub(super) fn canvas_interaction(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        geometry: CanvasGeometry,
    ) {
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll.abs() > 0.1 {
                self.zoom = (self.zoom * (scroll * 0.0015).exp()).clamp(0.1, 16.0);
            }
        }
        if ui.input(|input| input.pointer.middle_down()) && response.dragged() {
            self.pan += response.drag_delta();
            return;
        }
        if self.tool == Tool::Rotate && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        let pointer = response.interact_pointer_pos();
        let hover_pointer = ui
            .input(|input| input.pointer.hover_pos())
            .filter(|pointer| response.rect.contains(*pointer));
        let resize_hover = if self.tool == Tool::Move {
            if let Some(DragState {
                action: DragAction::Resize(handle),
                ..
            }) = self.drag
            {
                Some(handle)
            } else {
                hover_pointer.and_then(|pointer| {
                    self.selected_layer().and_then(|layer| {
                        resize_handle_at(
                            geometry,
                            layer,
                            self.layer_source_geometry(layer),
                            pointer,
                        )
                    })
                })
            }
        } else {
            None
        };
        if let Some(handle) = resize_hover {
            ui.ctx().set_cursor_icon(resize_cursor(handle));
        }
        if response.drag_started()
            && let Some(pointer) = pointer
        {
            let press_pointer = ui
                .input(|input| input.pointer.press_origin())
                .unwrap_or(pointer);
            let canvas = geometry.screen_to_canvas(press_pointer);
            let resize = if self.tool == Tool::Move {
                self.selected_layer().and_then(|layer| {
                    resize_handle_at(
                        geometry,
                        layer,
                        self.layer_source_geometry(layer),
                        press_pointer,
                    )
                })
            } else {
                None
            };
            if self.tool == Tool::Move && resize.is_none() {
                let hit = self.hit_test_layer(canvas);
                if hit != self.workspace.document.selected {
                    self.execute(Command::SelectLayer { id: hit });
                }
            }
            let selected = self.selected_layer().cloned();
            let action = match (self.tool, resize) {
                (Tool::Move, Some(handle)) => DragAction::Resize(handle),
                (Tool::Move, None) => DragAction::Move,
                (Tool::Rotate, _) => DragAction::Rotate,
                _ => DragAction::Draw,
            };
            let editable = selected.as_ref().is_some_and(|layer| !layer.locked)
                && matches!(
                    action,
                    DragAction::Move | DragAction::Rotate | DragAction::Resize(_)
                );
            if editable {
                self.workspace.begin_interaction();
            }
            let visual_rotation_bounds = selected
                .as_ref()
                .is_some_and(|layer| matches!(layer.kind, LayerKind::Text { .. }));
            self.drag = Some(DragState {
                start_canvas: canvas,
                current_canvas: geometry.screen_to_canvas(pointer),
                layer_id: selected
                    .as_ref()
                    .filter(|layer| !layer.locked)
                    .map(|layer| layer.id),
                transform: selected
                    .as_ref()
                    .map(|layer| layer.transform)
                    .unwrap_or_default(),
                bounds: self
                    .selected_layer()
                    .and_then(|layer| layer_bounds(layer, self.layer_source_geometry(layer))),
                action,
                visual_rotation_bounds,
            });
        }
        if response.dragged()
            && let (Some(pointer), Some(drag)) = (pointer, self.drag.as_mut())
        {
            drag.current_canvas = geometry.screen_to_canvas(pointer);
        }
        if response.dragged()
            && let Some(drag) = self.drag
            && let Some(id) = drag.layer_id
            && matches!(
                drag.action,
                DragAction::Move | DragAction::Rotate | DragAction::Resize(_)
            )
        {
            if drag.action == DragAction::Rotate {
                let snap = ui.input(|input| input.modifiers.shift);
                self.preview_command(Command::SetRotation {
                    id,
                    degrees: drag_rotation(drag, snap),
                });
            } else {
                let preserve_aspect = !ui.input(|input| input.modifiers.shift);
                let transform = drag_transform(drag, preserve_aspect);
                self.preview_command(Command::SetTransform { id, transform });
            }
        }
        if response.drag_stopped()
            && let Some(drag) = self.drag.take()
        {
            self.finish_canvas_drag(drag);
        } else if response.double_clicked()
            && let Some(pointer) = pointer
        {
            self.canvas_double_click(geometry.screen_to_canvas(pointer));
        } else if response.clicked()
            && let Some(pointer) = pointer
        {
            self.canvas_click(geometry.screen_to_canvas(pointer));
        }
    }

    pub(super) fn paint_drag(&self, ui: &egui::Ui, geometry: CanvasGeometry, drag: DragState) {
        let start = geometry.canvas_to_screen(drag.start_canvas);
        let current = geometry.canvas_to_screen(drag.current_canvas);
        match drag.action {
            DragAction::Resize(_) => {
                if let Some(id) = drag.layer_id
                    && let Ok(layer) = self.workspace.document.layer(id)
                {
                    paint_layer_outline(
                        ui,
                        geometry,
                        layer,
                        self.layer_source_geometry(layer),
                        Vec2::ZERO,
                    );
                }
                ui.painter().text(
                    current,
                    Align2::LEFT_BOTTOM,
                    if ui.input(|input| input.modifiers.shift) {
                        "Free resize"
                    } else {
                        "Proportional resize · Shift for free"
                    },
                    FontId::monospace(11.0),
                    ACCENT,
                );
            }
            DragAction::Draw => match (self.tool, self.shape_kind) {
                (Tool::Shape, chrome::ShapeKind::Rectangle) | (Tool::Crop, _) | (Tool::Mask, _) => {
                    let rect = Rect::from_two_pos(start, current);
                    ui.painter().rect_filled(rect, 1.0, with_alpha(ACCENT, 30));
                    ui.painter().rect_stroke(
                        rect,
                        1.0,
                        Stroke::new(1.5, ACCENT),
                        egui::StrokeKind::Inside,
                    );
                }
                (Tool::Shape, chrome::ShapeKind::Ellipse) => {
                    let rect = Rect::from_two_pos(start, current);
                    let radius = rect.size() * 0.5;
                    ui.painter().add(egui::Shape::ellipse_filled(
                        rect.center(),
                        radius,
                        with_alpha(ACCENT, 30),
                    ));
                    ui.painter().add(egui::Shape::ellipse_stroke(
                        rect.center(),
                        radius,
                        Stroke::new(1.5, ACCENT),
                    ));
                }
                _ => {}
            },
            DragAction::Move => {
                let delta = drag.current_canvas - drag.start_canvas;
                if let Some(id) = drag.layer_id
                    && let Ok(layer) = self.workspace.document.layer(id)
                {
                    paint_layer_outline(
                        ui,
                        geometry,
                        layer,
                        self.layer_source_geometry(layer),
                        Vec2::ZERO,
                    );
                }
                ui.painter().text(
                    current,
                    Align2::LEFT_BOTTOM,
                    format!("{:+.0}, {:+.0}", delta.x, delta.y),
                    FontId::monospace(11.0),
                    ACCENT,
                );
            }
            DragAction::Rotate => {
                let snapped = ui.input(|input| input.modifiers.shift);
                ui.painter().text(
                    current,
                    Align2::LEFT_BOTTOM,
                    if snapped {
                        format!("{:.0}° · snapped 15°", drag_rotation(drag, true))
                    } else {
                        format!("{:.1}° · Shift to snap", drag_rotation(drag, false))
                    },
                    FontId::monospace(11.0),
                    ACCENT,
                );
            }
        }
    }

    pub(super) fn finish_canvas_drag(&mut self, drag: DragState) {
        let min = drag.start_canvas.min(drag.current_canvas);
        let max = drag.start_canvas.max(drag.current_canvas);
        let size = max - min;
        match drag.action {
            DragAction::Move | DragAction::Resize(_) => self.finish_interaction(),
            DragAction::Rotate => {
                self.finish_interaction();
                self.tool = Tool::Move;
            }
            DragAction::Draw => match (self.tool, self.shape_kind) {
                (Tool::Shape, chrome::ShapeKind::Rectangle) if size.x > 2.0 && size.y > 2.0 => {
                    self.execute(Command::AddRectangle {
                        name: None,
                        width: size.x.round().max(1.0) as u32,
                        height: size.y.round().max(1.0) as u32,
                        color: [93, 216, 199, 255],
                        corner_radius: 10.0,
                        x: min.x,
                        y: min.y,
                    });
                    self.tool = Tool::Move;
                }
                (Tool::Shape, chrome::ShapeKind::Ellipse) if size.x > 2.0 && size.y > 2.0 => {
                    self.execute(Command::AddEllipse {
                        name: None,
                        width: size.x.round().max(1.0) as u32,
                        height: size.y.round().max(1.0) as u32,
                        color: [247, 178, 102, 255],
                        x: min.x,
                        y: min.y,
                    });
                    self.tool = Tool::Move;
                }
                (Tool::Crop, _) if size.x > 2.0 && size.y > 2.0 => {
                    self.execute(Command::CropCanvas {
                        x: min.x.max(0.0).round() as u32,
                        y: min.y.max(0.0).round() as u32,
                        width: size.x.round() as u32,
                        height: size.y.round() as u32,
                    });
                    self.fit_requested = true;
                    self.tool = Tool::Move;
                }
                (Tool::Mask, _) if size.x > 2.0 && size.y > 2.0 => {
                    if let Some(id) = drag.layer_id {
                        let width = self.workspace.document.width as f32;
                        let height = self.workspace.document.height as f32;
                        self.execute(Command::SetMask {
                            id,
                            mask: LayerMask {
                                enabled: true,
                                x: (min.x / width).clamp(0.0, 0.99),
                                y: (min.y / height).clamp(0.0, 0.99),
                                width: (size.x / width).clamp(0.001, 1.0),
                                height: (size.y / height).clamp(0.001, 1.0),
                                invert: false,
                            },
                        });
                        self.tool = Tool::Move;
                    }
                }
                _ => {}
            },
        }
    }

    pub(super) fn canvas_click(&mut self, position: Pos2) {
        match self.tool {
            Tool::Move => {
                let hit = self.hit_test_layer(position);
                if hit != self.workspace.document.selected {
                    self.execute(Command::SelectLayer { id: hit });
                }
            }
            Tool::Text => {
                self.text_dialog = Some(TextDialogDraft {
                    target: TextDialogTarget::New { position },
                    text: "Text".into(),
                    font_size: 72.0,
                    color: [245, 246, 250, 255],
                });
            }
            Tool::Shape => {
                match self.shape_kind {
                    chrome::ShapeKind::Rectangle => {
                        self.execute(Command::AddRectangle {
                            name: None,
                            width: 320,
                            height: 180,
                            color: [93, 216, 199, 255],
                            corner_radius: 12.0,
                            x: position.x,
                            y: position.y,
                        });
                    }
                    chrome::ShapeKind::Ellipse => {
                        self.execute(Command::AddEllipse {
                            name: None,
                            width: 240,
                            height: 240,
                            color: [247, 178, 102, 255],
                            x: position.x,
                            y: position.y,
                        });
                    }
                }
                self.tool = Tool::Move;
            }
            _ => {}
        }
    }

    fn canvas_double_click(&mut self, position: Pos2) {
        let hit = self.hit_test_layer(position);
        if hit != self.workspace.document.selected {
            self.execute(Command::SelectLayer { id: hit });
        }
        if let Some(id) = hit {
            self.open_text_editor(id);
        }
    }

    pub(super) fn hit_test_layer(&self, position: Pos2) -> Option<u64> {
        self.workspace
            .document
            .layers
            .iter()
            .rev()
            .filter(|layer| layer.visible)
            .find(|layer| layer_contains_point(layer, self.layer_source_geometry(layer), position))
            .map(|layer| layer.id)
    }
}

fn direct_manipulation_preview(drag: Option<DragState>) -> bool {
    drag.is_some_and(|drag| {
        drag.layer_id.is_some()
            && matches!(
                drag.action,
                DragAction::Move | DragAction::Rotate | DragAction::Resize(_)
            )
    })
}

#[cfg(test)]
mod direct_manipulation_tests {
    use super::*;

    fn drag(action: DragAction, layer_id: Option<u64>) -> DragState {
        DragState {
            start_canvas: Pos2::ZERO,
            current_canvas: Pos2::new(20.0, 10.0),
            layer_id,
            transform: Transform::default(),
            action,
            bounds: None,
            visual_rotation_bounds: false,
        }
    }

    #[test]
    fn layer_transform_gestures_bypass_the_async_compositor() {
        assert!(direct_manipulation_preview(Some(drag(
            DragAction::Move,
            Some(7)
        ))));
        assert!(direct_manipulation_preview(Some(drag(
            DragAction::Rotate,
            Some(7)
        ))));
        assert!(direct_manipulation_preview(Some(drag(
            DragAction::Resize(ResizeHandle::BottomRight),
            Some(7)
        ))));
        assert!(!direct_manipulation_preview(Some(drag(
            DragAction::Draw,
            Some(7)
        ))));
        assert!(!direct_manipulation_preview(Some(drag(
            DragAction::Move,
            None
        ))));
        assert!(!direct_manipulation_preview(None));
    }
}

fn selection_outline_has_resize_handles(tool: Tool) -> bool {
    tool != Tool::Rotate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_uses_a_stroke_only_selection_outline() {
        assert!(!selection_outline_has_resize_handles(Tool::Rotate));
        assert!(selection_outline_has_resize_handles(Tool::Move));
    }
}
