use crate::{
    theme::*,
    workspace::{LibraryView, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable, WindowExt, button::ButtonVariant, dialog::DialogButtonProps,
};

/// Index of the sample image placed in the sample canvas.
const PLACED: usize = 0;

impl Workspace {
    /// Ask before moving an asset to the trash, listing the projects it is in.
    pub fn confirm_delete(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        let name = self.assets[index].name.clone();
        let projects: Vec<(usize, SharedString)> = self
            .projects
            .iter()
            .enumerate()
            .filter(|(_, p)| p.assets.contains(&index))
            .map(|(i, p)| (i, p.name.clone()))
            .collect();
        window.open_dialog(cx, move |dialog, _, _| {
            let rows = projects.iter().map(|(project, name)| {
                let view = view.clone();
                let project = *project;
                div()
                    .id(("delete-project", project))
                    .h(px(34.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .rounded_md()
                    .text_sm()
                    .hover(|el| el.bg(rgb(HOVER)))
                    .child(Icon::new(IconName::FolderClosed).small().text_color(rgb(MUTED)))
                    .child(div().flex_1().child(name.clone()))
                    .child(Icon::new(IconName::ChevronRight).xsmall().text_color(rgb(FAINT)))
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        view.update(cx, |this, cx| {
                            this.view = LibraryView::Project(project);
                            cx.notify();
                        })
                    })
            });
            let count = projects.len();
            let view = view.clone();
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
                        .when(index == PLACED, |el| {
                            el.child(div().text_color(rgb(MUTED)).child(
                                "Spring poster uses it and will show a placeholder until you restore it.",
                            ))
                        }),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(ButtonVariant::Danger),
                )
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.assets[index].trashed = true;
                        cx.notify();
                    });
                    true
                })
        });
    }
}
