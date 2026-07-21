use super::*;
use std::sync::Arc;

const COMPOSITOR_WORKERS: usize = 2;
const MAX_FALLBACK_OFFSCREEN_DIMENSION: u32 = 4_096;

#[derive(Clone, Debug, PartialEq)]
struct CompositePreviewKey {
    tab_id: u64,
    generation: u64,
    document: Document,
    scale_sixty_fourths: u32,
    region: prism_core::RenderRegion,
    raster_mode: RasterRenderMode,
}

impl CompositePreviewKey {
    fn new_with_sources(
        tab_id: u64,
        generation: u64,
        document: &Document,
        geometry: CanvasGeometry,
        physical_pixels_per_point: f32,
        raster_sources: &RasterSourceSnapshot,
    ) -> Option<Self> {
        let display_scale = geometry.pixels_per_point * physical_pixels_per_point;
        let requested_units = ((display_scale.max(1.0 / 64.0) * 64.0).ceil() as u32).max(1);
        let raster_mode = raster_sources.render_mode(document);
        let scale_sixty_fourths = if !matches!(raster_mode, RasterRenderMode::FallbackCapped) {
            requested_units
        } else {
            let longest = document.width.max(document.height).max(1) as f32;
            let maximum_units = ((MAX_FALLBACK_OFFSCREEN_DIMENSION as f32 / longest) * 64.0)
                .floor()
                .max(1.0) as u32;
            requested_units.min(maximum_units)
        };
        let scale = scale_sixty_fourths as f32 / 64.0;
        let region = visible_render_region(geometry, document, scale)?;
        let mut document = document.clone();
        document.selected = None;
        Some(Self {
            tab_id,
            generation,
            document,
            scale_sixty_fourths,
            region,
            raster_mode,
        })
    }

    fn scale(&self) -> f32 {
        self.scale_sixty_fourths as f32 / 64.0
    }

    #[cfg(test)]
    fn new(
        tab_id: u64,
        generation: u64,
        document: &Document,
        geometry: CanvasGeometry,
        physical_pixels_per_point: f32,
    ) -> Option<Self> {
        Self::new_with_sources(
            tab_id,
            generation,
            document,
            geometry,
            physical_pixels_per_point,
            &RasterSourceSnapshot::empty(),
        )
    }
}

#[derive(Clone)]
pub(super) struct CompositeFrame {
    key: CompositePreviewKey,
    texture: TextureHandle,
}

struct CompositeRenderRequest {
    sequence: u64,
    key: CompositePreviewKey,
    raster_sources: Arc<RasterSourceSnapshot>,
}

struct CompositeRenderResult {
    worker: usize,
    sequence: u64,
    key: CompositePreviewKey,
    raster_sources: Arc<RasterSourceSnapshot>,
    result: Result<image::DynamicImage, String>,
}

struct CompositeFailure {
    key: CompositePreviewKey,
    error: String,
    attempts: u32,
    retry_at: std::time::Instant,
    retry_queued: bool,
    raster_sources: Arc<RasterSourceSnapshot>,
}

pub(super) struct CompositePreview {
    generation: u64,
    next_sequence: u64,
    desired: Option<(u64, CompositePreviewKey)>,
    ready: Option<(u64, CompositeFrame)>,
    failed: Option<CompositeFailure>,
    pending: Option<CompositeRenderRequest>,
    workers_busy: [bool; COMPOSITOR_WORKERS],
    workers_available: [bool; COMPOSITOR_WORKERS],
    senders: [Sender<CompositeRenderRequest>; COMPOSITOR_WORKERS],
    receiver: Receiver<CompositeRenderResult>,
}

fn render_composite_request(
    request: &CompositeRenderRequest,
) -> Result<image::DynamicImage, String> {
    let result = match request.key.raster_mode {
        RasterRenderMode::Provider { .. } => {
            prism_core::render_document_region_scaled_with_sources(
                &request.key.document,
                request.key.scale(),
                request.key.region,
                request.raster_sources.as_ref(),
            )
        }
        RasterRenderMode::LegacyNative | RasterRenderMode::FallbackCapped => {
            prism_core::render_document_region_scaled(
                &request.key.document,
                request.key.scale(),
                request.key.region,
            )
        }
    };
    result.map_err(|error| format!("{error:#}"))
}

impl CompositePreview {
    pub(super) fn new(repaint: egui::Context) -> Self {
        let (result_sender, receiver) = mpsc::channel::<CompositeRenderResult>();
        let senders = std::array::from_fn(|worker| {
            let (sender, worker_receiver) = mpsc::channel::<CompositeRenderRequest>();
            let result_sender = result_sender.clone();
            let repaint = repaint.clone();
            std::thread::spawn(move || {
                while let Ok(request) = worker_receiver.recv() {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        render_composite_request(&request)
                    }))
                    .unwrap_or_else(|_| Err("Document region preview renderer panicked".into()));
                    let _ = result_sender.send(CompositeRenderResult {
                        worker,
                        sequence: request.sequence,
                        key: request.key,
                        raster_sources: request.raster_sources,
                        result,
                    });
                    repaint.request_repaint();
                }
            });
            sender
        });
        Self {
            generation: 0,
            next_sequence: 0,
            desired: None,
            ready: None,
            failed: None,
            pending: None,
            workers_busy: [false; COMPOSITOR_WORKERS],
            workers_available: [true; COMPOSITOR_WORKERS],
            senders,
            receiver,
        }
    }

    pub(super) fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.desired = None;
        self.ready = None;
        self.failed = None;
        self.pending = None;
    }

    pub(super) fn ensure(
        &mut self,
        context: &egui::Context,
        tab_id: u64,
        document: &Document,
        geometry: CanvasGeometry,
        physical_pixels_per_point: f32,
        raster_sources: Arc<RasterSourceSnapshot>,
    ) -> Result<Option<CompositeFrame>, String> {
        let Some(desired_key) = CompositePreviewKey::new_with_sources(
            tab_id,
            self.generation,
            document,
            geometry,
            physical_pixels_per_point,
            raster_sources.as_ref(),
        ) else {
            return Ok(None);
        };

        if self
            .desired
            .as_ref()
            .is_none_or(|(_, key)| key != &desired_key)
        {
            if self
                .failed
                .as_ref()
                .is_some_and(|failure| failure.key != desired_key)
            {
                self.failed = None;
            }
            self.next_sequence = self.next_sequence.wrapping_add(1);
            self.desired = Some((self.next_sequence, desired_key.clone()));
            self.pending = Some(CompositeRenderRequest {
                sequence: self.next_sequence,
                key: desired_key.clone(),
                raster_sources: Arc::clone(&raster_sources),
            });
        }

        while let Ok(response) = self.receiver.try_recv() {
            self.workers_busy[response.worker] = false;
            if !completed_preview_is_safe_to_display(&response.key, &desired_key) {
                continue;
            }
            let image = match response.result {
                Ok(image) => image,
                Err(error) if response.key == desired_key => {
                    let attempts = self
                        .failed
                        .as_ref()
                        .filter(|failure| failure.key == response.key)
                        .map_or(1, |failure| failure.attempts.saturating_add(1));
                    self.failed = Some(CompositeFailure {
                        key: response.key,
                        error,
                        attempts,
                        retry_at: std::time::Instant::now() + retry_delay(attempts),
                        retry_queued: false,
                        raster_sources: response.raster_sources,
                    });
                    continue;
                }
                Err(_) => continue,
            };
            let rgba = image.to_rgba8();
            let pixels = egui::ColorImage::from_rgba_unmultiplied(
                [rgba.width() as usize, rgba.height() as usize],
                rgba.as_raw(),
            );
            let texture = context.load_texture(
                format!("prism-document-region-{tab_id}-{}", response.sequence),
                pixels,
                TextureOptions::LINEAR,
            );
            if self
                .failed
                .as_ref()
                .is_some_and(|failure| failure.key == response.key)
            {
                self.failed = None;
            }
            if completed_sequence_may_replace(
                self.ready
                    .as_ref()
                    .map(|(ready_sequence, _)| *ready_sequence),
                response.sequence,
            ) {
                self.ready = Some((
                    response.sequence,
                    CompositeFrame {
                        key: response.key,
                        texture,
                    },
                ));
            }
        }

        self.queue_retry_when_ready(context, &desired_key);
        self.dispatch_pending()?;
        if let Some(failure) = self
            .failed
            .as_ref()
            .filter(|failure| failure.key == desired_key)
        {
            return Err(failure.error.clone());
        }
        Ok(self
            .ready
            .as_ref()
            .filter(|(_, frame)| completed_preview_is_safe_to_display(&frame.key, &desired_key))
            .map(|(_, frame)| frame.clone()))
    }

    fn dispatch_pending(&mut self) -> Result<(), String> {
        while self.pending.is_some() {
            let Some(worker) =
                available_compositor_worker(&self.workers_available, &self.workers_busy)
            else {
                return if self.workers_available.iter().any(|available| *available) {
                    Ok(())
                } else {
                    Err("All document region preview workers stopped".to_owned())
                };
            };
            let request = self.pending.take().expect("pending request was checked");
            match self.senders[worker].send(request) {
                Ok(()) => {
                    self.workers_busy[worker] = true;
                    return Ok(());
                }
                Err(error) => {
                    self.workers_available[worker] = false;
                    self.workers_busy[worker] = false;
                    self.pending = Some(error.0);
                }
            }
        }
        Ok(())
    }

    fn queue_retry_when_ready(
        &mut self,
        context: &egui::Context,
        desired_key: &CompositePreviewKey,
    ) {
        let Some(failure) = self
            .failed
            .as_mut()
            .filter(|failure| failure.key == *desired_key)
        else {
            return;
        };
        match retry_action(
            failure.retry_at,
            failure.retry_queued,
            std::time::Instant::now(),
        ) {
            RetryAction::Wait(duration) => context.request_repaint_after(duration),
            RetryAction::Queue => {
                self.next_sequence = self.next_sequence.wrapping_add(1);
                self.desired = Some((self.next_sequence, desired_key.clone()));
                self.pending = Some(CompositeRenderRequest {
                    sequence: self.next_sequence,
                    key: desired_key.clone(),
                    raster_sources: Arc::clone(&failure.raster_sources),
                });
                failure.retry_queued = true;
            }
            RetryAction::InFlight => {}
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RetryAction {
    Wait(std::time::Duration),
    Queue,
    InFlight,
}

fn retry_delay(attempts: u32) -> std::time::Duration {
    let exponent = attempts.saturating_sub(1).min(5);
    std::time::Duration::from_millis((100_u64 << exponent).min(2_000))
}

fn retry_action(
    retry_at: std::time::Instant,
    retry_queued: bool,
    now: std::time::Instant,
) -> RetryAction {
    if retry_queued {
        RetryAction::InFlight
    } else if now >= retry_at {
        RetryAction::Queue
    } else {
        RetryAction::Wait(retry_at.duration_since(now))
    }
}

fn available_compositor_worker(
    available: &[bool; COMPOSITOR_WORKERS],
    busy: &[bool; COMPOSITOR_WORKERS],
) -> Option<usize> {
    available
        .iter()
        .zip(busy)
        .position(|(available, busy)| *available && !*busy)
}

fn completed_sequence_may_replace(ready: Option<u64>, completed: u64) -> bool {
    ready.is_none_or(|ready| completed >= ready)
}

fn visible_render_region(
    geometry: CanvasGeometry,
    document: &Document,
    scale: f32,
) -> Option<prism_core::RenderRegion> {
    let visible = geometry.canvas.intersect(geometry.viewport);
    if visible.is_negative() || visible.width() <= 0.0 || visible.height() <= 0.0 {
        return None;
    }
    let scaled_width = (document.width as f64 * f64::from(scale)).round().max(1.0) as u32;
    let scaled_height = (document.height as f64 * f64::from(scale)).round().max(1.0) as u32;
    let relative_left =
        ((visible.left() - geometry.canvas.left()) / geometry.canvas.width()).clamp(0.0, 1.0);
    let relative_top =
        ((visible.top() - geometry.canvas.top()) / geometry.canvas.height()).clamp(0.0, 1.0);
    let relative_right =
        ((visible.right() - geometry.canvas.left()) / geometry.canvas.width()).clamp(0.0, 1.0);
    let relative_bottom =
        ((visible.bottom() - geometry.canvas.top()) / geometry.canvas.height()).clamp(0.0, 1.0);
    let x = ((relative_left * scaled_width as f32).floor() as u32).saturating_sub(1);
    let y = ((relative_top * scaled_height as f32).floor() as u32).saturating_sub(1);
    let right = ((relative_right * scaled_width as f32).ceil() as u32 + 1).min(scaled_width);
    let bottom = ((relative_bottom * scaled_height as f32).ceil() as u32 + 1).min(scaled_height);
    Some(prism_core::RenderRegion {
        x,
        y,
        width: right.saturating_sub(x).max(1),
        height: bottom.saturating_sub(y).max(1),
    })
}

fn completed_preview_is_safe_to_display(
    completed: &CompositePreviewKey,
    desired: &CompositePreviewKey,
) -> bool {
    completed == desired
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
    frame: Option<&CompositeFrame>,
) {
    paint_transparency_background(ui, geometry);
    if let Some(frame) = frame {
        let scale = frame.key.scale();
        let region = frame.key.region;
        let rect = Rect::from_min_size(
            geometry.canvas.min
                + Vec2::new(region.x as f32 / scale, region.y as f32 / scale)
                    * geometry.pixels_per_point,
            Vec2::new(region.width as f32 / scale, region.height as f32 / scale)
                * geometry.pixels_per_point,
        );
        ui.painter().with_clip_rect(geometry.viewport).image(
            frame.texture.id(),
            rect,
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
    use std::convert::Infallible;

    use prism_core::{RasterSourceEpoch, ResolvedRasterSource};
    use spectrum_imaging::{
        ExactRegionSource, PixelRegion, RegionReadCapability, RegionReadiness,
        RegionSourceDescriptor, RegionSourceInfo, SourceSampleDepth,
    };

    use super::*;

    struct ConstantSource {
        info: RegionSourceInfo,
        pixel: [u8; 4],
    }

    impl ExactRegionSource for ConstantSource {
        type Error = Infallible;

        fn info(&self) -> &RegionSourceInfo {
            &self.info
        }

        fn read_exact_region(&self, region: PixelRegion) -> Result<image::RgbaImage, Self::Error> {
            Ok(image::RgbaImage::from_pixel(
                region.width,
                region.height,
                image::Rgba(self.pixel),
            ))
        }
    }

    fn provider_snapshot(
        path: PathBuf,
        epoch: u64,
        pixel: [u8; 4],
        dimensions: (u32, u32),
    ) -> Arc<RasterSourceSnapshot> {
        let source = ResolvedRasterSource::new(
            RasterSourceEpoch::new(format!("source-{epoch}")).unwrap(),
            Arc::new(ConstantSource {
                info: RegionSourceInfo {
                    descriptor: RegionSourceDescriptor {
                        width: dimensions.0,
                        height: dimensions.1,
                        color_encoding: "rgba8".into(),
                        sample_depth: SourceSampleDepth::EightBit,
                        frame_index: 0,
                        page_index: 0,
                        decoder_contract: "test".into(),
                    },
                    capability: RegionReadCapability::DerivedBacking,
                    readiness: RegionReadiness::Ready,
                },
                pixel,
            }),
        )
        .unwrap();
        RasterSourceSnapshot::with_test_provider(epoch, path, source)
    }

    fn geometry(canvas: Rect, viewport: Rect) -> CanvasGeometry {
        CanvasGeometry {
            viewport,
            canvas,
            pixels_per_point: canvas.width() / 100.0,
        }
    }

    #[test]
    fn visible_region_tracks_negative_and_offscreen_canvas_bounds() {
        let document = Document::new("Viewport", 100, 80);
        let region = visible_render_region(
            geometry(
                Rect::from_min_size(Pos2::new(-50.0, -25.0), Vec2::new(200.0, 160.0)),
                Rect::from_min_size(Pos2::ZERO, Vec2::new(120.0, 90.0)),
            ),
            &document,
            2.0,
        )
        .unwrap();
        assert_eq!(region.x, 49);
        assert_eq!(region.y, 24);
        assert_eq!(region.width, 122);
        assert_eq!(region.height, 92);

        assert!(
            visible_render_region(
                geometry(
                    Rect::from_min_size(Pos2::new(200.0, 200.0), Vec2::new(100.0, 80.0)),
                    Rect::from_min_size(Pos2::ZERO, Vec2::new(120.0, 90.0)),
                ),
                &document,
                1.0,
            )
            .is_none()
        );
    }

    #[test]
    fn high_zoom_region_stays_bounded_to_the_visible_viewport() {
        let document = Document::new("Zoom", 16_384, 16_384);
        let region = visible_render_region(
            geometry(
                Rect::from_min_size(Pos2::new(-80_000.0, -70_000.0), Vec2::splat(262_144.0)),
                Rect::from_min_size(Pos2::ZERO, Vec2::new(1_600.0, 900.0)),
            ),
            &document,
            16.0,
        )
        .unwrap();
        assert!(region.width <= 1_603);
        assert!(region.height <= 903);
    }

    #[test]
    fn provider_readiness_switches_from_capped_to_exact_high_zoom() {
        let path = PathBuf::from("not-present.jpg");
        let mut document = Document::new("Provider zoom", 16_384, 16_384);
        document.layers.push(Layer {
            kind: LayerKind::Raster {
                path: path.clone(),
                original_path: None,
            },
            ..Layer::default()
        });
        let geometry = CanvasGeometry {
            canvas: Rect::from_min_size(Pos2::ZERO, Vec2::splat(262_144.0)),
            viewport: Rect::from_min_size(Pos2::ZERO, Vec2::new(1_600.0, 900.0)),
            pixels_per_point: 16.0,
        };
        let pending = RasterSourceSnapshot::empty();
        let capped =
            CompositePreviewKey::new_with_sources(1, 0, &document, geometry, 1.0, &pending)
                .unwrap();
        assert_eq!(capped.raster_mode, RasterRenderMode::FallbackCapped);
        assert!(capped.scale() < 1.0);

        let ready = provider_snapshot(path, 9, [1, 2, 3, 255], (16_384, 16_384));
        let exact =
            CompositePreviewKey::new_with_sources(1, 0, &document, geometry, 1.0, ready.as_ref())
                .unwrap();
        assert_eq!(
            exact.raster_mode,
            RasterRenderMode::Provider { snapshot_epoch: 9 }
        );
        assert_eq!(exact.scale(), 16.0);
        assert_ne!(capped, exact);
    }

    #[test]
    fn render_request_uses_its_exact_snapshot_without_path_fallback() {
        let path = PathBuf::from("definitely-missing.jpg");
        let mut document = Document::new("Exact snapshot", 4, 4);
        document.layers.push(Layer {
            kind: LayerKind::Raster {
                path: path.clone(),
                original_path: None,
            },
            ..Layer::default()
        });
        let first = provider_snapshot(path.clone(), 11, [12, 34, 56, 255], (4, 4));
        let newer = provider_snapshot(path, 12, [200, 210, 220, 255], (4, 4));
        let geometry = geometry(
            Rect::from_min_size(Pos2::ZERO, Vec2::splat(4.0)),
            Rect::from_min_size(Pos2::ZERO, Vec2::splat(4.0)),
        );
        let key =
            CompositePreviewKey::new_with_sources(1, 0, &document, geometry, 1.0, first.as_ref())
                .unwrap();
        let newer_key =
            CompositePreviewKey::new_with_sources(1, 0, &document, geometry, 1.0, newer.as_ref())
                .unwrap();
        assert!(!completed_preview_is_safe_to_display(&key, &newer_key));
        let rendered = render_composite_request(&CompositeRenderRequest {
            sequence: 1,
            key,
            raster_sources: first,
        })
        .unwrap()
        .to_rgba8();
        assert_eq!(rendered.get_pixel(0, 0).0, [12, 34, 56, 255]);
    }

    #[test]
    fn procedural_vector_layers_use_the_requested_high_zoom_scale() {
        let mut document = Document::new("Vector viewport", 16_384, 16_384);
        document.layers.push(Layer {
            transform: Transform {
                rotation: 17.0,
                ..Transform::default()
            },
            stroke: ShapeStroke {
                enabled: true,
                width: 12.0,
                color: [93, 216, 199, 255],
            },
            kind: LayerKind::Rectangle {
                width: 16_384,
                height: 16_384,
                color: [255, 255, 255, 255],
                corner_radius: 4.0,
            },
            ..Layer::default()
        });
        let geometry = CanvasGeometry {
            canvas: Rect::from_min_size(Pos2::ZERO, Vec2::splat(262_144.0)),
            viewport: Rect::from_min_size(Pos2::ZERO, Vec2::new(1_600.0, 900.0)),
            pixels_per_point: 16.0,
        };
        let key = CompositePreviewKey::new(1, 0, &document, geometry, 1.0).unwrap();
        assert_eq!(key.scale(), 16.0);

        document.layers[0].kind = LayerKind::Ellipse {
            width: 16_384,
            height: 16_384,
            color: [255, 255, 255, 255],
        };
        let key = CompositePreviewKey::new(1, 0, &document, geometry, 1.0).unwrap();
        assert_eq!(key.scale(), 16.0);

        document.layers[0].adjustments.exposure = 0.25;
        let key = CompositePreviewKey::new(1, 0, &document, geometry, 1.0).unwrap();
        assert_eq!(key.scale(), 16.0);
    }

    #[test]
    fn stale_generations_and_tabs_never_replace_the_current_surface() {
        let document = Document::new("Lifecycle", 100, 80);
        let geometry = geometry(
            Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 80.0)),
            Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 80.0)),
        );
        let completed = CompositePreviewKey::new(4, 7, &document, geometry, 1.0).unwrap();
        assert!(completed_preview_is_safe_to_display(&completed, &completed));
        let different_scale = CompositePreviewKey::new(4, 7, &document, geometry, 2.0).unwrap();
        assert!(!completed_preview_is_safe_to_display(
            &completed,
            &different_scale
        ));
        assert!(!completed_preview_is_safe_to_display(
            &completed,
            &CompositePreviewKey::new(5, 7, &document, geometry, 1.0).unwrap()
        ));
        assert!(!completed_preview_is_safe_to_display(
            &completed,
            &CompositePreviewKey::new(4, 8, &document, geometry, 1.0).unwrap()
        ));

        let mut moved = document.clone();
        moved.layers.push(Layer {
            transform: Transform {
                x: 20.0,
                ..Transform::default()
            },
            ..Layer::default()
        });
        assert!(!completed_preview_is_safe_to_display(
            &completed,
            &CompositePreviewKey::new(4, 7, &moved, geometry, 1.0).unwrap()
        ));
    }

    #[test]
    fn newest_gesture_can_start_while_an_older_region_is_rendering() {
        assert_eq!(
            available_compositor_worker(&[true, true], &[true, false]),
            Some(1)
        );
        assert_eq!(
            available_compositor_worker(&[false, true], &[false, false]),
            Some(1)
        );
        assert_eq!(
            available_compositor_worker(&[false, false], &[false, false]),
            None
        );
        assert_eq!(
            available_compositor_worker(&[true, true], &[true, true]),
            None
        );
        assert!(completed_sequence_may_replace(Some(7), 8));
        assert!(!completed_sequence_may_replace(Some(8), 7));
    }

    #[test]
    fn failed_desired_region_retries_with_backoff_without_busy_looping() {
        let now = std::time::Instant::now();
        assert_eq!(retry_delay(1), std::time::Duration::from_millis(100));
        assert_eq!(retry_delay(8), std::time::Duration::from_secs(2));
        assert_eq!(
            retry_action(now + std::time::Duration::from_millis(100), false, now),
            RetryAction::Wait(std::time::Duration::from_millis(100))
        );
        assert_eq!(retry_action(now, false, now), RetryAction::Queue);
        assert_eq!(retry_action(now, true, now), RetryAction::InFlight);
    }

    #[test]
    fn semantic_layer_features_select_the_exact_region_compositor() {
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
    fn translucent_document_background_is_present_once_in_the_region_texture() {
        let mut document = Document::new("Alpha", 1, 1);
        document.background = [80, 120, 160, 128];
        document.layers.push(Layer {
            blend_mode: BlendMode::Multiply,
            opacity: 0.0,
            ..Layer::default()
        });
        let texture_pixel = prism_core::render_document_region_scaled(
            &document,
            1.0,
            prism_core::RenderRegion {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
        )
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
