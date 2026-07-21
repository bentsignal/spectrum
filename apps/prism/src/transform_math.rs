/// Returns an exact basis for cardinal rotations and Rust's raw trigonometric
/// result for every other angle. Interactive clients use this with core
/// geometry so meshes, handles, and sampled output cannot disagree at 90°.
pub fn rotation_sin_cos(degrees: f32) -> (f32, f32) {
    match degrees.rem_euclid(360.0) {
        0.0 => (0.0, 1.0),
        90.0 => (1.0, 0.0),
        180.0 => (0.0, -1.0),
        270.0 => (-1.0, 0.0),
        _ => degrees.to_radians().sin_cos(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cardinal_angles_have_exact_sampling_components() {
        assert_eq!(rotation_sin_cos(0.0), (0.0, 1.0));
        assert_eq!(rotation_sin_cos(90.0), (1.0, 0.0));
        assert_eq!(rotation_sin_cos(180.0), (0.0, -1.0));
        assert_eq!(rotation_sin_cos(270.0), (-1.0, 0.0));
        assert_eq!(rotation_sin_cos(-270.0), (1.0, 0.0));
        assert_eq!(rotation_sin_cos(450.0), (1.0, 0.0));
    }

    #[test]
    fn near_cardinal_angles_keep_the_raw_trigonometric_result() {
        for degrees in [0.000_01_f32, 89.999_99, 90.000_01, 179.999_98, 270.000_03] {
            assert_eq!(rotation_sin_cos(degrees), degrees.to_radians().sin_cos());
        }
    }
}
