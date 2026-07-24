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
            "operations_family": "spectrum.prism.commands",
            "supported_operation_versions": (1..=prism_core::PRISM_COMMAND_OPERATIONS_VERSION).collect::<Vec<_>>(),
            "selection_operations_version": 4,
            "crop_to_selection_operations_version": 5,
            "color_selection_operations_version": 6,
            "path_operations_version": 7,
            "paint_operations_version": 8,
            "lasso_operations_version": prism_core::PRISM_COMMAND_OPERATIONS_VERSION,
            "examples": command_examples
        },
        "gui_interactions": {
            "rotate_focused_object": "Option-R on macOS or Alt-R on Windows/Linux arms the next canvas drag; Shift snaps the absolute angle to 15-degree increments; Escape cancels",
            "move_with_smart_guides": "Move gestures snap transformed edges and centers to the canvas, persistent guides, and other visible layers; the document Snap toggle controls this behavior",
            "drag_guides": "Persistent horizontal and vertical guides can be added numerically, then dragged directly on the canvas",
            "pen": "Pen clicks create editable anchors; dragging creates paired cubic handles; Enter finishes an open path, clicking the first anchor closes it, and Escape cancels the local draft",
            "brush": "B selects Brush; a canvas drag previews the exact core renderer and commits one nondestructive stroke revision on release; Escape cancels the local draft",
            "eraser": "E selects Eraser; a canvas drag applies destination-out marks to the selected unlocked Paint layer and commits one nondestructive stroke revision on release",
            "lasso": "L selects Lasso; one bounded freehand drag previews locally and commits exactly one fixed-point selection revision on release; Escape cancels"
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
        "layer_types": ["raster", "text", "rectangle", "ellipse", "path", "paint"],
        "paint": {
            "program_version": prism_core::BRUSH_PROGRAM_VERSION,
            "cli": "paint add-layer --width <px> --height <px>; paint stroke <layer> <stroke.json> [--no-selection]",
            "modes": ["paint", "erase"],
            "pressure_v1": "pressure multiplies both dab diameter and coverage; mouse input records 1.0",
            "rendering": "ordered source-over Paint and destination-out Erase strokes share one global-coordinate tiled CPU sampler across preview, region render, and export",
            "selection": "the current rectangular or soft-alpha selection is baked into Paint-local stroke clip coordinates when the stroke commits",
            "geometric_adjustments_v1": "Brush and Eraser fail closed on Paint layers with rotation, flips, straighten, or crop adjustments; reset those adjustments before painting and reapply them afterward",
            "mask_order": "stroke clip, Paint pixel mask, adjustments, vector mask, then outer rectangular mask/clipping/shadow",
            "limits": {
                "samples_per_stroke": prism_core::MAX_BRUSH_SAMPLES_PER_STROKE,
                "strokes_per_layer": prism_core::MAX_BRUSH_STROKES_PER_LAYER,
                "samples_per_document": prism_core::MAX_BRUSH_SAMPLES_PER_DOCUMENT,
                "dabs_per_stroke": prism_core::MAX_BRUSH_DABS_PER_STROKE,
                "dabs_per_program_and_document": prism_core::MAX_BRUSH_DABS_PER_PROGRAM,
                "clip_bytes_per_program": prism_core::MAX_BRUSH_CLIP_BYTES_PER_PROGRAM,
                "requested_region_pixels": prism_core::MAX_PAINT_REGION_PIXELS
            },
            "history": "one completed pointer drag is one AddBrushStroke revision; a first drag uses atomic AddPaintLayerWithStroke"
        },
        "paths": {
            "geometry_version": prism_core::PATH_GEOMETRY_VERSION,
            "anchor_limit": prism_core::MAX_PATH_ANCHORS,
            "geometry": "explicit local viewport with bounded cubic anchors and relative incoming/outgoing control handles; closed paths use even-odd fill",
            "cli": "path add <geometry.json> [--name <label>] [--color <RRGGBBAA>] [--x <px>] [--y <px>]; path replace <id> <geometry.json>",
            "history": "a finished creation or completed anchor/control-point drag is one durable command and revision"
        },
        "vector_masks": {
            "cli": "vector-mask <layer> <closed-geometry.json> [--invert] or vector-mask <layer> --clear",
            "fitting": "the path viewport is normalized and independently stretched to the complete target layer source width and height",
            "rendering": "closed nondegenerate fill alpha is applied after source adjustments and before layer transform, shadow, rectangular mask, and clipping",
            "reuse": "the same immutable PathGeometry value can back a path layer and any number of vector masks"
        },
        "layer_styles": {
            "drop_shadow": "shadow <layer> [--x <px>] [--y <px>] [--blur <px>] [--color <RRGGBBAA>] [--clear]",
            "shape_gradient": "gradient <shape> [--angle <degrees>] [--start <RRGGBBAA>] [--end <RRGGBBAA>] [--clear]",
            "rendering": "portable CPU export and exact interactive composite preview share the same fixed-kernel shadow and shape sampler"
        },
        "selection": {
            "rectangle": "selection rectangle <x> <y> <width> <height> uses integer document pixels and clips at canvas edges",
            "magic_wand": "selection magic-wand <x> <y> [--tolerance <0..255>] [--noncontiguous] [--no-antialias] samples the exact CPU composite; tolerance is deterministic max-channel distance over premultiplied RGBA (hidden RGB at alpha 0 is ignored, alpha differences remain visible), and anti-aliasing adds one soft boundary pixel",
            "lasso": "selection lasso --point <x,y> --point <x,y> --point <x,y> [--mode <replace|add|subtract|intersect>] [--no-antialias] quantizes coordinates to 1/256 pixel and applies a deterministic even-odd polygon selection",
            "lasso_limits": {"input_points": prism_core::MAX_LASSO_INPUT_POINTS, "simplified_vertices": prism_core::MAX_LASSO_VERTICES, "raster_edge_tests": prism_core::MAX_LASSO_RASTER_EDGE_TESTS, "mask_pixels": prism_core::MAX_COLOR_SELECTION_PIXELS},
            "clear": "selection clear removes the persistent marquee",
            "crop": "selection crop atomically crops the canvas to the current marquee and clears the selection in one revision",
            "fill": "selection fill [--color <RRGGBBAA>] [--name <label>] creates one new editable solid layer honoring rectangular or soft color-selection alpha without changing source pixels",
            "combination": "replace uses the new lasso; add uses a+b-round(ab/255); subtract uses round(a*(255-b)/255); intersect uses round(ab/255)",
            "history": "each completed marquee, lasso drag, magic wand click, clear, fill, or crop is one command and one durable revision"
        },
        "typography": {
            "portable_fonts": "font-import binds a bounded no-follow regular-file snapshot and transactionally embeds those exact bytes as a content-addressed project asset; installable, editable, preview/print, and restricted embedding classes, including bitmap-only flags, import directly for local text, while malformed, unparseable, oversized, or unsafe sources fail closed; Windows final-handle proof rejects junction and 8.3 aliases unless the normalized handle path exactly matches",
            "source_snapshot": "font-source <font-id> reads one full-font blob directly from an immutable SQLite view that ignores live caches and recovery sidecars, verifies its deterministic SHA-256 identity and embedding metadata, and reports proof without modifying the project; --session is rejected",
            "subset_plan": "font-subset-plan <font-id> immutably replays the current document, derives exact Unicode and per-line shaping requirements, runs the fail-closed in-process candidate in memory, and reports deterministic candidate identity/reduction or blockers without emitting bytes or modifying the project; --session is rejected",
            "subset_storage_decision": "appending a subset cannot shrink a durable project because replayable history retains content-addressed full-font blobs; physical reduction requires a future fresh-database compact-copy transaction that rewrites all retained branches and copies only reachable assets",
            "discovery": "font-list --query <text> searches embedded family and style metadata",
            "optimization_analysis": "font-usage [--font-id <id>] reports deterministic Unicode cmap subset-retention requirements, variation sequences, embedding metadata, provenance, and source size without changing font bytes; --session retains standard session-resume behavior",
            "optimization_limitations": "analysis excludes symbol and other non-Unicode cmaps, shaping, and renderer fallback",
            "embedding_metadata": "font-import, font-list, font-usage, and font-source preserve and report the decoded OS/2 embedding class and technical subsetting result; preview/print and restricted classes, including bitmap-only flags, import directly for local text while action-specific sharing or optimized-copy limitations remain visible; malformed permissions fail closed and original bytes remain immutable",
            "editable_default": "complete imported font bytes remain embedded as the immutable source snapshot so portable projects can introduce new characters in later edits",
            "selection": "typography <layer> accepts --font-id or --family with optional --weight and --style",
            "paragraph": ["multiline", "wrap", "left/center/right alignment", "line height", "tracking"],
            "effects": ["outline", "offset shadow"]
        },
        "layer_transfer": {
            "format": "spectrum.prism.layer",
            "version": prism_core::LAYER_TRANSFER_VERSION,
            "scope": "exactly one layer; document-local layer and embedded-font IDs are remapped on insertion",
            "copy": "prism --project <source> layer-copy [<id>] --output <new-transfer.json>",
            "paste": "prism --project <destination> layer-paste <transfer.json> [--index <bottom-to-top-index>]",
            "assets": "referenced raster and OpenType bytes are embedded by the destination durable revision; v3 preserves bounded shape pixel masks with verified content identity; v4 preserves paths and reusable vector masks; v5 preserves bounded nondestructive Paint programs",
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
        json!({"command": "add_path", "name": "Curve", "geometry": {"version": 1, "width": 320, "height": 240, "closed": false, "fill_rule": "even_odd", "anchors": [{"point": [20.0,200.0]}, {"point": [160.0,20.0], "handle_in": [-80.0,0.0], "handle_out": [80.0,0.0]}, {"point": [300.0,200.0]}]}, "color": [255,255,255,255], "x": 100.0, "y": 120.0}),
        json!({"command": "add_paint_layer_with_stroke", "name": "Paint", "width": 1920, "height": 1080, "stroke": {"style": {"mode": "paint", "color": [255,255,255,255], "size": 32.0, "hardness": 0.8, "opacity": 1.0, "spacing": 0.15}, "samples": [{"x": 120.0, "y": 80.0, "pressure": 1.0}]}, "selection": {"source": "current"}}),
        json!({"command": "add_brush_stroke", "id": 1, "stroke": {"style": {"mode": "erase", "color": [255,255,255,255], "size": 48.0, "hardness": 0.7, "opacity": 0.6, "spacing": 0.12}, "samples": [{"x": 160.0, "y": 120.0, "pressure": 1.0}, {"x": 240.0, "y": 160.0, "pressure": 0.75}]}, "selection": {"source": "none"}}),
        json!({"command": "set_vector_mask", "id": 1, "mask": {"enabled": true, "invert": false, "path": {"version": 1, "width": 100, "height": 100, "closed": true, "fill_rule": "even_odd", "anchors": [{"point": [50.0,0.0]}, {"point": [100.0,100.0]}, {"point": [0.0,100.0]}]}}}),
        json!({"command": "set_shape_stroke", "id": 1, "stroke": {"enabled": true, "width": 6.0, "color": [255,255,255,255]}}),
        json!({"command": "set_shape_fill", "id": 1, "fill": {"type": "gradient", "kind": "linear", "angle": 30.0, "stops": [{"position": 0.0, "color": [93,216,199,255]}, {"position": 1.0, "color": [174,123,255,255]}]}}),
        json!({"command": "set_layer_style", "id": 1, "style": {"drop_shadow": {"color": [0,0,0,160], "offset_x": 12.0, "offset_y": 12.0, "blur_radius": 10.0}}}),
        json!({"command": "rasterize_shape", "id": 1, "path": "/generated/shape.png", "scale": 2.0}),
        json!({"command": "set_transform", "id": 1, "transform": {"x": 220.0, "y": 160.0, "scale_x": 1.2, "scale_y": 1.2, "rotation": 8.0}}),
        json!({"command": "set_rotation", "id": 1, "degrees": 15.0}),
        json!({"command": "align_layer", "id": 1, "alignment": "horizontal_center", "reference": {"kind": "canvas"}}),
        json!({"command": "set_snapping", "enabled": true}),
        json!({"command": "set_selection", "selection": {"type": "rectangle", "x": 120, "y": 80, "width": 640, "height": 360}}),
        json!({"command": "magic_wand_selection", "x": 120, "y": 80, "tolerance": 32, "contiguous": true, "antialias": true}),
        json!({"command": "lasso_selection", "points": [{"x": 30720, "y": 20480}, {"x": 153600, "y": 20480}, {"x": 30720, "y": 102400}], "mode": "replace", "antialias": true}),
        json!({"command": "fill_selection", "color": [93,216,199,255], "name": "Selection fill"}),
        json!({"command": "crop_to_selection"}),
        json!({"command": "add_guide", "orientation": "vertical", "position": 960.0}),
        json!({"command": "move_guide", "id": 1, "position": 800.0}),
        json!({"command": "set_mask", "id": 1, "mask": {"enabled": true, "x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8, "invert": false}}),
        json!({"command": "adjust_layer", "id": 1, "patch": {"exposure": 0.5, "contrast": 12.0}}),
    ]
}
