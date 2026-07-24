use super::*;

#[test]
fn schema_keeps_guides_typography_and_pixel_deletion_commands_together() {
    let schema = schema();
    let examples = schema["command_protocol"]["examples"].as_array().unwrap();
    for command in [
        "align_layer",
        "add_guide",
        "import_font",
        "set_text_typography",
        "insert_layer",
        "add_paint_layer_with_stroke",
        "add_brush_stroke",
        "lasso_selection",
        "delete_selected_pixels",
        "rename_document",
    ] {
        assert!(examples.iter().any(|example| example["command"] == command));
    }
    assert!(schema["alignment"].is_object());
    assert_eq!(
        schema["command_protocol"]["supported_operation_versions"],
        serde_json::json!([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])
    );
    assert_eq!(
        schema["command_protocol"]["selection_operations_version"],
        4
    );
    assert_eq!(
        schema["command_protocol"]["crop_to_selection_operations_version"],
        5
    );
    assert_eq!(
        schema["command_protocol"]["color_selection_operations_version"],
        6
    );
    assert_eq!(schema["command_protocol"]["path_operations_version"], 7);
    assert_eq!(
        schema["command_protocol"]["document_lifecycle_operations_version"],
        10
    );
    assert_eq!(
        schema["command_protocol"]["raster_pixel_mask_operations_version"],
        11
    );
    assert_eq!(schema["paths"]["geometry_version"], 1);
    assert_eq!(
        schema["layer_transfer"]["version"],
        prism_core::LAYER_TRANSFER_VERSION
    );
    assert!(schema["gui_interactions"]["brush"].is_string());
    assert!(schema["gui_interactions"]["eraser"].is_string());
    assert!(schema["selection"].is_object());
    assert!(schema["selection"]["crop"].is_string());
    assert!(schema["selection"]["delete"].is_string());
    assert!(
        examples
            .iter()
            .any(|example| example["command"] == "crop_to_selection")
    );
    assert!(schema["typography"].is_object());
    assert!(schema["typography"]["subset_plan"].is_string());
    assert!(schema["typography"]["optimization_analysis"].is_string());
    assert!(schema["typography"]["optimization_limitations"].is_string());
    assert!(schema["typography"]["embedding_metadata"].is_string());
    let embedding_policy = schema["typography"]["embedding_metadata"].as_str().unwrap();
    assert!(embedding_policy.contains("restricted"));
    assert!(embedding_policy.contains("import directly"));
    assert!(embedding_policy.contains("optimized-copy limitations"));
    assert!(embedding_policy.contains("original bytes remain immutable"));
    assert!(schema["typography"]["editable_default"].is_string());
    assert!(schema["typography"]["source_snapshot"].is_string());
    let insert = examples
        .iter()
        .find(|example| example["command"] == "insert_layer")
        .unwrap();
    assert!(matches!(
        serde_json::from_value::<Command>(insert.clone()).unwrap(),
        Command::InsertLayer { .. }
    ));
}
