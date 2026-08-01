use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use fontdue::Font;
use image::{Rgba, RgbaImage};

use crate::{
    Command, Document, Layer, LayerKind, TextAlignment, TextTypography, Workspace,
    measure_text_with_typography, render_document, render_layer_base,
};

fn test_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("prism-typography-{label}-{stamp}"))
}

fn test_actor() -> spectrum_revisions::Actor {
    spectrum_revisions::Actor {
        id: "person:typography-test".into(),
        display_name: "Typography test".into(),
        kind: spectrum_revisions::ActorKind::Human,
    }
}

#[test]
fn old_text_json_migrates_to_stable_bundled_typography() {
    let mut value = serde_json::to_value(Document::new("Legacy", 320, 200)).unwrap();
    value["version"] = serde_json::json!(1);
    value["layers"] = serde_json::json!([{
        "id": 1,
        "name": "Legacy text",
        "visible": true,
        "locked": false,
        "opacity": 1.0,
        "blend_mode": "normal",
        "transform": {},
        "adjustments": {},
        "mask": {},
        "stroke": {},
        "clip_to_below": false,
        "kind": {"type": "text", "text": "Legacy", "font_size": 48.0, "color": [255,255,255,255]}
    }]);
    value.as_object_mut().unwrap().remove("font_assets");
    value.as_object_mut().unwrap().remove("next_font_id");
    let mut document: Document = serde_json::from_value(value).unwrap();
    document.migrate().unwrap();
    let LayerKind::Text { typography, .. } = &document.layers[0].kind else {
        panic!("legacy layer should remain text");
    };
    assert_eq!(typography, &TextTypography::default());
    assert!(document.font_assets.is_empty());
    assert!(render_document(&document, None).is_ok());
}

#[test]
fn default_bundled_typography_is_pixel_exact_with_legacy_rasterizer() {
    let text = "Legacy descenders: gypqj\nSecond line";
    let font_size = 48.0;
    let color = [213, 227, 244, 173];
    let layer = Layer {
        kind: LayerKind::Text {
            text: text.into(),
            font_size,
            color,
            typography: TextTypography::default(),
        },
        ..Default::default()
    };
    let actual = render_layer_base(&layer, None).unwrap().to_rgba8();
    let expected = legacy_render_text(text, font_size, color);
    assert_eq!(actual.dimensions(), expected.dimensions());
    assert_eq!(actual.as_raw(), expected.as_raw());
}

#[test]
fn paragraph_metrics_cover_wrap_alignment_tracking_and_effects() {
    let base =
        measure_text_with_typography("Prism typography", 48.0, &TextTypography::default(), None)
            .unwrap();
    let compact = TextTypography {
        line_height: 0.5,
        ..Default::default()
    };
    let compact_height = measure_text_with_typography("First\nSecond", 48.0, &compact, None)
        .unwrap()
        .1;
    let default_height =
        measure_text_with_typography("First\nSecond", 48.0, &TextTypography::default(), None)
            .unwrap()
            .1;
    assert!(compact_height < default_height);
    let mut typography = TextTypography {
        alignment: TextAlignment::Center,
        line_height: 1.8,
        tracking: 6.0,
        box_width: Some(180.0),
        ..Default::default()
    };
    let wrapped =
        measure_text_with_typography("Prism typography", 48.0, &typography, None).unwrap();
    assert!(wrapped.1 > base.1);
    assert!(wrapped.0 >= 180);
    typography.effects.outline_width = 4.0;
    typography.effects.shadow_offset_x = 9.0;
    typography.effects.shadow_offset_y = -6.0;
    typography.effects.shadow_color = [0, 0, 0, 180];
    let effected =
        measure_text_with_typography("Prism typography", 48.0, &typography, None).unwrap();
    assert!(effected.0 > wrapped.0);
    assert!(effected.1 > wrapped.1);
}

#[test]
fn imported_font_is_deduplicated_and_round_trips_inside_durable_project() {
    let directory = test_directory("portable-font");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Hack-Regular.ttf");
    fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let project = directory.join("portable.prism");
    let mut workspace = Workspace::create_durable(
        Document::new("Portable typography", 640, 360),
        &project,
        test_actor(),
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();
    workspace
        .execute(Command::ImportFont {
            path: source.clone(),
        })
        .unwrap();
    workspace
        .execute(Command::ImportFont {
            path: source.clone(),
        })
        .unwrap();
    assert_eq!(workspace.document.font_assets.len(), 1);
    let font_id = workspace.document.font_assets[0].id;
    assert!(
        workspace.document.font_assets[0]
            .family
            .to_ascii_lowercase()
            .contains("hack")
    );
    workspace
        .execute(Command::AddText {
            text: "Portable\nfont".into(),
            name: None,
            font_size: 56.0,
            color: [240, 220, 180, 255],
            x: 30.0,
            y: 40.0,
        })
        .unwrap();
    let layer_id = workspace.document.selected.unwrap();
    let typography = TextTypography {
        font_id: Some(font_id),
        alignment: TextAlignment::Right,
        line_height: 1.4,
        tracking: 3.0,
        box_width: Some(260.0),
        ..Default::default()
    };
    workspace
        .execute(Command::SetTextTypography {
            id: layer_id,
            typography: typography.clone(),
        })
        .unwrap();
    workspace.save(None).unwrap();
    drop(workspace);
    fs::remove_file(&source).unwrap();

    let loaded = Workspace::load_read_only(&project).unwrap();
    assert_eq!(loaded.font_assets.len(), 1);
    assert!(loaded.font_assets[0].path.exists());
    assert_eq!(
        loaded.font_assets[0].bytes().unwrap(),
        epaint_default_fonts::HACK_REGULAR
    );
    let LayerKind::Text {
        typography: loaded_typography,
        ..
    } = &loaded.layer(layer_id).unwrap().kind
    else {
        panic!("text layer should survive durable replay");
    };
    assert_eq!(loaded_typography, &typography);
    assert!(render_document(&loaded, None).is_ok());

    fs::remove_dir_all(directory).unwrap();
}

fn legacy_render_text(text: &str, font_size: f32, color: [u8; 4]) -> RgbaImage {
    let font = Font::from_bytes(
        epaint_default_fonts::UBUNTU_LIGHT,
        fontdue::FontSettings::default(),
    )
    .unwrap();
    let line_metrics = font.horizontal_line_metrics(font_size);
    let ascent = line_metrics.map_or(font_size, |metrics| metrics.ascent);
    let natural_height = line_metrics.map_or(font_size, |metrics| metrics.new_line_size);
    let line_height = natural_height.max(font_size * 1.25).ceil().max(1.0);
    let lines: Vec<_> = text.split('\n').collect();
    let mut glyphs = Vec::new();
    let mut min_x = 0;
    let mut min_y = 0;
    let mut max_x = 1;
    let mut max_y = (line_height * lines.len().max(1) as f32).ceil() as i32;
    for (line_index, line) in lines.iter().enumerate() {
        let mut cursor_x: f32 = 0.0;
        for character in line.chars() {
            let metrics = font.metrics(character, font_size);
            let x = cursor_x.round() as i32 + metrics.xmin;
            let baseline = line_index as f32 * line_height + ascent;
            let y = baseline.round() as i32 - metrics.ymin - metrics.height as i32;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + metrics.width as i32);
            max_y = max_y.max(y + metrics.height as i32);
            glyphs.push((character, x, y));
            cursor_x += metrics.advance_width;
        }
        max_x = max_x.max(cursor_x.ceil() as i32);
    }
    let width = (max_x - min_x).max(1) as u32;
    let height = (max_y - min_y).max(1) as u32;
    let mut output = RgbaImage::new(width, height);
    for (character, glyph_x, glyph_y) in glyphs {
        let (metrics, bitmap) = font.rasterize(character, font_size);
        for row in 0..metrics.height {
            for column in 0..metrics.width {
                let x = glyph_x + column as i32 - min_x;
                let y = glyph_y + row as i32 - min_y;
                if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height {
                    let alpha = bitmap[row * metrics.width + column] as u16 * color[3] as u16 / 255;
                    output.put_pixel(
                        x as u32,
                        y as u32,
                        Rgba([color[0], color[1], color[2], alpha as u8]),
                    );
                }
            }
        }
    }
    output
}
