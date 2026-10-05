use std::{collections::BTreeSet, path::PathBuf};

use crate::{Document, LayerKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterAssetRequirement {
    pub path: PathBuf,
    pub content_sha256: Option<String>,
}

impl Document {
    /// Lists the raster providers required by the current visible document state.
    ///
    /// Ordinary raster layers accept the current bytes at their path. Sampled
    /// sources additionally carry the exact encoded-byte digest captured by the
    /// Clone Stamp command.
    pub fn raster_asset_requirements(&self) -> Vec<RasterAssetRequirement> {
        let mut requirements = Vec::new();
        for layer in &self.layers {
            match &layer.kind {
                LayerKind::Raster { path, .. } if layer.visible && layer.opacity > 0.0 => {
                    requirements.push(RasterAssetRequirement {
                        path: path.clone(),
                        content_sha256: None,
                    });
                }
                LayerKind::Paint { program } if layer.visible && layer.opacity > 0.0 => {
                    program.for_each_sampled_source_id(|source_id| {
                        if let Some(source) = self.sampled_sources.get(source_id) {
                            requirements.push(RasterAssetRequirement {
                                path: source.path.clone(),
                                content_sha256: Some(source.content_hash.clone()),
                            });
                        }
                    });
                }
                _ => {}
            }
        }
        if let Some(source) = self
            .clone_source
            .as_ref()
            .and_then(|source_id| self.sampled_sources.get(source_id))
        {
            requirements.push(RasterAssetRequirement {
                path: source.path.clone(),
                content_sha256: Some(source.content_hash.clone()),
            });
        }
        requirements
    }

    pub fn raster_asset_paths(&self) -> Vec<PathBuf> {
        self.raster_asset_requirements()
            .into_iter()
            .map(|requirement| requirement.path)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}
