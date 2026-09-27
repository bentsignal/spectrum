use gpui::{prelude::*, *};

/// Sample library contents. Nothing here touches the real Spectrum library.
#[derive(Clone)]
pub struct Asset {
    pub name: SharedString,
    pub canvas: bool,
    pub dimensions: &'static str,
    /// Sky and ground hues for the placeholder artwork.
    pub hues: (f32, f32),
    pub look: Look,
}

pub struct Project {
    pub name: SharedString,
    pub assets: Vec<usize>,
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

pub fn library() -> (Vec<Asset>, Vec<Project>) {
    let photo = |name: &str, dimensions, hues| Asset {
        name: SharedString::from(name.to_string()),
        canvas: false,
        dimensions,
        hues,
        look: Look::default(),
    };
    let assets = vec![
        photo("Harbor at dusk", "6000 × 4000", (0.62, 0.07)),
        photo("Ridge line", "5472 × 3648", (0.55, 0.3)),
        photo("Studio portrait", "4000 × 3000", (0.08, 0.02)),
        photo("Market street", "6000 × 4000", (0.12, 0.58)),
        Asset {
            name: "Spring poster".into(),
            canvas: true,
            dimensions: "1920 × 1080",
            hues: (0.95, 0.62),
            look: Look::default(),
        },
        photo("Field notes", "4032 × 3024", (0.25, 0.14)),
    ];
    let projects = vec![
        Project {
            name: "Spring campaign".into(),
            assets: vec![0, 1, 3, 4],
        },
        Project {
            name: "Portfolio".into(),
            assets: vec![0, 2, 5],
        },
        Project {
            name: "Archive".into(),
            assets: vec![1, 2, 3, 5],
        },
    ];
    (assets, projects)
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

/// Height over width for an asset's artwork.
pub fn aspect(asset: &Asset) -> f32 {
    if asset.canvas { 9. / 16. } else { 0.75 }
}

/// An asset's artwork at `width` with `look` applied. Canvases show `placed`.
pub fn artwork(asset: &Asset, look: Look, placed: &Asset, width: f32) -> Div {
    if asset.canvas {
        composition(asset, look, placed, width, [true; 3], [1.; 3])
    } else {
        photo(asset, look, width)
    }
}

/// A library thumbnail with a 4:3 frame for every kind of asset.
pub fn thumbnail(asset: &Asset, placed: &Asset, width: f32) -> Div {
    if !asset.canvas {
        return photo(asset, asset.look, width);
    }
    div()
        .w(px(width))
        .h(px(width * 0.75))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(0x1c1c1c))
        .child(composition(
            asset,
            asset.look,
            placed,
            width * 0.86,
            [true; 3],
            [1.; 3],
        ))
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
