//! The font list's favorites, which come first, and hidden fonts, which
//! stay out of the way until shown; fonts without Latin letters can be left
//! out too. All of it is kept between launches.
use serde_json::Value;
use std::collections::BTreeSet;

pub struct FontLists {
    pub favorites: BTreeSet<String>,
    pub hidden: BTreeSet<String>,
    /// Leave out fonts that cannot show English text.
    pub latin_only: bool,
    /// Show hidden fonts, to bring them back.
    pub show_hidden: bool,
    /// Show only favorites.
    pub favorites_only: bool,
}

fn names(key: &str) -> BTreeSet<String> {
    crate::prefs::get(key)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

impl FontLists {
    pub fn load() -> Self {
        Self {
            favorites: names("font_favorites"),
            hidden: names("font_hidden"),
            latin_only: crate::prefs::get("font_latin_only")
                .and_then(|value| value.as_bool())
                .unwrap_or(true),
            show_hidden: false,
            favorites_only: crate::prefs::get("font_favorites_only")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
        }
    }

    pub fn save(&self) {
        crate::prefs::set(
            "font_favorites",
            Value::from_iter(self.favorites.iter().cloned()),
        );
        crate::prefs::set("font_hidden", Value::from_iter(self.hidden.iter().cloned()));
        crate::prefs::set("font_latin_only", Value::from(self.latin_only));
        crate::prefs::set("font_favorites_only", Value::from(self.favorites_only));
    }

    /// Adds `name` to `set`, or takes it out.
    pub fn toggle(set: &mut BTreeSet<String>, name: &str) {
        if !set.remove(name) {
            set.insert(name.to_owned());
        }
    }
}
