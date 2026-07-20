use super::*;

const MAX_OFFSCREEN_DIMENSION: u32 = 4_096;

#[derive(Clone, Debug, PartialEq)]
struct CompositePreviewKey {
    tab_id: u64,
    generation: u64,
    document: Document,
    scale_quarters: u32,
}

impl CompositePreviewKey {
    fn new(tab_id: u64, generation: u64, document: &Document, display_scale: f32) -> Self {
        let mut document = document.clone();
        document.selected = None;
        let longest = document.width.max(document.height).max(1) as f32;
        let maximum_quarters = ((MAX_OFFSCREEN_DIMENSION as f32 / longest) * 4.0)
            .floor()
            .max(1.0) as u32;
        let scale_quarters = ((display_scale.max(0.25) * 4.0).ceil() as u32)
            .max(1)
            .min(maximum_quarters);
        Self {
            tab_id,
            generation,
            document,
            scale_quarters,
        }
    }

    fn scale(&self) -> f32 {
        self.scale_quarters as f32 * 0.25
    }
}

struct CompositeRenderRequest {
    key: CompositePreviewKey,
}

struct CompositeRenderResult {
    key: CompositePreviewKey,
    result: Result<image::DynamicImage, String>,
}

pub(super) struct CompositePreview {
    generation: u64,
    ready: Option<(CompositePreviewKey, TextureHandle)>,
    failed: Option<(CompositePreviewKey, String)>,
    pending: Option<CompositeRenderRequest>,
    in_flight: bool,
    sender: Sender<CompositeRenderRequest>,
    receiver: Receiver<CompositeRenderResult>,
}

impl CompositePreview {
    pub(super) fn new(repaint: egui::Context) -> Self {
        let (sender, worker_receiver) = mpsc::channel::<CompositeRenderRequest>();
        let (worker_sender, receiver) = mpsc::channel::<CompositeRenderResult>();
        std::thread::spawn(move || {
            while let Ok(request) = worker_receiver.recv() {
                let result =
                    prism_core::render_document_scaled(&request.key.document, request.key.scale())
                        .map_err(|error| format!("{error:#}"));
                let _ = worker_sender.send(CompositeRenderResult {
                    key: request.key,
                    result,
                });
                repaint.request_repaint();
            }
        });
        Self {
            generation: 0,
            ready: None,
            failed: None,
            pending: None,
            in_flight: false,
            sender,
            receiver,
        }
    }

    pub(super) fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.ready = None;
        self.failed = None;
        self.pending = None;
    }

    pub(super) fn ensure(
        &mut self,
        context: &egui::Context,
        tab_id: u64,
        document: &Document,
        display_scale: f32,
    ) -> Result<Option<TextureHandle>, String> {
        let desired = CompositePreviewKey::new(tab_id, self.generation, document, display_scale);
        while let Ok(response) = self.receiver.try_recv() {
            self.in_flight = false;
            if !completed_preview_is_safe_to_display(&response.key, &desired) {
                continue;
            }
            let image = match response.result {
                Ok(image) => image,
                Err(error) if response.key == desired => {
                    self.failed = Some((response.key, error.clone()));
                    return Err(error);
                }
                Err(_) => continue,
            };
            let rgba = image.to_rgba8();
            let pixels = egui::ColorImage::from_rgba_unmultiplied(
                [rgba.width() as usize, rgba.height() as usize],
                rgba.as_raw(),
            );
            let texture = context.load_texture(
                format!("prism-document-composite-{tab_id}"),
                pixels,
                TextureOptions::LINEAR,
            );
            if self
                .failed
                .as_ref()
                .is_some_and(|(key, _)| key == &response.key)
            {
                self.failed = None;
            }
            self.ready = Some((response.key, texture));
        }

        if self.ready.as_ref().is_some_and(|(key, _)| key == &desired) {
            return Ok(self.ready.as_ref().map(|(_, texture)| texture.clone()));
        }
        if let Some((_, error)) = self.failed.as_ref().filter(|(key, _)| key == &desired) {
            return Err(error.clone());
        }
        if self
            .pending
            .as_ref()
            .is_none_or(|request| request.key != desired)
        {
            self.pending = Some(CompositeRenderRequest {
                key: desired.clone(),
            });
        }
        if !self.in_flight
            && let Some(request) = self.pending.take()
        {
            self.sender
                .send(request)
                .map_err(|_| "Document preview worker stopped".to_owned())?;
            self.in_flight = true;
        }
        Ok(self.ready.as_ref().map(|(_, texture)| texture.clone()))
    }
}

fn completed_preview_is_safe_to_display(
    completed: &CompositePreviewKey,
    desired: &CompositePreviewKey,
) -> bool {
    completed.tab_id == desired.tab_id && completed.generation == desired.generation
}

pub(super) fn document_requires_composite_preview(document: &Document) -> bool {
    document.layers.iter().any(|layer| {
        layer.visible
            && (layer.blend_mode != BlendMode::Normal
                || layer.clip_to_below
                || (layer.mask.enabled && layer.mask.invert))
    })
}

pub(super) fn paint_composite_preview(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    texture: Option<&TextureHandle>,
) {
    // The texture already contains the document background. Only paint the
    // transparency backing here so a translucent background is applied once.
    paint_transparency_background(ui, geometry);
    if let Some(texture) = texture {
        ui.painter().with_clip_rect(geometry.viewport).image(
            texture.id(),
            geometry.canvas,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    ui.painter().rect_stroke(
        geometry.canvas,
        1.0,
        Stroke::new(1.0, CANVAS_EDGE),
        egui::StrokeKind::Outside,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_layer_features_select_the_parity_compositor() {
        let mut document = Document::new("Parity", 100, 100);
        document.layers.push(Layer::default());
        assert!(!document_requires_composite_preview(&document));
        document.layers[0].blend_mode = BlendMode::Color;
        assert!(document_requires_composite_preview(&document));
        document.layers[0].blend_mode = BlendMode::Normal;
        document.layers[0].clip_to_below = true;
        assert!(document_requires_composite_preview(&document));
        document.layers[0].clip_to_below = false;
        document.layers[0].mask.enabled = true;
        document.layers[0].mask.invert = true;
        assert!(document_requires_composite_preview(&document));
    }

    #[test]
    fn composite_keys_ignore_selection_but_track_pixels_and_scale() {
        let mut document = Document::new("Parity", 100, 100);
        document.layers.push(Layer::default());
        let before = CompositePreviewKey::new(4, 7, &document, 0.4);
        document.selected = Some(1);
        assert_eq!(before, CompositePreviewKey::new(4, 7, &document, 0.4));
        document.layers[0].opacity = 0.5;
        assert_ne!(before, CompositePreviewKey::new(4, 7, &document, 0.4));
        assert_ne!(
            CompositePreviewKey::new(4, 7, &document, 0.4),
            CompositePreviewKey::new(4, 7, &document, 1.0)
        );
        document.width = 10_000;
        assert_eq!(CompositePreviewKey::new(4, 7, &document, 2.0).scale(), 0.25);
    }

    #[test]
    fn superseded_frames_are_provisional_only_within_the_same_tab_generation() {
        let mut document = Document::new("Lifecycle", 100, 100);
        let completed = CompositePreviewKey::new(4, 7, &document, 1.0);
        document.background = [20, 40, 60, 255];
        let desired = CompositePreviewKey::new(4, 7, &document, 1.0);
        assert_ne!(completed, desired);
        assert!(completed_preview_is_safe_to_display(&completed, &desired));
        assert!(!completed_preview_is_safe_to_display(
            &completed,
            &CompositePreviewKey::new(5, 7, &document, 1.0)
        ));
        assert!(!completed_preview_is_safe_to_display(
            &completed,
            &CompositePreviewKey::new(4, 8, &document, 1.0)
        ));
    }

    #[test]
    fn semi_transparent_document_background_is_present_once_in_composite_texture() {
        let mut document = Document::new("Alpha", 1, 1);
        document.background = [80, 120, 160, 128];
        document.layers.push(Layer {
            blend_mode: BlendMode::Multiply,
            opacity: 0.0,
            ..Layer::default()
        });
        let texture_pixel = prism_core::render_document(&document, None)
            .unwrap()
            .to_rgba8()
            .get_pixel(0, 0)
            .0;
        assert_eq!(texture_pixel, document.background);

        let checker = [64, 67, 73, 255];
        let painted_once = alpha_over(texture_pixel, checker);
        let painted_twice = alpha_over(texture_pixel, painted_once);
        assert_ne!(painted_once, painted_twice);
    }

    fn alpha_over(foreground: [u8; 4], background: [u8; 4]) -> [u8; 4] {
        let alpha = foreground[3] as f32 / 255.0;
        [
            (foreground[0] as f32 * alpha + background[0] as f32 * (1.0 - alpha)).round() as u8,
            (foreground[1] as f32 * alpha + background[1] as f32 * (1.0 - alpha)).round() as u8,
            (foreground[2] as f32 * alpha + background[2] as f32 * (1.0 - alpha)).round() as u8,
            255,
        ]
    }
}
