use crate::{
    theme::*,
    workspace::{LibraryView, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable, WindowExt, button::ButtonVariant, dialog::DialogButtonProps,
};
use spectrum::library::Service;
use spectrum_library::AssetId;

impl Workspace {
    /// Ask before moving an asset to the trash, listing the projects it is in.
    pub fn confirm_delete(&mut self, id: AssetId, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(store) = &self.store else {
            return;
        };
        let (name, usage) = match store
            .service
            .library
            .get(id)
            .and_then(|asset| Ok((asset.name, store.service.usage(id)?)))
        {
            Ok(found) => found,
            Err(error) => return self.notify_error(error, window, cx),
        };
        let root = store.root.clone();
        let projects: Vec<_> = usage
            .projects
            .iter()
            .map(|p| (p.id, SharedString::from(p.name.clone())))
            .collect();
        let dependents: Vec<String> = usage.dependents.iter().map(|a| a.name.clone()).collect();
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let rows = projects.iter().map(|(project, name)| {
                let view = view.clone();
                let project = *project;
                div()
                    .id(SharedString::from(format!("delete-project-{project}")))
                    .h(px(34.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .rounded_md()
                    .text_sm()
                    .hover(|el| el.bg(rgb(HOVER)))
                    .child(
                        Icon::new(IconName::FolderClosed)
                            .small()
                            .text_color(rgb(MUTED)),
                    )
                    .child(div().flex_1().truncate().child(name.clone()))
                    .child(
                        Icon::new(IconName::ChevronRight)
                            .xsmall()
                            .text_color(rgb(FAINT)),
                    )
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        view.update(cx, |this, cx| {
                            this.show(LibraryView::Project(project), window, cx)
                        })
                    })
            });
            let count = projects.len();
            let used = match dependents.as_slice() {
                [] => None,
                [one] => Some(format!(
                    "{one} uses it and will show a placeholder until you restore it."
                )),
                many => Some(format!(
                    "{} canvases use it and will show a placeholder until you restore it.",
                    many.len()
                )),
            };
            let view = view.clone();
            let root = root.clone();
            dialog
                .title(format!("Delete \"{name}\"?"))
                .w(px(420.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .text_sm()
                        .child(div().text_color(rgb(MUTED)).child(
                            "It moves to the trash. Spectrum deletes it permanently after 30 days.",
                        ))
                        .when(count > 0, |el| {
                            el.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                                        "Removed from {count} project{}",
                                        if count == 1 { "" } else { "s" }
                                    )))
                                    .child(
                                        div()
                                            .id("delete-projects")
                                            .max_h(px(168.))
                                            .overflow_y_scroll()
                                            .p_1()
                                            .rounded_lg()
                                            .bg(rgb(SURFACE))
                                            .children(rows),
                                    ),
                            )
                        })
                        .children(
                            used.clone()
                                .map(|text| div().text_color(rgb(MUTED)).child(text)),
                        ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(ButtonVariant::Danger),
                )
                .on_ok(move |_, window, cx| {
                    let root = root.clone();
                    view.update(cx, |_, cx| {
                        // Deleting renders the image once to size its placeholder.
                        let delete = cx
                            .background_executor()
                            .spawn(async move { Service::open(&root)?.delete(id).map(|_| ()) });
                        cx.spawn_in(window, async move |this, cx| {
                            let result = delete.await;
                            this.update_in(cx, |this, window, cx| {
                                if let Err(error) = result.and_then(|_| this.reload()) {
                                    this.notify_error(error, window, cx);
                                }
                                cx.notify();
                            })
                            .ok();
                        })
                        .detach();
                    });
                    true
                })
        });
    }
}
