use serde_json::{Value, json};

pub(super) fn schema() -> Value {
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
            "examples": [
                {"command": "add_text", "text": "Hello", "name": null, "font_size": 72.0, "color": [255,255,255,255], "x": 100.0, "y": 120.0},
                {"command": "import_font", "path": "/fonts/Inter-Regular.ttf"},
                {"command": "set_text_typography", "id": 1, "typography": {"font_id": 1, "alignment": "center", "line_height": 1.3, "tracking": 2.0, "box_width": 480.0, "effects": {"outline_width": 1.0, "outline_color": [0,0,0,255], "shadow_offset_x": 4.0, "shadow_offset_y": 6.0, "shadow_color": [0,0,0,128]}}},
                {"command": "add_ellipse", "name": "Badge", "width": 320, "height": 320, "color": [247,178,102,255], "x": 100.0, "y": 120.0},
                {"command": "set_shape_stroke", "id": 1, "stroke": {"enabled": true, "width": 6.0, "color": [255,255,255,255]}},
                {"command": "rasterize_shape", "id": 1, "path": "/generated/shape.png", "scale": 2.0},
                {"command": "set_transform", "id": 1, "transform": {"x": 220.0, "y": 160.0, "scale_x": 1.2, "scale_y": 1.2, "rotation": 8.0}},
                {"command": "set_rotation", "id": 1, "degrees": 15.0},
                {"command": "align_layer", "id": 1, "alignment": "horizontal_center", "reference": {"kind": "canvas"}},
                {"command": "set_snapping", "enabled": true},
                {"command": "add_guide", "orientation": "vertical", "position": 960.0},
                {"command": "move_guide", "id": 1, "position": 800.0},
                {"command": "set_mask", "id": 1, "mask": {"enabled": true, "x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8, "invert": false}},
                {"command": "adjust_layer", "id": 1, "patch": {"exposure": 0.5, "contrast": 12.0}}
            ]
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
        "typography": {
            "portable_fonts": "font-import embeds permitted OpenType font bytes as content-addressed project assets",
            "discovery": "font-list --query <text> searches embedded family and style metadata",
            "selection": "typography <layer> accepts --font-id or --family with optional --weight and --style",
            "paragraph": ["multiline", "wrap", "left/center/right alignment", "line height", "tracking"],
            "effects": ["outline", "offset shadow"]
        },
        "color": "RRGGBB or RRGGBBAA",
        "coordinates": "canvas pixels; guides use canvas pixels; layer masks are normalized 0..1"
    })
}
