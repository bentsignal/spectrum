use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    time::{Duration, Instant},
};

use prism_core::{
    DerivedBackingCache, DerivedBackingIdentity, DerivedBackingLimits, Document, LayerKind,
    PrepareDerivedBacking, RasterSourceEpoch, RasterSourceResolver, ResolvedRasterSource,
};
use spectrum_imaging::RegionReadCapability;

use super::PrismApp;

const PREPARATION_QUEUE_CAPACITY: usize = 16;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RasterRenderMode {
    LegacyNative,
    Provider { snapshot_epoch: u64 },
    FallbackCapped,
}

#[derive(Clone)]
pub(super) struct RasterSourceSnapshot {
    epoch: u64,
    providers: Arc<HashMap<PathBuf, ResolvedRasterSource>>,
    legacy_native: Arc<HashSet<PathBuf>>,
}

impl RasterSourceSnapshot {
    pub(super) fn empty() -> Self {
        Self {
            epoch: 0,
            providers: Arc::new(HashMap::new()),
            legacy_native: Arc::new(HashSet::new()),
        }
    }

    pub(super) fn render_mode(&self, document: &Document) -> RasterRenderMode {
        let mut raster_class = None;
        for layer in document
            .layers
            .iter()
            .filter(|layer| layer.visible && layer.opacity > 0.0)
        {
            if !layer.adjustments.spots.is_empty()
                || !layer.transform.scale_x.is_finite()
                || !layer.transform.scale_y.is_finite()
                || layer.transform.scale_x <= 0.0
                || layer.transform.scale_y <= 0.0
            {
                return RasterRenderMode::FallbackCapped;
            }
            let LayerKind::Raster { path, .. } = &layer.kind else {
                continue;
            };
            let class = if self.providers.contains_key(path) {
                RasterClass::Provider
            } else if self.legacy_native.contains(path) {
                RasterClass::LegacyNative
            } else {
                return RasterRenderMode::FallbackCapped;
            };
            if raster_class.is_some_and(|existing| existing != class) {
                // The provider renderer intentionally cannot fall back to paths. Keep mixed
                // sequential/provider documents bounded until sequential sources have an
                // immutable provider of their own.
                return RasterRenderMode::FallbackCapped;
            }
            raster_class = Some(class);
        }

        match raster_class {
            Some(RasterClass::Provider)
                if prism_core::document_supports_region_native_zoom_with_sources(
                    document, self,
                ) =>
            {
                RasterRenderMode::Provider {
                    snapshot_epoch: self.epoch,
                }
            }
            Some(RasterClass::Provider) => RasterRenderMode::FallbackCapped,
            Some(RasterClass::LegacyNative) | None => RasterRenderMode::LegacyNative,
        }
    }

    #[cfg(test)]
    pub(super) fn with_test_provider(
        epoch: u64,
        path: PathBuf,
        source: ResolvedRasterSource,
    ) -> Arc<Self> {
        Arc::new(Self {
            epoch,
            providers: Arc::new(HashMap::from([(path, source)])),
            legacy_native: Arc::new(HashSet::new()),
        })
    }
}

impl RasterSourceResolver for RasterSourceSnapshot {
    fn snapshot_epoch(&self) -> u64 {
        self.epoch
    }

    fn resolve(&self, path: &Path) -> Option<ResolvedRasterSource> {
        self.providers.get(path).cloned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RasterClass {
    LegacyNative,
    Provider,
}

enum PathPhase {
    Needed,
    InFlight,
    Retry {
        identity: Option<DerivedBackingIdentity>,
        attempts: u32,
        retry_at: Instant,
    },
    LegacyNative,
    Provider(ResolvedRasterSource),
    Unsupported,
}

struct PathState {
    generation: u64,
    required_tabs: HashSet<u64>,
    phase: PathPhase,
}

struct PreparationRequest {
    path: PathBuf,
    generation: u64,
    identity: Option<DerivedBackingIdentity>,
    attempts: u32,
}

struct PreparationResult {
    path: PathBuf,
    generation: u64,
    attempts: u32,
    outcome: PreparationOutcome,
}

enum PreparationOutcome {
    LegacyNative,
    Ready(ResolvedRasterSource),
    InProgress(DerivedBackingIdentity),
    Unsupported,
    Failed(String),
}

pub(super) struct RasterSourceCoordinator {
    request_sender: Option<SyncSender<PreparationRequest>>,
    result_receiver: Receiver<PreparationResult>,
    tab_paths: HashMap<u64, HashSet<PathBuf>>,
    paths: HashMap<PathBuf, PathState>,
    snapshot: Arc<RasterSourceSnapshot>,
    next_generation: u64,
}

impl RasterSourceCoordinator {
    pub(super) fn new(repaint: egui::Context) -> Self {
        let cache_root = eframe::storage_dir("Prism").map(|directory| {
            directory
                .join("Derived Raster Backings")
                .join(env!("CARGO_PKG_VERSION"))
        });
        let (result_sender, result_receiver) = mpsc::channel();
        let request_sender = cache_root.map(|root| {
            let (request_sender, request_receiver) = mpsc::sync_channel(PREPARATION_QUEUE_CAPACITY);
            let cache = DerivedBackingCache::new(root, DerivedBackingLimits::default());
            std::thread::spawn(move || {
                while let Ok(request) = request_receiver.recv() {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        prepare_source(&cache, &request.path, request.identity.as_ref())
                    }))
                    .unwrap_or_else(|_| {
                        PreparationOutcome::Failed("Raster source preparation panicked".into())
                    });
                    if result_sender
                        .send(PreparationResult {
                            path: request.path,
                            generation: request.generation,
                            attempts: request.attempts,
                            outcome,
                        })
                        .is_err()
                    {
                        break;
                    }
                    repaint.request_repaint();
                }
            });
            request_sender
        });
        Self {
            request_sender,
            result_receiver,
            tab_paths: HashMap::new(),
            paths: HashMap::new(),
            snapshot: Arc::new(RasterSourceSnapshot::empty()),
            next_generation: 0,
        }
    }

    pub(super) fn snapshot(&self) -> Arc<RasterSourceSnapshot> {
        Arc::clone(&self.snapshot)
    }

    pub(super) fn set_tab_document(&mut self, tab_id: u64, document: &Document) {
        let desired: HashSet<_> = document
            .layers
            .iter()
            .filter_map(|layer| match &layer.kind {
                LayerKind::Raster { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        if self.tab_paths.get(&tab_id) == Some(&desired) {
            return;
        }
        let previous = self
            .tab_paths
            .insert(tab_id, desired.clone())
            .unwrap_or_default();
        let mut publish = false;
        for path in previous.difference(&desired) {
            if let Some(state) = self.paths.get_mut(path) {
                state.required_tabs.remove(&tab_id);
                if state.required_tabs.is_empty() {
                    publish |= matches!(
                        &state.phase,
                        PathPhase::LegacyNative | PathPhase::Provider(_)
                    );
                }
            }
        }
        self.paths
            .retain(|_, state| !state.required_tabs.is_empty());
        for path in desired.difference(&previous) {
            if let Some(state) = self.paths.get_mut(path) {
                state.required_tabs.insert(tab_id);
                continue;
            }
            self.next_generation = self
                .next_generation
                .checked_add(1)
                .expect("raster source generation exhausted");
            self.paths.insert(
                path.clone(),
                PathState {
                    generation: self.next_generation,
                    required_tabs: HashSet::from([tab_id]),
                    phase: PathPhase::Needed,
                },
            );
        }
        if publish {
            self.publish_snapshot();
        }
        self.dispatch_ready(Instant::now());
    }

    pub(super) fn remove_tab(&mut self, tab_id: u64) {
        let Some(paths) = self.tab_paths.remove(&tab_id) else {
            return;
        };
        let mut publish = false;
        for path in paths {
            if let Some(state) = self.paths.get_mut(&path) {
                state.required_tabs.remove(&tab_id);
                if state.required_tabs.is_empty() {
                    publish |= matches!(
                        &state.phase,
                        PathPhase::LegacyNative | PathPhase::Provider(_)
                    );
                }
            }
        }
        self.paths
            .retain(|_, state| !state.required_tabs.is_empty());
        if publish {
            self.publish_snapshot();
        }
    }

    pub(super) fn poll(&mut self, context: &egui::Context) {
        while let Ok(result) = self.result_receiver.try_recv() {
            self.apply_result(result, Instant::now());
        }
        let now = Instant::now();
        self.dispatch_ready(now);
        if let Some(delay) = self.next_retry_delay(now) {
            context.request_repaint_after(delay.max(Duration::from_millis(50)));
        }
    }

    fn apply_result(&mut self, result: PreparationResult, now: Instant) {
        let Some(state) = self.paths.get_mut(&result.path) else {
            return;
        };
        if state.generation != result.generation || !matches!(&state.phase, PathPhase::InFlight) {
            return;
        }
        let mut publish = false;
        state.phase = match result.outcome {
            PreparationOutcome::LegacyNative => {
                publish = true;
                PathPhase::LegacyNative
            }
            PreparationOutcome::Ready(source) => {
                publish = true;
                PathPhase::Provider(source)
            }
            PreparationOutcome::InProgress(identity) => PathPhase::Retry {
                identity: Some(identity),
                attempts: result.attempts.saturating_add(1),
                retry_at: now + preparation_retry_delay(result.attempts.saturating_add(1)),
            },
            PreparationOutcome::Unsupported => PathPhase::Unsupported,
            PreparationOutcome::Failed(error) => {
                drop(error);
                PathPhase::Retry {
                    identity: None,
                    attempts: result.attempts.saturating_add(1),
                    retry_at: now + preparation_retry_delay(result.attempts.saturating_add(1)),
                }
            }
        };
        if publish {
            self.publish_snapshot();
        }
    }

    fn dispatch_ready(&mut self, now: Instant) {
        let Some(sender) = self.request_sender.as_ref().cloned() else {
            return;
        };
        let mut disconnected = false;
        for (path, state) in &mut self.paths {
            let (identity, attempts) = match &state.phase {
                PathPhase::Needed => (None, 0),
                PathPhase::Retry {
                    identity,
                    attempts,
                    retry_at,
                } if *retry_at <= now => (identity.clone(), *attempts),
                _ => continue,
            };
            let request = PreparationRequest {
                path: path.clone(),
                generation: state.generation,
                identity,
                attempts,
            };
            match sender.try_send(request) {
                Ok(()) => state.phase = PathPhase::InFlight,
                Err(TrySendError::Full(_)) => break,
                Err(TrySendError::Disconnected(_)) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected {
            self.request_sender = None;
        }
    }

    fn next_retry_delay(&self, now: Instant) -> Option<Duration> {
        self.paths
            .values()
            .filter_map(|state| match &state.phase {
                PathPhase::Retry { retry_at, .. } => Some(retry_at.saturating_duration_since(now)),
                _ => None,
            })
            .min()
    }

    fn publish_snapshot(&mut self) {
        let providers = self
            .paths
            .iter()
            .filter_map(|(path, state)| match &state.phase {
                PathPhase::Provider(source) => Some((path.clone(), source.clone())),
                _ => None,
            })
            .collect();
        let legacy_native = self
            .paths
            .iter()
            .filter_map(|(path, state)| {
                matches!(&state.phase, PathPhase::LegacyNative).then_some(path.clone())
            })
            .collect();
        let epoch = self
            .snapshot
            .epoch
            .checked_add(1)
            .expect("raster source snapshot epoch exhausted");
        self.snapshot = Arc::new(RasterSourceSnapshot {
            epoch,
            providers: Arc::new(providers),
            legacy_native: Arc::new(legacy_native),
        });
    }
}

impl PrismApp {
    pub(super) fn sync_active_raster_sources(&mut self) {
        self.raster_sources
            .set_tab_document(self.active_tab_id, &self.workspace.document);
    }
}

fn prepare_source(
    cache: &DerivedBackingCache,
    path: &Path,
    identity: Option<&DerivedBackingIdentity>,
) -> PreparationOutcome {
    if let Some(identity) = identity {
        return prepared_outcome(identity.clone(), cache.prepare_identified(path, identity));
    }
    let inspection = match prism_core::inspect_raster_region_source(path) {
        Ok(inspection) => inspection,
        Err(error) => return PreparationOutcome::Failed(format!("{error:#}")),
    };
    match inspection.info.capability {
        RegionReadCapability::SequentialBounded if inspection.info.supports_region_reads_now() => {
            PreparationOutcome::LegacyNative
        }
        RegionReadCapability::DerivedBacking => {
            let identity = match cache.identify(path) {
                Ok(identity) => identity,
                Err(error) => return PreparationOutcome::Failed(format!("{error:#}")),
            };
            prepared_outcome(identity.clone(), cache.prepare_identified(path, &identity))
        }
        RegionReadCapability::SequentialBounded
        | RegionReadCapability::SeekableChunks
        | RegionReadCapability::FullDecodeOnly => PreparationOutcome::Unsupported,
    }
}

fn prepared_outcome(
    identity: DerivedBackingIdentity,
    result: anyhow::Result<PrepareDerivedBacking>,
) -> PreparationOutcome {
    match result {
        Ok(PrepareDerivedBacking::Ready { backing, .. }) => {
            let source_epoch = match RasterSourceEpoch::new(backing.key().to_owned()) {
                Ok(epoch) => epoch,
                Err(error) => return PreparationOutcome::Failed(format!("{error:#}")),
            };
            let source = Arc::new(backing);
            let source = match ResolvedRasterSource::new(source_epoch, source) {
                Ok(source) => source,
                Err(error) => return PreparationOutcome::Failed(format!("{error:#}")),
            };
            debug_assert_eq!(identity.key(), source.source_epoch().as_str());
            PreparationOutcome::Ready(source)
        }
        Ok(PrepareDerivedBacking::InProgress(identity)) => PreparationOutcome::InProgress(identity),
        Err(error) => PreparationOutcome::Failed(format!("{error:#}")),
    }
}

fn preparation_retry_delay(attempts: u32) -> Duration {
    let exponent = attempts.saturating_sub(1).min(8);
    Duration::from_millis(100_u64 << exponent).min(MAX_RETRY_DELAY)
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use image::{Rgba, RgbaImage};
    use spectrum_imaging::{
        ExactRegionSource, PixelRegion, RegionReadCapability, RegionReadiness,
        RegionSourceDescriptor, RegionSourceInfo, SourceSampleDepth,
    };

    use super::*;
    use prism_core::Layer;

    struct TestSource {
        info: RegionSourceInfo,
        drops: Option<Arc<AtomicUsize>>,
    }

    impl Drop for TestSource {
        fn drop(&mut self) {
            if let Some(drops) = &self.drops {
                drops.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    impl ExactRegionSource for TestSource {
        type Error = Infallible;

        fn info(&self) -> &RegionSourceInfo {
            &self.info
        }

        fn read_exact_region(&self, region: PixelRegion) -> Result<RgbaImage, Self::Error> {
            Ok(RgbaImage::from_pixel(
                region.width,
                region.height,
                Rgba([17, 31, 47, 255]),
            ))
        }
    }

    fn resolved(epoch: &str, drops: Option<Arc<AtomicUsize>>) -> ResolvedRasterSource {
        ResolvedRasterSource::new(
            RasterSourceEpoch::new(epoch.to_owned()).unwrap(),
            Arc::new(TestSource {
                info: RegionSourceInfo {
                    descriptor: RegionSourceDescriptor {
                        width: 4,
                        height: 4,
                        color_encoding: "rgba8".into(),
                        sample_depth: SourceSampleDepth::EightBit,
                        frame_index: 0,
                        page_index: 0,
                        decoder_contract: "test".into(),
                    },
                    capability: RegionReadCapability::DerivedBacking,
                    readiness: RegionReadiness::Ready,
                },
                drops,
            }),
        )
        .unwrap()
    }

    fn raster_document(path: impl Into<PathBuf>) -> Document {
        let mut document = Document::new("Raster", 4, 4);
        document.layers.push(Layer {
            kind: LayerKind::Raster {
                path: path.into(),
                original_path: None,
            },
            ..Layer::default()
        });
        document
    }

    fn detached_coordinator() -> (RasterSourceCoordinator, Receiver<PreparationRequest>) {
        detached_coordinator_with_capacity(16)
    }

    fn detached_coordinator_with_capacity(
        capacity: usize,
    ) -> (RasterSourceCoordinator, Receiver<PreparationRequest>) {
        let (request_sender, request_receiver) = mpsc::sync_channel(capacity);
        let (_result_sender, result_receiver) = mpsc::channel();
        (
            RasterSourceCoordinator {
                request_sender: Some(request_sender),
                result_receiver,
                tab_paths: HashMap::new(),
                paths: HashMap::new(),
                snapshot: Arc::new(RasterSourceSnapshot::empty()),
                next_generation: 0,
            },
            request_receiver,
        )
    }

    #[test]
    fn full_preparation_queue_leaves_later_work_needed_without_blocking() {
        let first = raster_document("first.jpg");
        let second = raster_document("second.jpg");
        let (mut coordinator, requests) = detached_coordinator_with_capacity(1);
        coordinator.set_tab_document(1, &first);
        coordinator.set_tab_document(2, &second);
        assert!(matches!(
            &coordinator.paths[Path::new("second.jpg")].phase,
            PathPhase::Needed
        ));
        let _ = requests.try_recv().unwrap();
        coordinator.dispatch_ready(Instant::now());
        assert_eq!(
            requests.try_recv().unwrap().path,
            PathBuf::from("second.jpg")
        );
    }

    #[test]
    fn duplicate_layers_and_tabs_share_one_preparation() {
        let path = PathBuf::from("same.jpg");
        let mut document = raster_document(path.clone());
        document.layers.push(document.layers[0].clone());
        let (mut coordinator, requests) = detached_coordinator();
        coordinator.set_tab_document(1, &document);
        let first = requests.try_recv().unwrap();
        assert_eq!(first.path, path);
        coordinator.set_tab_document(2, &document);
        assert!(requests.try_recv().is_err());
        assert_eq!(coordinator.paths[&path].required_tabs.len(), 2);
    }

    #[test]
    fn stale_completion_is_rejected_after_path_reappears() {
        let path = PathBuf::from("generation.jpg");
        let document = raster_document(path.clone());
        let (mut coordinator, requests) = detached_coordinator();
        coordinator.set_tab_document(1, &document);
        let stale = requests.try_recv().unwrap();
        coordinator.set_tab_document(1, &Document::new("Empty", 4, 4));
        coordinator.set_tab_document(1, &document);
        let current = requests.try_recv().unwrap();
        assert_ne!(stale.generation, current.generation);

        coordinator.apply_result(
            PreparationResult {
                path: stale.path,
                generation: stale.generation,
                attempts: 0,
                outcome: PreparationOutcome::LegacyNative,
            },
            Instant::now(),
        );
        assert!(coordinator.snapshot.legacy_native.is_empty());
        coordinator.apply_result(
            PreparationResult {
                path: current.path,
                generation: current.generation,
                attempts: 0,
                outcome: PreparationOutcome::LegacyNative,
            },
            Instant::now(),
        );
        assert!(coordinator.snapshot.legacy_native.contains(&path));
    }

    #[test]
    fn retry_backoff_retains_the_identified_source() {
        let directory = std::env::temp_dir().join(format!(
            "prism-source-coordinator-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.jpg");
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(1, 1, image::Rgb([4, 8, 12])))
            .save(&source)
            .unwrap();
        let cache =
            DerivedBackingCache::new(directory.join("cache"), DerivedBackingLimits::default());
        let identity = cache.identify(&source).unwrap();
        let document = raster_document(source.clone());
        let (mut coordinator, requests) = detached_coordinator();
        coordinator.set_tab_document(1, &document);
        let request = requests.try_recv().unwrap();
        let now = Instant::now();
        coordinator.apply_result(
            PreparationResult {
                path: request.path,
                generation: request.generation,
                attempts: 0,
                outcome: PreparationOutcome::InProgress(identity.clone()),
            },
            now,
        );
        assert_eq!(coordinator.snapshot.epoch, 0);
        coordinator.dispatch_ready(now + preparation_retry_delay(1));
        let retry = requests.try_recv().unwrap();
        assert_eq!(retry.identity.unwrap().key(), identity.key());
        assert_eq!(retry.attempts, 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn only_ready_results_publish_and_old_snapshots_retain_provider_lifetimes() {
        let path = PathBuf::from("leased.jpg");
        let document = raster_document(path.clone());
        let (mut coordinator, requests) = detached_coordinator();
        coordinator.set_tab_document(1, &document);
        let request = requests.try_recv().unwrap();
        assert_eq!(coordinator.snapshot.epoch, 0);
        let drops = Arc::new(AtomicUsize::new(0));
        coordinator.apply_result(
            PreparationResult {
                path: request.path,
                generation: request.generation,
                attempts: 0,
                outcome: PreparationOutcome::Ready(resolved("ready", Some(Arc::clone(&drops)))),
            },
            Instant::now(),
        );
        let retained = coordinator.snapshot();
        assert!(retained.resolve(&path).is_some());
        coordinator.remove_tab(1);
        assert!(coordinator.snapshot.resolve(&path).is_none());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(retained);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn memory_only_modes_preserve_png_zoom_and_cap_mixed_documents() {
        let png = PathBuf::from("missing-but-classified.png");
        let jpg = PathBuf::from("missing-but-backed.jpg");
        let png_document = raster_document(png.clone());
        let (mut coordinator, requests) = detached_coordinator();
        coordinator.set_tab_document(1, &png_document);
        let request = requests.try_recv().unwrap();
        coordinator.apply_result(
            PreparationResult {
                path: request.path,
                generation: request.generation,
                attempts: 0,
                outcome: PreparationOutcome::LegacyNative,
            },
            Instant::now(),
        );
        assert_eq!(
            coordinator.snapshot.render_mode(&png_document),
            RasterRenderMode::LegacyNative
        );

        let mut mixed = png_document;
        let mut jpg_document = raster_document(jpg.clone());
        mixed.layers.push(jpg_document.layers.remove(0));
        let snapshot = RasterSourceSnapshot {
            epoch: 7,
            providers: Arc::new(HashMap::from([(jpg, resolved("provider", None))])),
            legacy_native: Arc::new(HashSet::from([png])),
        };
        assert_eq!(
            snapshot.render_mode(&mixed),
            RasterRenderMode::FallbackCapped
        );
    }
}
