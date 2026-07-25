use std::{
    collections::HashMap,
    convert::Infallible,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use image::RgbaImage;
use sha2::{Digest, Sha256};
use spectrum_imaging::{
    ExactRegionSource, PixelRegion, RegionReadCapability, RegionReadiness, RegionSourceDescriptor,
    RegionSourceInfo, SourceSampleDepth,
};

use crate::{
    DerivedBackingCache, DerivedBackingLimits, Document, PrepareDerivedBacking, RasterSourceEpoch,
    RasterSourceResolver, ResolvedRasterSource, SequentialPngLimits, SequentialPngSource,
};

const EXPORT_PREPARATION_TIMEOUT: Duration = Duration::from_secs(30);
const EXPORT_PREPARATION_RETRY: Duration = Duration::from_millis(25);
pub const RASTER_BACKING_CACHE_COMPATIBILITY: &str = "derived-rgba8-schema-v2";

#[derive(Clone)]
pub struct PreparedRasterSources {
    providers: Arc<HashMap<PathBuf, ResolvedRasterSource>>,
}

impl RasterSourceResolver for PreparedRasterSources {
    fn snapshot_epoch(&self) -> u64 {
        1
    }

    fn resolve(&self, path: &Path) -> Option<ResolvedRasterSource> {
        self.providers.get(path).cloned().or_else(|| {
            std::fs::canonicalize(path)
                .ok()
                .and_then(|canonical| self.providers.get(&canonical).cloned())
        })
    }
}

pub fn raster_backing_cache_root(storage_directory: &Path, app_version: &str) -> PathBuf {
    storage_directory
        .join("Derived Raster Backings")
        .join(RASTER_BACKING_CACHE_COMPATIBILITY)
        .join(app_version)
}

pub fn default_raster_backing_cache_root() -> Result<PathBuf> {
    eframe::storage_dir("Prism")
        .map(|directory| raster_backing_cache_root(&directory, env!("CARGO_PKG_VERSION")))
        .context("Prism could not locate its local raster backing cache")
}

pub fn prepare_export_raster_sources(
    document: &Document,
    cache_root: &Path,
) -> Result<PreparedRasterSources> {
    let expected = aggregate_requirements(document)?;
    let cache = DerivedBackingCache::new(cache_root, DerivedBackingLimits::default());
    let mut providers = HashMap::with_capacity(expected.len());
    for (canonical, requirement) in expected {
        let source = prepare_source(&cache, &requirement.source_path)?;
        if let Some(expected) = requirement.content_sha256.as_deref()
            && source.content_sha256() != Some(expected)
        {
            bail!(
                "prepared export raster provider for {} does not match the exact sampled-source SHA-256",
                requirement.source_path.display()
            );
        }
        providers.insert(canonical, source.clone());
        for alias in requirement.aliases {
            providers.insert(alias, source.clone());
        }
    }
    Ok(PreparedRasterSources {
        providers: Arc::new(providers),
    })
}

struct AggregatedRequirement {
    source_path: PathBuf,
    aliases: Vec<PathBuf>,
    content_sha256: Option<String>,
}

fn aggregate_requirements(document: &Document) -> Result<HashMap<PathBuf, AggregatedRequirement>> {
    let mut expected = HashMap::new();
    for requirement in document.raster_asset_requirements() {
        let canonical =
            std::fs::canonicalize(&requirement.path).unwrap_or_else(|_| requirement.path.clone());
        match expected.entry(canonical) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(AggregatedRequirement {
                    source_path: requirement.path.clone(),
                    aliases: vec![requirement.path],
                    content_sha256: requirement.content_sha256,
                });
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                match (
                    entry.get().content_sha256.as_deref(),
                    requirement.content_sha256.as_deref(),
                ) {
                    (Some(left), Some(right)) if left != right => {
                        bail!(
                            "one export raster path is required with multiple exact content identities"
                        );
                    }
                    (None, Some(_)) => {
                        entry.get_mut().content_sha256 = requirement.content_sha256;
                    }
                    _ => {}
                }
                if !entry.get().aliases.contains(&requirement.path) {
                    entry.get_mut().aliases.push(requirement.path);
                }
            }
        }
    }
    Ok(expected)
}

fn prepare_source(cache: &DerivedBackingCache, path: &Path) -> Result<ResolvedRasterSource> {
    let inspection = crate::inspect_raster_region_source(path)?;
    match inspection.info.capability {
        RegionReadCapability::SequentialBounded if inspection.info.supports_region_reads_now() => {
            let source = SequentialPngSource::open(path, SequentialPngLimits::default())?;
            ResolvedRasterSource::new_authenticated(
                source.source_epoch().clone(),
                source.source_sha256().to_owned(),
                Arc::new(source),
            )
        }
        RegionReadCapability::DerivedBacking => prepare_derived(cache, path),
        RegionReadCapability::SequentialBounded
        | RegionReadCapability::SeekableChunks
        | RegionReadCapability::FullDecodeOnly => {
            prepare_small_memory_source(path, inspection.info)
        }
    }
}

fn prepare_derived(cache: &DerivedBackingCache, path: &Path) -> Result<ResolvedRasterSource> {
    let identity = cache.identify(path)?;
    let deadline = Instant::now() + EXPORT_PREPARATION_TIMEOUT;
    loop {
        match cache.prepare_identified(path, &identity)? {
            PrepareDerivedBacking::Ready { backing, .. } => {
                return ResolvedRasterSource::new_authenticated(
                    RasterSourceEpoch::new(backing.key().to_owned())?,
                    identity.source_sha256().to_owned(),
                    Arc::new(backing),
                );
            }
            PrepareDerivedBacking::InProgress(_) if Instant::now() < deadline => {
                std::thread::sleep(EXPORT_PREPARATION_RETRY);
            }
            PrepareDerivedBacking::InProgress(_) => {
                bail!(
                    "timed out waiting for bounded raster preparation of {}",
                    path.display()
                );
            }
        }
    }
}

struct MemoryRegionSource {
    info: RegionSourceInfo,
    image: RgbaImage,
}

impl ExactRegionSource for MemoryRegionSource {
    type Error = Infallible;

    fn info(&self) -> &RegionSourceInfo {
        &self.info
    }

    fn read_exact_region(&self, region: PixelRegion) -> Result<RgbaImage, Self::Error> {
        Ok(
            image::imageops::crop_imm(&self.image, region.x, region.y, region.width, region.height)
                .to_image(),
        )
    }
}

fn prepare_small_memory_source(
    path: &Path,
    info: RegionSourceInfo,
) -> Result<ResolvedRasterSource> {
    let pixels = u64::from(info.descriptor.width) * u64::from(info.descriptor.height);
    if pixels > crate::MAX_PAINT_REGION_PIXELS {
        bail!(
            "large export raster {} requires a region-native prepared backing",
            path.display()
        );
    }
    let (_, bytes) = crate::font_source::read_secure_regular_file(
        path,
        crate::revisions::MAX_EMBEDDED_RASTER_BYTES,
        "export raster source",
    )?;
    let digest = hex_sha256(&bytes);
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(crate::MAX_CANVAS_DIMENSION);
    limits.max_image_height = Some(crate::MAX_CANVAS_DIMENSION);
    limits.max_alloc = Some(crate::MAX_PAINT_REGION_PIXELS * 8);
    reader.limits(limits);
    let image = reader.decode()?.to_rgba8();
    let info = RegionSourceInfo {
        descriptor: RegionSourceDescriptor {
            width: image.width(),
            height: image.height(),
            color_encoding: "rgba8".into(),
            sample_depth: SourceSampleDepth::EightBit,
            frame_index: info.descriptor.frame_index,
            page_index: info.descriptor.page_index,
            decoder_contract: "prism-export-memory-rgba8-v1".into(),
        },
        capability: RegionReadCapability::SeekableChunks,
        readiness: RegionReadiness::Ready,
    };
    ResolvedRasterSource::new_authenticated(
        RasterSourceEpoch::new(format!("memory:{digest}"))?,
        digest,
        Arc::new(MemoryRegionSource { info, image }),
    )
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::{fs, fs::File, io::BufWriter};

    use crate::{
        BrushMode, BrushProgram, BrushSample, BrushStroke, BrushStyle, Command, Layer, LayerKind,
        PaintSelection, PixelMask, RenderRegion, SampledSourceSnapshot, Transform, Workspace,
        export_document_with_sources, render_document_region_scaled_with_sources,
        render_document_with_sources,
    };
    use spectrum_imaging::Adjustments;

    use super::*;

    fn test_directory() -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        fs::canonicalize(std::env::temp_dir())
            .unwrap_or_else(|_| std::env::temp_dir())
            .join(format!(
                "prism-export-provider-{}-{stamp}",
                std::process::id()
            ))
    }

    fn write_nonuniform_tiff(path: &Path, width: u32, height: u32) {
        let mut encoder =
            tiff::encoder::TiffEncoder::new(BufWriter::new(File::create(path).unwrap())).unwrap();
        let image = encoder
            .new_image::<tiff::encoder::colortype::Gray8>(width, height)
            .unwrap();
        let pixels = (0..width * height)
            .map(|index| {
                let x = index % width;
                let y = index / width;
                x.wrapping_mul(17).wrapping_add(y.wrapping_mul(29)) as u8
            })
            .collect::<Vec<_>>();
        image.write_data(&pixels).unwrap();
    }

    fn relative_to_current_directory(path: &Path) -> PathBuf {
        let current = fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
        let target = fs::canonicalize(path).unwrap();
        let current_components = current.components().collect::<Vec<_>>();
        let target_components = target.components().collect::<Vec<_>>();
        let shared = current_components
            .iter()
            .zip(&target_components)
            .take_while(|(left, right)| left == right)
            .count();
        let mut relative = PathBuf::new();
        for _ in shared..current_components.len() {
            relative.push("..");
        }
        for component in &target_components[shared..] {
            relative.push(component.as_os_str());
        }
        relative
    }

    #[test]
    fn derived_clone_full_region_and_encoded_export_share_exact_provider() {
        let directory = test_directory();
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.tiff");
        write_nonuniform_tiff(&source, 64, 48);
        let mut document = Document::new("Derived export", 32, 24);
        document.background = [0; 4];
        document.layers.push(Layer {
            id: 1,
            visible: false,
            name: "Hidden TIFF".into(),
            kind: LayerKind::Raster {
                path: source,
                original_path: None,
            },
            ..Layer::default()
        });
        document.next_id = 2;
        let mut workspace = Workspace::new(document, None);
        workspace
            .execute(Command::SetCloneSource {
                id: 1,
                document_x: 9.5,
                document_y: 8.5,
                resolved_source: None,
            })
            .unwrap();
        workspace
            .execute(Command::AddPaintLayerWithStroke {
                name: Some("Clone".into()),
                width: 32,
                height: 24,
                stroke: BrushStroke::new(
                    BrushStyle {
                        mode: BrushMode::Paint,
                        size: 8.0,
                        hardness: 0.8,
                        ..BrushStyle::default()
                    },
                    [
                        BrushSample {
                            x: 20.5,
                            y: 12.5,
                            pressure: 1.0,
                        },
                        BrushSample {
                            x: 23.5,
                            y: 15.5,
                            pressure: 1.0,
                        },
                    ],
                )
                .unwrap()
                .as_current_clone()
                .unwrap(),
                selection: PaintSelection::None,
            })
            .unwrap();
        let providers =
            prepare_export_raster_sources(&workspace.document, &directory.join("cache")).unwrap();
        let full = render_document_with_sources(&workspace.document, None, &providers)
            .unwrap()
            .to_rgba8();
        let region = render_document_region_scaled_with_sources(
            &workspace.document,
            1.0,
            RenderRegion {
                x: 0,
                y: 0,
                width: 32,
                height: 24,
            },
            &providers,
        )
        .unwrap()
        .to_rgba8();
        assert_eq!(region, full);
        assert!(full.pixels().any(|pixel| pixel[3] > 0));

        let export = directory.join("export.png");
        export_document_with_sources(&workspace.document, &export, 92, &providers).unwrap();
        assert_eq!(image::open(export).unwrap().to_rgba8(), full);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn relative_preparation_resolves_canonical_transformed_masked_raster_export() {
        let directory = test_directory();
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.tiff");
        write_nonuniform_tiff(&source, 64, 48);
        let canonical = fs::canonicalize(&source).unwrap();
        let relative = relative_to_current_directory(&canonical);
        assert!(relative.is_relative());

        let mut document = Document::new("Raster aliases", 48, 36);
        document.background = [0; 4];
        document.layers.push(Layer {
            id: 1,
            name: "Masked TIFF".into(),
            kind: LayerKind::Raster {
                path: relative,
                original_path: None,
            },
            transform: Transform {
                x: 7.0,
                y: 5.0,
                scale_x: 0.65,
                scale_y: 0.7,
                rotation: 13.0,
            },
            pixel_mask: Some(PixelMask::new(
                64,
                48,
                (0..64 * 48)
                    .map(|index| if index % 5 == 0 { 96 } else { 255 })
                    .collect::<Vec<_>>(),
            )),
            ..Layer::default()
        });
        document.next_id = 2;
        let providers = prepare_export_raster_sources(&document, &directory.join("cache")).unwrap();
        let LayerKind::Raster { path, .. } = &mut document.layers[0].kind else {
            unreachable!()
        };
        *path = canonical;
        let expected = crate::render_document(&document, None).unwrap().to_rgba8();
        let provided = render_document_with_sources(&document, None, &providers)
            .unwrap()
            .to_rgba8();
        assert_eq!(provided, expected);

        let export = directory.join("masked-export.png");
        export_document_with_sources(&document, &export, 92, &providers).unwrap();
        assert_eq!(image::open(export).unwrap().to_rgba8(), expected);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn canonical_aliases_with_conflicting_sampled_digests_fail_closed() {
        let directory = test_directory();
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.tiff");
        write_nonuniform_tiff(&source, 8, 8);
        let canonical = fs::canonicalize(&source).unwrap();
        let relative = relative_to_current_directory(&canonical);
        let mut document = Document::new("Conflicting sampled aliases", 8, 8);
        document.background = [0; 4];
        for (index, (path, content_hash)) in
            [(relative, "11".repeat(32)), (canonical, "22".repeat(32))]
                .into_iter()
                .enumerate()
        {
            let snapshot = SampledSourceSnapshot {
                version: crate::SAMPLED_SOURCE_VERSION,
                source_layer_id: index as u64 + 1,
                source_layer_name: format!("Source {index}"),
                path,
                content_hash,
                width: 8,
                height: 8,
                anchor_local: [1.5, 1.5],
                source_transform: Transform::default(),
                adjustments: Adjustments::default(),
                pixel_mask: None,
                vector_mask: None,
            };
            let source_id = snapshot.stable_id().unwrap();
            let stroke = BrushStroke::new_clone_stamp(
                BrushStyle::default(),
                [BrushSample {
                    x: 2.5,
                    y: 2.5,
                    pressure: 1.0,
                }],
                snapshot.clone(),
            )
            .unwrap();
            document.sampled_sources.insert(source_id, snapshot);
            document.layers.push(Layer {
                id: index as u64 + 1,
                name: format!("Clone {index}"),
                kind: LayerKind::Paint {
                    program: BrushProgram::new(8, 8).unwrap().append(stroke).unwrap(),
                },
                ..Layer::default()
            });
        }
        let error = match prepare_export_raster_sources(&document, &directory.join("cache")) {
            Ok(_) => panic!("conflicting exact sampled-source identities unexpectedly prepared"),
            Err(error) => error,
        };
        assert!(format!("{error:#}").contains("multiple exact content identities"));
        fs::remove_dir_all(directory).unwrap();
    }
}
