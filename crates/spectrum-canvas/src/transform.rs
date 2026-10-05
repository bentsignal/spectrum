use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub rotation: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation: 0.0,
        }
    }
}

impl Transform {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            x: self.x.clamp(-100_000.0, 100_000.0),
            y: self.y.clamp(-100_000.0, 100_000.0),
            scale_x: self.scale_x.clamp(0.01, 100.0),
            scale_y: self.scale_y.clamp(0.01, 100.0),
            rotation: self.rotation.rem_euclid(360.0),
        }
    }
}
