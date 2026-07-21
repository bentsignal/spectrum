use serde_json::{Value, json};

pub(super) fn schema() -> Value {
    let command_examples = command_examples();
    json!({
        "ok": true,
        "application": "Prism",
        "project_extension": ".prism",
        "legacy_project_extensions": [".mica"],
        "project_storage": {
            "container": "single transactional SQLite .prism file",
            "persistence": "each completed semantic action is an attributed durable revision",
            "batching": "run arrays commit atomically as one revision",
            "history": "immutable revision tree with session-specific cursors",
            "assets": "embedded and content-addressed"
        },
        "agent_collaboration": {
            "transport": "CLI JSON; no vendor-specific integration required",
            "start": "prism --project <path> agent start --mode <together|separate> --name <agent>",
            "continue": "pass the returned --session value to every list, edit, run, and export command",
            "status": "prism --project <path> --session <id> agent status",
            "together": "starts at the current human cursor; Prism follows until the human makes a competing edit, then both sessions continue separately",
            "separate": "starts at the current human cursor and never moves the human session",
            "agent_prompt": "before starting, ask whether the user wants to continue together or explore separately"
        },
        "command_protocol": {
            "encoding": "serde tagged JSON",
            "tag": "command",
            "examples": command_examples
        },
        "gui_interactions": {
            "rotate_focused_object": "Option-R on macOS or Alt-R on Windows/Linux arms the next canvas drag; Shift snaps the absolute angle to 15-degree increments; Escape cancels",
            "move_with_smart_guides": "Move gestures snap transformed edges and centers to the canvas, persistent guides, and other visible layers; the document Snap toggle controls this behavior",
            "drag_guides": "Persistent horizontal and vertical guides can be added numerically, then dragged directly on the canvas"
        },
        "alignment": {
            "cli": "prism align <id> <left|horizontal-center|right|top|vertical-center|bottom> [--to-layer <id>]",
            "geometry": "alignment and snapping use actual rotated visual bounds in canvas coordinates"
        },
        "blend_modes": [
            "normal", "darken", "multiply", "color_burn", "linear_burn", "darker_color",
            "lighten", "screen", "color_dodge", "linear_dodge", "lighter_color", "overlay",
            "soft_light", "hard_light", "vivid_light", "linear_light", "pin_light", "hard_mix",
            "difference", "exclusion", "subtract", "divide", "hue", "saturation", "color",
            "luminosity"
        ],
        "layer_types": ["raster", "text", "rectangle", "ellipse"],
        "layer_styles": {
            "drop_shadow": "shadow <layer> [--x <px>] [--y <px>] [--blur <px>] [--color <RRGGBBAA>] [--clear]",
            "shape_gradient": "gradient <shape> [--angle <degrees>] [--start <RRGGBBAA>] [--end <RRGGBBAA>] [--clear]",
            "rendering": "portable CPU export and exact interactive composite preview share the same fixed-kernel shadow and shape sampler"
        },
        "typography": {
            "portable_fonts": "font-import embeds permitted OpenType font bytes as content-addressed project assets",
            "discovery": "font-list --query <text> searches embedded family and style metadata",
            "optimization_analysis": "font-usage [--font-id <id>] reports deterministic Unicode cmap subset-retention requirements, variation sequences, embedding metadata, provenance, and source size without changing font bytes; --session retains standard session-resume behavior",
            "optimization_limitations": "analysis excludes symbol and other non-Unicode cmaps, shaping, renderer fallback, and legal license conclusions",
            "embedding_metadata": "OS/2 embedding bits can allow or disallow subsetting technically; users must verify the font license",
            "editable_default": "complete imported font bytes remain embedded so portable projects can introduce new characters in later edits",
            "selection": "typography <layer> accepts --font-id or --family with optional --weight and --style",
            "paragraph": ["multiline", "wrap", "left/center/right alignment", "line height", "tracking"],
            "effects": ["outline", "offset shadow"]
        },
        "layer_transfer": {
            "format": "spectrum.prism.layer",
            "version": 2,
            "scope": "exactly one layer; document-local layer and embedded-font IDs are remapped on insertion",
            "copy": "prism --project <source> layer-copy [<id>] --output <new-transfer.json>",
            "paste": "prism --project <destination> layer-paste <transfer.json> [--index <bottom-to-top-index>]",
            "assets": "referenced raster and OpenType bytes are embedded by the destination durable revision",
            "history": "layer-paste inserts and selects the new layer as one undoable revision"
        },
        "color": "RRGGBB or RRGGBBAA",
        "coordinates": "canvas pixels; guides use canvas pixels; layer masks are normalized 0..1"
    })
}

fn command_examples() -> Vec<Value> {
    vec![
        json!({"command": "add_text", "text": "Hello", "name": null, "font_size": 72.0, "color": [255,255,255,255], "x": 100.0, "y": 120.0}),
        json!({"command": "import_font", "path": "/fonts/Inter-Regular.ttf"}),
        json!({"command": "set_text_typography", "id": 1, "typography": {"font_id": 1, "alignment": "center", "line_height": 1.3, "tracking": 2.0, "box_width": 480.0, "effects": {"outline_width": 1.0, "outline_color": [0,0,0,255], "shadow_offset_x": 4.0, "shadow_offset_y": 6.0, "shadow_color": [0,0,0,128]}}}),
        json!({"command": "insert_layer", "transfer": {"format": "spectrum.prism.layer", "version": 1, "layer": {"id": 0, "name": "Card", "visible": true, "locked": false, "opacity": 1.0, "blend_mode": "normal", "transform": {}, "adjustments": {}, "mask": {}, "stroke": {}, "clip_to_below": false, "kind": {"type": "rectangle", "width": 320, "height": 180, "color": [174,123,255,255], "corner_radius": 24.0}}}}),
        json!({"command": "add_ellipse", "name": "Badge", "width": 320, "height": 320, "color": [247,178,102,255], "x": 100.0, "y": 120.0}),
        json!({"command": "set_shape_stroke", "id": 1, "stroke": {"enabled": true, "width": 6.0, "color": [255,255,255,255]}}),
        json!({"command": "set_shape_fill", "id": 1, "fill": {"type": "gradient", "kind": "linear", "angle": 30.0, "stops": [{"position": 0.0, "color": [93,216,199,255]}, {"position": 1.0, "color": [174,123,255,255]}]}}),
        json!({"command": "set_layer_style", "id": 1, "style": {"drop_shadow": {"color": [0,0,0,160], "offset_x": 12.0, "offset_y": 12.0, "blur_radius": 10.0}}}),
        json!({"command": "rasterize_shape", "id": 1, "path": "/generated/shape.png", "scale": 2.0}),
        json!({"command": "set_transform", "id": 1, "transform": {"x": 220.0, "y": 160.0, "scale_x": 1.2, "scale_y": 1.2, "rotation": 8.0}}),
        json!({"command": "set_rotation", "id": 1, "degrees": 15.0}),
        json!({"command": "align_layer", "id": 1, "alignment": "horizontal_center", "reference": {"kind": "canvas"}}),
        json!({"command": "set_snapping", "enabled": true}),
        json!({"command": "add_guide", "orientation": "vertical", "position": 960.0}),
        json!({"command": "move_guide", "id": 1, "position": 800.0}),
        json!({"command": "set_mask", "id": 1, "mask": {"enabled": true, "x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8, "invert": false}}),
        json!({"command": "adjust_layer", "id": 1, "patch": {"exposure": 0.5, "contrast": 12.0}}),
    ]
}
