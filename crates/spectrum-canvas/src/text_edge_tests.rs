use super::*;

/// White text on a white shape stays invisible at every render scale, in
/// it or clipped inside it: resizing weighs colors by their alpha, so the
/// transparent pixels around glyphs never darken their edges.
#[test]
fn white_text_on_white_has_no_dark_edges_at_any_scale() {
    for clip in [true, false] {
        let mut document = Document::new("Edges", 200, 120);
        document.background = [0, 0, 0, 0];
        let mut workspace = Workspace::new(document, None);
        workspace
            .execute(Command::AddRectangle {
                name: None,
                width: 120,
                height: 100,
                color: [255, 255, 255, 255],
                corner_radius: 0.0,
                x: 40.0,
                y: 10.0,
            })
            .unwrap();
        workspace
            .execute(Command::AddText {
                text: "esti".into(),
                name: None,
                font_size: 64.0,
                color: [255, 255, 255, 255],
                x: 20.0,
                y: 20.0,
                shaping: TextShaping::default(),
            })
            .unwrap();
        let text = workspace.document.selected.unwrap();
        workspace
            .execute(Command::SetClipping {
                id: text,
                enabled: clip,
            })
            .unwrap();
        for scale in [0.57_f32, 1.0, 1.14, 1.5, 2.28] {
            let image = render_document_scaled(&workspace.document, scale)
                .unwrap()
                .to_rgba8();
            for x in 45..155 {
                for y in 15..105 {
                    let pixel = image[((x as f32 * scale) as u32, (y as f32 * scale) as u32)];
                    assert_eq!(pixel.0, [255; 4], "clip {clip}, scale {scale}, at {x},{y}");
                }
            }
        }
    }
}
