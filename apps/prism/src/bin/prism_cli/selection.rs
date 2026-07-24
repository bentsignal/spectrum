use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use prism_core::{Command, LassoPath, LassoPoint, Selection, SelectionCombineMode};

use super::parse_color;

#[derive(Args)]
pub(super) struct SelectionArgs {
    #[command(subcommand)]
    action: SelectionAction,
}

#[derive(Subcommand)]
enum SelectionAction {
    /// Replace the current selection with a document-space pixel rectangle.
    Rectangle {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    /// Select pixels similar to the exact composited color at a canvas point.
    MagicWand {
        x: u32,
        y: u32,
        #[arg(long, default_value_t = 20)]
        /// Max-channel distance in premultiplied RGBA (0 exact, 255 all colors).
        tolerance: u8,
        /// Match similar pixels across the whole canvas instead of one connected region.
        #[arg(long)]
        noncontiguous: bool,
        /// Disable the soft one-pixel color boundary.
        #[arg(long)]
        no_antialias: bool,
    },
    /// Draw a bounded fixed-point freehand polygon selection.
    Lasso {
        /// Document-space point as x,y. Repeat at least three times.
        #[arg(long = "point", required = true, value_parser = parse_lasso_point)]
        points: Vec<LassoPoint>,
        #[arg(long, value_enum, default_value_t = SelectionModeArg::Replace)]
        mode: SelectionModeArg,
        /// Disable deterministic 4x4 coverage anti-aliasing.
        #[arg(long)]
        no_antialias: bool,
    },
    /// Clear the current document selection.
    Clear,
    /// Crop the canvas to the current rectangular selection and clear it atomically.
    Crop,
    /// Create a new editable solid-color layer at the selected bounds.
    Fill {
        #[arg(long, default_value = "5dd8c7ff")]
        color: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Nondestructively hide selected pixels on one raster image layer.
    Delete { layer: u64 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum SelectionModeArg {
    #[default]
    Replace,
    Add,
    Subtract,
    Intersect,
}

impl From<SelectionModeArg> for SelectionCombineMode {
    fn from(value: SelectionModeArg) -> Self {
        match value {
            SelectionModeArg::Replace => Self::Replace,
            SelectionModeArg::Add => Self::Add,
            SelectionModeArg::Subtract => Self::Subtract,
            SelectionModeArg::Intersect => Self::Intersect,
        }
    }
}

fn parse_lasso_point(value: &str) -> Result<LassoPoint, String> {
    let (x, y) = value
        .split_once(',')
        .ok_or_else(|| "lasso point must be x,y".to_owned())?;
    let x = x
        .trim()
        .parse::<f32>()
        .map_err(|error| format!("invalid lasso x coordinate: {error}"))?;
    let y = y
        .trim()
        .parse::<f32>()
        .map_err(|error| format!("invalid lasso y coordinate: {error}"))?;
    LassoPoint::from_canvas(x, y).map_err(|error| error.to_string())
}

pub(super) fn command(arguments: SelectionArgs) -> Result<Command> {
    Ok(match arguments.action {
        SelectionAction::Rectangle {
            x,
            y,
            width,
            height,
        } => Command::SetSelection {
            selection: Some(Selection::rectangle(x, y, width, height)),
        },
        SelectionAction::MagicWand {
            x,
            y,
            tolerance,
            noncontiguous,
            no_antialias,
        } => Command::MagicWandSelection {
            x,
            y,
            tolerance,
            contiguous: !noncontiguous,
            antialias: !no_antialias,
            resolved_selection: None,
        },
        SelectionAction::Lasso {
            points,
            mode,
            no_antialias,
        } => Command::LassoSelection {
            points: LassoPath::new(points)?,
            mode: mode.into(),
            antialias: !no_antialias,
        },
        SelectionAction::Clear => Command::SetSelection { selection: None },
        SelectionAction::Crop => Command::CropToSelection,
        SelectionAction::Fill { color, name } => Command::FillSelection {
            color: parse_color(&color)?,
            name,
        },
        SelectionAction::Delete { layer } => Command::DeleteSelectedPixels { id: layer },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct SelectionTestCli {
        #[command(flatten)]
        selection: SelectionArgs,
    }

    #[test]
    fn omitted_magic_wand_tolerance_matches_the_gui_default() {
        let parsed =
            SelectionTestCli::try_parse_from(["selection-test", "magic-wand", "4", "5"]).unwrap();
        assert!(matches!(
            parsed.selection.action,
            SelectionAction::MagicWand { tolerance: 20, .. }
        ));
    }

    #[test]
    fn selection_actions_map_to_the_public_command_protocol() {
        let rectangle = command(SelectionArgs {
            action: SelectionAction::Rectangle {
                x: 4,
                y: 5,
                width: 20,
                height: 10,
            },
        })
        .unwrap();
        assert_eq!(
            rectangle,
            Command::SetSelection {
                selection: Some(Selection::rectangle(4, 5, 20, 10))
            }
        );

        assert_eq!(
            command(SelectionArgs {
                action: SelectionAction::Delete { layer: 7 },
            })
            .unwrap(),
            Command::DeleteSelectedPixels { id: 7 }
        );

        assert_eq!(
            command(SelectionArgs {
                action: SelectionAction::Lasso {
                    points: vec![
                        parse_lasso_point("1,2").unwrap(),
                        parse_lasso_point("5,2").unwrap(),
                        parse_lasso_point("1,6").unwrap(),
                    ],
                    mode: SelectionModeArg::Add,
                    no_antialias: true,
                },
            })
            .unwrap(),
            Command::LassoSelection {
                points: LassoPath::new(vec![
                    LassoPoint::from_canvas(1.0, 2.0).unwrap(),
                    LassoPoint::from_canvas(5.0, 2.0).unwrap(),
                    LassoPoint::from_canvas(1.0, 6.0).unwrap(),
                ])
                .unwrap(),
                mode: SelectionCombineMode::Add,
                antialias: false,
            }
        );

        assert_eq!(
            command(SelectionArgs {
                action: SelectionAction::MagicWand {
                    x: 12,
                    y: 18,
                    tolerance: 24,
                    noncontiguous: true,
                    no_antialias: false,
                },
            })
            .unwrap(),
            Command::MagicWandSelection {
                x: 12,
                y: 18,
                tolerance: 24,
                contiguous: false,
                antialias: true,
                resolved_selection: None,
            }
        );

        assert_eq!(
            command(SelectionArgs {
                action: SelectionAction::Crop,
            })
            .unwrap(),
            Command::CropToSelection
        );

        let fill = command(SelectionArgs {
            action: SelectionAction::Fill {
                color: "12345678".into(),
                name: Some("Wash".into()),
            },
        })
        .unwrap();
        assert_eq!(
            fill,
            Command::FillSelection {
                color: [0x12, 0x34, 0x56, 0x78],
                name: Some("Wash".into())
            }
        );
    }
}
