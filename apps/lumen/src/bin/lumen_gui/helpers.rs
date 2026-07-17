use super::*;

impl Histogram {
    pub(super) fn from_image(image: &DynamicImage) -> Self {
        let mut histogram = Self {
            red: [0; 256],
            green: [0; 256],
            blue: [0; 256],
            luma: [0; 256],
        };
        let rgba = image.to_rgba8();
        for pixel in rgba.pixels().step_by(2) {
            histogram.red[pixel[0] as usize] += 1;
            histogram.green[pixel[1] as usize] += 1;
            histogram.blue[pixel[2] as usize] += 1;
            let luma =
                (pixel[0] as f32 * 0.2126 + pixel[1] as f32 * 0.7152 + pixel[2] as f32 * 0.0722)
                    .round() as usize;
            histogram.luma[luma.min(255)] += 1;
        }
        histogram
    }
}

pub(super) fn paint_histogram(ui: &egui::Ui, rect: Rect, histogram: &Histogram) {
    let painter = ui.painter_at(rect);
    for fraction in [0.25, 0.5, 0.75] {
        let x = rect.left() + rect.width() * fraction;
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0, Color32::from_gray(33)),
        );
    }
    let peak = histogram
        .luma
        .iter()
        .chain(histogram.red.iter())
        .chain(histogram.green.iter())
        .chain(histogram.blue.iter())
        .copied()
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    for (values, color, width) in [
        (&histogram.luma, Color32::from_white_alpha(115), 1.5),
        (
            &histogram.red,
            Color32::from_rgba_unmultiplied(238, 77, 72, 175),
            1.15,
        ),
        (
            &histogram.green,
            Color32::from_rgba_unmultiplied(78, 211, 115, 175),
            1.15,
        ),
        (
            &histogram.blue,
            Color32::from_rgba_unmultiplied(82, 137, 240, 185),
            1.15,
        ),
    ] {
        let points: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                Pos2::new(
                    rect.left() + index as f32 / 255.0 * rect.width(),
                    rect.bottom() - (*value as f32 / peak).sqrt() * rect.height(),
                )
            })
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(width, color)));
    }
}

pub(super) fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [58.0, 16.0],
            egui::Label::new(RichText::new(label).size(10.0).color(Color32::GRAY)),
        );
        ui.label(RichText::new(value).size(10.0));
    });
}

pub(super) fn format_shutter(seconds: Option<f32>) -> String {
    let Some(seconds) = seconds.filter(|value| *value > 0.0) else {
        return "-- s".into();
    };
    if seconds >= 1.0 {
        format!("{seconds:.1} s")
    } else {
        format!("1/{:.0} s", 1.0 / seconds)
    }
}

pub(super) fn grade_swatch(ui: &mut egui::Ui, grade: ColorGrade) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 11.0, hue_color(grade.hue, grade.saturation));
    ui.painter().circle_stroke(
        rect.center(),
        11.0,
        Stroke::new(1.0, Color32::from_gray(110)),
    );
}

pub(super) fn hue_color(hue: f32, saturation: f32) -> Color32 {
    let h = hue.rem_euclid(360.0) / 60.0;
    let s = saturation.clamp(0.0, 100.0) / 100.0;
    let x = 1.0 - (h.rem_euclid(2.0) - 1.0).abs();
    let rgb = match h as i32 {
        0 => [1.0, x, 0.0],
        1 => [x, 1.0, 0.0],
        2 => [0.0, 1.0, x],
        3 => [0.0, x, 1.0],
        4 => [x, 0.0, 1.0],
        _ => [1.0, 0.0, x],
    };
    let mix = |channel: f32| ((0.38 + channel * 0.62) * s + 0.42 * (1.0 - s)) * 255.0;
    Color32::from_rgb(mix(rgb[0]) as u8, mix(rgb[1]) as u8, mix(rgb[2]) as u8)
}

pub(super) fn paint_texture(ui: &egui::Ui, clip: Rect, texture: &TextureHandle, image: Rect) {
    ui.painter().with_clip_rect(clip).image(
        texture.id(),
        image,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}

pub(super) fn compare_label(ui: &egui::Ui, rect: Rect, label: &str) {
    let badge = Rect::from_min_size(rect.min + Vec2::splat(10.0), Vec2::new(78.0, 24.0));
    ui.painter()
        .rect_filled(badge, 5.0, Color32::from_black_alpha(185));
    ui.painter().text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(10.0),
        if label == "EDITED" {
            ACCENT
        } else {
            Color32::LIGHT_GRAY
        },
    );
}

pub(super) fn spot_interaction(
    response: &egui::Response,
    image: Rect,
    spots: &mut Vec<SpotRemoval>,
    radius: f32,
    stroke_start: &mut Option<usize>,
) -> (bool, bool) {
    let mut changed = false;
    let mut commit = false;
    let point = |position: Pos2| {
        image.contains(position).then_some(SpotRemoval {
            x: ((position.x - image.left()) / image.width()).clamp(0.0, 1.0),
            y: ((position.y - image.top()) / image.height()).clamp(0.0, 1.0),
            radius,
            opacity: 1.0,
        })
    };
    let add = |spots: &mut Vec<SpotRemoval>, spot: SpotRemoval| {
        let spaced = spots.last().is_none_or(|last| {
            let distance = ((last.x - spot.x).powi(2) + (last.y - spot.y).powi(2)).sqrt();
            distance >= radius * 0.55
        });
        if spaced && spots.len() < 512 {
            spots.push(spot);
            true
        } else {
            false
        }
    };
    if response.drag_started() {
        *stroke_start = Some(spots.len());
    }
    if (response.drag_started() || response.dragged())
        && let Some(position) = response.interact_pointer_pos()
        && let Some(spot) = point(position)
    {
        changed |= add(spots, spot);
    }
    if response.drag_stopped()
        && let Some(start) = stroke_start.take()
    {
        commit = spots.len() > start;
    }
    if response.clicked()
        && let Some(position) = response.interact_pointer_pos()
        && let Some(spot) = point(position)
    {
        changed |= add(spots, spot);
        commit = changed;
    }
    (changed, commit)
}

pub(super) fn paint_spot_overlay(
    ui: &egui::Ui,
    canvas: Rect,
    image: Rect,
    spots: &[SpotRemoval],
    brush_radius: f32,
) {
    let painter = ui.painter().with_clip_rect(canvas.intersect(image));
    let scale = image.width().min(image.height());
    for spot in spots {
        let center = Pos2::new(
            image.left() + spot.x * image.width(),
            image.top() + spot.y * image.height(),
        );
        painter.circle_stroke(
            center,
            spot.radius * scale,
            Stroke::new(1.2, Color32::from_rgba_unmultiplied(255, 255, 255, 150)),
        );
    }
    if let Some(position) = ui.ctx().pointer_hover_pos()
        && image.contains(position)
    {
        painter.circle_stroke(position, brush_radius * scale, Stroke::new(1.5, ACCENT));
    }
}

pub(super) fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    changed: &mut bool,
    commit: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [78.0, 18.0],
            egui::Label::new(RichText::new(label).size(10.5)),
        );
        let response = ui.add(
            egui::Slider::new(value, range)
                .show_value(true)
                .smart_aim(false),
        );
        *changed |= response.changed();
        *commit |= response.drag_stopped() || (response.changed() && !response.dragged());
        if ui
            .add_enabled(value.abs() > f32::EPSILON, egui::Button::new("0").small())
            .on_hover_text(format!("Reset {label}"))
            .clicked()
        {
            *value = 0.0;
            *changed = true;
            *commit = true;
        }
    });
}

pub(super) fn crop_screen_rect(image: Rect, crop: CropRect) -> Rect {
    Rect::from_min_max(
        Pos2::new(
            image.left() + crop.x * image.width(),
            image.top() + crop.y * image.height(),
        ),
        Pos2::new(
            image.left() + (crop.x + crop.width) * image.width(),
            image.top() + (crop.y + crop.height) * image.height(),
        ),
    )
}

pub(super) fn crop_handle_at(position: Pos2, crop: Rect) -> Option<CropHandle> {
    let threshold = 14.0;
    let near = |point: Pos2| point.distance(position) <= threshold;
    if near(crop.left_top()) {
        Some(CropHandle::TopLeft)
    } else if near(crop.right_top()) {
        Some(CropHandle::TopRight)
    } else if near(crop.left_bottom()) {
        Some(CropHandle::BottomLeft)
    } else if near(crop.right_bottom()) {
        Some(CropHandle::BottomRight)
    } else if (position.x - crop.left()).abs() <= threshold
        && (crop.top()..=crop.bottom()).contains(&position.y)
    {
        Some(CropHandle::Left)
    } else if (position.x - crop.right()).abs() <= threshold
        && (crop.top()..=crop.bottom()).contains(&position.y)
    {
        Some(CropHandle::Right)
    } else if (position.y - crop.top()).abs() <= threshold
        && (crop.left()..=crop.right()).contains(&position.x)
    {
        Some(CropHandle::Top)
    } else if (position.y - crop.bottom()).abs() <= threshold
        && (crop.left()..=crop.right()).contains(&position.x)
    {
        Some(CropHandle::Bottom)
    } else if crop.contains(position) {
        Some(CropHandle::Move)
    } else {
        None
    }
}

pub(super) fn crop_interaction(
    ui: &egui::Ui,
    response: &egui::Response,
    image: Rect,
    crop: &mut CropRect,
    drag: &mut Option<CropDrag>,
) {
    if response.drag_started()
        && let Some(pointer) = response.interact_pointer_pos()
        && let Some(handle) = crop_handle_at(pointer, crop_screen_rect(image, *crop))
    {
        *drag = Some(CropDrag {
            handle,
            start: *crop,
            pointer,
        });
    }
    if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
        && let Some(active) = *drag
    {
        let dx = (pointer.x - active.pointer.x) / image.width().max(1.0);
        let dy = (pointer.y - active.pointer.y) / image.height().max(1.0);
        let mut left = active.start.x;
        let mut top = active.start.y;
        let mut right = active.start.x + active.start.width;
        let mut bottom = active.start.y + active.start.height;
        const MINIMUM: f32 = 0.025;
        match active.handle {
            CropHandle::Move => {
                let width = right - left;
                let height = bottom - top;
                left = (left + dx).clamp(0.0, 1.0 - width);
                top = (top + dy).clamp(0.0, 1.0 - height);
                right = left + width;
                bottom = top + height;
            }
            CropHandle::Left | CropHandle::TopLeft | CropHandle::BottomLeft => {
                left = (left + dx).clamp(0.0, right - MINIMUM);
            }
            CropHandle::Right | CropHandle::TopRight | CropHandle::BottomRight => {
                right = (right + dx).clamp(left + MINIMUM, 1.0);
            }
            _ => {}
        }
        match active.handle {
            CropHandle::Top | CropHandle::TopLeft | CropHandle::TopRight => {
                top = (top + dy).clamp(0.0, bottom - MINIMUM);
            }
            CropHandle::Bottom | CropHandle::BottomLeft | CropHandle::BottomRight => {
                bottom = (bottom + dy).clamp(top + MINIMUM, 1.0);
            }
            _ => {}
        }
        *crop = CropRect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
        .sanitized();
        ui.ctx().request_repaint();
    }
    if response.drag_stopped() {
        *drag = None;
    }
}

pub(super) fn paint_crop_overlay(ui: &egui::Ui, canvas: Rect, image: Rect, crop: CropRect) {
    let crop = crop_screen_rect(image, crop);
    let painter = ui.painter().with_clip_rect(canvas.intersect(image));
    let shade = Color32::from_black_alpha(165);
    for rect in [
        Rect::from_min_max(image.min, Pos2::new(image.right(), crop.top())),
        Rect::from_min_max(Pos2::new(image.left(), crop.bottom()), image.max),
        Rect::from_min_max(
            Pos2::new(image.left(), crop.top()),
            Pos2::new(crop.left(), crop.bottom()),
        ),
        Rect::from_min_max(
            Pos2::new(crop.right(), crop.top()),
            Pos2::new(image.right(), crop.bottom()),
        ),
    ] {
        painter.rect_filled(rect, 0.0, shade);
    }
    painter.rect_stroke(
        crop,
        0.0,
        Stroke::new(2.0, Color32::WHITE),
        egui::StrokeKind::Inside,
    );
    for fraction in [1.0 / 3.0, 2.0 / 3.0] {
        let x = crop.left() + crop.width() * fraction;
        let y = crop.top() + crop.height() * fraction;
        painter.line_segment(
            [Pos2::new(x, crop.top()), Pos2::new(x, crop.bottom())],
            Stroke::new(1.0, Color32::from_white_alpha(150)),
        );
        painter.line_segment(
            [Pos2::new(crop.left(), y), Pos2::new(crop.right(), y)],
            Stroke::new(1.0, Color32::from_white_alpha(150)),
        );
    }
    let handles = [
        crop.left_top(),
        crop.right_top(),
        crop.left_bottom(),
        crop.right_bottom(),
        Pos2::new(crop.center().x, crop.top()),
        Pos2::new(crop.center().x, crop.bottom()),
        Pos2::new(crop.left(), crop.center().y),
        Pos2::new(crop.right(), crop.center().y),
    ];
    for center in handles {
        painter.rect_filled(
            Rect::from_center_size(center, Vec2::splat(9.0)),
            1.0,
            Color32::WHITE,
        );
        painter.rect_stroke(
            Rect::from_center_size(center, Vec2::splat(9.0)),
            1.0,
            Stroke::new(1.0, Color32::BLACK),
            egui::StrokeKind::Inside,
        );
    }
}

pub(super) fn set_crop_aspect(crop: &mut CropRect, output_ratio: f32, source_ratio: f32) {
    let normalized_ratio = output_ratio / source_ratio.max(0.01);
    let center = (crop.x + crop.width * 0.5, crop.y + crop.height * 0.5);
    let mut width = crop.width;
    let mut height = width / normalized_ratio;
    if height > crop.height {
        height = crop.height;
        width = height * normalized_ratio;
    }
    width = width.clamp(0.025, 1.0);
    height = height.clamp(0.025, 1.0);
    crop.x = (center.0 - width * 0.5).clamp(0.0, 1.0 - width);
    crop.y = (center.1 - height * 0.5).clamp(0.0, 1.0 - height);
    crop.width = width;
    crop.height = height;
}

pub(super) fn estimate_export_bytes(
    workspace: &Workspace,
    ids: &[u64],
    format: ExportFormat,
    quality: u8,
    max_size: u32,
) -> u64 {
    ids.iter()
        .filter_map(|id| workspace.project.photo(*id).ok())
        .map(|photo| {
            let crop = photo.adjustments.crop.unwrap_or_default();
            let mut width = photo.width as f64 * crop.width as f64;
            let mut height = photo.height as f64 * crop.height as f64;
            let long = width.max(height);
            if max_size > 0 && long > max_size as f64 {
                let scale = max_size as f64 / long;
                width *= scale;
                height *= scale;
            }
            format.estimate_bytes((width * height) as u64, quality)
        })
        .sum()
}

pub(super) fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else {
        format!("{:.0} KB", bytes as f64 / 1_000.0)
    }
}

pub(super) fn tone_curve_editor(
    ui: &mut egui::Ui,
    curve: &mut ToneCurve,
    channel: usize,
) -> (bool, bool) {
    // Keep the graph and its endpoint handles clear of the inspector scrollbar.
    let desired = Vec2::new((ui.available_width() - 18.0).max(140.0), 176.0);
    let (outer, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    let rect = outer.shrink(7.0);
    let painter = ui.painter_at(outer);
    painter.rect_filled(outer, 4.0, CANVAS);
    for i in 1..4 {
        let t = i as f32 / 4.0;
        let x = egui::lerp(rect.x_range(), t);
        let y = egui::lerp(rect.y_range(), t);
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0, Color32::from_gray(43)),
        );
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, Color32::from_gray(43)),
        );
    }
    let curve_color = [
        Color32::WHITE,
        Color32::from_rgb(235, 91, 91),
        Color32::from_rgb(89, 210, 119),
        Color32::from_rgb(94, 139, 240),
    ][channel];
    let to_screen = |point: CurvePoint| {
        Pos2::new(
            rect.left() + point.x * rect.width(),
            rect.bottom() - point.y * rect.height(),
        )
    };
    for pair in curve.points.windows(2) {
        painter.line_segment(
            [to_screen(pair[0]), to_screen(pair[1])],
            Stroke::new(2.0, curve_color),
        );
    }
    for point in &curve.points {
        painter.circle_filled(to_screen(*point), 4.0, curve_color);
        painter.circle_stroke(to_screen(*point), 5.0, Stroke::new(1.0, Color32::BLACK));
    }
    let mut changed = false;
    if response.clicked()
        && let Some(position) = response.interact_pointer_pos()
    {
        curve.points.push(CurvePoint {
            x: ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
            y: ((rect.bottom() - position.y) / rect.height()).clamp(0.0, 1.0),
        });
        *curve = curve.clone().sanitized();
        changed = true;
    }
    if response.dragged()
        && let Some(position) = response.interact_pointer_pos()
    {
        let pointer = CurvePoint {
            x: ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
            y: ((rect.bottom() - position.y) / rect.height()).clamp(0.0, 1.0),
        };
        if let Some((index, _)) = curve.points.iter().enumerate().min_by(|(_, a), (_, b)| {
            let da = (a.x - pointer.x).powi(2) + (a.y - pointer.y).powi(2);
            let db = (b.x - pointer.x).powi(2) + (b.y - pointer.y).powi(2);
            da.total_cmp(&db)
        }) {
            curve.points[index].y = pointer.y;
            if index > 0 && index + 1 < curve.points.len() {
                curve.points[index].x = pointer.x.clamp(
                    curve.points[index - 1].x + 0.005,
                    curve.points[index + 1].x - 0.005,
                );
            }
            changed = true;
        }
    }
    (
        changed,
        response.drag_stopped() || (changed && !response.dragged()),
    )
}

pub(super) fn load_texture(
    context: &egui::Context,
    name: String,
    image: DynamicImage,
) -> TextureHandle {
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    context.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
        TextureOptions::LINEAR,
    )
}

pub(super) fn fit_size(image: Vec2, available: Vec2) -> Vec2 {
    image * (available.x / image.x).min(available.y / image.y).min(1.0)
}

pub(super) fn preview_fit_size(layout: Option<Vec2>, raster: Vec2, available: Vec2) -> Vec2 {
    fit_size(layout.unwrap_or(raster), available)
}

pub(super) fn shoot_date_label(start: Option<&str>, end: Option<&str>, imported: &str) -> String {
    match (start, end) {
        (Some(start), Some(end)) if start != end => {
            format!("Shot {} – {}", friendly_date(start), friendly_date(end))
        }
        (Some(date), _) => format!("Shot {}", friendly_date(date)),
        _ if !imported.is_empty() => format!("Added {}", friendly_date(imported)),
        _ => "Added to catalog".into(),
    }
}

pub(super) fn friendly_date(date: &str) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let mut parts = date.split('-');
    let parsed = parts
        .next()
        .and_then(|year| year.parse::<u32>().ok())
        .zip(parts.next().and_then(|month| month.parse::<usize>().ok()))
        .zip(parts.next().and_then(|day| day.parse::<u32>().ok()));
    if let Some(((year, month), day)) = parsed
        && let Some(month) = month.checked_sub(1).and_then(|index| MONTHS.get(index))
    {
        return format!("{month} {day}, {year}");
    }
    date.to_owned()
}

pub(super) fn same_preview_geometry(left: &Adjustments, right: &Adjustments) -> bool {
    left.crop == right.crop
        && left.rotation == right.rotation
        && left.straighten == right.straighten
        && left.flip_horizontal == right.flip_horizontal
        && left.flip_vertical == right.flip_vertical
}

pub(super) fn shorten(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}...")
    } else {
        head
    }
}

pub(super) fn catalog_label(path: &Path) -> String {
    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Catalog");
    let parent = path
        .parent()
        .and_then(|value| value.file_name())
        .and_then(|value| value.to_str());
    parent.map_or_else(|| name.to_owned(), |parent| format!("{name}  /  {parent}"))
}

pub(super) fn current_catalog_name(workspace: &Workspace) -> String {
    workspace
        .catalog_path
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Unsaved catalog".into())
}

#[allow(dead_code)]
pub(super) fn _assert_photo_is_available(_: Photo) {}
