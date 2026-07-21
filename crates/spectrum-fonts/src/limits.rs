use crate::{SubsetError, SubsetRequest};

pub(crate) const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_SFNT_TABLES: usize = 128;
pub(crate) const MAX_GLYPHS: usize = 32_768;
pub(crate) const MAX_NAME_RECORDS: usize = 4_096;
pub(crate) const MAX_NAME_COPY_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_CMAP_ENCODING_RECORDS: usize = 256;
pub(crate) const MAX_UVS_SELECTOR_RECORDS: usize = 4_096;
pub(crate) const MAX_DEFAULT_UVS_RANGES: usize = 65_536;
pub(crate) const MAX_NONDEFAULT_UVS_MAPPINGS: usize = 65_536;
pub(crate) const MAX_NOMINAL_CODEPOINTS: usize = 1_000_000;
pub(crate) const MAX_SUBSET_CODEPOINTS: usize = 1_000_000;
pub(crate) const MAX_VARIATION_SEQUENCES: usize = 65_536;
pub(crate) const MAX_SHAPING_SAMPLES: usize = 256;
pub(crate) const MAX_SHAPING_SCALARS_PER_SAMPLE: usize = 256;
pub(crate) const MAX_TOTAL_SHAPING_SCALARS: usize = 16_384;
pub(crate) const MAX_SHAPED_OUTPUT_GLYPHS: usize = 16_384;
pub(crate) const MAX_SHAPED_CLOSURE_GLYPHS: usize = MAX_SHAPED_OUTPUT_GLYPHS;

pub(crate) fn validate_request(request: &SubsetRequest) -> Result<(), SubsetError> {
    if request.codepoints().len() > MAX_NOMINAL_CODEPOINTS {
        return Err(SubsetError::new(
            "nominal subset repertoire exceeds resource limit",
        ));
    }
    if request.subset_codepoints().len() > MAX_SUBSET_CODEPOINTS {
        return Err(SubsetError::new(
            "total subset repertoire exceeds resource limit",
        ));
    }
    if request.variation_sequences().len() > MAX_VARIATION_SEQUENCES {
        return Err(SubsetError::new(
            "variation-sequence repertoire exceeds resource limit",
        ));
    }
    if request.shaping_samples().len() > MAX_SHAPING_SAMPLES {
        return Err(SubsetError::new(
            "shaping sample count exceeds resource limit",
        ));
    }
    let mut total_scalars = 0_usize;
    for sample in request.shaping_samples() {
        if sample.len() > MAX_SHAPING_SCALARS_PER_SAMPLE {
            return Err(SubsetError::new(
                "shaping sample length exceeds resource limit",
            ));
        }
        total_scalars = total_scalars
            .checked_add(sample.len())
            .ok_or_else(|| SubsetError::new("total shaping repertoire length overflow"))?;
        if total_scalars > MAX_TOTAL_SHAPING_SCALARS {
            return Err(SubsetError::new(
                "total shaping repertoire exceeds resource limit",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_source_size(bytes: &[u8]) -> Result<(), SubsetError> {
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(SubsetError::new("font source exceeds resource limit"));
    }
    Ok(())
}
