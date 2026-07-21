use anyhow::Result;
use clap::{Args, Subcommand};
use prism_core::{Command, Selection};

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
    /// Clear the current document selection.
    Clear,
    /// Create a new editable solid-color layer at the selected bounds.
    Fill {
        #[arg(long, default_value = "5dd8c7ff")]
        color: String,
        #[arg(long)]
        name: Option<String>,
    },
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
        SelectionAction::Clear => Command::SetSelection { selection: None },
        SelectionAction::Fill { color, name } => Command::FillSelection {
            color: parse_color(&color)?,
            name,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_and_fill_map_to_the_public_command_protocol() {
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
