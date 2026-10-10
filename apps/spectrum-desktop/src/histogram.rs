//! A compact RGB histogram of the open image's latest render.
use crate::theme::*;
use gpui::{prelude::*, *};

pub struct Histogram {
    pub channels: [[u32; 256]; 3],
}

impl Histogram {
    /// Counts every other pixel, which is plenty for a sidebar graph, on
    /// every core.
    pub fn from_rgba(image: &image::RgbaImage) -> Self {
        use rayon::prelude::*;
        let channels = image
            .as_raw()
            .par_chunks(8 * 4096)
            .fold(
                || [[0u32; 256]; 3],
                |mut channels, chunk| {
                    for pixel in chunk.chunks_exact(8) {
                        for (channel, value) in pixel[..3].iter().enumerate() {
                            channels[channel][*value as usize] += 1;
                        }
                    }
                    channels
                },
            )
            .reduce(
                || [[0u32; 256]; 3],
                |mut total, part| {
                    for (total, part) in total.iter_mut().zip(part) {
                        for (total, part) in total.iter_mut().zip(part) {
                            *total += part;
                        }
                    }
                    total
                },
            );
        Self { channels }
    }

    /// Bin heights from 0 to 1. The clipped end bins are left out of the
    /// scale so a blown sky does not flatten the rest of the graph.
    fn heights(&self, channel: usize) -> [f32; 256] {
        let bins = &self.channels[channel];
        let peak = bins[1..255].iter().copied().max().unwrap_or(1).max(1) as f32;
        let mut out = [0.; 256];
        for (i, value) in bins.iter().enumerate() {
            out[i] = (*value as f32 / peak).min(1.);
        }
        out
    }
}

const COLORS: [(f32, f32, f32); 3] = [(0.0, 0.75, 0.62), (0.36, 0.6, 0.55), (0.6, 0.75, 0.62)];

/// Paints `histogram` as three translucent channel areas.
pub fn view(
    histogram: Option<std::sync::Arc<Histogram>>,
    width: f32,
    height: f32,
) -> impl IntoElement {
    div()
        .w(px(width))
        .h(px(height))
        .rounded_md()
        .overflow_hidden()
        .bg(rgb(SURFACE))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let Some(histogram) = &histogram else {
                        return;
                    };
                    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                    for (channel, (hue, sat, light)) in COLORS.iter().enumerate() {
                        let heights = histogram.heights(channel);
                        let mut path = PathBuilder::fill();
                        path.move_to(bounds.origin + point(px(0.), px(h)));
                        for (i, value) in heights.iter().enumerate() {
                            let x = i as f32 / 255. * w;
                            let y = h - value * (h - 4.);
                            path.line_to(bounds.origin + point(px(x), px(y)));
                        }
                        path.line_to(bounds.origin + point(px(w), px(h)));
                        path.close();
                        if let Ok(path) = path.build() {
                            window.paint_path(path, hsla(*hue, *sat, *light, 0.42));
                        }
                    }
                },
            )
            .size_full(),
        )
}
