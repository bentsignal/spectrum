//! Layers whose visible source is larger than one staging pass allows, such
//! as a large photo shown scaled down, still render on screen; the renderer
//! works through smaller regions.
use spectrum_canvas::*;

#[test]
fn a_large_photo_scaled_to_fit_renders_at_every_display_density() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("large.png");
    image::RgbaImage::from_pixel(4800, 4000, image::Rgba([200, 100, 50, 255]))
        .save(&source)
        .unwrap();
    let mut workspace = Workspace::new(Document::new("Large photo", 1920, 1080));
    workspace
        .execute(Command::AddRaster {
            path: source,
            name: None,
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    let mut document = workspace.document;
    document.layers[0].transform.scale_x = 0.25;
    document.layers[0].transform.scale_y = 0.25;
    for density in [0.5, 1.0, 2.0] {
        let sources =
            prepare_export_raster_sources(&document, &directory.path().join("cache")).unwrap();
        let image = render_document_scaled_with_sources(&document, density, &sources)
            .unwrap()
            .to_rgba8();
        let inside = (100.0 * density) as u32;
        assert_eq!(image.get_pixel(inside, inside).0, [200, 100, 50, 255]);
    }
}
