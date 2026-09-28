//! Copy one image's edits and paste them onto others, crop included.
use crate::workspace::{Open, Workspace};
use gpui::*;
use spectrum::library::Service;
use spectrum_library::AssetId;

impl Workspace {
    /// Images the edit shortcuts act on: the open image, or the selected images.
    fn edit_targets(&self) -> Vec<AssetId> {
        if let Some(id) = self.open_image() {
            return vec![id];
        }
        let Ok(store) = &self.store else {
            return Vec::new();
        };
        if !matches!(self.open, Open::Overview) {
            return Vec::new();
        }
        self.selection
            .iter()
            .copied()
            .filter(|id| store.is_image(*id))
            .collect()
    }

    /// Copies edits from `id`, or from the only target image.
    pub fn copy_edits(&mut self, id: Option<AssetId>, window: &mut Window, cx: &mut Context<Self>) {
        let id = id.or_else(|| match self.edit_targets().as_slice() {
            [one] => Some(*one),
            _ => None,
        });
        let (Some(id), Ok(store)) = (id, &self.store) else {
            return;
        };
        match store.service.image(id) {
            Ok(photo) => self.copied_edits = Some(photo.adjustments),
            Err(error) => self.notify_error(error, window, cx),
        }
        cx.notify();
    }

    /// Pastes copied edits onto `ids`, or onto the target images.
    pub fn paste_edits(
        &mut self,
        ids: Option<Vec<AssetId>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = ids.unwrap_or_else(|| self.edit_targets());
        let (Some(adjustments), Ok(store)) = (self.copied_edits.clone(), &self.store) else {
            return;
        };
        if ids.is_empty() {
            return;
        }
        let root = store.root.clone();
        let targets = ids.clone();
        let task = cx.background_executor().spawn(async move {
            let service = Service::open(&root)?;
            targets
                .iter()
                .try_for_each(|id| service.set_adjustments(*id, adjustments.clone()))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.notify_error(error, window, cx);
                }
                if let Ok(store) = &mut this.store {
                    for id in &ids {
                        store.thumbs.remove(id);
                    }
                }
                if let Some(open) = this.open_image().filter(|open| ids.contains(open)) {
                    this.refresh_open_image(open, window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
