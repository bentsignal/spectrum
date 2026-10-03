//! Preview preferences kept between launches, beside the library: the color
//! format every color picker shows.
use crate::color_text;
use serde_json::{Map, Value};
use std::path::PathBuf;

fn path() -> Option<PathBuf> {
    let root = spectrum::library::default_root().ok()?;
    Some(root.parent()?.join("preview-settings.json"))
}

fn read() -> Map<String, Value> {
    path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Restores saved preferences at startup.
pub fn load() {
    let prefs = read();
    if let Some(format) = prefs
        .get("color_format")
        .and_then(Value::as_str)
        .and_then(color_text::format_named)
    {
        color_text::set_preferred(format);
    }
}

/// Saves the current color format; a failed write only loses the preference.
pub fn save_color_format() {
    let Some(path) = path() else {
        return;
    };
    let mut prefs = read();
    let name = color_text::name(color_text::preferred());
    prefs.insert("color_format".into(), Value::from(name));
    if let Ok(json) = serde_json::to_vec_pretty(&prefs) {
        let _ = std::fs::write(path, json);
    }
}
