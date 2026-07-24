use anyhow::{Result, bail};
use spectrum_revisions::{Compatibility, Encoding};

use crate::Command;

pub(super) const SNAPSHOT_FAMILY: &str = "spectrum.prism.document";
pub(super) const OPERATIONS_FAMILY: &str = "spectrum.prism.commands";
pub(super) const LEGACY_SNAPSHOT_VERSION: u32 = 1;
pub(super) const COMPRESSED_SNAPSHOT_VERSION: u32 = 2;
pub(super) const LAYER_EFFECTS_SNAPSHOT_VERSION: u32 = 3;
pub(super) const SELECTION_SNAPSHOT_VERSION: u32 = 4;
pub(super) const COLOR_SELECTION_SNAPSHOT_VERSION: u32 = 5;
pub(super) const PATH_SNAPSHOT_VERSION: u32 = 6;
pub(super) const PAINT_SNAPSHOT_VERSION: u32 = 7;
pub(super) const DISSOLVE_SNAPSHOT_VERSION: u32 = 8;
pub(super) const RASTER_PIXEL_MASK_SNAPSHOT_VERSION: u32 = 9;
pub(super) const LEGACY_OPERATIONS_VERSION: u32 = 1;
pub(super) const LAYER_TRANSFER_OPERATIONS_VERSION: u32 = 2;
pub(super) const LAYER_EFFECTS_OPERATIONS_VERSION: u32 = 3;
pub(super) const SELECTION_OPERATIONS_VERSION: u32 = 4;
pub(super) const CROP_TO_SELECTION_OPERATIONS_VERSION: u32 = 5;
pub(super) const COLOR_SELECTION_OPERATIONS_VERSION: u32 = 6;
pub(super) const PATH_OPERATIONS_VERSION: u32 = 7;
pub(super) const PAINT_OPERATIONS_VERSION: u32 = 8;
pub(super) const LASSO_OPERATIONS_VERSION: u32 = 9;
pub(super) const DOCUMENT_LIFECYCLE_OPERATIONS_VERSION: u32 = 10;
pub(super) const DISSOLVE_OPERATIONS_VERSION: u32 = 11;
pub(super) const RASTER_PIXEL_MASK_OPERATIONS_VERSION: u32 = 12;
pub(super) const DEFLATE_CAPABILITY: &str = "deflate";

pub(super) struct PrismCompatibility;

impl Compatibility for PrismCompatibility {
    fn supports_snapshot(&self, encoding: &Encoding) -> bool {
        encoding.family == SNAPSHOT_FAMILY
            && match encoding.version {
                LEGACY_SNAPSHOT_VERSION => encoding.required_capabilities.is_empty(),
                COMPRESSED_SNAPSHOT_VERSION => {
                    encoding.required_capabilities == [DEFLATE_CAPABILITY]
                }
                LAYER_EFFECTS_SNAPSHOT_VERSION => {
                    encoding.required_capabilities.is_empty()
                        || encoding.required_capabilities == [DEFLATE_CAPABILITY]
                }
                SELECTION_SNAPSHOT_VERSION => {
                    encoding.required_capabilities.is_empty()
                        || encoding.required_capabilities == [DEFLATE_CAPABILITY]
                }
                COLOR_SELECTION_SNAPSHOT_VERSION => {
                    encoding.required_capabilities.is_empty()
                        || encoding.required_capabilities == [DEFLATE_CAPABILITY]
                }
                PATH_SNAPSHOT_VERSION => {
                    encoding.required_capabilities.is_empty()
                        || encoding.required_capabilities == [DEFLATE_CAPABILITY]
                }
                PAINT_SNAPSHOT_VERSION => {
                    encoding.required_capabilities.is_empty()
                        || encoding.required_capabilities == [DEFLATE_CAPABILITY]
                }
                DISSOLVE_SNAPSHOT_VERSION | RASTER_PIXEL_MASK_SNAPSHOT_VERSION => {
                    encoding.required_capabilities.is_empty()
                        || encoding.required_capabilities == [DEFLATE_CAPABILITY]
                }
                _ => false,
            }
    }

    fn supports_operations(&self, encoding: &Encoding) -> bool {
        encoding.family == OPERATIONS_FAMILY
            && (LEGACY_OPERATIONS_VERSION..=RASTER_PIXEL_MASK_OPERATIONS_VERSION)
                .contains(&encoding.version)
            && encoding.required_capabilities.is_empty()
    }
}

pub(super) fn operations_version(commands: &[Command]) -> u32 {
    if commands.iter().any(|command| {
        matches!(command, Command::DeleteSelectedPixels { .. })
            || matches!(
                command,
                Command::InsertLayer { transfer, .. }
                    if transfer.version >= crate::RASTER_PIXEL_MASK_LAYER_TRANSFER_VERSION
                        || matches!(transfer.layer.kind, crate::LayerKind::Raster { .. })
                            && transfer.layer.pixel_mask.is_some()
            )
    }) {
        return RASTER_PIXEL_MASK_OPERATIONS_VERSION;
    }
    if commands.iter().any(|command| {
        matches!(
            command,
            Command::SetDissolveSeed { .. }
                | Command::SetBlendMode {
                    blend_mode: crate::BlendMode::Dissolve,
                    ..
                }
        ) || matches!(
            command,
            Command::InsertLayer { transfer, .. }
                if transfer.version >= crate::DISSOLVE_LAYER_TRANSFER_VERSION
                    || transfer.layer.blend_mode == crate::BlendMode::Dissolve
                    || transfer.layer.dissolve_seed != 0
        )
    }) {
        return DISSOLVE_OPERATIONS_VERSION;
    }
    if commands
        .iter()
        .any(|command| matches!(command, Command::RenameDocument { .. }))
    {
        return DOCUMENT_LIFECYCLE_OPERATIONS_VERSION;
    }
    if commands
        .iter()
        .any(|command| matches!(command, Command::LassoSelection { .. }))
    {
        return LASSO_OPERATIONS_VERSION;
    }
    if commands.iter().any(|command| {
        matches!(
            command,
            Command::AddPaintLayer { .. }
                | Command::AddPaintLayerWithStroke { .. }
                | Command::AddBrushStroke { .. }
        ) || matches!(command, Command::InsertLayer { transfer, .. }
                if transfer.version >= crate::PAINT_LAYER_TRANSFER_VERSION
                    || matches!(transfer.layer.kind, crate::LayerKind::Paint { .. }))
    }) {
        return PAINT_OPERATIONS_VERSION;
    }
    if commands.iter().any(|command| {
        matches!(
            command,
            Command::AddPath { .. } | Command::ReplacePath { .. } | Command::SetVectorMask { .. }
        ) || matches!(
            command,
            Command::InsertLayer { transfer, .. }
                if transfer.version >= crate::PATH_LAYER_TRANSFER_VERSION
                    || transfer.layer.vector_mask.is_some()
                    || matches!(transfer.layer.kind, crate::LayerKind::Path { .. })
        )
    }) {
        return PATH_OPERATIONS_VERSION;
    }
    if commands.iter().any(|command| {
        matches!(command, Command::MagicWandSelection { .. })
            || matches!(command, Command::MagicWandSnapshot { .. })
            || matches!(command, Command::SetSelection { selection: Some(selection) } if selection.alpha().is_some())
            || matches!(
                command,
                Command::InsertLayer { transfer, .. }
                    if transfer.version == 3 || transfer.layer.pixel_mask.is_some()
            )
    }) {
        return COLOR_SELECTION_OPERATIONS_VERSION;
    }
    if commands
        .iter()
        .any(|command| matches!(command, Command::CropToSelection))
    {
        return CROP_TO_SELECTION_OPERATIONS_VERSION;
    }
    if commands.iter().any(|command| {
        matches!(
            command,
            Command::SetSelection { .. } | Command::FillSelection { .. }
        )
    }) {
        SELECTION_OPERATIONS_VERSION
    } else if commands.iter().any(|command| {
        matches!(
            command,
            Command::SetLayerStyle { .. } | Command::SetShapeFill { .. }
        ) || matches!(
            command,
            Command::InsertLayer { transfer, .. }
                if transfer.version == 2
                    || !transfer.layer.style.is_empty()
                    || transfer.layer.shape_fill.is_some()
        )
    }) {
        LAYER_EFFECTS_OPERATIONS_VERSION
    } else if commands
        .iter()
        .any(|command| matches!(command, Command::InsertLayer { .. }))
    {
        LAYER_TRANSFER_OPERATIONS_VERSION
    } else {
        LEGACY_OPERATIONS_VERSION
    }
}

pub(super) fn downgrade_compatible_transfers(commands: &mut [Command]) {
    for command in commands {
        if let Command::InsertLayer { transfer, .. } = command
            && transfer.validate_envelope().is_ok()
        {
            let minimal_version = if matches!(transfer.layer.kind, crate::LayerKind::Raster { .. })
                && transfer.layer.pixel_mask.is_some()
            {
                crate::RASTER_PIXEL_MASK_LAYER_TRANSFER_VERSION
            } else if transfer.layer.blend_mode == crate::BlendMode::Dissolve
                || transfer.layer.dissolve_seed != 0
            {
                crate::DISSOLVE_LAYER_TRANSFER_VERSION
            } else if matches!(transfer.layer.kind, crate::LayerKind::Paint { .. }) {
                crate::PAINT_LAYER_TRANSFER_VERSION
            } else if transfer.layer.vector_mask.is_some()
                || matches!(transfer.layer.kind, crate::LayerKind::Path { .. })
            {
                crate::PATH_LAYER_TRANSFER_VERSION
            } else if transfer.layer.pixel_mask.is_some() {
                3
            } else if transfer.layer.style.is_empty() && transfer.layer.shape_fill.is_none() {
                1
            } else {
                2
            };
            transfer.version = transfer.version.min(minimal_version);
        }
    }
}

pub(super) fn validate_operations_version(
    commands: &[Command],
    encoded_version: u32,
) -> Result<()> {
    let required_version = operations_version(commands);
    if required_version > encoded_version {
        bail!(
            "Prism operation payload version {encoded_version} contains commands requiring version {required_version}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DropShadow, LAYER_TRANSFER_FORMAT, LAYER_TRANSFER_VERSION, LassoPath, LassoPoint, Layer,
        LayerKind, LayerStyle, LayerTransfer, PAINT_LAYER_TRANSFER_VERSION, PixelMask,
        RASTER_PIXEL_MASK_LAYER_TRANSFER_VERSION, Selection, SelectionCombineMode,
    };

    fn insert_transfer(version: u32, layer: Layer) -> Vec<Command> {
        vec![Command::InsertLayer {
            transfer: Box::new(LayerTransfer {
                format: LAYER_TRANSFER_FORMAT.into(),
                version,
                layer,
                font_asset: None,
            }),
            index: None,
        }]
    }

    fn inserted_transfer(commands: &[Command]) -> &LayerTransfer {
        let Command::InsertLayer { transfer, .. } = &commands[0] else {
            unreachable!("test helper only accepts InsertLayer commands");
        };
        transfer
    }

    #[test]
    fn document_rename_requires_the_lifecycle_operation_envelope() {
        let commands = [Command::RenameDocument {
            name: "Campaign".into(),
        }];
        assert_eq!(
            operations_version(&commands),
            DOCUMENT_LIFECYCLE_OPERATIONS_VERSION
        );
        assert!(validate_operations_version(&commands, LASSO_OPERATIONS_VERSION).is_err());
        assert!(
            validate_operations_version(&commands, DOCUMENT_LIFECYCLE_OPERATIONS_VERSION).is_ok()
        );
    }

    #[test]
    fn raster_pixel_deletion_requires_the_v12_operation_envelope() {
        let commands = [Command::DeleteSelectedPixels { id: 7 }];
        assert_eq!(
            operations_version(&commands),
            RASTER_PIXEL_MASK_OPERATIONS_VERSION
        );
        assert!(
            validate_operations_version(&commands, DOCUMENT_LIFECYCLE_OPERATIONS_VERSION).is_err()
        );
        assert!(
            validate_operations_version(&commands, RASTER_PIXEL_MASK_OPERATIONS_VERSION).is_ok()
        );
    }

    #[test]
    fn effect_commands_cannot_use_a_legacy_operation_envelope() {
        let commands = [Command::SetLayerStyle {
            id: 7,
            style: LayerStyle {
                drop_shadow: Some(DropShadow::default()),
            },
        }];
        assert_eq!(
            operations_version(&commands),
            LAYER_EFFECTS_OPERATIONS_VERSION
        );
        assert!(validate_operations_version(&commands, LEGACY_OPERATIONS_VERSION).is_err());
        assert!(validate_operations_version(&commands, LAYER_EFFECTS_OPERATIONS_VERSION).is_ok());
    }

    #[test]
    fn legacy_commands_remain_valid_in_newer_operation_envelopes() {
        let commands = [Command::SetCanvas {
            width: 80,
            height: 60,
            background: [0; 4],
        }];
        assert_eq!(operations_version(&commands), LEGACY_OPERATIONS_VERSION);
        assert!(validate_operations_version(&commands, LEGACY_OPERATIONS_VERSION).is_ok());
        assert!(validate_operations_version(&commands, LAYER_EFFECTS_OPERATIONS_VERSION).is_ok());
        assert!(validate_operations_version(&commands, SELECTION_OPERATIONS_VERSION).is_ok());
        assert!(
            validate_operations_version(&commands, CROP_TO_SELECTION_OPERATIONS_VERSION).is_ok()
        );
    }

    #[test]
    fn existing_selection_commands_keep_the_v4_operation_envelope() {
        for command in [
            Command::SetSelection {
                selection: Some(Selection::rectangle(4, 5, 20, 10)),
            },
            Command::FillSelection {
                color: [10, 20, 30, 255],
                name: None,
            },
        ] {
            assert_eq!(
                operations_version(std::slice::from_ref(&command)),
                SELECTION_OPERATIONS_VERSION
            );
            assert!(
                validate_operations_version(
                    std::slice::from_ref(&command),
                    LAYER_EFFECTS_OPERATIONS_VERSION,
                )
                .is_err()
            );
            assert!(
                validate_operations_version(
                    std::slice::from_ref(&command),
                    SELECTION_OPERATIONS_VERSION,
                )
                .is_ok()
            );
        }
    }

    #[test]
    fn crop_to_selection_requires_v5_and_v4_rejects_its_payload() {
        let bytes = serde_json::to_vec(&[Command::CropToSelection]).unwrap();
        let commands: Vec<Command> = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(
            operations_version(&commands),
            CROP_TO_SELECTION_OPERATIONS_VERSION
        );
        assert!(validate_operations_version(&commands, SELECTION_OPERATIONS_VERSION).is_err());
        assert!(
            validate_operations_version(&commands, CROP_TO_SELECTION_OPERATIONS_VERSION).is_ok()
        );
    }

    #[test]
    fn color_selection_commands_require_v6_while_rectangles_remain_v4() {
        let rectangle = [Command::SetSelection {
            selection: Some(Selection::rectangle(1, 2, 3, 4)),
        }];
        assert_eq!(operations_version(&rectangle), SELECTION_OPERATIONS_VERSION);

        let color = [Command::SetSelection {
            selection: Some(Selection::color_mask(1, 2, 2, 1, vec![255, 64])),
        }];
        assert_eq!(
            operations_version(&color),
            COLOR_SELECTION_OPERATIONS_VERSION
        );
        assert!(validate_operations_version(&color, CROP_TO_SELECTION_OPERATIONS_VERSION).is_err());

        let marker = [Command::MagicWandSnapshot {
            x: 4,
            y: 5,
            tolerance: 32,
            contiguous: true,
            antialias: true,
        }];
        assert_eq!(
            operations_version(&marker),
            COLOR_SELECTION_OPERATIONS_VERSION
        );
        assert!(validate_operations_version(&marker, COLOR_SELECTION_OPERATIONS_VERSION).is_ok());
    }

    #[test]
    fn compatibility_advertises_operation_versions_one_through_twelve() {
        for version in LEGACY_OPERATIONS_VERSION..=RASTER_PIXEL_MASK_OPERATIONS_VERSION {
            assert!(
                PrismCompatibility.supports_operations(&Encoding::new(OPERATIONS_FAMILY, version,))
            );
        }
        for version in [0, RASTER_PIXEL_MASK_OPERATIONS_VERSION + 1] {
            assert!(
                !PrismCompatibility
                    .supports_operations(&Encoding::new(OPERATIONS_FAMILY, version,))
            );
        }
    }

    #[test]
    fn lasso_commands_require_v9_while_paint_stays_v8() {
        let lasso = [Command::LassoSelection {
            points: LassoPath::new(vec![
                LassoPoint::from_canvas(1.0, 1.0).unwrap(),
                LassoPoint::from_canvas(5.0, 1.0).unwrap(),
                LassoPoint::from_canvas(1.0, 5.0).unwrap(),
            ])
            .unwrap(),
            mode: SelectionCombineMode::Replace,
            antialias: true,
        }];
        assert_eq!(operations_version(&lasso), LASSO_OPERATIONS_VERSION);
        assert!(validate_operations_version(&lasso, PAINT_OPERATIONS_VERSION).is_err());
        assert!(validate_operations_version(&lasso, LASSO_OPERATIONS_VERSION).is_ok());
    }

    #[test]
    fn path_commands_require_v7_while_color_selection_stays_v6() {
        let geometry = crate::PathGeometry::new(
            10,
            10,
            false,
            crate::PathFillRule::EvenOdd,
            vec![
                crate::PathAnchor::corner(0.0, 0.0),
                crate::PathAnchor::corner(10.0, 10.0),
            ],
        )
        .unwrap();
        let path = [Command::AddPath {
            name: None,
            geometry,
            color: [255; 4],
            x: 0.0,
            y: 0.0,
        }];
        assert_eq!(operations_version(&path), PATH_OPERATIONS_VERSION);
        assert!(validate_operations_version(&path, COLOR_SELECTION_OPERATIONS_VERSION).is_err());
        assert!(validate_operations_version(&path, PATH_OPERATIONS_VERSION).is_ok());

        let color = [Command::MagicWandSnapshot {
            x: 1,
            y: 2,
            tolerance: 10,
            contiguous: true,
            antialias: true,
        }];
        assert_eq!(
            operations_version(&color),
            COLOR_SELECTION_OPERATIONS_VERSION
        );
    }

    #[test]
    fn transfer_envelopes_match_their_operation_schema() {
        let transfer = LayerTransfer {
            format: LAYER_TRANSFER_FORMAT.into(),
            version: LAYER_TRANSFER_VERSION,
            layer: Layer::default(),
            font_asset: None,
        };
        let mut compatible = [Command::InsertLayer {
            transfer: Box::new(transfer.clone()),
            index: None,
        }];
        assert_eq!(
            operations_version(&compatible),
            RASTER_PIXEL_MASK_OPERATIONS_VERSION
        );
        assert!(
            validate_operations_version(&compatible, LAYER_TRANSFER_OPERATIONS_VERSION).is_err()
        );
        downgrade_compatible_transfers(&mut compatible);
        assert_eq!(
            operations_version(&compatible),
            LAYER_TRANSFER_OPERATIONS_VERSION
        );

        let mut styled = transfer;
        styled.layer.style.drop_shadow = Some(DropShadow::default());
        let mut styled = [Command::InsertLayer {
            transfer: Box::new(styled),
            index: None,
        }];
        downgrade_compatible_transfers(&mut styled);
        assert_eq!(
            operations_version(&styled),
            LAYER_EFFECTS_OPERATIONS_VERSION
        );
    }

    #[test]
    fn raster_pixel_mask_transfers_require_v12_even_with_an_older_transfer_marker() {
        let raster_mask = PixelMask::new(2, 1, vec![255, 0]);
        for transfer_version in [
            PAINT_LAYER_TRANSFER_VERSION,
            RASTER_PIXEL_MASK_LAYER_TRANSFER_VERSION,
        ] {
            let commands = [Command::InsertLayer {
                transfer: Box::new(LayerTransfer {
                    format: LAYER_TRANSFER_FORMAT.into(),
                    version: transfer_version,
                    layer: Layer {
                        kind: LayerKind::Raster {
                            path: "source.png".into(),
                            original_path: None,
                        },
                        pixel_mask: Some(raster_mask.clone()),
                        ..Layer::default()
                    },
                    font_asset: None,
                }),
                index: None,
            }];
            assert_eq!(
                operations_version(&commands),
                RASTER_PIXEL_MASK_OPERATIONS_VERSION
            );
            assert!(
                validate_operations_version(&commands, DOCUMENT_LIFECYCLE_OPERATIONS_VERSION)
                    .is_err()
            );
            assert!(
                validate_operations_version(&commands, RASTER_PIXEL_MASK_OPERATIONS_VERSION)
                    .is_ok()
            );
        }
    }

    #[test]
    fn legacy_transfer_stamps_use_safe_operation_envelopes_and_normalize_when_possible() {
        for version in [
            crate::PATH_LAYER_TRANSFER_VERSION,
            crate::PAINT_LAYER_TRANSFER_VERSION,
        ] {
            let mut generic = insert_transfer(version, Layer::default());
            assert_eq!(
                operations_version(&generic),
                if version == crate::PATH_LAYER_TRANSFER_VERSION {
                    PATH_OPERATIONS_VERSION
                } else {
                    PAINT_OPERATIONS_VERSION
                }
            );
            assert!(
                validate_operations_version(&generic, LAYER_TRANSFER_OPERATIONS_VERSION).is_err()
            );
            downgrade_compatible_transfers(&mut generic);
            assert_eq!(inserted_transfer(&generic).version, 1);
            assert_eq!(
                operations_version(&generic),
                LAYER_TRANSFER_OPERATIONS_VERSION
            );
        }

        let geometry = crate::PathGeometry::new(
            8,
            8,
            false,
            crate::PathFillRule::EvenOdd,
            vec![
                crate::PathAnchor::corner(0.0, 0.0),
                crate::PathAnchor::corner(8.0, 8.0),
            ],
        )
        .unwrap();
        for version in [
            crate::PATH_LAYER_TRANSFER_VERSION,
            crate::PAINT_LAYER_TRANSFER_VERSION,
        ] {
            let mut path = insert_transfer(
                version,
                Layer {
                    kind: crate::LayerKind::Path {
                        geometry: geometry.clone(),
                        color: [255; 4],
                    },
                    ..Layer::default()
                },
            );
            downgrade_compatible_transfers(&mut path);
            assert_eq!(
                inserted_transfer(&path).version,
                crate::PATH_LAYER_TRANSFER_VERSION
            );
            assert_eq!(operations_version(&path), PATH_OPERATIONS_VERSION);
            assert!(
                validate_operations_version(&path, COLOR_SELECTION_OPERATIONS_VERSION).is_err()
            );
        }

        let mut paint = insert_transfer(
            crate::PAINT_LAYER_TRANSFER_VERSION,
            Layer {
                kind: crate::LayerKind::Paint {
                    program: crate::BrushProgram::new(8, 8).unwrap(),
                },
                ..Layer::default()
            },
        );
        downgrade_compatible_transfers(&mut paint);
        assert_eq!(
            inserted_transfer(&paint).version,
            crate::PAINT_LAYER_TRANSFER_VERSION
        );
        assert_eq!(operations_version(&paint), PAINT_OPERATIONS_VERSION);
        assert!(validate_operations_version(&paint, PATH_OPERATIONS_VERSION).is_err());
    }

    #[test]
    fn transfer_downgrade_never_repairs_invalid_legacy_feature_stamps() {
        let mut paint_v4 = insert_transfer(
            crate::PATH_LAYER_TRANSFER_VERSION,
            Layer {
                kind: crate::LayerKind::Paint {
                    program: crate::BrushProgram::new(8, 8).unwrap(),
                },
                ..Layer::default()
            },
        );
        assert!(inserted_transfer(&paint_v4).validate_envelope().is_err());
        downgrade_compatible_transfers(&mut paint_v4);
        assert_eq!(
            inserted_transfer(&paint_v4).version,
            crate::PATH_LAYER_TRANSFER_VERSION
        );
        assert_eq!(operations_version(&paint_v4), PAINT_OPERATIONS_VERSION);
        assert!(validate_operations_version(&paint_v4, PATH_OPERATIONS_VERSION).is_err());

        let mut dissolve_v5 = insert_transfer(
            crate::PAINT_LAYER_TRANSFER_VERSION,
            Layer {
                blend_mode: crate::BlendMode::Dissolve,
                ..Layer::default()
            },
        );
        assert!(inserted_transfer(&dissolve_v5).validate_envelope().is_err());
        downgrade_compatible_transfers(&mut dissolve_v5);
        assert_eq!(
            inserted_transfer(&dissolve_v5).version,
            crate::PAINT_LAYER_TRANSFER_VERSION
        );
        assert_eq!(
            operations_version(&dissolve_v5),
            DISSOLVE_OPERATIONS_VERSION
        );
        assert!(
            validate_operations_version(&dissolve_v5, DOCUMENT_LIFECYCLE_OPERATIONS_VERSION)
                .is_err()
        );
    }

    #[test]
    fn paint_commands_and_transfers_require_v8() {
        let program = crate::BrushProgram::new(16, 16).unwrap();
        let commands = [Command::AddPaintLayer {
            name: None,
            width: 16,
            height: 16,
        }];
        assert_eq!(operations_version(&commands), PAINT_OPERATIONS_VERSION);
        assert!(validate_operations_version(&commands, PATH_OPERATIONS_VERSION).is_err());

        let transfer = LayerTransfer {
            format: LAYER_TRANSFER_FORMAT.into(),
            version: PAINT_LAYER_TRANSFER_VERSION,
            layer: Layer {
                kind: crate::LayerKind::Paint { program },
                ..Layer::default()
            },
            font_asset: None,
        };
        let mut commands = [Command::InsertLayer {
            transfer: Box::new(transfer),
            index: None,
        }];
        downgrade_compatible_transfers(&mut commands);
        assert_eq!(operations_version(&commands), PAINT_OPERATIONS_VERSION);
    }

    #[test]
    fn dissolve_commands_require_v11() {
        for command in [
            Command::SetBlendMode {
                id: 1,
                blend_mode: crate::BlendMode::Dissolve,
            },
            Command::SetDissolveSeed {
                id: 1,
                seed: 0x1234_5678,
            },
        ] {
            assert_eq!(
                operations_version(std::slice::from_ref(&command)),
                DISSOLVE_OPERATIONS_VERSION
            );
            assert!(
                validate_operations_version(
                    std::slice::from_ref(&command),
                    DOCUMENT_LIFECYCLE_OPERATIONS_VERSION
                )
                .is_err()
            );
        }
    }
}
