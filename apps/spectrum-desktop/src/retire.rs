//! Images Spectrum no longer shows are freed a few frames later. GPUI copies
//! an image to the GPU at the frame after it is first drawn; freeing it
//! before then frees the texture under a pending copy, which crashes the
//! next frame. During a zoom or a fast edit, renders replace images within
//! a frame of each other, so every image waits a moment before it goes.
use gpui::{RenderImage, Window};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// How long an image waits after it stops showing before it is freed.
const WAIT: Duration = Duration::from_millis(250);

static RETIRED: Mutex<Vec<(Arc<RenderImage>, Instant)>> = Mutex::new(Vec::new());

/// Frees `image` once no frame can still be copying it.
pub fn drop_later(image: Arc<RenderImage>) {
    RETIRED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((image, Instant::now()));
}

/// Frees the images retired long enough ago; called as each frame renders.
pub fn flush(window: &mut Window) {
    let due: Vec<_> = {
        let mut retired = RETIRED.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let (due, waiting) = retired.drain(..).partition(|(_, at)| now - *at >= WAIT);
        *retired = waiting;
        due
    };
    for (image, _) in due {
        window.drop_image(image).ok();
    }
}

/// Whether images are still waiting to be freed, so frames keep coming.
pub fn waiting() -> bool {
    !RETIRED.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
}
