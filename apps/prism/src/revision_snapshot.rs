use super::*;

pub(super) struct PreparedSnapshot {
    pub(super) payload: Payload,
    pub(super) assets: Vec<Asset>,
}

impl PreparedSnapshot {
    pub(super) fn legacy(document: &Document) -> Result<Self> {
        Self::prepare(document, false)
    }

    pub(super) fn compressed(document: &Document) -> Result<Self> {
        Self::prepare(document, true)
    }

    pub(super) fn portable(document: &Document) -> Result<Self> {
        let mut portable = document.clone();
        for layer in &mut portable.layers {
            if let LayerKind::Raster {
                path,
                original_path,
            } = &mut layer.kind
            {
                AssetReference::parse(path).context("raster layer is not a project asset")?;
                *original_path = None;
            }
        }
        for font in &mut portable.font_assets {
            let reference =
                AssetReference::parse(&font.path).context("font is not a project asset")?;
            if reference.id.to_string() != font.content_hash {
                bail!("font path does not match its content identity");
            }
            font.original_path = None;
        }
        Self::encode(portable, true, Vec::new())
    }

    fn prepare(document: &Document, compressed: bool) -> Result<Self> {
        let mut portable = document.clone();
        let mut assets = Vec::new();
        for layer in &mut portable.layers {
            if let LayerKind::Raster {
                path,
                original_path,
            } = &mut layer.kind
            {
                let prepared = prepare_asset(path)?;
                *path = prepared.reference.path();
                *original_path = None;
                assets.push(prepared.asset);
            }
        }
        for font in &mut portable.font_assets {
            let prepared = prepare_verified_font_asset(font)?;
            font.path = prepared.reference.path();
            font.original_path = None;
            assets.push(prepared.asset);
        }
        Self::encode(portable, compressed, assets)
    }

    fn encode(mut portable: Document, compressed: bool, assets: Vec<Asset>) -> Result<Self> {
        let color_selection_schema = portable
            .selection
            .as_ref()
            .is_some_and(|selection| selection.alpha().is_some())
            || portable
                .layers
                .iter()
                .any(|layer| layer.pixel_mask.is_some());
        let path_schema = portable.layers.iter().any(|layer| {
            layer.vector_mask.is_some() || matches!(layer.kind, LayerKind::Path { .. })
        });
        let paint_schema = portable
            .layers
            .iter()
            .any(|layer| matches!(layer.kind, LayerKind::Paint { .. }));
        let dissolve_schema = portable.layers.iter().any(|layer| {
            layer.blend_mode == crate::BlendMode::Dissolve || layer.dissolve_seed != 0
        });
        let selection_schema =
            portable.version >= SELECTION_SNAPSHOT_VERSION || portable.selection.is_some();
        let effects_schema = portable.version >= LAYER_EFFECTS_SNAPSHOT_VERSION
            || portable
                .layers
                .iter()
                .any(|layer| !layer.style.is_empty() || layer.shape_fill.is_some());
        let snapshot_version = if dissolve_schema {
            DISSOLVE_SNAPSHOT_VERSION
        } else if paint_schema {
            PAINT_SNAPSHOT_VERSION
        } else if path_schema {
            PATH_SNAPSHOT_VERSION
        } else if color_selection_schema {
            COLOR_SELECTION_SNAPSHOT_VERSION
        } else if selection_schema {
            SELECTION_SNAPSHOT_VERSION
        } else if effects_schema {
            LAYER_EFFECTS_SNAPSHOT_VERSION
        } else if compressed {
            COMPRESSED_SNAPSHOT_VERSION
        } else {
            LEGACY_SNAPSHOT_VERSION
        };
        portable.version = snapshot_version;
        let serialized = serde_json::to_vec(&portable)?;
        let encoding = Encoding::new(SNAPSHOT_FAMILY, snapshot_version);
        let payload = if compressed {
            Payload::new(
                encoding.requiring(DEFLATE_CAPABILITY),
                deflate(&serialized)?,
            )
        } else {
            Payload::new(encoding, serialized)
        };
        Ok(Self { payload, assets })
    }
}

fn deflate(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?)
}

const MAX_SNAPSHOT_JSON_BYTES: usize = 128 * 1024 * 1024;

fn bounded_snapshot_bytes(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() > MAX_SNAPSHOT_JSON_BYTES {
        bail!("Prism snapshot exceeds the 128 MiB decoded JSON limit");
    }
    Ok(bytes.to_vec())
}

fn inflate_snapshot(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut decoded = Vec::new();
    ZlibDecoder::new(bytes)
        .take((MAX_SNAPSHOT_JSON_BYTES + 1) as u64)
        .read_to_end(&mut decoded)?;
    if decoded.len() > MAX_SNAPSHOT_JSON_BYTES {
        bail!("Prism snapshot exceeds the 128 MiB decoded JSON limit");
    }
    Ok(decoded)
}

pub(super) fn decode_snapshot(payload: &Payload) -> Result<Vec<u8>> {
    let version = payload.encoding.version;
    let plain = payload.encoding.required_capabilities.is_empty();
    let deflated = payload.encoding.required_capabilities == [DEFLATE_CAPABILITY];
    match (version, plain, deflated) {
        (LEGACY_SNAPSHOT_VERSION, true, false) => bounded_snapshot_bytes(&payload.bytes),
        (COMPRESSED_SNAPSHOT_VERSION, false, true) => inflate_snapshot(&payload.bytes),
        (
            LAYER_EFFECTS_SNAPSHOT_VERSION
            | SELECTION_SNAPSHOT_VERSION
            | COLOR_SELECTION_SNAPSHOT_VERSION
            | PATH_SNAPSHOT_VERSION
            | PAINT_SNAPSHOT_VERSION
            | DISSOLVE_SNAPSHOT_VERSION,
            true,
            false,
        ) => bounded_snapshot_bytes(&payload.bytes),
        (
            LAYER_EFFECTS_SNAPSHOT_VERSION
            | SELECTION_SNAPSHOT_VERSION
            | COLOR_SELECTION_SNAPSHOT_VERSION
            | PATH_SNAPSHOT_VERSION
            | PAINT_SNAPSHOT_VERSION
            | DISSOLVE_SNAPSHOT_VERSION,
            false,
            true,
        ) => inflate_snapshot(&payload.bytes),
        _ => bail!("unsupported Prism snapshot encoding"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_codec_accepts_only_the_advertised_version_capability_matrix() {
        let legacy_deflate = Payload::new(
            Encoding::new(SNAPSHOT_FAMILY, LEGACY_SNAPSHOT_VERSION).requiring(DEFLATE_CAPABILITY),
            Vec::new(),
        );
        assert!(decode_snapshot(&legacy_deflate).is_err());

        let compressed_plain = Payload::new(
            Encoding::new(SNAPSHOT_FAMILY, COMPRESSED_SNAPSHOT_VERSION),
            Vec::new(),
        );
        assert!(decode_snapshot(&compressed_plain).is_err());

        let unknown_capability = Payload::new(
            Encoding::new(SNAPSHOT_FAMILY, COLOR_SELECTION_SNAPSHOT_VERSION).requiring("unknown"),
            Vec::new(),
        );
        assert!(decode_snapshot(&unknown_capability).is_err());
    }

    #[test]
    fn v5_snapshot_remains_readable_with_default_empty_path_fields() {
        let mut document = Document::new("V5 compatibility", 32, 24);
        document.version = COLOR_SELECTION_SNAPSHOT_VERSION;
        document.layers.push(crate::Layer {
            id: 1,
            kind: LayerKind::Rectangle {
                width: 8,
                height: 6,
                color: [10, 20, 30, 255],
                corner_radius: 0.0,
            },
            ..crate::Layer::default()
        });
        let payload = Payload::new(
            Encoding::new(SNAPSHOT_FAMILY, COLOR_SELECTION_SNAPSHOT_VERSION),
            serde_json::to_vec(&document).unwrap(),
        );
        let decoded: Document =
            serde_json::from_slice(&decode_snapshot(&payload).unwrap()).unwrap();
        assert_eq!(decoded.version, COLOR_SELECTION_SNAPSHOT_VERSION);
        assert!(decoded.layers[0].vector_mask.is_none());
        assert!(matches!(
            decoded.layers[0].kind,
            LayerKind::Rectangle { .. }
        ));
    }

    #[test]
    fn v6_snapshot_remains_readable_with_default_empty_paint_fields() {
        let mut document = Document::new("V6 compatibility", 32, 24);
        document.version = PATH_SNAPSHOT_VERSION;
        document.layers.push(crate::Layer {
            id: 1,
            kind: LayerKind::Rectangle {
                width: 8,
                height: 6,
                color: [10, 20, 30, 255],
                corner_radius: 0.0,
            },
            ..crate::Layer::default()
        });
        let payload = Payload::new(
            Encoding::new(SNAPSHOT_FAMILY, PATH_SNAPSHOT_VERSION),
            serde_json::to_vec(&document).unwrap(),
        );
        let decoded: Document =
            serde_json::from_slice(&decode_snapshot(&payload).unwrap()).unwrap();
        assert_eq!(decoded.version, PATH_SNAPSHOT_VERSION);
        assert!(matches!(
            decoded.layers[0].kind,
            LayerKind::Rectangle { .. }
        ));
    }

    #[test]
    fn paint_snapshot_uses_v7_and_round_trips_exactly() {
        let stroke = crate::BrushStroke::new(
            crate::BrushStyle::default(),
            vec![crate::BrushSample {
                x: 4.5,
                y: 5.5,
                pressure: 0.8,
            }],
        )
        .unwrap();
        let program = crate::BrushProgram::new(32, 24)
            .unwrap()
            .append(stroke)
            .unwrap();
        let mut document = Document::new("V7 Paint", 32, 24);
        document.layers.push(crate::Layer {
            id: 1,
            kind: LayerKind::Paint { program },
            ..crate::Layer::default()
        });
        let prepared = PreparedSnapshot::compressed(&document).unwrap();
        assert_eq!(prepared.payload.encoding.version, PAINT_SNAPSHOT_VERSION);
        assert_eq!(
            prepared.payload.encoding.required_capabilities,
            [DEFLATE_CAPABILITY]
        );
        let decoded: Document =
            serde_json::from_slice(&decode_snapshot(&prepared.payload).unwrap()).unwrap();
        assert_eq!(decoded.version, PAINT_SNAPSHOT_VERSION);
        assert_eq!(decoded.layers, document.layers);
    }

    #[test]
    fn dissolve_snapshot_uses_v8_and_round_trips_the_seed() {
        let mut document = Document::new("V8 Dissolve", 32, 24);
        document.layers.push(crate::Layer {
            id: 1,
            blend_mode: crate::BlendMode::Dissolve,
            dissolve_seed: 0x1234_5678,
            ..crate::Layer::default()
        });
        let prepared = PreparedSnapshot::compressed(&document).unwrap();
        assert_eq!(prepared.payload.encoding.version, DISSOLVE_SNAPSHOT_VERSION);
        let decoded: Document =
            serde_json::from_slice(&decode_snapshot(&prepared.payload).unwrap()).unwrap();
        assert_eq!(decoded.layers[0].dissolve_seed, 0x1234_5678);
        assert_eq!(decoded.layers[0].blend_mode, crate::BlendMode::Dissolve);
    }
}
