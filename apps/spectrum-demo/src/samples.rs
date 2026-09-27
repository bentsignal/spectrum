use gpui::{prelude::*, *};

/// Sample library contents. Nothing here touches the real Spectrum library.
#[derive(Clone)]
pub struct Asset {
    pub name: SharedString,
    /// Sky and ground hues for the placeholder artwork.
    pub hues: (f32, f32),
    pub look: Look,
}

#[derive(Clone, Copy, Default, PartialEq)]
pub struct Look {
    pub exposure: f32,
    pub contrast: f32,
    pub temperature: f32,
    pub saturation: f32,
}

impl Look {
    fn apply(&self, color: Hsla) -> Hsla {
        let lightness = color.l + self.exposure * 0.14;
        let lightness = 0.5 + (lightness - 0.5) * (1. + self.contrast / 100.);
        let saturation = color.s * (1. + self.saturation / 100.);
        let hue = (color.h - self.temperature / 100. * 0.04).rem_euclid(1.);
        hsla(
            hue,
            saturation.clamp(0., 1.),
            lightness.clamp(0.02, 0.98),
            color.a,
        )
    }
}

/// The placed photo, then the canvas that places it.
pub fn library() -> Vec<Asset> {
    let asset = |name: &str, hues| Asset {
        name: SharedString::from(name.to_string()),
        hues,
        look: Look::default(),
    };
    vec![
        asset("Harbor at dusk", (0.62, 0.07)),
        asset("Spring poster", (0.95, 0.62)),
    ]
}

/// Stand-in artwork for a photo: a sky gradient with a sun setting behind
/// the horizon, over darker ground.
pub fn photo(asset: &Asset, look: Look, width: f32) -> Div {
    let height = width * 0.75;
    let horizon = height * 0.64;
    let (sky, ground) = asset.hues;
    let tone = |h, s, l| look.apply(hsla(h, s, l, 1.));
    let sun = width * 0.16;
    div()
        .relative()
        .w(px(width))
        .h(px(height))
        .overflow_hidden()
        .bg(linear_gradient(
            180.,
            linear_color_stop(tone(sky, 0.3, 0.14), 0.),
            linear_color_stop(tone(sky, 0.35, 0.07), 1.),
        ))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h(px(horizon))
                .overflow_hidden()
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(tone(sky, 0.45, 0.22), 0.),
                    linear_color_stop(tone(ground, 0.55, 0.55), 1.),
                ))
                .child(
                    div()
                        .absolute()
                        .left(px(width * 0.62))
                        .top(px(horizon - sun * 0.7))
                        .size(px(sun))
                        .rounded_full()
                        .bg(tone(ground, 0.7, 0.78)),
                ),
        )
}

/// The sample canvas. `visible` and `opacity` are ordered like the layer list:
/// title, placed photo, background. `look` applies to the background.
pub fn composition(
    canvas: &Asset,
    look: Look,
    placed: &Asset,
    width: f32,
    visible: [bool; 3],
    opacity: [f32; 3],
) -> Div {
    let height = width * 9. / 16.;
    let (a, b) = canvas.hues;
    let tone = |h, s, l| look.apply(hsla(h, s, l, 1.));
    let artboard = div()
        .relative()
        .w(px(width))
        .h(px(height))
        .overflow_hidden()
        .bg(rgb(0x2a2a2a));
    artboard
        .when(visible[2], |el| {
            el.child(
                div()
                    .absolute()
                    .size_full()
                    .opacity(opacity[2])
                    .bg(linear_gradient(
                        135.,
                        linear_color_stop(tone(a, 0.35, 0.82), 0.),
                        linear_color_stop(tone(b, 0.3, 0.7), 1.),
                    )),
            )
        })
        .when(visible[1], |el| {
            el.child(
                div()
                    .absolute()
                    .left(px(width * 0.07))
                    .top(px(height * 0.14))
                    .opacity(opacity[1])
                    .rounded(px(width * 0.008))
                    .overflow_hidden()
                    .child(photo(placed, placed.look, width * 0.46)),
            )
        })
        .when(visible[0], |el| {
            el.child(
                div()
                    .absolute()
                    .left(px(width * 0.6))
                    .top(px(height * 0.36))
                    .opacity(opacity[0])
                    .flex()
                    .flex_col()
                    .gap(px(width * 0.01))
                    .text_color(rgb(0x1c1c1c))
                    .child(
                        div()
                            .text_size(px(width * 0.075))
                            .font_weight(FontWeight::SEMIBOLD)
                            .line_height(relative(1.))
                            .child("Spring"),
                    )
                    .child(
                        div()
                            .text_size(px(width * 0.022))
                            .text_color(rgb(0x3a3a3a))
                            .child("New work, March 14"),
                    ),
            )
        })
}
