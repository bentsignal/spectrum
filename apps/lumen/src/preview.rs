use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use image::{DynamicImage, imageops::FilterType};

use crate::{
    Adjustments, Photo,
    engine::{RenderOptions, decode_photo, render_image},
};

pub const PREVIEW_MAX_SIZE: u32 = 1800;
const FAST_PREVIEW_MAX_SIZE: u32 = 960;
const PREFETCH_QUEUE_CAPACITY: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewRequest {
    Selected { generation: u64, epoch: u64 },
    Prefetch { epoch: u64 },
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
    pub request: PreviewRequest,
    pub photo_id: u64,
    pub adjustments: Adjustments,
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

    pub fn can_publish(&self, completion: &PreviewCompletion) -> bool {
        if self.photo_id != Some(completion.photo_id) || self.adjustments != completion.adjustments
        {
            return false;
        }
        match completion.request {
            PreviewRequest::Selected { generation, epoch } => {
                generation == self.generation && epoch == self.epoch
            }
            PreviewRequest::Prefetch { epoch } => epoch == self.epoch,
        }
    }
}

struct PreviewJob {
    request: PreviewRequest,
    photo: Photo,
    adjustments: Adjustments,
}

#[derive(Default)]
struct PreviewQueue {
    shutdown: bool,
    selected: Option<PreviewJob>,
    prefetch: VecDeque<PreviewJob>,
}

pub struct PreviewWorker {
    queue: Arc<(Mutex<PreviewQueue>, Condvar)>,
    completion_receiver: Receiver<PreviewCompletion>,
    thread: Option<JoinHandle<()>>,
}

impl PreviewWorker {
    pub fn new() -> Self {
        let queue = Arc::new((Mutex::new(PreviewQueue::default()), Condvar::new()));
        let (completion_sender, completion_receiver) = mpsc::channel();
        let worker_queue = Arc::clone(&queue);
        let thread = thread::Builder::new()
            .name("lumen-preview".into())
            .spawn(move || preview_worker(worker_queue, completion_sender))
            .expect("Lumen preview worker thread should start");
        Self {
            queue,
            completion_receiver,
            thread: Some(thread),
        }
    }

    pub fn request_selected(
        &self,
        generation: u64,
        epoch: u64,
        photo: Photo,
        adjustments: Adjustments,
    ) {
        let (queue, available) = &*self.queue;
        let mut queue = queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue
            .prefetch
            .retain(|job| job.photo.id != photo.id || job.adjustments != adjustments);
        queue.selected = Some(PreviewJob {
            request: PreviewRequest::Selected { generation, epoch },
            photo,
            adjustments,
        });
        available.notify_one();
    }

    pub fn request_prefetch(&self, epoch: u64, photo: Photo, adjustments: Adjustments) {
        let (queue, available) = &*self.queue;
        let mut queue = queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if queue
            .selected
            .as_ref()
            .is_some_and(|job| job.photo.id == photo.id && job.adjustments == adjustments)
            || queue
                .prefetch
                .iter()
                .any(|job| job.photo.id == photo.id && job.adjustments == adjustments)
        {
            return;
        }
        while queue.prefetch.len() >= PREFETCH_QUEUE_CAPACITY {
            queue.prefetch.pop_back();
        }
        queue.prefetch.push_back(PreviewJob {
            request: PreviewRequest::Prefetch { epoch },
            photo,
            adjustments,
        });
        available.notify_one();
    }

    pub fn try_recv(&self) -> Result<PreviewCompletion, TryRecvError> {
        self.completion_receiver.try_recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<PreviewCompletion, RecvTimeoutError> {
        self.completion_receiver.recv_timeout(timeout)
    }
}

impl Default for PreviewWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PreviewWorker {
    fn drop(&mut self) {
        let (queue, available) = &*self.queue;
        let mut queue = queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.shutdown = true;
        queue.selected = None;
        queue.prefetch.clear();
        available.notify_one();
        drop(queue);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn prepare_preview(photo: &Photo, adjustments: Adjustments) -> anyhow::Result<PreparedPreview> {
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

fn preview_worker(
    queue: Arc<(Mutex<PreviewQueue>, Condvar)>,
    completion_sender: Sender<PreviewCompletion>,
) {
    loop {
        let job = {
            let (queue, available) = &*queue;
            let mut queue = queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while !queue.shutdown && queue.selected.is_none() && queue.prefetch.is_empty() {
                queue = available
                    .wait(queue)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            if queue.shutdown {
                return;
            }
            queue.selected.take().or_else(|| queue.prefetch.pop_front())
        };
        let Some(job) = job else {
            continue;
        };
        let photo_id = job.photo.id;
        let adjustments = job.adjustments.clone();
        let result =
            prepare_preview(&job.photo, job.adjustments).map_err(|error| format!("{error:#}"));
        if completion_sender
            .send(PreviewCompletion {
                request: job.request,
                photo_id,
                adjustments,
                result,
            })
            .is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completion(
        photo_id: u64,
        adjustments: Adjustments,
        request: PreviewRequest,
    ) -> PreviewCompletion {
        PreviewCompletion {
            request,
            photo_id,
            adjustments,
            result: Err("not rendered in selection test".into()),
        }
    }

    #[test]
    fn stale_selected_generation_cannot_publish_over_current_photo() {
        let mut selection = PreviewSelection::default();
        let adjustments = Adjustments::default();
        let stale_generation = selection.select(7, adjustments.clone());
        let current_generation = selection.select(7, adjustments.clone());

        assert!(!selection.can_publish(&completion(
            7,
            adjustments.clone(),
            PreviewRequest::Selected {
                generation: stale_generation,
                epoch: selection.epoch(),
            },
        )));
        assert!(selection.can_publish(&completion(
            7,
            adjustments,
            PreviewRequest::Selected {
                generation: current_generation,
                epoch: selection.epoch(),
            },
        )));
    }

    #[test]
    fn prefetch_only_publishes_for_the_exact_current_photo_and_adjustments() {
        let mut selection = PreviewSelection::default();
        let adjustments = Adjustments {
            exposure: 0.5,
            ..Default::default()
        };
        selection.select(8, adjustments.clone());

        assert!(selection.can_publish(&completion(
            8,
            adjustments.clone(),
            PreviewRequest::Prefetch {
                epoch: selection.epoch(),
            },
        )));
        assert!(!selection.can_publish(&completion(
            9,
            adjustments,
            PreviewRequest::Prefetch {
                epoch: selection.epoch(),
            },
        )));
    }
}
