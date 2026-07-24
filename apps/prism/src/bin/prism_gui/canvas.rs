use super::*;

const MIN_PARAGRAPH_TEXT_SCREEN_POINTS: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct TextDragPlacement {
    position: Pos2,
    box_width: Option<f32>,
}

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
                self.handle_canvas_file_drops(ui, geometry);
                self.canvas_interaction(ui, &response, geometry);
                // Resolve the frame's gesture before deciding which preview raster work is
                // needed. In particular, the first resize frame must reuse the existing layer
                // texture instead of dispatching settled-resolution work for the pre-drag
                // transform and immediately invalidating it.
                self.ensure_layer_visuals(
                    ui.ctx(),
                    geometry.pixels_per_point * ui.ctx().pixels_per_point(),
                );
                let direct_manipulation = direct_manipulation_preview(self.drag);
                let preview_document = self
                    .brush
                    .preview
                    .as_ref()
                    .unwrap_or(&self.workspace.document);
                if (self.brush.preview.is_some()
                    || document_requires_composite_preview(preview_document))
                    && !direct_manipulation
                {
                    let raster_sources = self.raster_sources.snapshot();
                    let frame = match self.composite_preview.ensure(
                        ui.ctx(),
                        self.active_tab_id,
                        preview_document,
                        geometry,
                        ui.ctx().pixels_per_point(),
                        raster_sources,
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
                            preview_document,
                            &self.layer_visuals,
                        );
                    }
                } else {
                    paint_interactive_document(ui, geometry, preview_document, &self.layer_visuals);
                }
                self.paint_alignment_guides(ui, geometry);
                let replacing_marquee = (self.tool == Tool::Marquee
                    && self
                        .drag
                        .is_some_and(|drag| drag.action == DragAction::Draw))
                    || (self.tool == Tool::Lasso
                        && !self.selection_ui.lasso_points.is_empty()
                        && self.selection_ui.lasso_gesture_mode
                            == Some(prism_core::SelectionCombineMode::Replace));
                let selection_overlay = self.ensure_selection_overlay(geometry, ui.clip_rect());
                if !replacing_marquee && let Some(selection) = &self.workspace.document.selection {
                    paint_selection_overlay(ui, geometry, selection, selection_overlay.as_deref());
                }
                lasso_tool::paint_lasso_draft(ui, geometry, &self.selection_ui.lasso_points);
                if self.tool != Tool::Marquee
                    && let Some(layer) = self.selected_layer()
                {
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
                self.paint_pen_overlay(ui, geometry);
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
                self.inline_text_editor_ui(ui.ctx(), geometry);
            });
    }

    pub(super) fn canvas_interaction(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        geometry: CanvasGeometry,
    ) {
        if !inline_text::canvas_gestures_allowed(self.inline_text_editor.as_ref()) {
            return;
        }
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
        if self.tool == Tool::Pen {
            self.pen_canvas_interaction(ui, response, geometry);
            return;
        }
        if matches!(self.tool, Tool::Brush | Tool::Eraser) {
            self.brush_canvas_interaction(ui, response, geometry);
            return;
        }
        if self.tool == Tool::Lasso {
            self.lasso_canvas_interaction(ui, response, geometry);
            return;
        }
        if matches!(
            self.tool,
            Tool::Rotate | Tool::Marquee | Tool::Lasso | Tool::MagicWand
        ) && response.hovered()
        {
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
        let guide_hover = if self.tool == Tool::Move {
            hover_pointer.and_then(|pointer| self.guide_at_pointer(geometry, pointer))
        } else {
            None
        };
        if let Some(id) = guide_hover
            && let Some(cursor) = self.guide_cursor(id)
        {
            ui.ctx().set_cursor_icon(cursor);
        } else if let Some(handle) = resize_hover {
            let rotation = self
                .drag
                .filter(|drag| matches!(drag.action, DragAction::Resize(_)))
                .map(|drag| drag.transform.rotation)
                .or_else(|| self.selected_layer().map(|layer| layer.transform.rotation))
                .unwrap_or_default();
            ui.ctx().set_cursor_icon(resize_cursor(handle, rotation));
        }
        if response.drag_started()
            && let Some(pointer) = pointer
        {
            let press_pointer = ui
                .input(|input| input.pointer.press_origin())
                .unwrap_or(pointer);
            let Some(canvas) = canvas_interaction_position(self.tool, geometry, press_pointer)
            else {
                return;
            };
            let guide = if self.tool == Tool::Move {
                self.guide_at_pointer(geometry, press_pointer)
            } else {
                None
            };
            let resize = if self.tool == Tool::Move && guide.is_none() {
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
            if self.tool == Tool::Move && resize.is_none() && guide.is_none() {
                let hit = self.hit_test_layer(canvas);
                if hit != self.workspace.document.selected {
                    self.execute(Command::SelectLayer { id: hit });
                }
            }
            let selected = self.selected_layer().cloned();
            let selected_source_geometry = selected
                .as_ref()
                .and_then(|layer| self.layer_source_geometry(layer));
            let action = match (self.tool, resize, guide) {
                (Tool::Move, _, Some(id)) => DragAction::Guide(id),
                (Tool::Move, Some(handle), None) => DragAction::Resize(handle),
                (Tool::Move, None, None) => DragAction::Move,
                (Tool::Rotate, _, _) => DragAction::Rotate,
                _ => DragAction::Draw,
            };
            let editable = matches!(action, DragAction::Guide(_))
                || selected.as_ref().is_some_and(|layer| !layer.locked)
                    && matches!(
                        action,
                        DragAction::Move | DragAction::Rotate | DragAction::Resize(_)
                    );
            if editable {
                self.workspace.begin_interaction();
            }
            let paragraph_source_override = if matches!(
                action,
                DragAction::Resize(ResizeHandle::ParagraphLeft | ResizeHandle::ParagraphRight)
            ) {
                selected_source_geometry
            } else {
                None
            };
            self.drag = Some(DragState {
                start_canvas: canvas,
                current_canvas: geometry.screen_to_canvas(pointer),
                layer_id: if matches!(action, DragAction::Guide(_)) {
                    None
                } else {
                    selected
                        .as_ref()
                        .filter(|layer| !layer.locked)
                        .map(|layer| layer.id)
                },
                transform: selected
                    .as_ref()
                    .map(|layer| layer.transform)
                    .unwrap_or_default(),
                bounds: selected
                    .as_ref()
                    .and_then(|layer| layer_bounds(layer, selected_source_geometry)),
                paragraph_bounds: selected
                    .as_ref()
                    .and_then(|layer| paragraph_layer_bounds(layer, selected_source_geometry)),
                paragraph_width: selected.as_ref().and_then(paragraph_box_width),
                paragraph_source_override,
                action,
            });
            self.smart_guides = SmartGuides::default();
        }
        if response.dragged()
            && let (Some(pointer), Some(drag)) = (pointer, self.drag.as_mut())
        {
            drag.current_canvas = geometry.screen_to_canvas(pointer);
        }
        if response.dragged()
            && let Some(drag) = self.drag
        {
            match (drag.action, drag.layer_id) {
                (DragAction::Rotate, Some(id)) => {
                    let snap = ui.input(|input| input.modifiers.shift);
                    self.preview_command(Command::SetRotation {
                        id,
                        degrees: drag_rotation(drag, snap),
                    });
                }
                (DragAction::Move, Some(id)) => {
                    let moved = self.snapped_move(
                        id,
                        drag_transform(drag, true),
                        geometry.pixels_per_point,
                    );
                    self.smart_guides = moved.guides;
                    self.preview_command(Command::SetTransform {
                        id,
                        transform: moved.transform,
                    });
                }
                (
                    DragAction::Resize(
                        handle @ (ResizeHandle::ParagraphLeft | ResizeHandle::ParagraphRight),
                    ),
                    Some(id),
                ) => {
                    self.smart_guides = SmartGuides::default();
                    self.preview_paragraph_width_drag(id, drag, handle);
                }
                (DragAction::Resize(_), Some(id)) => {
                    self.smart_guides = SmartGuides::default();
                    let preserve_aspect = !ui.input(|input| input.modifiers.shift);
                    self.preview_command(Command::SetTransform {
                        id,
                        transform: drag_transform(drag, preserve_aspect),
                    });
                }
                (DragAction::Guide(_) | DragAction::Draw, _)
                | (DragAction::Move | DragAction::Rotate | DragAction::Resize(_), None) => {}
            }
        }
        if response.dragged()
            && let Some(drag) = self.drag
            && let DragAction::Guide(id) = drag.action
            && let Some(position) = self.guide_position(id, drag.current_canvas)
        {
            self.preview_command(Command::MoveGuide { id, position });
        }
        if response.drag_stopped()
            && let Some(drag) = self.drag.take()
        {
            self.finish_canvas_drag(drag, geometry);
            self.smart_guides = SmartGuides::default();
        } else if response.double_clicked()
            && let Some(pointer) = pointer
        {
            self.canvas_double_click(geometry.screen_to_canvas(pointer));
        } else if response.clicked()
            && let Some(pointer) = pointer
            && let Some(position) = canvas_interaction_position(self.tool, geometry, pointer)
        {
            self.canvas_click(position);
        }
    }

    pub(super) fn paint_drag(&self, ui: &egui::Ui, geometry: CanvasGeometry, drag: DragState) {
        let start = geometry.canvas_to_screen(drag.start_canvas);
        let current = geometry.canvas_to_screen(drag.current_canvas);
        match drag.action {
            DragAction::Resize(handle) => {
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
                    if matches!(
                        handle,
                        ResizeHandle::ParagraphLeft | ResizeHandle::ParagraphRight
                    ) {
                        paragraph_width_from_drag(drag, handle)
                            .map(|width| format!("{width:.0} px text width"))
                            .unwrap_or_else(|| "Text width".into())
                    } else if ui.input(|input| input.modifiers.shift) {
                        "Free resize".into()
                    } else {
                        "Proportional resize · Shift for free".into()
                    },
                    FontId::monospace(11.0),
                    ACCENT,
                );
            }
            DragAction::Draw => match (self.tool, self.shape_kind) {
                (Tool::Text, _) => {
                    let placement =
                        text_drag_placement(geometry, drag.start_canvas, drag.current_canvas);
                    let start = geometry.canvas_to_screen(placement.position);
                    let end = geometry.canvas_to_screen(Pos2::new(
                        placement.position.x + placement.box_width.unwrap_or_default(),
                        placement.position.y,
                    ));
                    ui.painter()
                        .line_segment([start, end], Stroke::new(1.5, ACCENT));
                    for point in [start, end] {
                        ui.painter().rect_filled(
                            Rect::from_center_size(point, Vec2::splat(6.0)),
                            1.0,
                            ACCENT,
                        );
                    }
                    if let Some(width) = placement.box_width {
                        ui.painter().text(
                            end + Vec2::new(6.0, -4.0),
                            Align2::LEFT_BOTTOM,
                            format!("{width:.0} px text width"),
                            FontId::monospace(11.0),
                            ACCENT,
                        );
                    }
                }
                (Tool::Marquee, _) => {
                    if let Some(selection) = selection_from_drag(
                        self.workspace.document.width,
                        self.workspace.document.height,
                        drag.start_canvas,
                        drag.current_canvas,
                    ) {
                        paint_selection_drag(ui, geometry, &selection);
                    }
                }
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
            DragAction::Guide(id) => {
                if let Ok(guide) = self.workspace.document.guide(id) {
                    ui.painter().text(
                        current,
                        Align2::LEFT_BOTTOM,
                        format!("{:.1} px", guide.position),
                        FontId::monospace(11.0),
                        ACCENT,
                    );
                }
            }
        }
    }

    pub(super) fn finish_canvas_drag(&mut self, drag: DragState, geometry: CanvasGeometry) {
        let min = drag.start_canvas.min(drag.current_canvas);
        let max = drag.start_canvas.max(drag.current_canvas);
        let size = max - min;
        match drag.action {
            DragAction::Move | DragAction::Resize(_) | DragAction::Guide(_) => {
                self.finish_interaction()
            }
            DragAction::Rotate => {
                self.finish_interaction();
                self.tool = Tool::Move;
            }
            DragAction::Draw => match (self.tool, self.shape_kind) {
                (Tool::Text, _) => {
                    let placement =
                        text_drag_placement(geometry, drag.start_canvas, drag.current_canvas);
                    self.open_new_text_editor(placement.position, placement.box_width);
                }
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
                (Tool::Marquee, _) => {
                    if let Some(selection) = selection_from_drag(
                        self.workspace.document.width,
                        self.workspace.document.height,
                        drag.start_canvas,
                        drag.current_canvas,
                    ) {
                        self.execute(Command::SetSelection {
                            selection: Some(selection),
                        });
                    }
                }
                _ => {}
            },
        }
    }

    fn preview_paragraph_width_drag(&mut self, id: u64, drag: DragState, handle: ResizeHandle) {
        let Ok(layer) = self.workspace.document.layer(id) else {
            return;
        };
        let font_asset = self.workspace.document.font_for_layer(layer);
        let Some((typography, transform, source)) =
            paragraph_width_preview(layer, drag, handle, font_asset)
        else {
            return;
        };
        if self.preview_commands(vec![
            Command::SetTextTypography { id, typography },
            Command::SetTransform { id, transform },
        ]) && let Ok(layer) = self.workspace.document.layer(id)
        {
            self.layer_source_overrides
                .insert(id, LayerSourceOverride::new(layer.kind.clone(), source));
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
                self.open_new_text_editor(position, None);
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
            Tool::Marquee => {
                self.execute(Command::SetSelection { selection: None });
            }
            Tool::MagicWand => {
                let x = position.x.floor().max(0.0) as u32;
                let y = position.y.floor().max(0.0) as u32;
                self.execute(Command::MagicWandSelection {
                    x,
                    y,
                    tolerance: self.selection_ui.magic_wand_tolerance,
                    contiguous: self.selection_ui.magic_wand_contiguous,
                    antialias: self.selection_ui.magic_wand_antialias,
                    resolved_selection: None,
                });
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

fn canvas_interaction_position(
    tool: Tool,
    geometry: CanvasGeometry,
    screen_position: Pos2,
) -> Option<Pos2> {
    if matches!(
        tool,
        Tool::Text | Tool::Marquee | Tool::Lasso | Tool::MagicWand
    ) && !geometry.canvas.contains(screen_position)
    {
        return None;
    }
    Some(geometry.screen_to_canvas(screen_position))
}

fn text_drag_placement(geometry: CanvasGeometry, start: Pos2, current: Pos2) -> TextDragPlacement {
    let width = (current.x - start.x).abs();
    let screen_width =
        (geometry.canvas_to_screen(current).x - geometry.canvas_to_screen(start).x).abs();
    if screen_width < MIN_PARAGRAPH_TEXT_SCREEN_POINTS {
        return TextDragPlacement {
            position: start,
            box_width: None,
        };
    }
    TextDragPlacement {
        position: Pos2::new(start.x.min(current.x), start.y),
        box_width: Some(width),
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
            paragraph_bounds: None,
            paragraph_width: None,
            paragraph_source_override: None,
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
    !matches!(tool, Tool::Rotate | Tool::Pen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_uses_a_stroke_only_selection_outline() {
        assert!(!selection_outline_has_resize_handles(Tool::Rotate));
        assert!(selection_outline_has_resize_handles(Tool::Move));
    }

    #[test]
    fn text_clicks_map_through_zoom_and_pan_and_ignore_the_pasteboard() {
        let geometry = canvas_geometry(
            Rect::from_min_size(Pos2::new(20.0, 30.0), Vec2::new(1000.0, 700.0)),
            400,
            300,
            1.75,
            Vec2::new(47.0, -31.0),
        );
        let placement = Pos2::new(123.0, 87.0);
        let screen = geometry.canvas_to_screen(placement);
        let mapped = canvas_interaction_position(Tool::Text, geometry, screen).unwrap();
        assert!((mapped.x - placement.x).abs() < 0.001);
        assert!((mapped.y - placement.y).abs() < 0.001);

        let outside = Pos2::new(geometry.canvas.left() - 1.0, geometry.canvas.center().y);
        assert_eq!(
            canvas_interaction_position(Tool::Text, geometry, outside),
            None
        );
        assert!(canvas_interaction_position(Tool::Move, geometry, outside).is_some());
    }

    #[test]
    fn text_drag_classification_is_screen_invariant_and_width_is_in_canvas_pixels() {
        let viewport = Rect::from_min_size(Pos2::new(20.0, 30.0), Vec2::new(1000.0, 700.0));
        let geometries = [
            canvas_geometry(viewport, 800, 600, 0.75, Vec2::new(-31.0, 19.0)),
            canvas_geometry(viewport, 800, 600, 3.25, Vec2::new(47.0, -28.0)),
        ];
        for geometry in geometries {
            let start = Pos2::new(300.0, 220.0);
            let start_screen = geometry.canvas_to_screen(start);
            let forward_current = geometry.screen_to_canvas(start_screen + Vec2::new(8.0, 60.0));
            let forward = text_drag_placement(geometry, start, forward_current);
            let expected_width = 8.0 / geometry.pixels_per_point;
            assert!((forward.position.x - start.x).abs() < 0.001);
            assert_eq!(forward.position.y, start.y);
            assert!((forward.box_width.unwrap() - expected_width).abs() < 0.001);

            let reverse_current = geometry.screen_to_canvas(start_screen + Vec2::new(-8.0, -45.0));
            let reverse = text_drag_placement(geometry, start, reverse_current);
            assert!((reverse.position.x - (start.x - expected_width)).abs() < 0.001);
            assert_eq!(reverse.position.y, start.y);
            assert!((reverse.box_width.unwrap() - expected_width).abs() < 0.001);
        }
    }

    #[test]
    fn tiny_screen_motion_remains_point_text_at_varied_zoom_and_pan() {
        let viewport = Rect::from_min_size(Pos2::new(20.0, 30.0), Vec2::new(1000.0, 700.0));
        for geometry in [
            canvas_geometry(viewport, 800, 600, 0.5, Vec2::new(-80.0, 24.0)),
            canvas_geometry(viewport, 800, 600, 4.0, Vec2::new(65.0, -17.0)),
        ] {
            let start = Pos2::new(400.0, 300.0);
            let start_screen = geometry.canvas_to_screen(start);
            let current = geometry.screen_to_canvas(start_screen + Vec2::new(2.0, 80.0));
            let placement = text_drag_placement(geometry, start, current);
            assert_eq!(placement.position, start);
            assert_eq!(placement.box_width, None);
        }
    }
}
