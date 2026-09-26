use clap::{Subcommand, ValueEnum};
use prism_core::{Alignment, GuideOrientation};

#[derive(Subcommand)]
pub(super) enum GuideCommand {
    Add {
        #[arg(value_enum)]
        orientation: CliGuideOrientation,
        #[arg(allow_negative_numbers = true)]
        position: f32,
    },
    Move {
        id: u64,
        #[arg(allow_negative_numbers = true)]
        position: f32,
    },
    Remove {
        id: u64,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum CliGuideOrientation {
    Horizontal,
    Vertical,
}

impl From<CliGuideOrientation> for GuideOrientation {
    fn from(value: CliGuideOrientation) -> Self {
        match value {
            CliGuideOrientation::Horizontal => Self::Horizontal,
            CliGuideOrientation::Vertical => Self::Vertical,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum CliAlignment {
    Left,
    HorizontalCenter,
    Right,
    Top,
    VerticalCenter,
    Bottom,
}

impl From<CliAlignment> for Alignment {
    fn from(value: CliAlignment) -> Self {
        match value {
            CliAlignment::Left => Self::Left,
            CliAlignment::HorizontalCenter => Self::HorizontalCenter,
            CliAlignment::Right => Self::Right,
            CliAlignment::Top => Self::Top,
            CliAlignment::VerticalCenter => Self::VerticalCenter,
            CliAlignment::Bottom => Self::Bottom,
        }
    }
}
