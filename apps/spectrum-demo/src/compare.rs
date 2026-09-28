//! Before and after: the image with no adjustments beside the edited one.
use crate::{color::LARGE, store::Thumb, theme::*, workspace::Workspace};
use gpui::{prelude::*, *};
use spectrum::library::Service;
use spectrum_library::AssetId;

/// Renders the unedited image once and keeps it with the library's previews.
fn render_original(root: &std::path::Path, id: AssetId) -> anyhow::Result<std::path::PathBuf> {
    let service = Service::open(root)?;
    let path = service
        .library
        .root()
        .join("previews")
        .join(format!("{id}-original-{LARGE}.png"));
    if !path.exists() {
        let photo = service.image(id)?;
        let image = lumen_core::engine::render_photo_with_adjustments(
            &photo,
            Default::default(),
            lumen_core::engine::RenderOptions {
                max_size: Some(LARGE),
            },
        )?;
        let temporary = path.with_file_name(format!("{}.png", AssetId::new_v4()));
        image.save(&temporary)?;
        std::fs::rename(&temporary, &path)?;
    }
    Ok(path)
}

impl Workspace {
    pub fn request_original(&mut self, id: AssetId, cx: &mut Context<Self>) {
        let Ok(store) = &mut self.store else {
            return;
        };
        if store.originals.contains_key(&id) {
            return;
        }
        store.originals.insert(id, Thumb::Loading);
        let root = store.root.clone();
        let render = cx
            .background_executor()
            .spawn(async move { render_original(&root, id) });
        cx.spawn(async move |this, cx| {
            let thumb = match render.await {
                Ok(path) => Thumb::ready(path),
                Err(_) => Thumb::Failed,
            };
            this.update(cx, |this, cx| {
                if let Ok(store) = &mut this.store {
                    store.originals.insert(id, thumb);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// One labeled half of the comparison.
    pub fn compare_half(thumb: Option<&Thumb>, label: &'static str) -> Div {
        let image = match thumb {
            Some(Thumb::Ready(path, _)) => img(path.clone())
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            _ => div()
                .text_sm()
                .text_color(rgb(FAINT))
                .child("Rendering…")
                .into_any_element(),
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(image),
            )
            .child(div().text_xs().text_color(rgb(FAINT)).child(label))
    }
}
