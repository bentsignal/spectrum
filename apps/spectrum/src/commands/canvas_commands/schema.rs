use serde_json::{Value, json};

pub(super) fn schema() -> Value {
    let command_examples = command_examples();
    json!({
        "ok": true,
        "application": "Spectrum canvas",
        "targeting": "spectrum canvas --asset <UUID> <command>; spectrum canvas new creates a canvas asset",
        "storage": {
            "document": "each canvas is one .spectrum document in the library",
            "history": "every edit is an attributed revision; undo, redo, and branches are kept",
            "batching": "run arrays apply as one revision",
            "files": "images and fonts a canvas uses are embedded by content"
        },
        "command_protocol": {
            "encoding": "serde tagged JSON",
            "tag": "command",
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
            "cli": "spectrum canvas align <id> <left|horizontal-center|right|top|vertical-center|bottom> [--to-layer <id>]",
            "geometry": "alignment and snapping use actual rotated visual bounds in canvas coordinates"
        },
        "blend_modes": [
            "normal", "dissolve", "darken", "multiply", "color_burn", "linear_burn", "darker_color",
            "lighten", "screen", "color_dodge", "linear_dodge", "lighter_color", "overlay",
            "soft_light", "hard_light", "vivid_light", "linear_light", "pin_light", "hard_mix",
            "difference", "exclusion", "subtract", "divide", "hue", "saturation", "color",
            "luminosity"
        ],
        "dissolve": {
            "cli": "spectrum canvas blend <layer> dissolve [--seed <u32>]",
            "seed": "persisted per layer; defaults to zero and can be changed independently with set_dissolve_seed",
            "sampling": "source/mask/clip UNORM8 and once-rounded layer-opacity UNORM16 are combined by rounded integer UNORM multiplication; an integer-only hash of seed and absolute scaled-document x/y turns coverage into present/absent pixels",
            "kept_pixels": "effective alpha is the presence probability; present pixels composite fully opaque, matching conventional Dissolve semantics",
            "parity": "export, tiled region rendering, and composite preview use the same core function"
        },
        "future_group_compositing": {
            "status": "semantic contract only; groups are not yet a canvas layer type",
            "default": "pass_through",
            "isolated": "children composite onto a transparent intermediate, then group opacity and blend mode apply once to that result",
            "pass_through": "children composite directly into the parent stack; group opacity scales each child effective opacity and the group has no independent blend operation",
            "dissolve": "an isolated group hashes its own seed at the group boundary; a pass-through group does not introduce a Dissolve boundary"
        },
        "layer_types": ["raster", "text", "rectangle", "ellipse", "path", "paint"],
        "paint": {
            "program_version": spectrum_canvas::BRUSH_PROGRAM_VERSION,
            "cli": "paint add-layer --width <px> --height <px>; paint stroke <layer> <stroke.json> [--no-selection]; paint clone-source <raster-layer> <document-x> <document-y>; paint clone-stroke <paint-layer> <stroke.json> [--no-selection]; paint erase <any-layer> <canvas-space-stroke.json>",
            "modes": ["paint", "erase", "clone_stamp"],
            "clone_stamp": "clone-source inverse-maps one document point into a Raster layer and interns its exact embedded bytes, identity, dimensions, transform, non-geometric Develop adjustments, pixel mask, and vector mask once in the document registry; every clone-stroke references that stable source identity with a frozen destination-to-source affine mapping, samples transparent outside it, and clips only its destination against the current selection",
            "pressure_v1": "pressure multiplies both dab diameter and coverage; mouse input records 1.0",
            "rendering": "ordered source-over Paint and destination-out Erase strokes share one global-coordinate tiled CPU sampler across preview, region render, and export",
            "selection": "the current rectangular or soft-alpha selection is baked into Paint-local stroke clip coordinates when the stroke commits",
            "geometric_adjustments_v1": "Brush and Eraser fail closed on Paint layers with rotation, flips, straighten, or crop adjustments; reset those adjustments before painting and reapply them afterward",
            "mask_order": "stroke clip, Paint pixel mask, adjustments, vector mask, then outer rectangular mask/clipping/shadow",
            "limits": {
                "samples_per_stroke": spectrum_canvas::MAX_BRUSH_SAMPLES_PER_STROKE,
                "strokes_per_layer": spectrum_canvas::MAX_BRUSH_STROKES_PER_LAYER,
                "samples_per_document": spectrum_canvas::MAX_BRUSH_SAMPLES_PER_DOCUMENT,
                "dabs_per_stroke": spectrum_canvas::MAX_BRUSH_DABS_PER_STROKE,
                "dabs_per_program_and_document": spectrum_canvas::MAX_BRUSH_DABS_PER_PROGRAM,
                "clip_bytes_per_program": spectrum_canvas::MAX_BRUSH_CLIP_BYTES_PER_PROGRAM,
                "requested_region_pixels": spectrum_canvas::MAX_PAINT_REGION_PIXELS
            },
            "history": "one completed pointer drag is one AddBrushStroke revision; a first drag uses atomic AddPaintLayerWithStroke"
        },
        "paths": {
            "geometry_version": spectrum_canvas::PATH_GEOMETRY_VERSION,
            "anchor_limit": spectrum_canvas::MAX_PATH_ANCHORS,
            "geometry": "explicit local viewport with bounded cubic anchors and relative incoming/outgoing control handles; closed paths use even-odd fill",
            "cli": "path add <geometry.json> [--name <label>] [--color <RRGGBBAA>] [--x <px>] [--y <px>]; path replace <id> <geometry.json>",
            "history": "a finished creation or completed anchor/control-point drag is one durable command and revision"
        },
        "vector_masks": {
            "cli": "vector-mask <layer> <closed-geometry.json> [--invert] or vector-mask <layer> --clear",
            "fitting": "the path viewport is normalized and independently stretched to the complete target layer source width and height",
            "rendering": "closed nondegenerate fill alpha is applied after source adjustments and before layer transform, shadow, rectangular mask, and clipping",
            "reuse": "the same immutable PathGeometry value can back a path layer and any number of vector masks",
            "painted": "selection delete <layer> and paint erase <layer> keep an alpha on non-image layers' vector masks, stretched over the layer and multiplied after the path, so text and shapes stay editable; image layers hide selections in source pixels"
        },
        "layer_styles": {
            "drop_shadow": "shadow <layer> [--x <px>] [--y <px>] [--blur <px>] [--color <RRGGBBAA>] [--clear]",
            "effects": "effect <layer> <stroke|outer-glow|inner-glow|inner-shadow|color-overlay|gradient-overlay|satin|bevel> [--color <RRGGBBAA>] [--size <px>] [--spread <0..1>] [--position <outside|inside|center>] [--x <px>] [--y <px>] [--angle <deg>] [--distance <px>] [--depth <n>] [--altitude <deg>] [--bevel-style <inner|outer|emboss|pillow>] [--down <bool>] [--highlight <RRGGBBAA>] [--shadow-color <RRGGBBAA>] [--invert <bool>] [--mode <blend>] [--stop <POSITION:RRGGBBAA>]... [--gradient-kind <linear|radial|angle>] [--center-x <0..1>] [--center-y <0..1>] [--radius <n>] [--scale <n>] [--clear]; unset flags keep the layer's current values and other styles stay",
            "shape_gradient": "gradient <shape> (--gradient-json <bounded strict object> | [--kind <linear|radial|angle>] [--angle <degrees>] [--spread <pad|repeat|reflect>] [--center-x <0..1>] [--center-y <0..1>] [--radius <positive normal>] [--offset <finite>] [--extent <positive normal>] (--stop <POSITION:RRGGBBAA> repeated 2..32 times | legacy --start <RRGGBBAA> and/or --end <RRGGBBAA>) | --clear); structured JSON, modern --stop, legacy endpoints, and --clear are mutually exclusive surfaces",
            "gradient_contract": {
                "model": "spectrum_shape_gradient_v1",
                "interpolation": "premultiplied_srgb_v1",
                "structured_json_max_bytes": 16384,
                "strict_json": "unknown and duplicate gradient or stop keys are rejected",
                "geometry": "Radial and Angle use a source-local pixel metric; Linear preserves normalized-box legacy projection; angle zero begins at positive X; offset defaults to 0 and extent defaults to 1",
                "transparent_color": "modern fully transparent stops canonicalize hidden RGB to zero; grandfathered legacy two-stop Linear bytes remain unchanged"
            },
            "rendering": "portable CPU export and exact interactive composite preview share the same fixed-kernel shadow and shape sampler"
        },
        "selection": {
            "rectangle": "selection rectangle <x> <y> <width> <height> uses integer document pixels and clips at canvas edges",
            "magic_wand": "selection magic-wand <x> <y> [--tolerance <0..255>] [--noncontiguous] [--no-antialias] defaults tolerance to 20 and samples the exact CPU composite; tolerance is deterministic max-channel distance over premultiplied RGBA (hidden RGB at alpha 0 is ignored, alpha differences remain visible), and anti-aliasing adds one soft boundary pixel",
            "lasso": "selection lasso --point <x,y> --point <x,y> --point <x,y> [--mode <replace|add|subtract|intersect>] [--no-antialias] quantizes coordinates to 1/256 pixel and applies a deterministic even-odd polygon selection",
            "lasso_limits": {"input_points": spectrum_canvas::MAX_LASSO_INPUT_POINTS, "simplified_vertices": spectrum_canvas::MAX_LASSO_VERTICES, "raster_edge_tests": spectrum_canvas::MAX_LASSO_RASTER_EDGE_TESTS, "mask_pixels": spectrum_canvas::MAX_COLOR_SELECTION_PIXELS},
            "clear": "selection clear removes the persistent pixel selection",
            "crop": "selection crop atomically crops the canvas to the current selection and clears it in one revision",
            "fill": "selection fill [--color <RRGGBBAA>] [--name <label>] creates one new editable solid layer honoring rectangular or soft color-selection alpha without changing source pixels",
            "delete": "selection delete <layer> hides what the active selection covers on any layer: raster layers multiply it into their pixel mask, other kinds into a painted vector-mask alpha; originals remain immutable, the selection stays active, and empty, locked, already-deleted, or non-overlapping targets fail without a revision",
            "combination": "replace uses the new lasso; add uses a+b-round(ab/255); subtract uses round(a*(255-b)/255); intersect uses round(ab/255)",
            "history": "each completed Selection-tool drag, lasso drag, magic wand click, clear, fill, delete, or crop is one command and one durable revision"
        },
        "typography": {
            "layout_engines": ["legacy-v1", "harfbuzz-v1"],
            "new_text_default": "harfbuzz-v1; existing text with no shaping field remains legacy-v1 pixel-exact",
            "cli": "add-text accepts --layout <legacy-v1|harfbuzz-v1> [--language <BCP47>]; typography <layer> upgrades or changes the same durable policy",
            "harfbuzz_v1_policy": {
                "engine": "bundled in-process HarfBuzz 8.2.2 with default OpenType features and default variation axes",
                "unicode_data": {"bidi": "16.0", "line_break": "15.0", "grapheme": "17.0", "script": "17.0"},
                "language": "canonical BCP-47; omitted or und resolves deterministically to und",
                "fallback": "whole extended grapheme clusters resolve against the exact primary font snapshot, then bundled Ubuntu Light; operating-system fonts and locale are never consulted",
                "rasterization": "glyph IDs are rasterized from indexed ttf-parser outlines through tiny-skia"
            },
            "portable_fonts": "font-import binds a bounded no-follow regular-file snapshot and transactionally embeds those exact bytes as a content-addressed project asset; installable, editable, preview/print, and restricted embedding classes, including bitmap-only flags, import directly for local text, while malformed, unparseable, oversized, or unsafe sources fail closed; Windows final-handle proof rejects junction and 8.3 aliases unless the normalized handle path exactly matches",
            "discovery": "font-list --query <text> searches embedded family and style metadata",
            "optimization_analysis": "font-usage [--font-id <id>] reports deterministic Unicode cmap subset-retention requirements, variation sequences, embedding metadata, provenance, and source size without changing font bytes",
            "optimization_limitations": "analysis excludes symbol and other non-Unicode cmaps, shaping, and renderer fallback",
            "embedding_metadata": "font-import, font-list, and font-usage report the decoded OS/2 embedding class and technical subsetting result; preview/print and restricted classes, including bitmap-only flags, import directly for local text; malformed permissions fail closed and original bytes remain immutable",
            "editable_default": "complete imported font bytes remain embedded as the immutable source snapshot so portable projects can introduce new characters in later edits",
            "selection": "typography <layer> accepts --font-id or --family with optional --weight and --style",
            "paragraph": ["multiline", "wrap", "left/center/right alignment", "line height", "tracking"],
            "effects": ["outline", "offset shadow"]
        },
        "layer_transfer": {
            "format": "spectrum.canvas.layer",
            "version": spectrum_canvas::LAYER_TRANSFER_VERSION,
            "scope": "exactly one layer; document-local layer and embedded-font IDs are remapped on insertion",
            "copy": "spectrum canvas --asset <source> layer-copy [<id>] --output <new-transfer.json>",
            "paste": "spectrum canvas --asset <destination> layer-paste <transfer.json> [--index <bottom-to-top-index>]",
            "assets": "referenced raster and OpenType bytes are embedded by the destination revision; every layer kind, mask, style, gradient, and Clone Stamp source transfers",
            "history": "layer-paste inserts and selects the new layer as one undoable revision"
        },
        "color": "RRGGBB or RRGGBBAA",
        "coordinates": "canvas pixels; guides use canvas pixels; layer masks are normalized 0..1"
    })
}

fn command_examples() -> Vec<Value> {
    vec![
        json!({"command": "rename_document", "name": "Campaign"}),
        json!({"command": "set_dissolve_seed", "id": 1, "seed": 305419896}),
        json!({"command": "add_text", "text": "Hello", "name": null, "font_size": 72.0, "color": [255,255,255,255], "x": 100.0, "y": 120.0}),
        json!({"command": "import_font", "path": "/fonts/Inter-Regular.ttf"}),
        json!({"command": "set_text_typography", "id": 1, "typography": {"font_id": 1, "alignment": "center", "line_height": 1.3, "tracking": 2.0, "box_width": 480.0, "effects": {"outline_width": 1.0, "outline_color": [0,0,0,255], "shadow_offset_x": 4.0, "shadow_offset_y": 6.0, "shadow_color": [0,0,0,128]}}}),
        json!({"command": "insert_layer", "transfer": {"format": "spectrum.canvas.layer", "version": 1, "layer": {"id": 0, "name": "Card", "visible": true, "locked": false, "opacity": 1.0, "blend_mode": "normal", "transform": {}, "adjustments": {}, "mask": {}, "stroke": {}, "clip_to_below": false, "kind": {"type": "rectangle", "width": 320, "height": 180, "color": [174,123,255,255], "corner_radius": 24.0}}}}),
        json!({"command": "add_ellipse", "name": "Badge", "width": 320, "height": 320, "color": [247,178,102,255], "x": 100.0, "y": 120.0}),
        json!({"command": "add_path", "name": "Curve", "geometry": {"version": 1, "width": 320, "height": 240, "closed": false, "fill_rule": "even_odd", "anchors": [{"point": [20.0,200.0]}, {"point": [160.0,20.0], "handle_in": [-80.0,0.0], "handle_out": [80.0,0.0]}, {"point": [300.0,200.0]}]}, "color": [255,255,255,255], "x": 100.0, "y": 120.0}),
        json!({"command": "add_paint_layer_with_stroke", "name": "Paint", "width": 1920, "height": 1080, "stroke": {"style": {"mode": "paint", "color": [255,255,255,255], "size": 32.0, "hardness": 0.8, "opacity": 1.0, "spacing": 0.15}, "samples": [{"x": 120.0, "y": 80.0, "pressure": 1.0}]}, "selection": {"source": "current"}}),
        json!({"command": "add_brush_stroke", "id": 1, "stroke": {"style": {"mode": "erase", "color": [255,255,255,255], "size": 48.0, "hardness": 0.7, "opacity": 0.6, "spacing": 0.12}, "samples": [{"x": 160.0, "y": 120.0, "pressure": 1.0}, {"x": 240.0, "y": 160.0, "pressure": 0.75}]}, "selection": {"source": "none"}}),
        json!({"command": "set_clone_source", "id": 2, "document_x": 240.0, "document_y": 160.0}),
        json!({"command": "add_brush_stroke", "id": 1, "stroke": {"style": {"mode": "clone_stamp", "color": [0,0,0,0], "size": 48.0, "hardness": 0.7, "opacity": 0.6, "spacing": 0.12}, "samples": [{"x": 160.0, "y": 120.0, "pressure": 1.0}], "source": {"type": "current_clone"}}, "selection": {"source": "current"}}),
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
        json!({"command": "delete_selected_pixels", "id": 1}),
        json!({"command": "crop_to_selection"}),
        json!({"command": "add_guide", "orientation": "vertical", "position": 960.0}),
        json!({"command": "move_guide", "id": 1, "position": 800.0}),
        json!({"command": "set_mask", "id": 1, "mask": {"enabled": true, "x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8, "invert": false}}),
        json!({"command": "adjust_layer", "id": 1, "patch": {"exposure": 0.5, "contrast": 12.0}}),
        json!({"command": "set_layer_adjustments", "id": 1, "adjustments": {"exposure": 0.5, "curves": {"master": {"points": [{"x": 0.0, "y": 0.0}, {"x": 0.5, "y": 0.6}, {"x": 1.0, "y": 1.0}]}}}}),
    ]
}
