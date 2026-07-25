use super::*;

impl Gradient {
    pub fn uniform_alpha(&self) -> Option<u8> {
        let alpha = self.stops.first()?.color[3];
        self.stops
            .iter()
            .all(|stop| stop.color[3] == alpha)
            .then_some(alpha)
    }
}

impl GradientSampler<'_> {
    /// Samples only alpha in source-local coordinates.
    ///
    /// Uniform-alpha gradients avoid geometry projection and stop search
    /// entirely. Nonuniform gradients preserve the exact alpha byte produced
    /// by full premultiplied RGBA sampling without computing RGB channels.
    pub fn sample_alpha_in_box(
        &self,
        source_x: f32,
        source_y: f32,
        source_width: f32,
        source_height: f32,
    ) -> u8 {
        if let Some(alpha) = self.uniform_alpha {
            return alpha;
        }
        let position = apply_spread(
            self.project_position(source_x, source_y, source_width, source_height),
            self.gradient.spread,
        );
        sample_stops_alpha(&self.gradient.stops, position)
    }
}

fn sample_stops_alpha(stops: &[GradientStop], position: f32) -> u8 {
    let Some(first) = stops.first().copied() else {
        return 0;
    };
    if !position.is_finite() || position <= first.position {
        return first.color[3];
    }
    let mut start_index = 0;
    let mut end_index = stops.len();
    while start_index < end_index {
        let middle = start_index + (end_index - start_index) / 2;
        if stops[middle].position < position {
            start_index = middle + 1;
        } else {
            end_index = middle;
        }
    }
    let index = start_index;
    if index == 0 {
        return first.color[3];
    }
    let Some(end) = stops.get(index).copied() else {
        return stops.last().map_or(first.color[3], |stop| stop.color[3]);
    };
    let start = stops[index - 1];
    let span = end.position - start.position;
    if !span.is_finite() || span <= 0.0 {
        return start.color[3];
    }
    let amount = (position - start.position) / span;
    if !amount.is_finite() {
        return start.color[3];
    }
    interpolate_alpha(start.color[3], end.color[3], amount.clamp(0.0, 1.0))
}

fn interpolate_alpha(start: u8, end: u8, amount: f32) -> u8 {
    let start_alpha = f32::from(start) / 255.0;
    let end_alpha = f32::from(end) / 255.0;
    ((start_alpha + (end_alpha - start_alpha) * amount) * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_only_sampling_exactly_matches_full_rgba_and_is_total() {
        for start in [0, 1, 63, 127, 218, 254, 255] {
            for end in [0, 1, 64, 128, 218, 254, 255] {
                for step in 0..=100 {
                    let amount = step as f32 / 100.0;
                    assert_eq!(
                        interpolate_alpha(start, end, amount),
                        interpolate_premultiplied([17, 91, 203, start], [249, 33, 7, end], amount)
                            [3]
                    );
                }
            }
        }

        let stop_sets = [
            vec![
                GradientStop::new(0.0, [255, 0, 0, 218]),
                GradientStop::new(0.4, [0, 255, 0, 218]),
                GradientStop::new(1.0, [0, 0, 255, 218]),
            ],
            vec![
                GradientStop::new(0.0, [255, 0, 0, 255]),
                GradientStop::new(0.3, [0, 255, 0, 17]),
                GradientStop::new(0.7, [0, 0, 255, 191]),
                GradientStop::new(1.0, [255, 255, 255, 0]),
            ],
            vec![
                GradientStop::new(f32::NAN, [1, 2, 3, 4]),
                GradientStop::new(f32::NEG_INFINITY, [5, 6, 7, 8]),
            ],
            Vec::new(),
        ];
        let boundary_values = [
            f32::NEG_INFINITY,
            -f32::MAX,
            -1.0,
            -0.0,
            0.0,
            f32::from_bits(1),
            f32::MIN_POSITIVE,
            0.5,
            1.0,
            f32::MAX,
            f32::INFINITY,
            f32::NAN,
        ];
        for kind in [
            GradientKind::Linear,
            GradientKind::Radial,
            GradientKind::Angle,
        ] {
            for spread in [
                GradientSpread::Pad,
                GradientSpread::Repeat,
                GradientSpread::Reflect,
            ] {
                for stops in &stop_sets {
                    let gradient = Gradient {
                        kind,
                        spread,
                        stops: stops.clone(),
                        ..Gradient::default()
                    };
                    let sampler = gradient.sampler();
                    for value in boundary_values {
                        assert_eq!(
                            sampler.sample_alpha_in_box(value, -value, value, -value),
                            sampler.sample_in_box(value, -value, value, -value)[3]
                        );
                    }
                }
            }
        }
    }
}
