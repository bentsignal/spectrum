use anyhow::{Result, bail};
use spectrum_revisions::{Compatibility, Encoding};

use crate::Command;

pub(super) const SNAPSHOT_FAMILY: &str = "spectrum.prism.document";
pub(super) const OPERATIONS_FAMILY: &str = "spectrum.prism.commands";
pub(super) const LEGACY_SNAPSHOT_VERSION: u32 = 1;
pub(super) const COMPRESSED_SNAPSHOT_VERSION: u32 = 2;
pub(super) const LEGACY_OPERATIONS_VERSION: u32 = 1;
pub(super) const LAYER_TRANSFER_OPERATIONS_VERSION: u32 = 2;
pub(super) const LAYER_EFFECTS_OPERATIONS_VERSION: u32 = 3;
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
                _ => false,
            }
    }

    fn supports_operations(&self, encoding: &Encoding) -> bool {
        encoding.family == OPERATIONS_FAMILY
            && (LEGACY_OPERATIONS_VERSION..=LAYER_EFFECTS_OPERATIONS_VERSION)
                .contains(&encoding.version)
            && encoding.required_capabilities.is_empty()
    }
}

pub(super) fn operations_version(commands: &[Command]) -> u32 {
    if commands.iter().any(|command| {
        matches!(
            command,
            Command::SetLayerStyle { .. } | Command::SetShapeFill { .. }
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
