//! Fonts installed on this computer, for browsing and importing into a
//! canvas with `Command::ImportFont`.
use serde::Serialize;
use std::path::PathBuf;

/// One installed font face.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SystemFont {
    pub family: String,
    /// A readable style name, such as "Regular" or "Bold Italic".
    pub style: String,
    pub weight: u16,
    pub italic: bool,
    pub path: PathBuf,
    /// Has the Latin letters and digits, so English text shows in it; fonts
    /// for other scripts or symbols show boxes for letters they lack.
    pub latin: bool,
}

fn style_name(weight: u16, italic: bool) -> String {
    let weight = match weight {
        0..=150 => "Thin",
        151..=250 => "Extra Light",
        251..=350 => "Light",
        351..=450 => "Regular",
        451..=550 => "Medium",
        551..=650 => "Semibold",
        651..=750 => "Bold",
        751..=850 => "Extra Bold",
        _ => "Black",
    };
    match (weight, italic) {
        ("Regular", true) => "Italic".into(),
        (weight, true) => format!("{weight} Italic"),
        (weight, false) => weight.into(),
    }
}

/// Every installed TrueType or OpenType file's face, sorted by family, then
/// weight, upright before italic. Collections (.ttc) are left out because a
/// canvas embeds whole font files.
pub fn system_fonts() -> Vec<SystemFont> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    let mut fonts: Vec<SystemFont> = database
        .faces()
        .filter(|face| face.index == 0)
        .filter_map(|face| {
            let fontdb::Source::File(path) = &face.source else {
                return None;
            };
            let extension = path.extension()?.to_str()?.to_ascii_lowercase();
            if !matches!(extension.as_str(), "ttf" | "otf") {
                return None;
            }
            let family = face.families.first()?.0.clone();
            let italic = face.style != fontdb::Style::Normal;
            let weight = face.weight.0;
            // Whether it has Latin letters, and its own style name, such as
            // "Display Black", which tells apart faces of the same weight.
            let (latin, named) = database
                .with_face_data(face.id, |data, index| {
                    ttf_parser::Face::parse(data, index).map_or((false, None), |parsed| {
                        let latin = "AZaz09".chars().all(|c| parsed.glyph_index(c).is_some());
                        let named = crate::font_source::font_name(
                            &parsed,
                            &[
                                ttf_parser::name_id::TYPOGRAPHIC_SUBFAMILY,
                                ttf_parser::name_id::SUBFAMILY,
                            ],
                        );
                        (latin, named)
                    })
                })
                .unwrap_or((false, None));
            Some(SystemFont {
                latin,
                style: named.unwrap_or_else(|| style_name(weight, italic)),
                family,
                weight,
                italic,
                path: path.clone(),
            })
        })
        .collect();
    fonts.sort_by(|a, b| {
        a.family
            .to_lowercase()
            .cmp(&b.family.to_lowercase())
            .then(a.italic.cmp(&b.italic))
            .then(a.weight.cmp(&b.weight))
    });
    fonts.dedup_by(|a, b| a.path == b.path);
    fonts
}

/// The face to use for a family by default: its upright face nearest to
/// regular weight.
pub fn regular_face<'a>(faces: impl IntoIterator<Item = &'a SystemFont>) -> Option<&'a SystemFont> {
    // A face named Regular wins; then upright, nearest regular weight, and
    // shortest name (plain "Regular" before "Display Regular").
    faces.into_iter().min_by_key(|font| {
        (
            !font.style.eq_ignore_ascii_case("regular"),
            font.italic,
            font.weight.abs_diff(400),
            font.style.len(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_names_read_naturally() {
        assert_eq!(style_name(400, false), "Regular");
        assert_eq!(style_name(400, true), "Italic");
        assert_eq!(style_name(700, true), "Bold Italic");
        assert_eq!(style_name(300, false), "Light");
    }

    #[test]
    fn a_family_opens_in_its_regular_face() {
        let face = |style: &str, weight, italic| SystemFont {
            family: "SF Compact".into(),
            style: style.into(),
            weight,
            italic,
            path: format!("{style}.otf").into(),
            latin: true,
        };
        let faces = [
            face("Display Black", 900, false),
            face("Display Regular", 400, false),
            face("Italic", 400, true),
            face("Regular", 400, false),
        ];
        assert_eq!(regular_face(&faces).unwrap().style, "Regular");
        assert_eq!(regular_face(&faces[..2]).unwrap().style, "Display Regular");
    }
}
