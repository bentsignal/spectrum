use serde::Serialize;

/// Stable metadata for the font bytes compiled into Prism.
///
/// `font_id: None` remains the document representation for this face. Older
/// automation may call it "Spectrum Sans"; that string is a compatibility
/// alias only and is never presented as the font's family or source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct BundledFontProvenance {
    pub id: Option<u64>,
    pub family: &'static str,
    pub style: &'static str,
    pub weight: u16,
    pub source_file: &'static str,
    pub designed_by: &'static str,
    pub distributed_by: &'static str,
    pub license_name: &'static str,
    pub packaged_license_file: &'static str,
    pub compatibility_aliases: &'static [&'static str],
}

pub const LEGACY_BUNDLED_FONT_ALIAS: &str = "Spectrum Sans";

pub const BUNDLED_FONT: BundledFontProvenance = BundledFontProvenance {
    id: None,
    family: "Ubuntu",
    style: "Light",
    weight: 300,
    source_file: "Ubuntu-Light.ttf",
    designed_by: "Dalton Maag",
    distributed_by: "epaint_default_fonts",
    license_name: "Ubuntu Font Licence 1.0",
    packaged_license_file: "UBUNTU-FONT-LICENCE-1.0.txt",
    compatibility_aliases: &[LEGACY_BUNDLED_FONT_ALIAS],
};

pub fn bundled_font_provenance() -> BundledFontProvenance {
    BUNDLED_FONT
}

pub fn is_bundled_font_family(value: &str) -> bool {
    value.eq_ignore_ascii_case(BUNDLED_FONT.family)
        || BUNDLED_FONT
            .compatibility_aliases
            .iter()
            .any(|alias| value.eq_ignore_ascii_case(alias))
}

pub fn font_metadata_matches_query(
    family: &str,
    style: &str,
    weight: u16,
    slant: &str,
    query: &str,
) -> bool {
    let family = family.to_ascii_lowercase();
    let style = style.to_ascii_lowercase();
    let weight = weight.to_string();
    let slant = slant.to_ascii_lowercase();
    query.split_whitespace().all(|term| {
        let term = term.to_ascii_lowercase();
        family.contains(&term)
            || style.contains(&term)
            || weight.contains(&term)
            || slant.contains(&term)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ttf_parser::{Face, name_id};

    fn name(face: &Face<'_>, wanted: u16) -> Option<String> {
        face.names()
            .into_iter()
            .filter(|name| name.name_id == wanted)
            .find_map(|name| name.to_string())
    }

    #[test]
    fn bundled_provenance_matches_the_compiled_font_bytes() {
        let face = Face::parse(epaint_default_fonts::UBUNTU_LIGHT, 0).unwrap();
        assert_eq!(
            name(&face, name_id::TYPOGRAPHIC_FAMILY)
                .or_else(|| name(&face, name_id::FAMILY))
                .as_deref(),
            Some(BUNDLED_FONT.family)
        );
        assert_eq!(
            name(&face, name_id::TYPOGRAPHIC_SUBFAMILY)
                .or_else(|| name(&face, name_id::SUBFAMILY))
                .as_deref(),
            Some(BUNDLED_FONT.style)
        );
        assert_eq!(face.weight().to_number(), BUNDLED_FONT.weight);
        assert_eq!(BUNDLED_FONT.designed_by, "Dalton Maag");
        assert_ne!(BUNDLED_FONT.family, LEGACY_BUNDLED_FONT_ALIAS);
    }

    #[test]
    fn legacy_name_is_an_explicit_non_presentational_alias() {
        assert!(is_bundled_font_family("Spectrum Sans"));
        assert!(is_bundled_font_family("ubuntu"));
        assert!(!is_bundled_font_family("Hack"));
        assert_eq!(bundled_font_provenance().family, "Ubuntu");
    }
}
