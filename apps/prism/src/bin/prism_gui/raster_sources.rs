use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    time::{Duration, Instant},
};

use eframe::egui;
use prism_core::{
    DerivedBackingCache, DerivedBackingIdentity, DerivedBackingLimits, Document, LayerKind,
    PrepareDerivedBacking, RasterSourceEpoch, RasterSourceResolver, ResolvedRasterSource,
    SequentialPngLimits, SequentialPngSource,
};
use spectrum_imaging::RegionReadCapability;

use super::PrismApp;

const PREPARATION_QUEUE_CAPACITY: usize = 16;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_GENERIC_FAILURE_ATTEMPTS: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RasterRenderMode {
    Provider { snapshot_epoch: u64 },
    FallbackCapped,
}

#[derive(Clone)]
pub(super) struct RasterSourceSnapshot {
    epoch: u64,
    providers: Arc<HashMap<PathBuf, ResolvedRasterSource>>,
}

impl RasterSourceSnapshot {
    pub(super) fn empty() -> Self {
        Self {
            epoch: 0,
            providers: Arc::new(HashMap::new()),
        }
    }

    pub(super) fn render_mode(&self, document: &Document) -> RasterRenderMode {
        for layer in document
            .layers
            .iter()
            .filter(|layer| layer.visible && layer.opacity > 0.0)
        {
            if !layer.transform.scale_x.is_finite()
                || !layer.transform.scale_y.is_finite()
                || layer.transform.scale_x <= 0.0
                || layer.transform.scale_y <= 0.0
            {
                return RasterRenderMode::FallbackCapped;
            }
            let LayerKind::Raster { path, .. } = &layer.kind else {
                continue;
            };
            if !self.providers.contains_key(path) {
                return RasterRenderMode::FallbackCapped;
            }
        }
        if document
            .raster_asset_paths()
            .iter()
            .any(|path| !self.providers.contains_key(path))
        {
            return RasterRenderMode::FallbackCapped;
        }

        if prism_core::document_supports_region_native_zoom_with_sources(document, self) {
            RasterRenderMode::Provider {
                snapshot_epoch: self.epoch,
            }
        } else {
            RasterRenderMode::FallbackCapped
        }
    }

    #[cfg(test)]
    pub(super) fn with_test_provider(
        epoch: u64,
        path: PathBuf,
        source: ResolvedRasterSource,
    ) -> Arc<Self> {
        Self::with_test_providers(epoch, [(path, source)])
    }

    #[cfg(test)]
    pub(super) fn with_test_providers(
        epoch: u64,
        sources: impl IntoIterator<Item = (PathBuf, ResolvedRasterSource)>,
    ) -> Arc<Self> {
        Arc::new(Self {
            epoch,
            providers: Arc::new(sources.into_iter().collect()),
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

enum PathPhase {
    Needed,
    InFlight,
    Retry {
        identity: Option<DerivedBackingIdentity>,
        attempts: u32,
        retry_at: Instant,
    },
    Provider(ResolvedRasterSource),
    Unsupported,
    Failed {
        diagnostic: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ContentRequirement {
    Any,
    Exact(String),
    ConflictingExactIdentities,
}

impl ContentRequirement {
    fn merge(&mut self, expected_sha256: Option<String>) {
        let Some(expected) = expected_sha256 else {
            return;
        };
        match self {
            Self::Any => *self = Self::Exact(expected),
            Self::Exact(current) if *current == expected => {}
            Self::Exact(_) | Self::ConflictingExactIdentities => {
                *self = Self::ConflictingExactIdentities;
            }
        }
    }

    fn accepts(&self, source: &ResolvedRasterSource) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(expected) => source.content_sha256() == Some(expected.as_str()),
            Self::ConflictingExactIdentities => false,
        }
    }

    fn conflict_diagnostic(&self) -> Option<String> {
        matches!(self, Self::ConflictingExactIdentities).then(|| {
            "one raster path is required with multiple exact content identities".to_owned()
        })
    }
}

struct PathState {
    generation: u64,
    requirement: ContentRequirement,
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
    Ready(ResolvedRasterSource),
    InProgress(DerivedBackingIdentity),
    Unsupported,
    Failed(String),
}

pub(super) struct RasterSourceCoordinator {
    request_sender: Option<SyncSender<PreparationRequest>>,
    result_receiver: Receiver<PreparationResult>,
    tab_requirements: HashMap<u64, HashMap<PathBuf, ContentRequirement>>,
    paths: HashMap<PathBuf, PathState>,
    active_tab: Option<u64>,
    active_generations: Arc<Mutex<HashMap<PathBuf, u64>>>,
    snapshot: Arc<RasterSourceSnapshot>,
    next_generation: u64,
}

fn spawn_preparation_worker<F, W>(
    request_receiver: Receiver<PreparationRequest>,
    result_sender: mpsc::Sender<PreparationResult>,
    active_generations: Arc<Mutex<HashMap<PathBuf, u64>>>,
    wake: W,
    prepare: F,
) -> std::thread::JoinHandle<()>
where
    F: Fn(&Path, Option<&DerivedBackingIdentity>) -> PreparationOutcome + Send + 'static,
    W: Fn() + Send + 'static,
{
    std::thread::spawn(move || {
        while let Ok(request) = request_receiver.recv() {
            let request_is_active = active_generations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&request.path)
                == Some(&request.generation);
            if !request_is_active {
                wake();
                continue;
            }
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                prepare(&request.path, request.identity.as_ref())
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
            wake();
        }
    })
}

impl RasterSourceCoordinator {
    pub(super) fn new(repaint: egui::Context) -> Self {
        let cache_root = eframe::storage_dir("Prism")
            .map(|directory| derived_cache_root(&directory, env!("CARGO_PKG_VERSION")));
        let (result_sender, result_receiver) = mpsc::channel();
        let active_generations = Arc::new(Mutex::new(HashMap::new()));
        let request_sender = cache_root.map(|root| {
            let (request_sender, request_receiver) = mpsc::sync_channel(PREPARATION_QUEUE_CAPACITY);
            let cache = DerivedBackingCache::new(root, DerivedBackingLimits::default());
            let wake = move || repaint.request_repaint();
            let _worker = spawn_preparation_worker(
                request_receiver,
                result_sender,
                Arc::clone(&active_generations),
                wake,
                move |path, identity| prepare_source(&cache, path, identity),
            );
            request_sender
        });
        Self {
            request_sender,
            result_receiver,
            tab_requirements: HashMap::new(),
            paths: HashMap::new(),
            active_tab: None,
            active_generations,
            snapshot: Arc::new(RasterSourceSnapshot::empty()),
            next_generation: 0,
        }
    }

    pub(super) fn snapshot(&self) -> Arc<RasterSourceSnapshot> {
        Arc::clone(&self.snapshot)
    }

    pub(super) fn terminal_failure(&self) -> Option<(PathBuf, String)> {
        self.paths
            .iter()
            .find_map(|(path, state)| match &state.phase {
                PathPhase::Failed { diagnostic } => Some((path.clone(), diagnostic.clone())),
                _ => None,
            })
    }

    pub(super) fn retry_terminal_failures(&mut self) -> usize {
        let failed: Vec<_> = self
            .paths
            .iter()
            .filter_map(|(path, state)| {
                (matches!(&state.phase, PathPhase::Failed { .. })
                    && state.requirement != ContentRequirement::ConflictingExactIdentities)
                    .then_some(path.clone())
            })
            .collect();
        for path in &failed {
            self.next_generation = self
                .next_generation
                .checked_add(1)
                .expect("raster source generation exhausted");
            if let Some(state) = self.paths.get_mut(path) {
                state.generation = self.next_generation;
                state.phase = PathPhase::Needed;
            }
        }
        if !failed.is_empty() {
            self.refresh_active_generations();
            self.dispatch_ready(Instant::now());
        }
        failed.len()
    }

    pub(super) fn set_tab_document(&mut self, tab_id: u64, document: &Document) {
        let desired = document_content_requirements(document);
        if self.tab_requirements.get(&tab_id) == Some(&desired) {
            return;
        }
        self.tab_requirements.insert(tab_id, desired);
        if self.active_tab == Some(tab_id) {
            self.reconcile_active_paths();
        }
    }

    pub(super) fn set_active_tab(&mut self, tab_id: u64) {
        if self.active_tab == Some(tab_id) {
            return;
        }
        self.active_tab = Some(tab_id);
        self.reconcile_active_paths();
    }

    pub(super) fn remove_tab(&mut self, tab_id: u64) {
        self.tab_requirements.remove(&tab_id);
        if self.active_tab == Some(tab_id) {
            self.active_tab = None;
            self.reconcile_active_paths();
        }
    }

    fn reconcile_active_paths(&mut self) {
        let desired = self
            .active_tab
            .and_then(|tab_id| self.tab_requirements.get(&tab_id))
            .cloned()
            .unwrap_or_default();
        let removed_published = self.paths.iter().any(|(path, state)| {
            desired.get(path) != Some(&state.requirement)
                && matches!(&state.phase, PathPhase::Provider(_))
        });
        self.paths
            .retain(|path, state| desired.get(path) == Some(&state.requirement));
        for (path, requirement) in desired {
            if self.paths.contains_key(&path) {
                continue;
            }
            self.next_generation = self
                .next_generation
                .checked_add(1)
                .expect("raster source generation exhausted");
            self.paths.insert(
                path,
                PathState {
                    generation: self.next_generation,
                    phase: requirement
                        .conflict_diagnostic()
                        .map_or(PathPhase::Needed, |diagnostic| PathPhase::Failed {
                            diagnostic,
                        }),
                    requirement,
                },
            );
        }
        self.refresh_active_generations();
        if removed_published {
            self.publish_snapshot();
        }
        self.dispatch_ready(Instant::now());
    }

    fn refresh_active_generations(&self) {
        *self
            .active_generations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = self
            .paths
            .iter()
            .map(|(path, state)| (path.clone(), state.generation))
            .collect();
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
            PreparationOutcome::Ready(source) if state.requirement.accepts(&source) => {
                publish = true;
                PathPhase::Provider(source)
            }
            PreparationOutcome::Ready(_) => PathPhase::Failed {
                diagnostic:
                    "prepared raster provider does not match the exact required content identity"
                        .into(),
            },
            PreparationOutcome::InProgress(identity) => PathPhase::Retry {
                identity: Some(identity),
                attempts: result.attempts.saturating_add(1),
                retry_at: now + preparation_retry_delay(result.attempts.saturating_add(1)),
            },
            PreparationOutcome::Unsupported => PathPhase::Unsupported,
            PreparationOutcome::Failed(error) => {
                let attempts = result.attempts.saturating_add(1);
                if attempts >= MAX_GENERIC_FAILURE_ATTEMPTS {
                    PathPhase::Failed { diagnostic: error }
                } else {
                    PathPhase::Retry {
                        identity: None,
                        attempts,
                        retry_at: now + preparation_retry_delay(attempts),
                    }
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
                PathPhase::Failed { diagnostic } => {
                    debug_assert!(!diagnostic.is_empty());
                    None
                }
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
        let epoch = self
            .snapshot
            .epoch
            .checked_add(1)
            .expect("raster source snapshot epoch exhausted");
        self.snapshot = Arc::new(RasterSourceSnapshot {
            epoch,
            providers: Arc::new(providers),
        });
    }
}

fn document_content_requirements(document: &Document) -> HashMap<PathBuf, ContentRequirement> {
    let mut desired = HashMap::new();
    for requirement in document.raster_asset_requirements() {
        desired
            .entry(requirement.path)
            .and_modify(|current: &mut ContentRequirement| {
                current.merge(requirement.content_sha256.clone());
            })
            .or_insert_with(|| {
                requirement
                    .content_sha256
                    .map_or(ContentRequirement::Any, ContentRequirement::Exact)
            });
    }
    desired
}

impl PrismApp {
    pub(super) fn sync_active_raster_sources(&mut self) {
        self.raster_sources
            .set_tab_document(self.active_tab_id, &self.workspace.document);
        self.raster_sources.set_active_tab(self.active_tab_id);
    }
}

fn derived_cache_root(storage_directory: &Path, app_version: &str) -> PathBuf {
    prism_core::raster_backing_cache_root(storage_directory, app_version)
}

pub(super) fn terminal_failure_status(path: &Path, diagnostic: &str) -> String {
    format!(
        "Bounded preview failed for {}: {diagnostic}",
        path.display()
    )
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
            let source = match SequentialPngSource::open(path, SequentialPngLimits::default()) {
                Ok(source) => source,
                Err(error) => return PreparationOutcome::Failed(format!("{error:#}")),
            };
            let source_epoch = source.source_epoch().clone();
            let source_sha256 = source.source_sha256().to_owned();
            match ResolvedRasterSource::new_authenticated(
                source_epoch,
                source_sha256,
                Arc::new(source),
            ) {
                Ok(source) => PreparationOutcome::Ready(source),
                Err(error) => PreparationOutcome::Failed(format!("{error:#}")),
            }
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
            let source = match ResolvedRasterSource::new_authenticated(
                source_epoch,
                identity.source_sha256().to_owned(),
                source,
            ) {
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

    pub(super) fn resolved(epoch: &str, drops: Option<Arc<AtomicUsize>>) -> ResolvedRasterSource {
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

    pub(super) fn resolved_authenticated(
        epoch: &str,
        content_sha256: &str,
    ) -> ResolvedRasterSource {
        ResolvedRasterSource::new_authenticated(
            RasterSourceEpoch::new(epoch.to_owned()).unwrap(),
            content_sha256.to_owned(),
            Arc::new(TestSource {
                info: RegionSourceInfo {
                    descriptor: RegionSourceDescriptor {
                        width: 4,
                        height: 4,
                        color_encoding: "rgba8".into(),
                        sample_depth: SourceSampleDepth::EightBit,
                        frame_index: 0,
                        page_index: 0,
                        decoder_contract: "authenticated-test".into(),
                    },
                    capability: RegionReadCapability::DerivedBacking,
                    readiness: RegionReadiness::Ready,
                },
                drops: None,
            }),
        )
        .unwrap()
    }

    pub(super) fn raster_document(path: impl Into<PathBuf>) -> Document {
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

    pub(super) fn detached_coordinator() -> (RasterSourceCoordinator, Receiver<PreparationRequest>)
    {
        detached_coordinator_with_capacity(16)
    }

    pub(super) fn detached_coordinator_with_capacity(
        capacity: usize,
    ) -> (RasterSourceCoordinator, Receiver<PreparationRequest>) {
        let (request_sender, request_receiver) = mpsc::sync_channel(capacity);
        let (_result_sender, result_receiver) = mpsc::channel();
        (
            RasterSourceCoordinator {
                request_sender: Some(request_sender),
                result_receiver,
                tab_requirements: HashMap::new(),
                paths: HashMap::new(),
                active_tab: None,
                active_generations: Arc::new(Mutex::new(HashMap::new())),
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
        coordinator.set_active_tab(1);
        coordinator.set_tab_document(2, &second);
        coordinator.set_active_tab(2);
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
        coordinator.set_active_tab(1);
        let first = requests.try_recv().unwrap();
        assert_eq!(first.path, path);
        coordinator.set_tab_document(2, &document);
        assert!(requests.try_recv().is_err());
        assert_eq!(coordinator.tab_requirements.len(), 2);
        assert_eq!(coordinator.paths.len(), 1);
    }

    #[test]
    fn stale_completion_is_rejected_after_path_reappears() {
        let path = PathBuf::from("generation.jpg");
        let document = raster_document(path.clone());
        let (mut coordinator, requests) = detached_coordinator();
        coordinator.set_tab_document(1, &document);
        coordinator.set_active_tab(1);
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
                outcome: PreparationOutcome::Ready(resolved("stale", None)),
            },
            Instant::now(),
        );
        assert!(coordinator.snapshot.providers.is_empty());
        coordinator.apply_result(
            PreparationResult {
                path: current.path,
                generation: current.generation,
                attempts: 0,
                outcome: PreparationOutcome::Ready(resolved("current", None)),
            },
            Instant::now(),
        );
        assert_eq!(
            coordinator
                .snapshot
                .resolve(&path)
                .unwrap()
                .source_epoch()
                .as_str(),
            "current"
        );
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
        coordinator.set_active_tab(1);
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
        coordinator.set_active_tab(1);
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
    fn unified_provider_map_keeps_mixed_documents_region_native() {
        let png = PathBuf::from("missing-but-classified.png");
        let jpg = PathBuf::from("missing-but-backed.jpg");
        let png_document = raster_document(png.clone());
        let (mut coordinator, requests) = detached_coordinator();
        coordinator.set_tab_document(1, &png_document);
        coordinator.set_active_tab(1);
        let request = requests.try_recv().unwrap();
        coordinator.apply_result(
            PreparationResult {
                path: request.path,
                generation: request.generation,
                attempts: 0,
                outcome: PreparationOutcome::Ready(resolved("png-provider", None)),
            },
            Instant::now(),
        );
        assert_eq!(
            coordinator.snapshot.render_mode(&png_document),
            RasterRenderMode::Provider { snapshot_epoch: 1 }
        );

        let mut mixed = png_document;
        let mut jpg_document = raster_document(jpg.clone());
        mixed.layers.push(jpg_document.layers.remove(0));
        let snapshot = RasterSourceSnapshot {
            epoch: 7,
            providers: Arc::new(HashMap::from([
                (png, resolved("png-provider", None)),
                (jpg, resolved("derived-provider", None)),
            ])),
        };
        assert_eq!(
            snapshot.render_mode(&mixed),
            RasterRenderMode::Provider { snapshot_epoch: 7 }
        );
    }
}

#[cfg(test)]
#[path = "raster_sources_review_tests.rs"]
mod review_tests;
