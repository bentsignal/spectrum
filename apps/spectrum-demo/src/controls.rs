use crate::{theme::*, workspace::SIDEBAR_WIDTH};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Selectable, Sizable,
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    slider::{Slider, SliderState},
};
use std::rc::Rc;

/// A titled group of sidebar controls, with an optional action on the right.
pub fn group(title: &'static str, action: Option<AnyElement>) -> Div {
    div().flex().flex_col().gap_3().child(
        div()
            .h(px(22.))
            .flex()
            .items_center()
            .justify_between()
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(MUTED))
            .child(title)
            .children(action),
    )
}

pub fn slider_row(label: &'static str, value: String, state: &Entity<SliderState>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .justify_between()
                .text_sm()
                .child(label)
                .child(div().text_color(rgb(MUTED)).child(value)),
        )
        .child(Slider::new(state))
}

/// Equal-width segments with one selected, such as sidebar modes or edit scope.
pub fn segmented(
    id: &'static str,
    items: impl IntoIterator<Item = (Option<IconName>, &'static str)>,
    selected: usize,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> Div {
    let on_select = Rc::new(on_select);
    div()
        .flex()
        .p(px(3.))
        .gap(px(2.))
        .rounded_lg()
        .bg(rgb(SURFACE))
        .children(items.into_iter().enumerate().map(|(index, (icon, label))| {
            let on_select = on_select.clone();
            div()
                .id((id, index))
                .flex_1()
                .h(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .gap_2()
                .rounded_md()
                .text_sm()
                .text_color(rgb(if index == selected { TEXT } else { MUTED }))
                .when(index == selected, |el| el.bg(rgb(SELECTED)))
                .when(index != selected, |el| {
                    el.hover(|el| el.text_color(rgb(TEXT)))
                })
                .children(icon.map(|icon| Icon::new(icon).small()))
                .child(label)
                .on_click(move |_, window, cx| on_select(index, window, cx))
        }))
}

type Handler = Rc<dyn Fn(&mut Window, &mut App)>;

/// A full-width dropdown field. Opens a popup menu, whose items are spaced
/// so hover and selection highlights never touch.
#[derive(IntoElement)]
pub struct Field {
    base: Stateful<Div>,
    label: SharedString,
    open: bool,
    action: Option<(SharedString, Handler)>,
}

impl Field {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            base: div().id(id.into()),
            label: label.into(),
            open: false,
            action: None,
        }
    }

    /// Add a command below the options, such as "New project…".
    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.action = Some((label.into(), Rc::new(handler)));
        self
    }

    /// Attach a menu listing `options`. `selected` is read each time it opens.
    pub fn options(
        self,
        options: Vec<SharedString>,
        selected: impl Fn(&App) -> usize + 'static,
        on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let on_select = Rc::new(on_select);
        let action = self.action.clone();
        self.w_full().dropdown_menu(move |menu, _, cx| {
            let current = selected(cx);
            let menu = options.iter().enumerate().fold(
                menu.min_w(px(SIDEBAR_WIDTH - 33.)),
                |menu: PopupMenu, (index, name)| {
                    let on_select = on_select.clone();
                    menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(index == current)
                            .on_click(move |_, window, cx| on_select(index, window, cx)),
                    )
                },
            );
            match action.clone() {
                Some((label, handler)) => menu.separator().item(
                    PopupMenuItem::new(label).on_click(move |_, window, cx| handler(window, cx)),
                ),
                None => menu,
            }
        })
    }
}

impl Styled for Field {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Field {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl Selectable for Field {
    fn selected(mut self, selected: bool) -> Self {
        self.open = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.open
    }
}

impl DropdownMenu for Field {}

impl RenderOnce for Field {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.base
            .h(px(34.))
            .px_3()
            .flex()
            .items_center()
            .justify_between()
            .rounded_lg()
            .border_1()
            .border_color(rgb(if self.open { 0x444444 } else { 0x2e2e2e }))
            .bg(rgb(SURFACE))
            .text_sm()
            .hover(|el| el.bg(rgb(HOVER)))
            .child(self.label)
            .child(
                Icon::new(IconName::ChevronsUpDown)
                    .xsmall()
                    .text_color(rgb(MUTED)),
            )
    }
}
