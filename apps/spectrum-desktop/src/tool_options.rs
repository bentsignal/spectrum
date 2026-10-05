//! Settings for the current tool: the magic wand's tolerance and reach, and
//! the brush's size, hardness, and opacity.
use crate::{
    controls::in_sidebar,
    workspace::{Workspace, slider},
};
use gpui::*;
use gpui_component::slider::{SliderEvent, SliderState};

pub struct ToolOptions {
    pub wand_tolerance: Entity<SliderState>,
    pub contiguous: bool,
    /// The Move tool picks the layer clicked on (its box first); off, drags
    /// move the selected layer wherever they start.
    pub auto_select: bool,
    pub brush_size: Entity<SliderState>,
    pub brush_hardness: Entity<SliderState>,
    pub brush_opacity: Entity<SliderState>,
    _subscriptions: Vec<Subscription>,
}

impl ToolOptions {
    pub fn new(cx: &mut Context<Workspace>) -> Self {
        let wand_tolerance = in_sidebar(slider(cx, 0., 255., 1., 20.));
        let brush_size = in_sidebar(slider(cx, 1., 400., 1., 32.));
        let brush_hardness = in_sidebar(slider(cx, 0., 100., 1., 80.));
        let brush_opacity = in_sidebar(slider(cx, 1., 100., 1., 100.));
        let _subscriptions = [
            &wand_tolerance,
            &brush_size,
            &brush_hardness,
            &brush_opacity,
        ]
        .into_iter()
        .map(|state| cx.subscribe(state, |_, _, _: &SliderEvent, cx| cx.notify()))
        .collect();
        Self {
            wand_tolerance,
            contiguous: true,
            auto_select: true,
            brush_size,
            brush_hardness,
            brush_opacity,
            _subscriptions,
        }
    }

    pub fn tolerance(&self, cx: &App) -> u8 {
        self.wand_tolerance
            .read(cx)
            .value()
            .start()
            .round()
            .clamp(0., 255.) as u8
    }

    fn value(state: &Entity<SliderState>, cx: &App) -> f32 {
        state.read(cx).value().start()
    }

    pub fn brush(&self, cx: &App) -> (f32, f32, f32) {
        (
            Self::value(&self.brush_size, cx),
            Self::value(&self.brush_hardness, cx) / 100.,
            Self::value(&self.brush_opacity, cx) / 100.,
        )
    }
}
