use super::*;

pub(super) fn completed_preview_is_safe_to_display(
    completed: &CompositePreviewKey,
    desired: &CompositePreviewKey,
) -> bool {
    completed == desired || progressive_brush_keys_are_compatible(completed, desired)
}

fn progressive_brush_keys_are_compatible(
    completed: &CompositePreviewKey,
    desired: &CompositePreviewKey,
) -> bool {
    let (Some(completed_brush), Some(desired_brush)) =
        (completed.progressive_brush, desired.progressive_brush)
    else {
        return false;
    };
    completed.tab_id == desired.tab_id
        && completed.generation == desired.generation
        && completed.scale_sixty_fourths == desired.scale_sixty_fourths
        && completed.region == desired.region
        && completed.raster_mode == desired.raster_mode
        && completed_brush.gesture_id == desired_brush.gesture_id
        && completed_brush.target_layer_id == desired_brush.target_layer_id
        && completed_brush.mode == desired_brush.mode
        && completed_brush.sample_count <= desired_brush.sample_count
        && brush_document_is_monotonic_prefix(
            &completed.document,
            completed_brush,
            &desired.document,
            desired_brush,
        )
}

fn brush_document_is_monotonic_prefix(
    completed: &Document,
    completed_brush: ProgressiveBrushPreview,
    desired: &Document,
    desired_brush: ProgressiveBrushPreview,
) -> bool {
    let (Ok(completed_layer), Ok(desired_layer)) = (
        completed.layer(completed_brush.target_layer_id),
        desired.layer(desired_brush.target_layer_id),
    ) else {
        return false;
    };
    let (
        LayerKind::Paint {
            program: completed_program,
        },
        LayerKind::Paint {
            program: desired_program,
        },
    ) = (&completed_layer.kind, &desired_layer.kind)
    else {
        return false;
    };
    let (Some(completed_stroke), Some(desired_stroke)) = (
        completed_program.strokes.last(),
        desired_program.strokes.last(),
    ) else {
        return false;
    };
    if completed_brush.sample_count != completed_stroke.samples.len()
        || desired_brush.sample_count != desired_stroke.samples.len()
        || completed_stroke.style.mode != completed_brush.mode
        || desired_stroke.style.mode != desired_brush.mode
        || completed_program.version != desired_program.version
        || completed_program.width != desired_program.width
        || completed_program.height != desired_program.height
        || completed_program.strokes.len() != desired_program.strokes.len()
        || completed_program.strokes[..completed_program.strokes.len() - 1]
            != desired_program.strokes[..desired_program.strokes.len() - 1]
        || completed_stroke.style != desired_stroke.style
        || completed_stroke.clip != desired_stroke.clip
        || !desired_stroke
            .samples
            .starts_with(completed_stroke.samples.as_ref())
    {
        return false;
    }
    let mut normalized = completed.clone();
    let Ok(layer) = normalized.layer_mut(completed_brush.target_layer_id) else {
        return false;
    };
    layer.kind = desired_layer.kind.clone();
    normalized == *desired
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_core::{BrushMode, BrushSample, BrushStroke, BrushStyle, Command, PaintSelection};

    fn progressive_brush_document(
        samples: &[[f32; 2]],
        gesture_id: u64,
    ) -> (Document, ProgressiveBrushPreview) {
        let document = Document::new("Brush prefix", 128, 96);
        let stroke = BrushStroke::new(
            BrushStyle {
                color: [23, 149, 211, 231],
                size: 17.0,
                hardness: 0.63,
                opacity: 0.72,
                spacing: 0.11,
                ..BrushStyle::default()
            },
            samples
                .iter()
                .map(|point| BrushSample {
                    x: point[0],
                    y: point[1],
                    pressure: 1.0,
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let preview = prism_core::preview_paint_command(
            &document,
            Command::AddPaintLayerWithStroke {
                name: None,
                width: 128,
                height: 96,
                stroke,
                selection: PaintSelection::Current,
            },
        )
        .unwrap();
        (
            preview,
            ProgressiveBrushPreview {
                gesture_id,
                target_layer_id: 1,
                sample_count: samples.len(),
                mode: BrushMode::Paint,
            },
        )
    }

    fn progressive_key(
        document: &Document,
        brush: ProgressiveBrushPreview,
        generation: u64,
    ) -> CompositePreviewKey {
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(128.0, 96.0));
        CompositePreviewKey::new_with_sources_and_brush(
            3,
            generation,
            document,
            CanvasGeometry {
                canvas: bounds,
                viewport: bounds,
                pixels_per_point: 1.0,
            },
            1.0,
            &RasterSourceSnapshot::empty(),
            Some(brush),
        )
        .unwrap()
    }

    #[test]
    fn accepts_only_a_monotonic_prefix_of_the_same_gesture() {
        let (short_document, short_brush) =
            progressive_brush_document(&[[12.0, 14.0], [30.0, 25.0]], 41);
        let (long_document, long_brush) =
            progressive_brush_document(&[[12.0, 14.0], [30.0, 25.0], [58.0, 61.0]], 41);
        let short = progressive_key(&short_document, short_brush, 7);
        let long = progressive_key(&long_document, long_brush, 7);
        assert!(completed_preview_is_safe_to_display(&short, &long));
        assert!(!completed_preview_is_safe_to_display(&long, &short));

        let mut new_gesture = long_brush;
        new_gesture.gesture_id += 1;
        assert!(!completed_preview_is_safe_to_display(
            &short,
            &progressive_key(&long_document, new_gesture, 7)
        ));
        let mut other_target = long_brush;
        other_target.target_layer_id += 1;
        assert!(!completed_preview_is_safe_to_display(
            &short,
            &progressive_key(&long_document, other_target, 7)
        ));
    }

    #[test]
    fn cancel_reset_and_tool_switch_cannot_resurrect_a_prefix() {
        let (short_document, short_brush) =
            progressive_brush_document(&[[12.0, 14.0], [30.0, 25.0]], 9);
        let (long_document, long_brush) =
            progressive_brush_document(&[[12.0, 14.0], [30.0, 25.0], [58.0, 61.0]], 9);
        let completed = progressive_key(&short_document, short_brush, 12);

        assert!(!completed_preview_is_safe_to_display(
            &completed,
            &progressive_key(&long_document, long_brush, 13)
        ));
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(128.0, 96.0));
        let settled = CompositePreviewKey::new(
            3,
            12,
            &long_document,
            CanvasGeometry {
                canvas: bounds,
                viewport: bounds,
                pixels_per_point: 1.0,
            },
            1.0,
        )
        .unwrap();
        assert!(!completed_preview_is_safe_to_display(&completed, &settled));
        let mut eraser = long_brush;
        eraser.mode = BrushMode::Erase;
        assert!(!completed_preview_is_safe_to_display(
            &completed,
            &progressive_key(&long_document, eraser, 12)
        ));
    }
}
