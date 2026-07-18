pub(crate) fn default_text_layer_name(text: &str) -> String {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or("Text")
        .chars()
        .take(24)
        .collect()
}
