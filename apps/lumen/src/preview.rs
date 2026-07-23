use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use image::{DynamicImage, imageops::FilterType};

use crate::{
    Adjustments, Photo,
    engine::{RenderOptions, decode_photo, render_image},
};

pub const PREVIEW_MAX_SIZE: u32 = 1800;
const FAST_PREVIEW_MAX_SIZE: u32 = 960;
const PREFETCH_QUEUE_CAPACITY: usize = 4;
const SELECTED_QUEUE_CAPACITY: usize = 1;
const PREVIEW_CACHE_CAPACITY: usize = 4;
const FAILURE_CACHE_CAPACITY: usize = 8;
const MAX_OUTSTANDING_REQUESTS: usize = PREFETCH_QUEUE_CAPACITY + SELECTED_QUEUE_CAPACITY + 2;
const FAILURE_RETRY_BASE: Duration = Duration::from_millis(250);
const FAILURE_RETRY_MAX: Duration = Duration::from_secs(30);

type PreviewPreparer =
    dyn Fn(&Photo, Adjustments) -> Result<PreparedPreview, String> + Send + Sync + 'static;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewRequestKind {
    Selected { generation: u64 },
    Prefetch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewRequestIdentity {
    pub id: u64,
    pub epoch: u64,
    pub photo_id: u64,
    pub adjustments: Adjustments,
    pub kind: PreviewRequestKind,
}

impl PreviewRequestIdentity {
    fn matches(&self, epoch: u64, photo_id: u64, adjustments: &Adjustments) -> bool {
        self.epoch == epoch && self.photo_id == photo_id && self.adjustments == *adjustments
    }
}

#[derive(Clone, Debug, Default)]
pub struct PreviewEnqueue {
    pub accepted: Option<PreviewRequestIdentity>,
    pub evicted: Vec<PreviewRequestIdentity>,
}

#[derive(Clone)]
pub struct PreparedPreview {
    pub photo_id: u64,
    pub adjustments: Adjustments,
    pub source: DynamicImage,
    pub fast_source: DynamicImage,
    pub rendered: DynamicImage,
    pub histogram: PreviewHistogram,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewHistogram {
    pub red: [u32; 256],
    pub green: [u32; 256],
    pub blue: [u32; 256],
    pub luma: [u32; 256],
}

impl PreviewHistogram {
    pub fn from_image(image: &DynamicImage) -> Self {
        let mut histogram = Self {
            red: [0; 256],
            green: [0; 256],
            blue: [0; 256],
            luma: [0; 256],
        };
        let rgba = image.to_rgba8();
        for pixel in rgba.pixels().step_by(2) {
            histogram.red[pixel[0] as usize] += 1;
            histogram.green[pixel[1] as usize] += 1;
            histogram.blue[pixel[2] as usize] += 1;
            let luma =
                (pixel[0] as f32 * 0.2126 + pixel[1] as f32 * 0.7152 + pixel[2] as f32 * 0.0722)
                    .round() as usize;
            histogram.luma[luma.min(255)] += 1;
        }
        histogram
    }
}

pub struct PreviewCompletion {
    pub identity: PreviewRequestIdentity,
    pub result: Result<PreparedPreview, String>,
}

#[derive(Clone, Debug, Default)]
pub struct PreviewSelection {
    generation: u64,
    epoch: u64,
    photo_id: Option<u64>,
    adjustments: Adjustments,
}

impl PreviewSelection {
    pub fn select(&mut self, photo_id: u64, adjustments: Adjustments) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.photo_id = Some(photo_id);
        self.adjustments = adjustments;
        self.generation
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.photo_id = None;
        self.adjustments = Adjustments::default();
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn reset_catalog(&mut self) {
        self.clear();
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub fn can_publish(&self, identity: &PreviewRequestIdentity) -> bool {
        if self.epoch != identity.epoch
            || self.photo_id != Some(identity.photo_id)
            || self.adjustments != identity.adjustments
        {
            return false;
        }
        match identity.kind {
            PreviewRequestKind::Selected { generation } => generation == self.generation,
            PreviewRequestKind::Prefetch => true,
        }
    }
}

struct PreviewJob {
    identity: PreviewRequestIdentity,
    photo: Photo,
}

struct PreviewLane {
    shutdown: bool,
    capacity: usize,
    active: Option<PreviewRequestIdentity>,
    pending: VecDeque<PreviewJob>,
}

impl PreviewLane {
    fn new(capacity: usize) -> Self {
        Self {
            shutdown: false,
            capacity,
            active: None,
            pending: VecDeque::new(),
        }
    }

    fn enqueue_latest(&mut self, job: PreviewJob) -> PreviewEnqueue {
        if self.shutdown {
            return PreviewEnqueue::default();
        }
        let mut evicted = Vec::new();
        while self.pending.len() >= self.capacity {
            if let Some(evicted_job) = self.pending.pop_front() {
                evicted.push(evicted_job.identity);
            }
        }
        let accepted = job.identity.clone();
        self.pending.push_back(job);
        PreviewEnqueue {
            accepted: Some(accepted),
            evicted,
        }
    }

    fn enqueue_prefetch(&mut self, job: PreviewJob) -> PreviewEnqueue {
        let identity = &job.identity;
        if self.shutdown
            || self.active.as_ref().is_some_and(|active| {
                active.matches(identity.epoch, identity.photo_id, &identity.adjustments)
            })
            || self.pending.iter().any(|pending| {
                pending
                    .identity
                    .matches(identity.epoch, identity.photo_id, &identity.adjustments)
            })
        {
            return PreviewEnqueue::default();
        }
        self.enqueue_latest(job)
    }

    fn purge_other_epochs(&mut self, epoch: u64) -> Vec<PreviewRequestIdentity> {
        let mut purged = Vec::new();
        self.pending.retain(|job| {
            if job.identity.epoch == epoch {
                true
            } else {
                purged.push(job.identity.clone());
                false
            }
        });
        purged
    }
}

pub struct PreviewWorker {
    selected_lane: Arc<(Mutex<PreviewLane>, Condvar)>,
    prefetch_lane: Arc<(Mutex<PreviewLane>, Condvar)>,
    completion_receiver: Receiver<PreviewCompletion>,
    next_request_id: AtomicU64,
}

impl PreviewWorker {
    pub fn new() -> Self {
        Self::with_preparer(Arc::new(|photo, adjustments| {
            prepare_preview(photo, adjustments).map_err(|error| format!("{error:#}"))
        }))
    }

    fn with_preparer(preparer: Arc<PreviewPreparer>) -> Self {
        let selected_lane = Arc::new((
            Mutex::new(PreviewLane::new(SELECTED_QUEUE_CAPACITY)),
            Condvar::new(),
        ));
        let prefetch_lane = Arc::new((
            Mutex::new(PreviewLane::new(PREFETCH_QUEUE_CAPACITY)),
            Condvar::new(),
        ));
        let (completion_sender, completion_receiver) = mpsc::sync_channel(MAX_OUTSTANDING_REQUESTS);
        spawn_preview_lane(
            "lumen-preview-selected",
            Arc::clone(&selected_lane),
            completion_sender.clone(),
            Arc::clone(&preparer),
        );
        spawn_preview_lane(
            "lumen-preview-prefetch",
            Arc::clone(&prefetch_lane),
            completion_sender,
            preparer,
        );
        Self {
            selected_lane,
            prefetch_lane,
            completion_receiver,
            next_request_id: AtomicU64::new(1),
        }
    }

    pub fn request_selected(
        &self,
        generation: u64,
        epoch: u64,
        photo: Photo,
        adjustments: Adjustments,
    ) -> PreviewEnqueue {
        let identity = self.identity(
            epoch,
            photo.id,
            adjustments,
            PreviewRequestKind::Selected { generation },
        );
        self.enqueue(&self.selected_lane, identity, photo, false)
    }

    pub fn request_prefetch(
        &self,
        epoch: u64,
        photo: Photo,
        adjustments: Adjustments,
    ) -> PreviewEnqueue {
        let identity = self.identity(epoch, photo.id, adjustments, PreviewRequestKind::Prefetch);
        self.enqueue(&self.prefetch_lane, identity, photo, true)
    }

    pub fn purge_other_epochs(&self, epoch: u64) -> Vec<PreviewRequestIdentity> {
        let mut purged = purge_lane(&self.selected_lane, epoch);
        purged.extend(purge_lane(&self.prefetch_lane, epoch));
        purged
    }

    pub fn try_recv(&self) -> Result<PreviewCompletion, TryRecvError> {
        self.completion_receiver.try_recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<PreviewCompletion, RecvTimeoutError> {
        self.completion_receiver.recv_timeout(timeout)
    }

    pub fn is_active(&self, request_id: u64) -> bool {
        lane_is_active(&self.selected_lane, request_id)
            || lane_is_active(&self.prefetch_lane, request_id)
    }

    fn identity(
        &self,
        epoch: u64,
        photo_id: u64,
        adjustments: Adjustments,
        kind: PreviewRequestKind,
    ) -> PreviewRequestIdentity {
        PreviewRequestIdentity {
            id: self.next_request_id.fetch_add(1, Ordering::Relaxed),
            epoch,
            photo_id,
            adjustments,
            kind,
        }
    }

    fn enqueue(
        &self,
        lane: &Arc<(Mutex<PreviewLane>, Condvar)>,
        identity: PreviewRequestIdentity,
        photo: Photo,
        prefetch: bool,
    ) -> PreviewEnqueue {
        let (lane, available) = &**lane;
        let mut lane = lane.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let job = PreviewJob { identity, photo };
        let outcome = if prefetch {
            lane.enqueue_prefetch(job)
        } else {
            lane.enqueue_latest(job)
        };
        if outcome.accepted.is_some() {
            available.notify_one();
        }
        outcome
    }
}

impl Default for PreviewWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PreviewWorker {
    fn drop(&mut self) {
        shutdown_lane(&self.selected_lane);
        shutdown_lane(&self.prefetch_lane);
        // Worker threads own only immutable job data and their lane. Detaching
        // lets the app close immediately even if an authoritative RAW decode is
        // inside a non-cancelable decoder; its eventual send observes the
        // dropped receiver and exits.
    }
}

#[derive(Clone)]
struct PreviewFailure {
    epoch: u64,
    photo_id: u64,
    adjustments: Adjustments,
    attempts: u32,
    retry_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewRequestDecision {
    Request,
    Pending,
    Backoff(Duration),
}

pub enum PreviewCompletionDisposition {
    Publish(Box<PreparedPreview>),
    Cached,
    Failed(String),
    Ignored,
}

#[derive(Default)]
pub struct PreviewPipeline {
    selection: PreviewSelection,
    outstanding: Vec<PreviewRequestIdentity>,
    cache: VecDeque<(u64, PreparedPreview)>,
    failures: VecDeque<PreviewFailure>,
}

impl PreviewPipeline {
    pub fn select(&mut self, photo_id: u64, adjustments: Adjustments) -> u64 {
        self.selection.select(photo_id, adjustments)
    }

    pub fn clear(&mut self) {
        self.selection.clear();
    }

    pub fn generation(&self) -> u64 {
        self.selection.generation()
    }

    pub fn epoch(&self) -> u64 {
        self.selection.epoch()
    }

    pub fn reset_catalog(&mut self, worker: &PreviewWorker) {
        self.selection.reset_catalog();
        worker.purge_other_epochs(self.selection.epoch());
        self.outstanding.clear();
        self.cache.clear();
        self.failures.clear();
    }

    pub fn track_enqueue(&mut self, enqueue: PreviewEnqueue) {
        for evicted in enqueue.evicted {
            self.outstanding.retain(|pending| pending.id != evicted.id);
        }
        if let Some(accepted) = enqueue.accepted {
            self.outstanding.retain(|pending| pending.id != accepted.id);
            self.outstanding.push(accepted);
        }
        debug_assert!(self.outstanding.len() <= MAX_OUTSTANDING_REQUESTS);
    }

    pub fn request_decision(
        &self,
        now: Instant,
        photo_id: u64,
        adjustments: &Adjustments,
    ) -> PreviewRequestDecision {
        let epoch = self.epoch();
        if self
            .outstanding
            .iter()
            .any(|request| request.matches(epoch, photo_id, adjustments))
        {
            return PreviewRequestDecision::Pending;
        }
        if let Some(failure) = self.failures.iter().find(|failure| {
            failure.epoch == epoch
                && failure.photo_id == photo_id
                && failure.adjustments == *adjustments
        }) && now < failure.retry_at
        {
            return PreviewRequestDecision::Backoff(
                failure.retry_at.saturating_duration_since(now),
            );
        }
        PreviewRequestDecision::Request
    }

    pub fn has_cached_or_outstanding(&self, photo_id: u64, adjustments: &Adjustments) -> bool {
        let epoch = self.epoch();
        self.outstanding
            .iter()
            .any(|request| request.matches(epoch, photo_id, adjustments))
            || self.cache.iter().any(|(cached_epoch, preview)| {
                *cached_epoch == epoch
                    && preview.photo_id == photo_id
                    && preview.adjustments == *adjustments
            })
    }

    pub fn has_outstanding_work(&self) -> bool {
        !self.outstanding.is_empty()
    }

    pub fn take_cached(
        &mut self,
        photo_id: u64,
        adjustments: &Adjustments,
    ) -> Option<PreparedPreview> {
        let epoch = self.epoch();
        let index = self.cache.iter().position(|(cached_epoch, preview)| {
            *cached_epoch == epoch
                && preview.photo_id == photo_id
                && preview.adjustments == *adjustments
        })?;
        self.cache.remove(index).map(|(_, preview)| preview)
    }

    pub fn complete(
        &mut self,
        completion: PreviewCompletion,
        now: Instant,
    ) -> PreviewCompletionDisposition {
        self.outstanding
            .retain(|pending| pending.id != completion.identity.id);
        let can_publish = self.selection.can_publish(&completion.identity);
        match completion.result {
            Ok(preview) if can_publish => {
                self.clear_failure(&completion.identity);
                PreviewCompletionDisposition::Publish(Box::new(preview))
            }
            Ok(preview) if completion.identity.epoch == self.epoch() => {
                self.clear_failure(&completion.identity);
                self.cache_preview(completion.identity.epoch, preview);
                PreviewCompletionDisposition::Cached
            }
            Ok(_) => PreviewCompletionDisposition::Ignored,
            Err(error) if can_publish => {
                self.record_failure(&completion.identity, now);
                PreviewCompletionDisposition::Failed(error)
            }
            Err(_) => PreviewCompletionDisposition::Ignored,
        }
    }

    fn cache_preview(&mut self, epoch: u64, preview: PreparedPreview) {
        self.cache.retain(|(cached_epoch, cached)| {
            *cached_epoch != epoch
                || cached.photo_id != preview.photo_id
                || cached.adjustments != preview.adjustments
        });
        self.cache.push_front((epoch, preview));
        self.cache.truncate(PREVIEW_CACHE_CAPACITY);
    }

    fn clear_failure(&mut self, identity: &PreviewRequestIdentity) {
        self.failures.retain(|failure| {
            failure.epoch != identity.epoch
                || failure.photo_id != identity.photo_id
                || failure.adjustments != identity.adjustments
        });
    }

    fn record_failure(&mut self, identity: &PreviewRequestIdentity, now: Instant) {
        let prior_attempts = self
            .failures
            .iter()
            .find(|failure| {
                failure.epoch == identity.epoch
                    && failure.photo_id == identity.photo_id
                    && failure.adjustments == identity.adjustments
            })
            .map_or(0, |failure| failure.attempts);
        self.clear_failure(identity);
        let attempts = prior_attempts.saturating_add(1);
        let multiplier = 1_u32 << attempts.saturating_sub(1).min(7);
        let delay = (FAILURE_RETRY_BASE * multiplier).min(FAILURE_RETRY_MAX);
        self.failures.push_back(PreviewFailure {
            epoch: identity.epoch,
            photo_id: identity.photo_id,
            adjustments: identity.adjustments.clone(),
            attempts,
            retry_at: now + delay,
        });
        while self.failures.len() > FAILURE_CACHE_CAPACITY {
            self.failures.pop_front();
        }
    }
}

pub fn prepare_preview(photo: &Photo, adjustments: Adjustments) -> anyhow::Result<PreparedPreview> {
    // This must remain the authoritative decode path. When #70 is rebased,
    // thumbnails retain decode_photo_proxy while settled selected previews
    // continue through decode_photo for export parity.
    let source = decode_photo(photo, Some(PREVIEW_MAX_SIZE))?;
    let fast_source =
        if source.width() > FAST_PREVIEW_MAX_SIZE || source.height() > FAST_PREVIEW_MAX_SIZE {
            source.resize(
                FAST_PREVIEW_MAX_SIZE,
                FAST_PREVIEW_MAX_SIZE,
                FilterType::Triangle,
            )
        } else {
            source.clone()
        };
    let rendered = render_image(
        source.clone(),
        adjustments.clone(),
        RenderOptions::default(),
    );
    let histogram = PreviewHistogram::from_image(&rendered);
    Ok(PreparedPreview {
        photo_id: photo.id,
        adjustments,
        source,
        fast_source,
        rendered,
        histogram,
    })
}

fn spawn_preview_lane(
    name: &str,
    lane: Arc<(Mutex<PreviewLane>, Condvar)>,
    completion_sender: SyncSender<PreviewCompletion>,
    preparer: Arc<PreviewPreparer>,
) {
    thread::Builder::new()
        .name(name.into())
        .spawn(move || preview_lane(lane, completion_sender, preparer))
        .expect("Lumen preview worker thread should start");
}

fn preview_lane(
    lane: Arc<(Mutex<PreviewLane>, Condvar)>,
    completion_sender: SyncSender<PreviewCompletion>,
    preparer: Arc<PreviewPreparer>,
) {
    loop {
        let job = {
            let (lane, available) = &*lane;
            let mut lane = lane.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            while !lane.shutdown && lane.pending.is_empty() {
                lane = available
                    .wait(lane)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            if lane.shutdown {
                return;
            }
            let job = lane.pending.pop_back();
            lane.active = job.as_ref().map(|job| job.identity.clone());
            job
        };
        let Some(job) = job else {
            continue;
        };
        let result = preparer(&job.photo, job.identity.adjustments.clone());
        let shutdown = {
            let (lane, _) = &*lane;
            let mut lane = lane.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            lane.active = None;
            lane.shutdown
        };
        if shutdown
            || completion_sender
                .send(PreviewCompletion {
                    identity: job.identity,
                    result,
                })
                .is_err()
        {
            return;
        }
    }
}

fn purge_lane(
    lane: &Arc<(Mutex<PreviewLane>, Condvar)>,
    epoch: u64,
) -> Vec<PreviewRequestIdentity> {
    let (lane, _) = &**lane;
    lane.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .purge_other_epochs(epoch)
}

fn shutdown_lane(lane: &Arc<(Mutex<PreviewLane>, Condvar)>) {
    let (lane, available) = &**lane;
    let mut lane = lane.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    lane.shutdown = true;
    lane.pending.clear();
    available.notify_one();
}

fn lane_is_active(lane: &Arc<(Mutex<PreviewLane>, Condvar)>, request_id: u64) -> bool {
    let (lane, _) = &**lane;
    lane.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .active
        .as_ref()
        .is_some_and(|active| active.id == request_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(id: u64, epoch: u64, photo_id: u64) -> PreviewRequestIdentity {
        PreviewRequestIdentity {
            id,
            epoch,
            photo_id,
            adjustments: Adjustments::default(),
            kind: PreviewRequestKind::Prefetch,
        }
    }

    fn photo(id: u64) -> Photo {
        Photo::new(id, format!("{id}.jpg").into(), format!("{id}.jpg"), 1, 1)
    }

    fn prepared(id: u64, adjustments: Adjustments) -> PreparedPreview {
        let rendered = DynamicImage::new_rgba8(2, 2);
        PreparedPreview {
            photo_id: id,
            adjustments,
            source: rendered.clone(),
            fast_source: rendered.clone(),
            histogram: PreviewHistogram::from_image(&rendered),
            rendered,
        }
    }

    #[test]
    fn prefetch_dedup_includes_epoch_and_evicts_the_oldest_request() {
        let mut lane = PreviewLane::new(2);
        lane.active = Some(identity(1, 4, 8));
        let duplicate = PreviewJob {
            identity: identity(2, 4, 8),
            photo: photo(8),
        };
        assert!(lane.enqueue_prefetch(duplicate).accepted.is_none());

        for (request_id, epoch, photo_id) in [(3, 5, 8), (4, 5, 9)] {
            let outcome = lane.enqueue_prefetch(PreviewJob {
                identity: identity(request_id, epoch, photo_id),
                photo: photo(photo_id),
            });
            assert!(outcome.accepted.is_some());
        }
        let outcome = lane.enqueue_prefetch(PreviewJob {
            identity: identity(5, 5, 10),
            photo: photo(10),
        });
        assert_eq!(
            outcome
                .evicted
                .iter()
                .map(|request| request.id)
                .collect::<Vec<_>>(),
            vec![3]
        );
        assert_eq!(
            lane.pending
                .iter()
                .map(|job| job.identity.id)
                .collect::<Vec<_>>(),
            vec![4, 5]
        );
    }

    #[test]
    fn catalog_purge_removes_only_stale_queued_work() {
        let mut lane = PreviewLane::new(4);
        for (request_id, epoch) in [(1, 2), (2, 3), (3, 2)] {
            lane.enqueue_latest(PreviewJob {
                identity: identity(request_id, epoch, request_id),
                photo: photo(request_id),
            });
        }
        let purged = lane.purge_other_epochs(3);
        assert_eq!(
            purged.iter().map(|request| request.id).collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert_eq!(lane.pending.front().unwrap().identity.id, 2);
    }

    #[test]
    fn stale_selected_generation_cannot_publish_over_current_photo() {
        let mut selection = PreviewSelection::default();
        let adjustments = Adjustments::default();
        let stale_generation = selection.select(7, adjustments.clone());
        let current_generation = selection.select(7, adjustments.clone());
        let mut stale = identity(1, selection.epoch(), 7);
        stale.kind = PreviewRequestKind::Selected {
            generation: stale_generation,
        };
        let mut current = stale.clone();
        current.id = 2;
        current.kind = PreviewRequestKind::Selected {
            generation: current_generation,
        };
        assert!(!selection.can_publish(&stale));
        assert!(selection.can_publish(&current));
    }

    #[test]
    fn pipeline_tracks_acceptance_eviction_completion_and_publication() {
        let mut pipeline = PreviewPipeline::default();
        let adjustments = Adjustments::default();
        let generation = pipeline.select(7, adjustments.clone());
        let first = identity(1, pipeline.epoch(), 6);
        let mut selected = identity(2, pipeline.epoch(), 7);
        selected.kind = PreviewRequestKind::Selected { generation };
        pipeline.track_enqueue(PreviewEnqueue {
            accepted: Some(first.clone()),
            evicted: vec![],
        });
        pipeline.track_enqueue(PreviewEnqueue {
            accepted: Some(selected.clone()),
            evicted: vec![first.clone()],
        });
        assert_eq!(pipeline.outstanding.len(), 1);
        assert!(matches!(
            pipeline.complete(
                PreviewCompletion {
                    identity: selected,
                    result: Ok(prepared(7, adjustments)),
                },
                Instant::now(),
            ),
            PreviewCompletionDisposition::Publish(_)
        ));
        assert!(pipeline.outstanding.is_empty());

        pipeline.select(8, Adjustments::default());
        assert!(matches!(
            pipeline.complete(
                PreviewCompletion {
                    identity: identity(3, pipeline.epoch(), 8),
                    result: Ok(prepared(8, Adjustments::default())),
                },
                Instant::now(),
            ),
            PreviewCompletionDisposition::Publish(_)
        ));
    }

    #[test]
    fn catalog_reset_purges_queued_worker_jobs_and_ignores_active_old_epoch() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (started_sender, started_receiver) = mpsc::channel();
        let preparer_gate = Arc::clone(&gate);
        let preparer = Arc::new(move |photo: &Photo, adjustments: Adjustments| {
            if photo.id == 1 {
                started_sender.send(()).unwrap();
                let (gate, released) = &*preparer_gate;
                let mut release = gate.lock().unwrap();
                while !*release {
                    release = released.wait(release).unwrap();
                }
            }
            Ok(prepared(photo.id, adjustments))
        });
        let worker = PreviewWorker::with_preparer(preparer);
        let mut pipeline = PreviewPipeline::default();
        pipeline.track_enqueue(worker.request_prefetch(
            pipeline.epoch(),
            photo(1),
            Adjustments::default(),
        ));
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        pipeline.track_enqueue(worker.request_prefetch(
            pipeline.epoch(),
            photo(2),
            Adjustments::default(),
        ));
        pipeline.reset_catalog(&worker);
        assert!(pipeline.outstanding.is_empty());

        let (gate, released) = &*gate;
        *gate.lock().unwrap() = true;
        released.notify_one();
        let completion = worker.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            pipeline.complete(completion, Instant::now()),
            PreviewCompletionDisposition::Ignored
        ));
        assert!(matches!(
            worker.recv_timeout(Duration::from_millis(50)),
            Err(RecvTimeoutError::Timeout)
        ));
    }

    #[test]
    fn pipeline_bounds_prepared_and_failure_state() {
        let now = Instant::now();
        let mut pipeline = PreviewPipeline::default();
        for id in 1..=10 {
            pipeline.select(99, Adjustments::default());
            let request = identity(id, pipeline.epoch(), id);
            assert!(matches!(
                pipeline.complete(
                    PreviewCompletion {
                        identity: request,
                        result: Ok(prepared(id, Adjustments::default())),
                    },
                    now,
                ),
                PreviewCompletionDisposition::Cached
            ));
        }
        assert_eq!(pipeline.cache.len(), PREVIEW_CACHE_CAPACITY);

        for id in 1..=10 {
            let generation = pipeline.select(id, Adjustments::default());
            let mut request = identity(100 + id, pipeline.epoch(), id);
            request.kind = PreviewRequestKind::Selected { generation };
            assert!(matches!(
                pipeline.complete(
                    PreviewCompletion {
                        identity: request,
                        result: Err("missing source".into()),
                    },
                    now,
                ),
                PreviewCompletionDisposition::Failed(_)
            ));
        }
        assert_eq!(pipeline.failures.len(), FAILURE_CACHE_CAPACITY);
    }

    #[test]
    fn permanent_errors_back_off_exponentially_without_eight_ms_resubmission() {
        let now = Instant::now();
        let mut pipeline = PreviewPipeline::default();
        let adjustments = Adjustments::default();
        let generation = pipeline.select(7, adjustments.clone());
        let mut failed = identity(1, pipeline.epoch(), 7);
        failed.kind = PreviewRequestKind::Selected { generation };
        pipeline.track_enqueue(PreviewEnqueue {
            accepted: Some(failed.clone()),
            evicted: vec![],
        });
        assert!(matches!(
            pipeline.complete(
                PreviewCompletion {
                    identity: failed,
                    result: Err("corrupt JPEG".into()),
                },
                now,
            ),
            PreviewCompletionDisposition::Failed(_)
        ));
        assert_eq!(
            pipeline.request_decision(now + Duration::from_millis(8), 7, &adjustments),
            PreviewRequestDecision::Backoff(Duration::from_millis(242))
        );
        assert_eq!(
            pipeline.request_decision(now + FAILURE_RETRY_BASE, 7, &adjustments),
            PreviewRequestDecision::Request
        );
    }

    #[test]
    fn selected_lane_bypasses_blocked_prefetch_and_drop_does_not_join() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (started_sender, started_receiver) = mpsc::channel();
        let preparer_gate = Arc::clone(&gate);
        let preparer = Arc::new(move |photo: &Photo, adjustments: Adjustments| {
            if photo.id == 1 {
                started_sender.send(()).unwrap();
                let (gate, released) = &*preparer_gate;
                let mut release = gate.lock().unwrap();
                while !*release {
                    release = released.wait(release).unwrap();
                }
            }
            Ok(prepared(photo.id, adjustments))
        });
        let worker = PreviewWorker::with_preparer(preparer);
        assert!(
            worker
                .request_prefetch(0, photo(1), Adjustments::default())
                .accepted
                .is_some()
        );
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        let selected = worker
            .request_selected(1, 0, photo(2), Adjustments::default())
            .accepted
            .unwrap();
        let completion = worker.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(completion.identity.id, selected.id);

        let drop_started = Instant::now();
        drop(worker);
        assert!(drop_started.elapsed() < Duration::from_millis(50));
        let (gate, released) = &*gate;
        *gate.lock().unwrap() = true;
        released.notify_one();
    }
}
