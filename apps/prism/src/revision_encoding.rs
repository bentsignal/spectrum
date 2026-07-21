use anyhow::{Result, bail};
use spectrum_revisions::{Compatibility, Encoding};

use crate::Command;

pub(super) const SNAPSHOT_FAMILY: &str = "spectrum.prism.document";
pub(super) const OPERATIONS_FAMILY: &str = "spectrum.prism.commands";
pub(super) const LEGACY_SNAPSHOT_VERSION: u32 = 1;
pub(super) const COMPRESSED_SNAPSHOT_VERSION: u32 = 2;
pub(super) const LAYER_EFFECTS_SNAPSHOT_VERSION: u32 = 3;
pub(super) const SELECTION_SNAPSHOT_VERSION: u32 = 4;
pub(super) const LEGACY_OPERATIONS_VERSION: u32 = 1;
pub(super) const LAYER_TRANSFER_OPERATIONS_VERSION: u32 = 2;
pub(super) const LAYER_EFFECTS_OPERATIONS_VERSION: u32 = 3;
pub(super) const SELECTION_OPERATIONS_VERSION: u32 = 4;
pub(super) const CROP_TO_SELECTION_OPERATIONS_VERSION: u32 =
    crate::PRISM_COMMAND_OPERATIONS_VERSION;
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
                _ => false,
            }
    }

    fn supports_operations(&self, encoding: &Encoding) -> bool {
        encoding.family == OPERATIONS_FAMILY
            && (LEGACY_OPERATIONS_VERSION..=CROP_TO_SELECTION_OPERATIONS_VERSION)
                .contains(&encoding.version)
            && encoding.required_capabilities.is_empty()
    }
}

pub(super) fn operations_version(commands: &[Command]) -> u32 {
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
                if transfer.version >= crate::LAYER_TRANSFER_VERSION
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
            && transfer.version == crate::LAYER_TRANSFER_VERSION
            && transfer.layer.style.is_empty()
            && transfer.layer.shape_fill.is_none()
        {
            transfer.version = 1;
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
        DropShadow, LAYER_TRANSFER_FORMAT, LAYER_TRANSFER_VERSION, Layer, LayerStyle,
        LayerTransfer, Selection,
    };

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
    fn compatibility_advertises_operation_versions_one_through_five() {
        for version in LEGACY_OPERATIONS_VERSION..=CROP_TO_SELECTION_OPERATIONS_VERSION {
            assert!(
                PrismCompatibility.supports_operations(&Encoding::new(OPERATIONS_FAMILY, version,))
            );
        }
        for version in [0, CROP_TO_SELECTION_OPERATIONS_VERSION + 1] {
            assert!(
                !PrismCompatibility
                    .supports_operations(&Encoding::new(OPERATIONS_FAMILY, version,))
            );
        }
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
            LAYER_EFFECTS_OPERATIONS_VERSION
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
}
