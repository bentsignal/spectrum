//! Where laid-out text sits inside its rendered image.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextGeometry {
    pub width: u32,
    pub height: u32,
    pub visual_left: f32,
    pub visual_top: f32,
    pub visual_width: f32,
    pub visual_height: f32,
    /// Logical paragraph layout bounds inside the returned image.
    pub layout_left: f32,
    pub layout_top: f32,
    pub layout_width: f32,
    pub layout_height: f32,
}

impl TextGeometry {
    pub fn visual_center(self) -> (f32, f32) {
        (
            self.visual_left + self.visual_width * 0.5,
            self.visual_top + self.visual_height * 0.5,
        )
    }
}
