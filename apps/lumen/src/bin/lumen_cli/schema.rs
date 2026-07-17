use super::*;

pub(super) fn schema() -> serde_json::Value {
    json!({
        "ok": true,
        "catalog_version": lumen_core::project::CATALOG_VERSION,
        "output": "JSON on stdout; structured errors on stderr; nonzero exit on failure",
        "adjustments": {
            "exposure": { "range": [-5.0, 5.0], "unit": "stops", "default": 0.0 },
            "temperature": { "range": [-100, 100], "default": 0 },
            "tint": { "range": [-100, 100], "default": 0 },
            "contrast": { "range": [-100, 100], "default": 0 },
            "highlights": { "range": [-100, 100], "default": 0 },
            "shadows": { "range": [-100, 100], "default": 0 },
            "whites": { "range": [-100, 100], "default": 0 },
            "blacks": { "range": [-100, 100], "default": 0 },
            "texture": { "range": [-100, 100], "default": 0 },
            "clarity": { "range": [-100, 100], "default": 0 },
            "dehaze": { "range": [-100, 100], "default": 0 },
            "vibrance": { "range": [-100, 100], "default": 0 },
            "saturation": { "range": [-100, 100], "default": 0 },
            "vignette": { "range": [-100, 100], "default": 0 },
            "sharpening": { "range": [0, 100], "default": 0 },
            "noise_reduction": { "range": [0, 100], "default": 0 },
            "crop": { "type": "normalized rectangle", "fields": ["x", "y", "width", "height"] },
            "straighten": { "range": [-45, 45], "unit": "degrees" },
            "hsl": { "colors": ["red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta"], "range": [-100, 100] },
            "curves": { "channels": ["master", "red", "green", "blue"], "points": "normalized x,y pairs" },
            "color_grading": { "ranges": ["shadows", "midtones", "highlights"], "hue": [0, 360], "saturation": [0, 100], "luminance": [-100, 100], "balance": [-100, 100] },
            "spots": { "type": "normalized repair dabs", "fields": ["x", "y", "radius", "opacity"] },
            "rotation": { "values": [0, 90, 180, 270], "unit": "degrees clockwise" },
            "flip_horizontal": { "type": "boolean" },
            "flip_vertical": { "type": "boolean" }
        },
        "raw_command_examples": [
            { "command": "adjust", "id": 1, "patch": { "exposure": 0.7, "shadows": 18 } },
            { "command": "copy-edits", "id": 1 },
            { "command": "paste-edits", "ids": [2, 3] },
            { "command": "history-back", "id": 1 },
            { "command": "set-pick", "ids": [1, 2], "state": "keep" },
            { "command": "rename-batch", "id": 1, "name": "Night walk" },
            { "command": "save-preset", "name": "Warm portrait", "from_id": 1 },
            { "command": "apply-preset", "preset_id": 1, "ids": [2, 3] },
            { "command": "export-batch", "ids": [1, 2], "directory": "finished", "format": "jpeg", "max_size": 3000, "quality": 90 },
            { "command": "export", "id": 1, "path": "output.jpg", "max_size": 2400, "quality": 92 }
        ]
    })
}

#[allow(dead_code)]
pub(super) fn _assert_adjustments_are_public(_: Adjustments) {}
