use super::*;

use spectrum_terminal::TerminalSize;
use terminal::{CellPosition, TerminalTab};

const FONT_SIZE: f32 = 13.0;
const HORIZONTAL_PADDING: f32 = 8.0;
const VERTICAL_PADDING: f32 = 6.0;

pub(super) fn show_terminal(
    ui: &mut egui::Ui,
    session: &mut TerminalTab,
    focus_requested: &mut bool,
) {
    let outer = ui.available_rect_before_wrap();
    ui.painter().rect_filled(outer, 0.0, INK);
    let viewport = outer.shrink2(Vec2::new(HORIZONTAL_PADDING, VERTICAL_PADDING));
    let font = FontId::new(FONT_SIZE, egui::FontFamily::Monospace);
    let cell_size = terminal_cell_size(ui, &font);
    let size = size_for_viewport(viewport.size(), cell_size);
    session.resize(size);

    let response = ui
        .interact(
            viewport,
            ui.id().with(("terminal", session.id)),
            Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Text);
    if response.clicked() || response.drag_started() || *focus_requested {
        response.request_focus();
        *focus_requested = false;
    }

    paint_screen(
        ui,
        viewport,
        cell_size,
        &font,
        session,
        response.has_focus(),
    );
    terminal_input::handle_terminal_input(ui, &response, viewport, cell_size, session);
}

pub(super) fn terminal_cell_size(ui: &egui::Ui, font: &FontId) -> Vec2 {
    let galley = ui.painter().layout_no_wrap("M".into(), font.clone(), TEXT);
    Vec2::new(galley.size().x.max(1.0), (galley.size().y + 2.0).ceil())
}

pub(super) fn size_for_viewport(viewport: Vec2, cell: Vec2) -> TerminalSize {
    TerminalSize::new(
        (viewport.y / cell.y).floor().clamp(2.0, 180.0) as u16,
        (viewport.x / cell.x).floor().clamp(10.0, 300.0) as u16,
    )
}

fn paint_screen(
    ui: &egui::Ui,
    viewport: Rect,
    cell_size: Vec2,
    font: &FontId,
    session: &TerminalTab,
    focused: bool,
) {
    let painter = ui.painter().with_clip_rect(viewport);
    let screen = session.parser.screen();
    for row in 0..session.size.rows {
        let (job, has_bold) = row_layout_job(
            screen,
            row,
            session.size.cols,
            font,
            cell_size.y,
            session.selection,
            false,
        );
        let position = viewport.min + Vec2::new(0.0, f32::from(row) * cell_size.y);
        painter.galley(position, painter.layout_job(job), TEXT);
        if has_bold {
            let (bold_job, _) = row_layout_job(
                screen,
                row,
                session.size.cols,
                font,
                cell_size.y,
                session.selection,
                true,
            );
            // egui's bundled monospace family has no separate weight. A tiny
            // second pass supplies weight without changing exact ANSI/RGB color.
            painter.galley(
                position + Vec2::new(0.45, 0.0),
                painter.layout_job(bold_job),
                TEXT,
            );
        }
    }
    if focused && screen.scrollback() == 0 && !screen.hide_cursor() {
        let (row, col) = screen.cursor_position();
        let cursor = Rect::from_min_size(
            viewport.min + Vec2::new(f32::from(col) * cell_size.x, f32::from(row) * cell_size.y),
            cell_size,
        );
        painter.rect_stroke(
            cursor,
            0.0,
            Stroke::new(1.4, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
}

fn row_layout_job(
    screen: &vt100::Screen,
    row: u16,
    cols: u16,
    font: &FontId,
    line_height: f32,
    selection: Option<terminal::TerminalSelection>,
    bold_only: bool,
) -> (egui::text::LayoutJob, bool) {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    job.keep_trailing_whitespace = true;
    job.first_row_min_height = line_height;
    job.break_on_newline = false;
    let mut has_bold = false;
    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else {
            continue;
        };
        if cell.is_wide_continuation() {
            continue;
        }
        has_bold |= cell.bold() && cell.has_contents();
        let selected =
            selection.is_some_and(|selection| selection.contains(CellPosition { row, col }));
        let mut foreground = terminal_color(cell.fgcolor(), true);
        let mut background = terminal_color(cell.bgcolor(), false);
        if cell.inverse() {
            std::mem::swap(&mut foreground, &mut background);
        }
        if selected {
            foreground = TEXT;
            background = ACTIVE_SURFACE;
        }
        if cell.dim() {
            foreground = dimmed(foreground);
        }
        if bold_only {
            background = Color32::TRANSPARENT;
            if !cell.bold() {
                foreground = Color32::TRANSPARENT;
            }
        } else if background == INK {
            background = Color32::TRANSPARENT;
        }
        let contents = if cell.has_contents() {
            cell.contents()
        } else {
            " "
        };
        job.append(
            contents,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                line_height: Some(line_height),
                color: foreground,
                background,
                expand_bg: 0.0,
                italics: cell.italic(),
                underline: if cell.underline() {
                    Stroke::new(1.0, foreground)
                } else {
                    Stroke::NONE
                },
                valign: egui::Align::TOP,
                ..Default::default()
            },
        );
    }
    (job, has_bold)
}

pub(super) fn terminal_color(color: vt100::Color, foreground: bool) -> Color32 {
    match color {
        vt100::Color::Default => {
            if foreground {
                Color32::from_rgb(218, 216, 222)
            } else {
                INK
            }
        }
        vt100::Color::Rgb(red, green, blue) => Color32::from_rgb(red, green, blue),
        vt100::Color::Idx(index) => indexed_color(index),
    }
}

fn indexed_color(index: u8) -> Color32 {
    const ANSI: [(u8, u8, u8); 16] = [
        (20, 21, 25),
        (210, 91, 99),
        (126, 190, 128),
        (202, 172, 102),
        (103, 153, 207),
        (174, 132, 196),
        (104, 187, 190),
        (211, 211, 216),
        (104, 104, 112),
        (235, 119, 126),
        (151, 210, 151),
        (225, 197, 126),
        (130, 176, 224),
        (199, 158, 220),
        (132, 208, 210),
        (239, 238, 242),
    ];
    match index {
        0..=15 => {
            let (red, green, blue) = ANSI[usize::from(index)];
            Color32::from_rgb(red, green, blue)
        }
        16..=231 => {
            let value = index - 16;
            let component = |part: u8| if part == 0 { 0 } else { 55 + part * 40 };
            Color32::from_rgb(
                component(value / 36),
                component((value / 6) % 6),
                component(value % 6),
            )
        }
        232..=255 => {
            let value = 8 + (index - 232) * 10;
            Color32::from_gray(value)
        }
    }
}

fn dimmed(color: Color32) -> Color32 {
    Color32::from_rgb(color.r() * 2 / 3, color.g() * 2 / 3, color.b() * 2 / 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_geometry_has_stable_clamps() {
        assert_eq!(
            size_for_viewport(Vec2::new(800.0, 400.0), Vec2::new(8.0, 16.0)),
            TerminalSize::new(25, 100)
        );
        assert_eq!(
            size_for_viewport(Vec2::ZERO, Vec2::new(8.0, 16.0)),
            TerminalSize::new(2, 10)
        );
    }

    #[test]
    fn indexed_palette_covers_ansi_cube_and_grayscale() {
        assert_eq!(indexed_color(16), Color32::BLACK);
        assert_eq!(indexed_color(231), Color32::WHITE);
        assert_eq!(indexed_color(232), Color32::from_gray(8));
        assert_eq!(indexed_color(255), Color32::from_gray(238));
    }

    #[test]
    fn default_terminal_colors_belong_to_prism_dark_surface() {
        assert_eq!(terminal_color(vt100::Color::Default, false), INK);
        assert_ne!(terminal_color(vt100::Color::Default, true), INK);
    }
}
