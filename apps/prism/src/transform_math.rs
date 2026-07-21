pub(crate) fn rotation_sin_cos(degrees: f32) -> (f32, f32) {
    let (sin, cos) = degrees.to_radians().sin_cos();
    (stabilize_cardinal(sin), stabilize_cardinal(cos))
}

fn stabilize_cardinal(component: f32) -> f32 {
    if component.abs() < 0.000_001 {
        0.0
    } else if (component.abs() - 1.0).abs() < 0.000_001 {
        component.signum()
    } else {
        component
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
    }
}
