use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use anyhow::{Result, bail};
use image::{Rgba, RgbaImage};
use prism_core::{
    BrushMode, BrushSample, BrushStroke, BrushStyle, Command, Document, Layer, LayerKind,
    PaintSelection, RasterSourceEpoch, RasterSourceResolver, RenderRegion, ResolvedRasterSource,
    Workspace, preview_paint_command, render_document_region_scaled_with_sources_and_stats,
};
use spectrum_imaging::{
    ExactRegionSource, PixelRegion, RegionReadCapability, RegionReadiness, RegionRequestError,
    RegionSourceDescriptor, RegionSourceInfo, SourceSampleDepth, validate_region_request,
};

const SOURCE_SIZE: u32 = 16_384;
const REGION_WIDTH: u32 = 640;
const REGION_HEIGHT: u32 = 400;
const LIVE_FRAMES: usize = 24;
const LIVE_SAMPLES_PER_FRAME: usize = 4;

pub(super) struct CloneStampMeasurements {
    pub viewport_samples: Vec<f64>,
    pub live_samples: Vec<f64>,
    pub max_provider_region_pixels: u64,
    pub source_full_plane_bytes: u64,
}

pub(super) fn measure() -> Result<CloneStampMeasurements> {
    let fixture = SolidRasterFixture::new(SOURCE_SIZE, SOURCE_SIZE)?;
    let mut document = Document::new("Clone Stamp benchmark", SOURCE_SIZE, SOURCE_SIZE);
    document.background = [0; 4];
    document.layers.push(Layer {
        id: 1,
        visible: false,
        name: "Immutable source".into(),
        kind: LayerKind::Raster {
            path: fixture.path.clone(),
            original_path: None,
        },
        ..Layer::default()
    });
    document.selected = Some(1);
    document.next_id = 2;
    let mut workspace = Workspace::new(document, None);
    workspace.execute(Command::SetCloneSource {
        id: 1,
        document_x: 7_680.5,
        document_y: 7_900.5,
        resolved_source: None,
    })?;
    workspace.execute(Command::AddPaintLayer {
        name: Some("Clone".into()),
        width: SOURCE_SIZE,
        height: SOURCE_SIZE,
    })?;
    let base = workspace.document;
    let region = RenderRegion {
        x: 8_000,
        y: 8_000,
        width: REGION_WIDTH,
        height: REGION_HEIGHT,
    };
    let settled = preview_paint_command(&base, clone_command(384)?)?;
    let mut viewport_samples = Vec::with_capacity(7);
    for _ in 0..7 {
        let started = Instant::now();
        let (rendered, stats) =
            render_document_region_scaled_with_sources_and_stats(&settled, 1.0, region, &fixture)?;
        viewport_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        validate_render(&rendered, stats.fallback_decode_bytes, region)?;
    }

    let mut live_samples = Vec::with_capacity(LIVE_FRAMES);
    let mut final_pixels = None;
    for frame in 0..LIVE_FRAMES {
        let started = Instant::now();
        let preview =
            preview_paint_command(&base, clone_command((frame + 1) * LIVE_SAMPLES_PER_FRAME)?)?;
        let (rendered, stats) =
            render_document_region_scaled_with_sources_and_stats(&preview, 1.0, region, &fixture)?;
        validate_render(&rendered, stats.fallback_decode_bytes, region)?;
        live_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        final_pixels = Some(rendered.into_rgba8().into_raw());
    }
    let final_document =
        preview_paint_command(&base, clone_command(LIVE_FRAMES * LIVE_SAMPLES_PER_FRAME)?)?;
    let (final_render, stats) = render_document_region_scaled_with_sources_and_stats(
        &final_document,
        1.0,
        region,
        &fixture,
    )?;
    validate_render(&final_render, stats.fallback_decode_bytes, region)?;
    if final_pixels.as_deref() != Some(final_render.to_rgba8().as_raw()) {
        bail!("live Clone Stamp release did not match its final preview pixels");
    }
    let max_provider_region_pixels = fixture.max_region_pixels.load(Ordering::Relaxed);
    if max_provider_region_pixels == 0
        || max_provider_region_pixels > u64::from(REGION_WIDTH) * u64::from(REGION_HEIGHT)
    {
        bail!("Clone Stamp provider read exceeded the viewport-sized source bound");
    }
    Ok(CloneStampMeasurements {
        viewport_samples,
        live_samples,
        max_provider_region_pixels,
        source_full_plane_bytes: u64::from(SOURCE_SIZE) * u64::from(SOURCE_SIZE) * 4,
    })
}

fn clone_command(sample_count: usize) -> Result<Command> {
    let samples = (0..sample_count)
        .map(|index| BrushSample {
            x: 8_040.5 + index as f32 * 1.25,
            y: 8_160.5 + (index as f32 * 0.17).sin() * 72.0,
            pressure: 0.75 + index as f32 / sample_count.max(1) as f32 * 0.25,
        })
        .collect::<Vec<_>>();
    let stroke = BrushStroke::new(
        BrushStyle {
            mode: BrushMode::Paint,
            color: [0; 4],
            size: 36.0,
            hardness: 0.72,
            opacity: 0.9,
            spacing: 0.15,
        },
        samples,
    )?
    .as_current_clone()?;
    Ok(Command::AddBrushStroke {
        id: 2,
        stroke,
        selection: PaintSelection::None,
    })
}

fn validate_render(
    rendered: &image::DynamicImage,
    fallback_decode_bytes: u64,
    region: RenderRegion,
) -> Result<()> {
    if (rendered.width(), rendered.height()) != (region.width, region.height) {
        bail!("Clone Stamp benchmark returned unexpected output dimensions");
    }
    if fallback_decode_bytes != 0 {
        bail!("Clone Stamp benchmark regressed to full-source decode");
    }
    Ok(())
}

struct SolidRasterFixture {
    path: PathBuf,
    source: ResolvedRasterSource,
    max_region_pixels: Arc<AtomicU64>,
}

impl SolidRasterFixture {
    fn new(width: u32, height: u32) -> Result<Self> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let root =
            std::fs::canonicalize(std::env::temp_dir()).unwrap_or_else(|_| std::env::temp_dir());
        let path = root.join(format!("prism-clone-benchmark-{stamp}.png"));
        write_solid_png(&path, width, height)?;
        let max_region_pixels = Arc::new(AtomicU64::new(0));
        let source = ResolvedRasterSource::new(
            RasterSourceEpoch::new(format!("solid-black-{width}x{height}"))?,
            Arc::new(SolidSource::new(
                width,
                height,
                Arc::clone(&max_region_pixels),
            )),
        )?;
        Ok(Self {
            path,
            source,
            max_region_pixels,
        })
    }
}

impl RasterSourceResolver for SolidRasterFixture {
    fn snapshot_epoch(&self) -> u64 {
        1
    }

    fn resolve(&self, path: &Path) -> Option<ResolvedRasterSource> {
        (path == self.path).then(|| self.source.clone())
    }
}

impl Drop for SolidRasterFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn write_solid_png(path: &Path, width: u32, height: u32) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    let mut stream = writer.stream_writer()?;
    let row = vec![0; width as usize];
    for _ in 0..height {
        stream.write_all(&row)?;
    }
    stream.finish()?;
    Ok(())
}

struct SolidSource {
    info: RegionSourceInfo,
    max_region_pixels: Arc<AtomicU64>,
}

impl SolidSource {
    fn new(width: u32, height: u32, max_region_pixels: Arc<AtomicU64>) -> Self {
        Self {
            info: RegionSourceInfo {
                descriptor: RegionSourceDescriptor {
                    width,
                    height,
                    color_encoding: "l8".into(),
                    sample_depth: SourceSampleDepth::EightBit,
                    frame_index: 0,
                    page_index: 0,
                    decoder_contract: "prism-benchmark-solid-l8".into(),
                },
                capability: RegionReadCapability::SequentialBounded,
                readiness: RegionReadiness::Ready,
            },
            max_region_pixels,
        }
    }
}

impl ExactRegionSource for SolidSource {
    type Error = RegionRequestError;

    fn info(&self) -> &RegionSourceInfo {
        &self.info
    }

    fn read_exact_region(&self, region: PixelRegion) -> Result<RgbaImage, Self::Error> {
        validate_region_request(&self.info.descriptor, region, 4_096 * 4_096)?;
        let pixels = u64::from(region.width) * u64::from(region.height);
        self.max_region_pixels.fetch_max(pixels, Ordering::Relaxed);
        Ok(RgbaImage::from_pixel(
            region.width,
            region.height,
            Rgba([0, 0, 0, 255]),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_stamp_benchmark_is_provider_backed_and_bounded() {
        let measured = measure().unwrap();
        assert!(measured.source_full_plane_bytes > 1_000_000_000);
        assert!(
            measured.max_provider_region_pixels
                <= u64::from(REGION_WIDTH) * u64::from(REGION_HEIGHT)
        );
        assert!(!measured.viewport_samples.is_empty());
        assert_eq!(measured.live_samples.len(), LIVE_FRAMES);
    }
}
