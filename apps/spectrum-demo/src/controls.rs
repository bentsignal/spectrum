use crate::{theme::*, workspace::SIDEBAR_WIDTH};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Selectable, Sizable,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    slider::{Slider, SliderEvent, SliderState},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
};

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

thread_local! {
    static LAID_OUT: RefCell<HashSet<EntityId>> = RefCell::default();
    static NEEDS_FRAME: Cell<bool> = const { Cell::new(false) };
    static SIDEBAR: RefCell<Vec<WeakEntity<SliderState>>> = RefCell::default();
}

/// Records a slider shown in the sidebar, to be laid out ahead of time.
pub fn in_sidebar(state: Entity<SliderState>) -> Entity<SliderState> {
    SIDEBAR.with(|all| all.borrow_mut().push(state.downgrade()));
    state
}

/// Sidebar sliders not laid out yet, drawn once off screen at the sidebar's
/// width so they show their value from their first visible frame.
pub fn warm_sliders() -> Option<Div> {
    let cold: Vec<_> = SIDEBAR.with(|all| {
        all.borrow()
            .iter()
            .filter_map(|weak| weak.upgrade())
            .filter(|state| LAID_OUT.with(|seen| seen.borrow_mut().insert(state.entity_id())))
            .collect()
    });
    (!cold.is_empty()).then(|| {
        div()
            .absolute()
            .top(px(-2000.))
            .left_0()
            .w(px(SIDEBAR_WIDTH - 32.))
            .flex()
            .flex_col()
            .children(cold.iter().map(Slider::new))
    })
}

/// Sliders place their thumb from the previous frame's width, which a new
/// slider does not have yet, so its first frame is drawn clear.
fn first_frame(state: &Entity<SliderState>) -> bool {
    let first = LAID_OUT.with(|seen| seen.borrow_mut().insert(state.entity_id()));
    if first {
        NEEDS_FRAME.with(|needs| needs.set(true));
    }
    first
}

/// Whether a slider was drawn clear this frame and needs another frame.
pub fn take_slider_frame() -> bool {
    NEEDS_FRAME.with(|needs| needs.replace(false))
}

thread_local! {
    /// Each slider's range, so typed values stay within it.
    static RANGES: RefCell<std::collections::HashMap<EntityId, (f32, f32)>> =
        RefCell::default();
    /// The slider whose value is being typed, its field, and the field's
    /// subscription.
    static TYPING: RefCell<Option<(EntityId, Entity<InputState>, Subscription)>> =
        const { RefCell::new(None) };
}

/// Records a slider's range for typed values.
pub fn remember_range(state: &Entity<SliderState>, min: f32, max: f32) {
    RANGES.with(|ranges| ranges.borrow_mut().insert(state.entity_id(), (min, max)));
}

/// The first number in `text`, ignoring units and accepting a typographic
/// minus sign.
fn typed_number(text: &str) -> Option<f32> {
    let text = text.replace('−', "-");
    let start = text.find(|c: char| c.is_ascii_digit() || c == '-' || c == '.')?;
    let number: String = text[start..]
        .chars()
        .enumerate()
        .take_while(|(i, c)| c.is_ascii_digit() || *c == '.' || (*i == 0 && *c == '-'))
        .map(|(_, c)| c)
        .collect();
    number.parse().ok().filter(|v: &f32| v.is_finite())
}

/// Ends typing into a slider; with `apply`, the typed value moves the
/// slider and is reported as a change, so its usual handler applies it.
fn finish_typing(apply: bool, window: &mut Window, cx: &mut App) {
    let Some((id, input, _)) = TYPING.with(|typing| typing.borrow_mut().take()) else {
        return;
    };
    let typed = typed_number(&input.read(cx).value());
    if let (true, Some(value)) = (apply, typed)
        && let Some(slider) = SLIDERS.with(|sliders| sliders.borrow().get(&id).cloned())
    {
        let (min, max) = RANGES
            .with(|ranges| ranges.borrow().get(&id).copied())
            .unwrap_or((f32::MIN, f32::MAX));
        let value = value.clamp(min, max);
        slider.update(cx, |state, cx| {
            state.set_value(value, window, cx);
            cx.emit(SliderEvent::Change(value.into()));
        });
    }
    window.refresh();
}

/// Turns a slider's readout into a field holding its value.
fn start_typing(state: &Entity<SliderState>, window: &mut Window, cx: &mut App) {
    finish_typing(true, window, cx);
    let value = state.read(cx).value().start();
    let text = format!("{value:.2}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string();
    // Empty with the value as a hint, so typing replaces it.
    let input = cx.new(|cx| InputState::new(window, cx).placeholder(text));
    input.update(cx, |input, cx| input.focus(window, cx));
    let subscription = window.subscribe(&input, cx, |_, event: &InputEvent, window, cx| {
        if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
            // After this event, so the field outlives its own handler.
            window.defer(cx, |window, cx| finish_typing(true, window, cx));
        }
    });
    SLIDERS.with(|sliders| {
        sliders
            .borrow_mut()
            .insert(state.entity_id(), state.clone())
    });
    TYPING.with(|typing| *typing.borrow_mut() = Some((state.entity_id(), input, subscription)));
    window.refresh();
}

thread_local! {
    static SLIDERS: RefCell<std::collections::HashMap<EntityId, Entity<SliderState>>> =
        RefCell::default();
}

/// A label, a readout that can be clicked to type an exact value, and the
/// slider. Enter or clicking away applies the value; Escape cancels.
pub fn slider_row(label: &'static str, value: String, state: &Entity<SliderState>) -> Div {
    let first = first_frame(state);
    let id = state.entity_id();
    let typing = TYPING.with(|typing| {
        typing
            .borrow()
            .as_ref()
            .filter(|(editing, ..)| *editing == id)
            .map(|(_, input, _)| input.clone())
    });
    let readout = readout(value, state, typing);
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .text_sm()
                .child(label)
                .child(readout),
        )
        .child(
            div()
                .when(first, |el| el.opacity(0.))
                .child(Slider::new(state)),
        )
}

/// A compact slider for the tool options bar: label, slider, and a readout
/// that can be clicked to type a value.
pub fn inline_slider(
    label: &'static str,
    value: String,
    state: &Entity<SliderState>,
    width: f32,
) -> Div {
    let id = state.entity_id();
    let typing = TYPING.with(|typing| {
        typing
            .borrow()
            .as_ref()
            .filter(|(editing, ..)| *editing == id)
            .map(|(_, input, _)| input.clone())
    });
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .child(div().text_color(rgb(MUTED)).child(label))
        .child(div().w(px(width)).child(Slider::new(state)))
        .child(div().min_w(px(44.)).child(readout(value, state, typing)))
}

fn readout(
    value: String,
    state: &Entity<SliderState>,
    typing: Option<Entity<InputState>>,
) -> AnyElement {
    let id = state.entity_id();
    match typing {
        Some(input) => div()
            .w(px(76.))
            .on_action(|_: &gpui_component::input::Escape, window, cx| {
                finish_typing(false, window, cx)
            })
            .child(Input::new(&input).xsmall())
            .into_any_element(),
        None => {
            let state = state.clone();
            div()
                .id(("slider-value", id.as_u64()))
                .px_1()
                .rounded_sm()
                .cursor_text()
                .text_color(rgb(MUTED))
                .hover(|el| el.text_color(rgb(TEXT)).bg(rgb(HOVER)))
                .child(value)
                .on_click(move |_, window, cx| start_typing(&state, window, cx))
                .into_any_element()
        }
    }
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

/// A full-width dropdown field. Opens a popup menu, whose items are spaced
/// so hover and selection highlights never touch.
#[derive(IntoElement)]
pub struct Field {
    base: Stateful<Div>,
    label: SharedString,
    open: bool,
}

impl Field {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            base: div().id(id.into()),
            label: label.into(),
            open: false,
        }
    }

    /// Attach a menu listing `options`. `selected` is read each time it opens.
    pub fn options(
        self,
        options: Vec<SharedString>,
        selected: impl Fn(&App) -> usize + 'static,
        on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let on_select = Rc::new(on_select);
        self.w_full().dropdown_menu(move |menu, _, cx| {
            let current = selected(cx);
            options.iter().enumerate().fold(
                menu.min_w(px(SIDEBAR_WIDTH - 33.)),
                |menu: PopupMenu, (index, name)| {
                    let on_select = on_select.clone();
                    menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(index == current)
                            .on_click(move |_, window, cx| on_select(index, window, cx)),
                    )
                },
            )
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

/// A toggle that fills its row share; on shows a raised surface.
pub fn chip(id: &'static str, label: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .h(px(30.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .border_1()
        .text_sm()
        .map(|el| {
            if on {
                el.bg(rgb(SELECTED))
                    .border_color(rgb(0x3a3a3a))
                    .text_color(rgb(TEXT))
            } else {
                el.border_color(rgb(0x262626))
                    .text_color(rgb(FAINT))
                    .hover(|el| el.text_color(rgb(MUTED)))
            }
        })
        .child(label)
}

fn field_box(base: Stateful<Div>, label: SharedString, open: bool) -> Stateful<Div> {
    base.w_full()
        .h(px(34.))
        .px_3()
        .flex()
        .items_center()
        .justify_between()
        .rounded_lg()
        .border_1()
        .border_color(rgb(if open { 0x444444 } else { 0x2e2e2e }))
        .bg(rgb(SURFACE))
        .text_sm()
        .hover(|el| el.bg(rgb(HOVER)))
        .child(div().truncate().child(label))
        .child(
            Icon::new(IconName::ChevronsUpDown)
                .xsmall()
                .text_color(rgb(MUTED)),
        )
}

impl RenderOnce for Field {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        field_box(self.base, self.label, self.open)
    }
}

/// An on/off switch with its label. On is a light track with a dark knob at
/// the right, off a dark track with a light knob at the left, so the state
/// reads at a glance. Click toggles; the caller flips `on`.
pub fn toggle(id: impl Into<ElementId>, label: &'static str, on: bool) -> Stateful<Div> {
    let (track, knob) = if on {
        (0xe6e6e6, 0x141414)
    } else {
        (0x333333, 0xf4f4f4)
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .child(
            div()
                .w(px(32.))
                .h(px(18.))
                .flex_none()
                .rounded_full()
                .bg(rgb(track))
                .flex()
                .items_center()
                .px(px(2.))
                .when(on, |el| el.justify_end())
                .child(div().size(px(14.)).rounded_full().bg(rgb(knob))),
        )
        .child(div().text_sm().child(label))
}
