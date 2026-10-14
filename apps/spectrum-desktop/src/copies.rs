//! Playback copies, made in the background for footage that needs one, one
//! at a time, at background priority, and paused while the player plays,
//! so they never compete with what a person is doing. The player switches
//! to a copy as soon as it is ready.
use crate::workspace::Workspace;
use gpui::*;
use spectrum_assets::Service;
use spectrum_library::{AssetId, AssetKind};
use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
pub struct Copies {
    /// Footage looked at this session, so each is checked once.
    checked: HashSet<AssetId>,
    queue: VecDeque<AssetId>,
    /// The footage whose copy is being made.
    pub making: Option<AssetId>,
    /// Its share done, as `f32` bits.
    progress: Arc<AtomicU32>,
    /// Holds the copy while the player plays.
    pub hold: Arc<AtomicBool>,
}

impl Copies {
    /// How far along footage's copy is, while it is being made.
    pub fn progress(&self, id: AssetId) -> Option<f32> {
        (self.making == Some(id)).then(|| f32::from_bits(self.progress.load(Ordering::Relaxed)))
    }
}

/// Lowers this thread's priority; threads it starts (the decoder's) inherit
/// it.
pub fn background_priority() {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
    }
    #[cfg(target_os = "macos")]
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
}

impl Workspace {
    /// Queues footage not checked yet and starts the next copy. Cheap when
    /// there is nothing new; called as the window renders.
    pub fn prepare_copies(&mut self, cx: &mut Context<Self>) {
        let Ok(store) = &mut self.store else {
            return;
        };
        let footage: Vec<AssetId> = store
            .entries
            .iter()
            .filter(|entry| entry.asset.kind == AssetKind::Footage && entry.purge_after.is_none())
            .map(|entry| entry.asset.id)
            .filter(|id| !store.copies.checked.contains(id))
            .collect();
        for id in footage {
            store.copies.checked.insert(id);
            store.copies.queue.push_back(id);
        }
        // What is open plays first: the footage in the player, then the
        // open video's.
        let mut wanted: Vec<AssetId> = self.player.iter().map(|p| p.id).collect();
        if let Some(video) = &self.video {
            wanted.extend(
                video
                    .doc
                    .tracks
                    .iter()
                    .flat_map(|t| t.clips.iter())
                    .map(|c| c.source.asset()),
            );
        }
        for id in wanted.into_iter().rev() {
            if let Some(index) = store.copies.queue.iter().position(|q| *q == id) {
                store.copies.queue.remove(index);
                store.copies.queue.push_front(id);
            }
        }
        if store.copies.making.is_some() {
            return;
        }
        let Some(id) = store.copies.queue.pop_front() else {
            return;
        };
        store.copies.making = Some(id);
        store
            .copies
            .progress
            .store(0f32.to_bits(), Ordering::Relaxed);
        let (root, progress, hold) = (
            store.root.clone(),
            store.copies.progress.clone(),
            store.copies.hold.clone(),
        );
        let (done, finished) = flume::bounded(1);
        std::thread::Builder::new()
            .name("spectrum-copy".into())
            .spawn(move || {
                background_priority();
                let made = Service::open(&root).and_then(|service| {
                    if !service.needs_playback_copy(id)? {
                        return Ok(false);
                    }
                    service.make_playback_copy(id, &mut |share| {
                        progress.store(share.to_bits(), Ordering::Relaxed);
                        while hold.load(Ordering::Relaxed) {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        true
                    })?;
                    Ok(true)
                });
                let _ = done.send(made);
            })
            .ok();
        cx.spawn(async move |this, cx| {
            // Redraw now and then so the share done shows.
            let made = loop {
                if let Ok(made) = finished.try_recv() {
                    break made;
                }
                if finished.is_disconnected() {
                    break Ok(false);
                }
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            };
            this.update(cx, |this, cx| {
                if let Ok(store) = &mut this.store {
                    store.copies.making = None;
                }
                match made {
                    Ok(true) => this.copy_ready(id, cx),
                    Ok(false) => {}
                    Err(error) => eprintln!("spectrum: playback copy: {error:#}"),
                }
                this.prepare_copies(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
