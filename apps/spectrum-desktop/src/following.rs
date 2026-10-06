//! Following an agent you work together with: while the open image or
//! canvas has nothing waiting to save, Spectrum checks whether you have
//! moved on to the agent's newer revisions and shows them. Your own edit
//! ends the following; the agent's later work stays on its own branch.
use crate::workspace::Workspace;
use gpui::*;
use spectrum_assets::Service;
use std::time::Duration;

const EVERY: Duration = Duration::from_millis(1500);

impl Workspace {
    pub fn start_following(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(EVERY).await;
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
        self.following = true;
        let root = store.root.clone();
        let task = cx
            .background_executor()
            .spawn(async move { Service::open(&root)?.follow(id) });
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
