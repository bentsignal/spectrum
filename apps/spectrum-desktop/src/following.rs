//! Following an agent you work together with: while the open image or
//! canvas has nothing waiting to save, Spectrum checks whether you have
//! moved on to the agent's newer revisions and shows them. Your own edit
//! ends the following; the agent's later work stays on its own branch.
use crate::workspace::{FileStamp, Workspace};
use gpui::*;
use spectrum_assets::Service;
use spectrum_library::AssetId;
use std::time::Duration;

const EVERY: Duration = Duration::from_millis(1500);
/// Checks come this often while watching a session, so it shows live.
const WATCHING: Duration = Duration::from_millis(150);

/// The stamp of an asset's document file, which changes whenever anyone
/// saves the asset.
pub fn document_stamp(service: &Service, id: AssetId) -> Option<FileStamp> {
    let asset = service.library.get(id).ok()?;
    let metadata = std::fs::metadata(service.library.path(&asset).ok()?).ok()?;
    Some((metadata.modified().ok(), metadata.len()))
}

impl Workspace {
    /// Notes the document file this app just saved, so its own saves never
    /// look like an agent's work.
    pub fn saved_stamp(&mut self, id: AssetId, stamp: Option<FileStamp>) {
        if stamp.is_some() {
            self.followed_stamp = Some((id, stamp));
        }
    }

    pub fn start_following(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let watching = this
                    .update(cx, |this, _| this.history.watching.is_some())
                    .unwrap_or(false);
                cx.background_executor()
                    .timer(if watching { WATCHING } else { EVERY })
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.check_following(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn check_following(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(store) = &self.store else {
            return;
        };
        if self.following {
            return;
        }
        let target = if let Some((id, shown)) = self.canvas_settled() {
            (id, shown, true)
        } else if let Some(id) = self.open_image()
            && self.image.edits.idle()
            && !self.image.color_busy
        {
            (id, self.image.edits.revisions.get(&id).copied(), false)
        } else {
            return;
        };
        let (id, shown, canvas) = target;
        // Only a document whose file changed can hold an agent's new work;
        // checking costs one file stat until then.
        let stamp = document_stamp(&store.service, id);
        let root = store.root.clone();
        if stamp.is_some() && self.followed_stamp == Some((id, stamp)) {
            return;
        }
        self.followed_stamp = Some((id, stamp));
        // Someone saved: the history view shows it, and watching follows it.
        if self.mode == crate::workspace::Mode::History {
            self.load_history(window, cx);
        }
        if self.history.watching.is_some() {
            return self.check_watching(window, cx);
        }
        self.following = true;
        let task = cx.background_executor().spawn(async move {
            let started = std::time::Instant::now();
            let result = Service::open(&root)?.follow(id);
            crate::perf::record("follow_check", started.elapsed());
            result
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.following = false;
                let Ok(revision) = result else {
                    return;
                };
                if shown == Some(revision) {
                    return;
                }
                if canvas {
                    if this.canvas_settled().is_some_and(|(open, _)| open == id) {
                        this.show_saved_canvas(window, cx);
                    }
                } else if this.open_image() == Some(id) && this.image.edits.idle() {
                    this.refresh_open_image(id, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }
}
