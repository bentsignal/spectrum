//! The Eyedropper. While it is the tool, a flattened copy of the canvas is
//! kept so a loupe can follow the pointer: the pixels around it magnified,
//! the color under it, and its code in the color format chosen in the
//! pickers. A click takes the color as the foreground (Option: background);
//! Command+C copies the code. `spectrum canvas sample` reads the same color.
use crate::{color_text, theme::*, tools::Tool, workspace::Workspace};
use gpui::{prelude::*, *};
use image::RgbaImage;
use spectrum::library::Service;
use std::sync::Arc;

/// The loupe shows this many pixels across, each this many points wide.
const SPAN: i64 = 11;
const CELL: f32 = 14.;

impl Workspace {
    /// Renders the flattened canvas for the loupe when it is missing or out
    /// of date.
    pub fn ensure_pixels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        let version = canvas.look_version();
        let fresh = canvas.pixels.as_ref().is_some_and(|(v, _)| *v == version);
        if canvas.tool != Tool::Eyedropper || fresh || canvas.pixels_busy {
            return;
        }
        canvas.pixels_busy = true;
        let (root, doc, id) = (store.root.clone(), canvas.render_doc(), canvas.id);
        let task = cx.background_executor().spawn(async move {
            let mut resolved = doc;
            Service::open(&root)?.resolve(&mut resolved)?;
            let sources = prism_core::prepare_export_raster_sources(
                &resolved,
                &prism_core::default_raster_backing_cache_root()?,
            )?;
            let image = prism_core::render_document_scaled_with_sources(&resolved, 1., &sources)?;
            anyhow::Ok(Arc::new(image.to_rgba8()))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(canvas) = this.canvas.as_mut().filter(|c| c.id == id) else {
                    return;
                };
                canvas.pixels_busy = false;
                match result {
                    Ok(pixels) => {
                        canvas.pixels = Some((version, pixels));
                        // The canvas may have changed while this rendered.
                        this.ensure_pixels(window, cx);
                        cx.notify();
                    }
                    Err(error) => this.notify_error(error, window, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// The color under the pointer, from the flattened canvas.
    pub fn color_under_pointer(&self) -> Option<[u8; 4]> {
        let canvas = self.canvas.as_ref()?;
        let (_, pixels) = canvas.pixels.as_ref()?;
        let (x, y) = canvas.pointer?;
        pixel(pixels, x.floor() as i64, y.floor() as i64)
    }

    /// The Eyedropper's click: the color becomes the foreground color (the
    /// background color with Option), at once from the flattened canvas.
    pub fn pick_canvas_color(
        &mut self,
        at: (f32, f32),
        background: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cached = self
            .canvas
            .as_ref()
            .and_then(|c| c.pixels.as_ref())
            .and_then(|(_, pixels)| pixel(pixels, at.0.floor() as i64, at.1.floor() as i64));
        if let Some(color) = cached {
            return self.set_default_color(color, background, window, cx);
        }
        let (Some(canvas), Ok(store)) = (&self.canvas, &self.store) else {
            return;
        };
        let (x, y) = (at.0.floor(), at.1.floor());
        if x < 0. || y < 0. || x >= canvas.doc.width as f32 || y >= canvas.doc.height as f32 {
            return;
        }
        let (root, doc) = (store.root.clone(), canvas.render_doc());
        let task = cx.background_executor().spawn(async move {
            let mut resolved = doc;
            Service::open(&root)?.resolve(&mut resolved)?;
            prism_core::sample_document_color(&resolved, x as u32, y as u32)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(color) => this.set_default_color(color, background, window, cx),
                Err(error) => this.notify_error(error, window, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Command+C with the Eyedropper: copies the code of the color under
    /// the pointer. Returns whether it did.
    pub fn copy_picked_color(&mut self, cx: &mut Context<Self>) -> bool {
        if self
            .canvas
            .as_ref()
            .is_none_or(|c| c.tool != Tool::Eyedropper)
        {
            return false;
        }
        let Some(color) = self.color_under_pointer() else {
            return false;
        };
        let code = color_text::format(color, color_text::preferred());
        cx.write_to_clipboard(ClipboardItem::new_string(code));
        true
    }

    /// The loupe beside the pointer.
    pub fn eyedropper_overlay(&self, offset: Point<Pixels>, scale: f32) -> Option<AnyElement> {
        let canvas = self.canvas.as_ref()?;
        if canvas.tool != Tool::Eyedropper {
            return None;
        }
        let (x, y) = canvas.pointer?;
        let (_, pixels) = canvas.pixels.as_ref()?;
        let (cx0, cy0) = (x.floor() as i64, y.floor() as i64);
        let color = pixel(pixels, cx0, cy0)?;
        let half = SPAN / 2;
        let cells = (0..SPAN * SPAN).map(|i| {
            let (dx, dy) = (i % SPAN - half, i / SPAN - half);
            let fill = pixel(pixels, cx0 + dx, cy0 + dy)
                .map_or(rgb(0x101010).into(), |[r, g, b, _]| {
                    Hsla::from(rgb(u32::from_be_bytes([0, r, g, b])))
                });
            let center = dx == 0 && dy == 0;
            div()
                .absolute()
                .left(px((dx + half) as f32 * CELL))
                .top(px((dy + half) as f32 * CELL))
                .size(px(CELL))
                .bg(fill)
                .when(center, |el| el.border_1().border_color(rgb(0xffffff)))
        });
        let code = color_text::format(color, color_text::preferred());
        let side = SPAN as f32 * CELL;
        let [r, g, b, _] = color;
        let at = offset + point(px(x * scale + 22.), px(y * scale + 22.));
        Some(
            div()
                .absolute()
                .left(at.x)
                .top(at.y)
                .w(px(side + 24.))
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .rounded_lg()
                .bg(rgb(0x1a1a1a))
                .border_1()
                .border_color(rgb(BORDER))
                .shadow_lg()
                .child(
                    div()
                        .relative()
                        .size(px(side))
                        .rounded_sm()
                        .overflow_hidden()
                        .children(cells),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .size(px(16.))
                                .rounded_sm()
                                .border_1()
                                .border_color(rgb(BORDER))
                                .bg(rgb(u32::from_be_bytes([0, r, g, b]))),
                        )
                        .child(div().text_sm().truncate().child(code)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .text_xs()
                        .text_color(rgb(FAINT))
                        .child("Click: foreground color")
                        .child("Option-click: background color")
                        .child("⌘C: copy the code"),
                )
                .into_any_element(),
        )
    }
}

fn pixel(pixels: &RgbaImage, x: i64, y: i64) -> Option<[u8; 4]> {
    (x >= 0 && y >= 0 && x < i64::from(pixels.width()) && y < i64::from(pixels.height()))
        .then(|| pixels.get_pixel(x as u32, y as u32).0)
}
