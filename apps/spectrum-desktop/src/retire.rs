//! Images Spectrum no longer shows are freed two frames later. GPUI copies
//! an image to the GPU when the frame that first draws it is presented;
//! freeing it before then frees the texture under a pending copy, which
//! crashes the next frame. During a zoom or a fast edit, renders replace
//! images within a frame of each other, so each image waits until two more
//! frames have rendered, and no longer, so a drag holds few images.
use gpui::{RenderImage, Window};
use std::sync::{Arc, Mutex};

/// Frames rendered so far, and the images waiting with the frame each was
/// retired in.
struct Retired {
    frame: u64,
    images: Vec<(Arc<RenderImage>, u64)>,
}

static RETIRED: Mutex<Retired> = Mutex::new(Retired {
    frame: 0,
    images: Vec::new(),
});

/// Frees `image` once no frame can still be copying it.
pub fn drop_later(image: Arc<RenderImage>) {
    let mut retired = RETIRED.lock().unwrap_or_else(|e| e.into_inner());
    let frame = retired.frame;
    retired.images.push((image, frame));
}

/// Counts a frame and frees the images retired two frames ago or earlier;
/// called as each frame renders.
pub fn flush(window: &mut Window) {
    let due: Vec<_> = {
        let mut retired = RETIRED.lock().unwrap_or_else(|e| e.into_inner());
        retired.frame += 1;
        let frame = retired.frame;
        let (due, waiting) = retired
            .images
            .drain(..)
            .partition(|(_, at)| frame >= at + 2);
        retired.images = waiting;
        due
    };
    for (image, _) in due {
        window.drop_image(image).ok();
    }
}

/// Whether images are still waiting to be freed, so frames keep coming.
pub fn waiting() -> bool {
    !RETIRED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .images
        .is_empty()
}
