//! Two-finger scrolls on the canvas and the open image. A gesture that
//! starts sideways (or with ⌘ held) pans for its whole length, in every
//! direction; one that starts up or down zooms. Changing between them takes
//! lifting the fingers and starting again. A mouse wheel decides each notch:
//! up and down zoom, sideways or with ⌘ held it pans.
use gpui::*;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    Pan,
    Zoom,
}

/// One scroll event, read as part of its gesture.
pub struct Scroll {
    pub gesture: Gesture,
    /// The movement in screen pixels, for panning.
    pub dx: f32,
    pub dy: f32,
    /// How much to zoom by, for zooming.
    pub factor: f32,
}

/// A pause this long ends a gesture where the system does not report when
/// fingers lift.
const GAP: Duration = Duration::from_millis(300);

#[derive(Default)]
pub struct ScrollGestures {
    current: Option<Gesture>,
    last: Option<Instant>,
}

impl ScrollGestures {
    pub fn read(&mut self, event: &ScrollWheelEvent) -> Option<Scroll> {
        let (dx, dy, factor, trackpad) = match event.delta {
            ScrollDelta::Pixels(delta) => {
                let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
                (dx, dy, (dy * 0.004).exp(), true)
            }
            ScrollDelta::Lines(lines) => {
                (lines.x * 40., lines.y * 40., 1.1_f32.powf(lines.y), false)
            }
        };
        let now = Instant::now();
        let paused = self.last.is_none_or(|last| now - last > GAP);
        self.last = Some(now);
        let started = matches!(event.touch_phase, TouchPhase::Started);
        if started || paused || !trackpad {
            self.current = None;
        }
        if dx == 0. && dy == 0. {
            return None;
        }
        let gesture =
            *self
                .current
                .get_or_insert(if event.modifiers.secondary() || dx.abs() > dy.abs() {
                    Gesture::Pan
                } else {
                    Gesture::Zoom
                });
        Some(Scroll {
            gesture,
            dx,
            dy,
            factor: factor.clamp(0.5, 2.),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Gesture, ScrollGestures};
    use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase, point, px};

    fn scroll(dx: f32, dy: f32, phase: TouchPhase) -> ScrollWheelEvent {
        ScrollWheelEvent {
            position: point(px(0.), px(0.)),
            delta: ScrollDelta::Pixels(point(px(dx), px(dy))),
            modifiers: Modifiers::default(),
            touch_phase: phase,
        }
    }

    #[test]
    fn a_gesture_keeps_the_mode_it_started_with_until_the_fingers_lift() {
        let mut gestures = ScrollGestures::default();
        let read = |gestures: &mut ScrollGestures, dx, dy, phase| {
            gestures.read(&scroll(dx, dy, phase)).map(|s| s.gesture)
        };
        assert_eq!(
            read(&mut gestures, 8., 1., TouchPhase::Started),
            Some(Gesture::Pan)
        );
        // Turning to move up and down keeps panning.
        assert_eq!(
            read(&mut gestures, 0., 9., TouchPhase::Moved),
            Some(Gesture::Pan)
        );
        assert_eq!(
            read(&mut gestures, 1., 12., TouchPhase::Moved),
            Some(Gesture::Pan)
        );
        // A new gesture decides again.
        assert_eq!(
            read(&mut gestures, 0., 6., TouchPhase::Started),
            Some(Gesture::Zoom)
        );
        assert_eq!(
            read(&mut gestures, 9., 0., TouchPhase::Moved),
            Some(Gesture::Zoom)
        );
    }
}
