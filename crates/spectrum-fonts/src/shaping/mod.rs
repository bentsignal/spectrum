//! Deterministic, bounded shaping of one already-resolved font run.
//!
//! This module intentionally does not resolve bidi paragraphs, choose fallback
//! fonts, break lines, or implement caret and IME behavior. Those layers must
//! divide text into runs and select a font before calling this API.

mod harfbuzz_ffi;
mod subset;

use std::ops::Range;

use crate::ShapeError;

pub(crate) use subset::validate_parity;

/// Maximum UTF-8 bytes accepted in one shaping run.
pub const MAX_SHAPE_TEXT_BYTES: usize = 64 * 1024;
/// Maximum Unicode scalar values accepted in one shaping run.
pub const MAX_SHAPE_SCALARS: usize = 16_384;
/// Maximum glyphs accepted from HarfBuzz for one shaping run.
pub const MAX_SHAPE_GLYPHS: usize = 65_536;
/// Maximum explicit OpenType feature overrides accepted per run.
pub const MAX_SHAPE_FEATURES: usize = 32;
/// Maximum variable-font coordinates accepted per run.
pub const MAX_VARIATION_COORDINATES: usize = 64;

/// Horizontal direction for one already-resolved shaping run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDirection {
    /// Logical text is shaped from left to right.
    LeftToRight,
    /// Logical text is shaped from right to left.
    RightToLeft,
}

/// A normalized four-letter ISO 15924 script tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Script([u8; 4]);

impl Script {
    /// Creates a script from a four-letter ISO 15924 tag such as `Latn`.
    ///
    /// ASCII case is normalized to title case before it reaches HarfBuzz.
    pub fn from_iso15924(tag: [u8; 4]) -> Result<Self, ShapeError> {
        if !tag.iter().all(u8::is_ascii_alphabetic) {
            return Err(ShapeError::new(
                "ISO 15924 script tag must contain four ASCII letters",
            ));
        }
        Ok(Self([
            tag[0].to_ascii_uppercase(),
            tag[1].to_ascii_lowercase(),
            tag[2].to_ascii_lowercase(),
            tag[3].to_ascii_lowercase(),
        ]))
    }

    /// Returns the normalized ISO 15924 bytes.
    pub fn iso15924(self) -> [u8; 4] {
        self.0
    }

    pub(crate) fn from_resolved_tag(tag: [u8; 4]) -> Self {
        Self(tag)
    }
}

/// One bounded OpenType feature override.
///
/// HarfBuzz defaults remain active unless overridden. In particular, Spectrum
/// does not disable default `kern`, `liga`, or `clig`. Ranged overrides use
/// UTF-8 byte offsets, matching the clusters returned by [`ShapedGlyph`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenTypeFeature {
    tag: [u8; 4],
    value: u32,
    byte_range: Option<(u32, u32)>,
}

impl OpenTypeFeature {
    /// Applies a feature value to the complete run.
    pub fn global(tag: [u8; 4], value: u32) -> Result<Self, ShapeError> {
        validate_feature_tag(tag)?;
        Ok(Self {
            tag,
            value,
            byte_range: None,
        })
    }

    /// Applies a feature value to a UTF-8 byte range.
    ///
    /// The range is validated against the request text at shape time, including
    /// both UTF-8 character boundaries.
    pub fn for_byte_range(
        tag: [u8; 4],
        value: u32,
        byte_range: Range<usize>,
    ) -> Result<Self, ShapeError> {
        validate_feature_tag(tag)?;
        if byte_range.start >= byte_range.end {
            return Err(ShapeError::new(
                "OpenType feature byte range must be non-empty",
            ));
        }
        let start = u32::try_from(byte_range.start)
            .map_err(|_| ShapeError::new("OpenType feature range start is out of range"))?;
        let end = u32::try_from(byte_range.end)
            .map_err(|_| ShapeError::new("OpenType feature range end is out of range"))?;
        Ok(Self {
            tag,
            value,
            byte_range: Some((start, end)),
        })
    }

    /// Returns the four-byte OpenType feature tag.
    pub fn tag(self) -> [u8; 4] {
        self.tag
    }

    /// Returns the feature value passed to HarfBuzz.
    pub fn value(self) -> u32 {
        self.value
    }

    /// Returns the optional UTF-8 byte range.
    pub fn byte_range(self) -> Option<Range<u32>> {
        self.byte_range.map(|(start, end)| start..end)
    }

    pub(crate) fn raw_range(self) -> (u32, u32) {
        self.byte_range.unwrap_or((0, u32::MAX))
    }
}

fn validate_feature_tag(tag: [u8; 4]) -> Result<(), ShapeError> {
    if !tag
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(ShapeError::new(
            "OpenType feature tag must contain four lowercase ASCII letters or digits",
        ));
    }
    Ok(())
}

/// Inputs for shaping one text run with one already-selected font face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeRequest<'text> {
    text: &'text str,
    item_range: Option<Range<usize>>,
    direction: Option<TextDirection>,
    script: Option<Script>,
    language: Option<String>,
    features: Vec<OpenTypeFeature>,
    variations: Vec<VariationCoordinate>,
}

impl<'text> ShapeRequest<'text> {
    /// Creates a request whose direction and script are guessed from the text.
    ///
    /// An omitted language deterministically resolves to `und`; process locale
    /// is never consulted.
    pub fn new(text: &'text str) -> Self {
        Self {
            text,
            item_range: None,
            direction: None,
            script: None,
            language: None,
            features: Vec::new(),
            variations: Vec::new(),
        }
    }

    /// Selects one non-empty UTF-8 item while retaining the complete source context.
    pub fn item_range(mut self, range: Range<usize>) -> Self {
        self.item_range = Some(range);
        self
    }

    /// Sets an explicit run direction.
    pub fn direction(mut self, direction: TextDirection) -> Self {
        self.direction = Some(direction);
        self
    }

    /// Sets an explicit ISO 15924 script.
    pub fn script(mut self, script: Script) -> Self {
        self.script = Some(script);
        self
    }

    /// Sets an explicit BCP 47 language.
    pub fn language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Replaces the bounded OpenType feature override list.
    pub fn features(mut self, features: impl IntoIterator<Item = OpenTypeFeature>) -> Self {
        self.features = features.into_iter().collect();
        self
    }

    /// Replaces the canonical variable-font coordinate list.
    ///
    /// Coordinates are sorted by tag. Duplicate axes fail closed rather than
    /// depending on caller order.
    pub fn variations(
        mut self,
        variations: impl IntoIterator<Item = VariationCoordinate>,
    ) -> Result<Self, ShapeError> {
        self.variations = variations.into_iter().collect();
        self.variations
            .sort_unstable_by_key(|coordinate| coordinate.tag);
        if self
            .variations
            .windows(2)
            .any(|pair| pair[0].tag == pair[1].tag)
        {
            return Err(ShapeError::new(
                "variable-font coordinate tags must be unique",
            ));
        }
        Ok(self)
    }

    /// Returns the UTF-8 source text.
    pub fn text(&self) -> &'text str {
        self.text
    }

    /// Returns the selected item range, or the complete source text.
    pub fn selected_item_range(&self) -> Range<usize> {
        self.item_range.clone().unwrap_or(0..self.text.len())
    }

    /// Returns the explicit direction, or `None` when it will be guessed.
    pub fn requested_direction(&self) -> Option<TextDirection> {
        self.direction
    }

    /// Returns the explicit script, or `None` when it will be guessed.
    pub fn requested_script(&self) -> Option<Script> {
        self.script
    }

    /// Returns the explicit language, or `None` when it resolves to `und`.
    pub fn requested_language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    /// Returns the ordered feature overrides.
    pub fn feature_overrides(&self) -> &[OpenTypeFeature] {
        &self.features
    }

    /// Returns canonical variable-font coordinates ordered by tag.
    pub fn variation_coordinates(&self) -> &[VariationCoordinate] {
        &self.variations
    }
}

/// One canonical OpenType variable-font axis coordinate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VariationCoordinate {
    tag: [u8; 4],
    value_bits: u32,
}

impl VariationCoordinate {
    /// Creates a finite coordinate, normalizing negative zero.
    pub fn new(tag: [u8; 4], value: f32) -> Result<Self, ShapeError> {
        validate_feature_tag(tag)?;
        if !value.is_finite() {
            return Err(ShapeError::new("variable-font coordinate must be finite"));
        }
        let value = if value == 0.0 { 0.0 } else { value };
        Ok(Self {
            tag,
            value_bits: value.to_bits(),
        })
    }

    /// Returns the four-byte OpenType axis tag.
    pub fn tag(self) -> [u8; 4] {
        self.tag
    }

    /// Returns the axis value.
    pub fn value(self) -> f32 {
        f32::from_bits(self.value_bits)
    }
}

/// Flags HarfBuzz attached to one shaped glyph.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlyphFlags(u8);

impl GlyphFlags {
    const UNSAFE_TO_BREAK: u8 = 1;
    const UNSAFE_TO_CONCAT: u8 = 2;
    const SAFE_TO_INSERT_TATWEEL: u8 = 4;

    /// Returns the HarfBuzz-defined flag bits.
    pub fn bits(self) -> u8 {
        self.0
    }

    /// Whether breaking before this glyph can change shaping on either side.
    pub fn unsafe_to_break(self) -> bool {
        self.0 & Self::UNSAFE_TO_BREAK != 0
    }

    /// Whether concatenating text before this glyph can change shaping.
    pub fn unsafe_to_concat(self) -> bool {
        self.0 & Self::UNSAFE_TO_CONCAT != 0
    }

    /// Whether inserting an Arabic tatweel before this glyph is safe.
    pub fn safe_to_insert_tatweel(self) -> bool {
        self.0 & Self::SAFE_TO_INSERT_TATWEEL != 0
    }

    pub(crate) fn from_bits(bits: u32) -> Result<Self, ShapeError> {
        let bits = u8::try_from(bits)
            .map_err(|_| ShapeError::new("HarfBuzz returned out-of-range glyph flags"))?;
        if bits & !(Self::UNSAFE_TO_BREAK | Self::UNSAFE_TO_CONCAT | Self::SAFE_TO_INSERT_TATWEEL)
            != 0
        {
            return Err(ShapeError::new(
                "HarfBuzz returned unknown shaped-glyph flags",
            ));
        }
        Ok(Self(bits))
    }
}

/// One positioned glyph in a shaped run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapedGlyph {
    /// Glyph ID within the selected face.
    pub glyph_id: u16,
    /// UTF-8 byte offset of the source cluster.
    pub cluster: u32,
    /// Exclusive UTF-8 byte end of the source cluster.
    pub cluster_end: u32,
    /// HarfBuzz safety flags.
    pub flags: GlyphFlags,
    /// Horizontal advance in font units.
    pub x_advance: i32,
    /// Vertical advance in font units.
    pub y_advance: i32,
    /// Horizontal placement offset in font units.
    pub x_offset: i32,
    /// Vertical placement offset in font units.
    pub y_offset: i32,
}

/// Resolved properties and positioned glyphs for one font run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapedRun {
    glyphs: Vec<ShapedGlyph>,
    direction: TextDirection,
    script: Option<Script>,
    language: String,
    direction_was_guessed: bool,
    script_was_guessed: bool,
    language_was_defaulted: bool,
    units_per_em: u16,
    ascender: i16,
    descender: i16,
    line_gap: i16,
    face_index: u32,
    face_identity: [u8; 32],
    source_text_bytes: u32,
    item_start: u32,
    item_end: u32,
}

impl ShapedRun {
    /// Returns positioned glyphs in HarfBuzz output order.
    pub fn glyphs(&self) -> &[ShapedGlyph] {
        &self.glyphs
    }

    /// Returns the resolved shaping direction.
    pub fn direction(&self) -> TextDirection {
        self.direction
    }

    /// Returns the resolved script, or `None` if no strong script was found.
    pub fn script(&self) -> Option<Script> {
        self.script
    }

    /// Returns the resolved BCP 47 language.
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Whether HarfBuzz guessed the direction from the run.
    pub fn direction_was_guessed(&self) -> bool {
        self.direction_was_guessed
    }

    /// Whether HarfBuzz guessed the script from the run.
    pub fn script_was_guessed(&self) -> bool {
        self.script_was_guessed
    }

    /// Whether an omitted language was deterministically defaulted to `und`.
    pub fn language_was_defaulted(&self) -> bool {
        self.language_was_defaulted
    }

    /// Returns the face units per em used for advances and offsets.
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// Returns the face ascender in font units.
    pub fn ascender(&self) -> i16 {
        self.ascender
    }

    /// Returns the face descender in font units.
    pub fn descender(&self) -> i16 {
        self.descender
    }

    /// Returns the face line gap in font units.
    pub fn line_gap(&self) -> i16 {
        self.line_gap
    }

    /// Returns the selected face index.
    pub fn face_index(&self) -> u32 {
        self.face_index
    }

    /// Returns SHA-256(font bytes || big-endian face index).
    pub fn face_identity(&self) -> [u8; 32] {
        self.face_identity
    }

    /// Returns the UTF-8 byte length of the source run.
    pub fn source_text_bytes(&self) -> u32 {
        self.source_text_bytes
    }

    /// Returns the shaped item's UTF-8 byte range within the source context.
    pub fn item_range(&self) -> Range<u32> {
        self.item_start..self.item_end
    }
}

/// Public contract for deterministic shaping of one already-resolved font run.
pub trait TextShaper {
    /// Shapes one bounded request or fails closed.
    fn shape(&self, request: &ShapeRequest<'_>) -> Result<ShapedRun, ShapeError>;
}

/// Bundled HarfBuzz 8.2.2 shaper over immutable caller-owned font bytes.
///
/// The font is linked in process through the pinned bundled dependency. This
/// type never invokes a system executable or loads a runtime HarfBuzz dylib.
pub struct HarfBuzzShaper<'font> {
    native: harfbuzz_ffi::NativeShaper<'font>,
}

impl<'font> HarfBuzzShaper<'font> {
    /// Validates and opens one face from immutable font bytes.
    pub fn new(bytes: &'font [u8], face_index: u32) -> Result<Self, ShapeError> {
        harfbuzz_ffi::NativeShaper::new(bytes, face_index).map(|native| Self { native })
    }

    /// Shapes one bounded request.
    pub fn shape(&self, request: &ShapeRequest<'_>) -> Result<ShapedRun, ShapeError> {
        <Self as TextShaper>::shape(self, request)
    }

    /// Returns the selected face index.
    pub fn face_index(&self) -> u32 {
        self.native.face_index()
    }

    /// Returns the selected face units per em.
    pub fn units_per_em(&self) -> u16 {
        self.native.units_per_em()
    }
}

impl TextShaper for HarfBuzzShaper<'_> {
    fn shape(&self, request: &ShapeRequest<'_>) -> Result<ShapedRun, ShapeError> {
        self.native.shape(request)
    }
}

fn validate_language(language: &str) -> Result<(), ShapeError> {
    if language.is_empty() || language.len() > 63 {
        return Err(ShapeError::new(
            "BCP 47 language must contain between 1 and 63 bytes",
        ));
    }
    if !language.is_ascii()
        || language.starts_with('-')
        || language.ends_with('-')
        || language.contains("--")
    {
        return Err(ShapeError::new(
            "BCP 47 language must contain non-empty ASCII subtags",
        ));
    }
    for subtag in language.split('-') {
        if subtag.len() > 8 || !subtag.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(ShapeError::new(
                "BCP 47 language contains an invalid subtag",
            ));
        }
    }
    Ok(())
}

fn validate_request(request: &ShapeRequest<'_>) -> Result<(), ShapeError> {
    let text = request.text();
    if text.is_empty() {
        return Err(ShapeError::new("shaping text cannot be empty"));
    }
    if text.len() > MAX_SHAPE_TEXT_BYTES {
        return Err(ShapeError::new(
            "shaping text byte length exceeds resource limit",
        ));
    }
    if text.chars().count() > MAX_SHAPE_SCALARS {
        return Err(ShapeError::new(
            "shaping scalar count exceeds resource limit",
        ));
    }
    if request.feature_overrides().len() > MAX_SHAPE_FEATURES {
        return Err(ShapeError::new(
            "OpenType feature count exceeds resource limit",
        ));
    }
    if request.variation_coordinates().len() > MAX_VARIATION_COORDINATES {
        return Err(ShapeError::new(
            "variable-font coordinate count exceeds resource limit",
        ));
    }
    let item = request.selected_item_range();
    if item.start >= item.end
        || item.end > text.len()
        || !text.is_char_boundary(item.start)
        || !text.is_char_boundary(item.end)
    {
        return Err(ShapeError::new(
            "shaping item range must be non-empty UTF-8 byte boundaries",
        ));
    }
    if let Some(language) = request.requested_language() {
        validate_language(language)?;
    }
    for feature in request.feature_overrides() {
        if let Some(range) = feature.byte_range() {
            let start = usize::try_from(range.start)
                .map_err(|_| ShapeError::new("OpenType feature range is out of range"))?;
            let end = usize::try_from(range.end)
                .map_err(|_| ShapeError::new("OpenType feature range is out of range"))?;
            if end > text.len() || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                return Err(ShapeError::new(
                    "OpenType feature range must use valid UTF-8 byte boundaries",
                ));
            }
        }
    }
    Ok(())
}
