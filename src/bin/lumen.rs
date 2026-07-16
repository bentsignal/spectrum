use std::{path::PathBuf, process::ExitCode};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use lumen_core::{
    AdjustmentPatch, Adjustments, Command, CropRect, CurvePoint, Project, ToneCurve, Workspace,
};
use serde::Serialize;
use serde_json::json;

#[derive(Parser)]
#[command(
    name = "lumen",
    version,
    about = "CLI-first nondestructive photo editor",
    long_about = "Lumen's CLI exposes the same command engine as its native GUI. All successful output is JSON."
)]
struct Cli {
    /// Catalog sidecar used by this command.
    #[arg(short, long, global = true, default_value = "lumen.lumencatalog")]
    catalog: PathBuf,

    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Subcommand)]
enum CliCommand {
    /// Create a new empty catalog.
    Init {
        #[arg(default_value = "Untitled")]
        name: String,
        /// Replace an existing catalog.
        #[arg(long)]
        force: bool,
    },
    /// Import one or more photos without changing the originals.
    Import { paths: Vec<PathBuf> },
    /// List catalog photos and their current edits.
    List,
    /// Inspect one photo.
    Get { id: u64 },
    /// Set one or more edit values on a photo.
    Edit {
        id: u64,
        #[command(flatten)]
        adjustments: EditArgs,
    },
    /// Set or clear a normalized crop rectangle and straighten angle.
    Crop {
        id: u64,
        #[arg(long)]
        x: Option<f32>,
        #[arg(long)]
        y: Option<f32>,
        #[arg(long)]
        width: Option<f32>,
        #[arg(long)]
        height: Option<f32>,
        #[arg(long, allow_hyphen_values = true)]
        straighten: Option<f32>,
        #[arg(long)]
        clear: bool,
    },
    /// Adjust hue, saturation, and luminance for one color range.
    Hsl {
        id: u64,
        color: ColorBand,
        #[arg(long, allow_hyphen_values = true)]
        hue: Option<f32>,
        #[arg(long, allow_hyphen_values = true)]
        saturation: Option<f32>,
        #[arg(long, allow_hyphen_values = true)]
        luminance: Option<f32>,
        #[arg(long)]
        reset: bool,
    },
    /// Set a global or per-channel tone curve from normalized x,y points.
    Curve {
        id: u64,
        channel: CurveChannel,
        /// Semicolon-separated points, for example: 0,0;0.4,0.55;1,1
        #[arg(long)]
        points: Option<String>,
        #[arg(long)]
        reset: bool,
    },
    /// Show the persistent edit history for a photo.
    History { id: u64 },
    /// Move one step backward in a photo's persistent history.
    HistoryBack { id: u64 },
    /// Move one step forward in a photo's persistent history.
    HistoryForward { id: u64 },
    /// Jump to a specific zero-based persistent history entry.
    HistoryJump { id: u64, index: usize },
    /// Reset edits for one or more photos.
    Reset { ids: Vec<u64> },
    /// Copy every edit from one photo to one or more others.
    CopyEdits {
        #[arg(long)]
        from: u64,
        #[arg(long, num_args = 1..)]
        to: Vec<u64>,
    },
    /// Rotate a photo 90 degrees.
    Rotate {
        id: u64,
        #[arg(long)]
        counterclockwise: bool,
    },
    /// Flip a photo horizontally or vertically.
    Flip {
        id: u64,
        #[arg(long, conflicts_with = "vertical")]
        horizontal: bool,
        #[arg(long)]
        vertical: bool,
    },
    /// Remove photos from the catalog (source files stay untouched).
    Remove { ids: Vec<u64> },
    /// Render a photo to a new image file.
    Export {
        id: u64,
        path: PathBuf,
        /// Optional maximum long-edge size in pixels.
        #[arg(long)]
        max_size: Option<u32>,
        #[arg(long, default_value_t = 92, value_parser = clap::value_parser!(u8).range(1..=100))]
        quality: u8,
    },
    /// Execute a serialized core command. Useful for agents and integrations.
    Run {
        /// JSON object matching the tagged core Command enum.
        json: String,
    },
    /// Print the JSON command protocol and adjustment ranges.
    Schema,
}

#[derive(Args, Default)]
struct EditArgs {
    #[arg(long, allow_hyphen_values = true)]
    exposure: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    temperature: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    tint: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    contrast: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    highlights: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    shadows: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    whites: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    blacks: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    texture: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    clarity: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    dehaze: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    vibrance: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    saturation: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    vignette: Option<f32>,
    #[arg(long)]
    sharpening: Option<f32>,
    #[arg(long)]
    noise_reduction: Option<f32>,
}

#[derive(Clone, Copy, ValueEnum)]
enum ColorBand {
    Red,
    Orange,
    Yellow,
    Green,
    Aqua,
    Blue,
    Purple,
    Magenta,
}

impl ColorBand {
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum CurveChannel {
    Master,
    Red,
    Green,
    Blue,
}

impl From<EditArgs> for AdjustmentPatch {
    fn from(args: EditArgs) -> Self {
        Self {
            exposure: args.exposure,
            temperature: args.temperature,
            tint: args.tint,
            contrast: args.contrast,
            highlights: args.highlights,
            shadows: args.shadows,
            whites: args.whites,
            blacks: args.blacks,
            texture: args.texture,
            clarity: args.clarity,
            dehaze: args.dehaze,
            vibrance: args.vibrance,
            saturation: args.saturation,
            vignette: args.vignette,
            sharpening: args.sharpening,
            noise_reduction: args.noise_reduction,
            ..Default::default()
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(value) => {
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ok": false,
                    "error": format!("{error:#}")
                }))
                .unwrap()
            );
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<serde_json::Value> {
    if matches!(&cli.command, CliCommand::Schema) {
        return Ok(schema());
    }

    if let CliCommand::Init { name, force } = &cli.command {
        if cli.catalog.exists() && !force {
            anyhow::bail!(
                "catalog {} already exists; pass --force to replace it",
                cli.catalog.display()
            );
        }
        let project = Project::new(name.clone());
        project.save(&cli.catalog)?;
        return Ok(json!({
            "ok": true,
            "action": "init",
            "catalog": cli.catalog,
            "project": project,
        }));
    }

    let project = Project::load(&cli.catalog).with_context(|| {
        format!(
            "open {} or create it first with `lumen init`",
            cli.catalog.display()
        )
    })?;
    let mut workspace = Workspace::new(project, Some(cli.catalog.clone()));

    let (result, should_save) = match cli.command {
        CliCommand::Import { paths } => (
            output(&workspace_command(
                &mut workspace,
                Command::Import { paths },
            )?),
            true,
        ),
        CliCommand::List => {
            return Ok(json!({
                "ok": true,
                "catalog": cli.catalog,
                "project": workspace.project,
            }));
        }
        CliCommand::Get { id } => {
            return Ok(json!({
                "ok": true,
                "catalog": cli.catalog,
                "photo": workspace.project.photo(id)?,
            }));
        }
        CliCommand::Edit { id, adjustments } => (
            output(&workspace_command(
                &mut workspace,
                Command::Adjust {
                    id,
                    patch: adjustments.into(),
                },
            )?),
            true,
        ),
        CliCommand::Crop {
            id,
            x,
            y,
            width,
            height,
            straighten,
            clear,
        } => {
            let mut adjustments = workspace.project.photo(id)?.adjustments.clone();
            if clear {
                adjustments.crop = None;
            } else if x.is_some() || y.is_some() || width.is_some() || height.is_some() {
                let current = adjustments.crop.unwrap_or_default();
                adjustments.crop = Some(CropRect {
                    x: x.unwrap_or(current.x),
                    y: y.unwrap_or(current.y),
                    width: width.unwrap_or(current.width),
                    height: height.unwrap_or(current.height),
                });
            }
            if let Some(value) = straighten {
                adjustments.straighten = value;
            }
            (
                output(&workspace_command(
                    &mut workspace,
                    Command::SetAdjustments { id, adjustments },
                )?),
                true,
            )
        }
        CliCommand::Hsl {
            id,
            color,
            hue,
            saturation,
            luminance,
            reset,
        } => {
            let mut adjustments = workspace.project.photo(id)?.adjustments.clone();
            let band = adjustments.hsl.band_mut(color.index());
            if reset {
                *band = Default::default();
            }
            if let Some(value) = hue {
                band.hue = value;
            }
            if let Some(value) = saturation {
                band.saturation = value;
            }
            if let Some(value) = luminance {
                band.luminance = value;
            }
            (
                output(&workspace_command(
                    &mut workspace,
                    Command::SetAdjustments { id, adjustments },
                )?),
                true,
            )
        }
        CliCommand::Curve {
            id,
            channel,
            points,
            reset,
        } => {
            let mut adjustments = workspace.project.photo(id)?.adjustments.clone();
            let curve = match channel {
                CurveChannel::Master => &mut adjustments.curves.master,
                CurveChannel::Red => &mut adjustments.curves.red,
                CurveChannel::Green => &mut adjustments.curves.green,
                CurveChannel::Blue => &mut adjustments.curves.blue,
            };
            if reset {
                *curve = ToneCurve::default();
            }
            if let Some(points) = points {
                *curve = parse_curve(&points)?;
            }
            (
                output(&workspace_command(
                    &mut workspace,
                    Command::SetAdjustments { id, adjustments },
                )?),
                true,
            )
        }
        CliCommand::History { id } => {
            let photo = workspace.project.photo(id)?;
            return Ok(
                json!({ "ok": true, "photo_id": id, "cursor": photo.history_cursor, "entries": photo.history }),
            );
        }
        CliCommand::HistoryBack { id } => (
            output(&workspace_command(
                &mut workspace,
                Command::HistoryBack { id },
            )?),
            true,
        ),
        CliCommand::HistoryForward { id } => (
            output(&workspace_command(
                &mut workspace,
                Command::HistoryForward { id },
            )?),
            true,
        ),
        CliCommand::HistoryJump { id, index } => (
            output(&workspace_command(
                &mut workspace,
                Command::HistoryJump { id, index },
            )?),
            true,
        ),
        CliCommand::Reset { ids } => (
            output(&workspace_command(&mut workspace, Command::Reset { ids })?),
            true,
        ),
        CliCommand::CopyEdits { from, to } => {
            workspace.execute(Command::CopyEdits { id: from })?;
            (
                output(&workspace_command(
                    &mut workspace,
                    Command::PasteEdits { ids: to },
                )?),
                true,
            )
        }
        CliCommand::Rotate {
            id,
            counterclockwise,
        } => (
            output(&workspace_command(
                &mut workspace,
                Command::Rotate {
                    id,
                    clockwise: !counterclockwise,
                },
            )?),
            true,
        ),
        CliCommand::Flip {
            id,
            horizontal,
            vertical,
        } => {
            let command = if vertical && !horizontal {
                Command::FlipVertical { id }
            } else {
                Command::FlipHorizontal { id }
            };
            (output(&workspace_command(&mut workspace, command)?), true)
        }
        CliCommand::Remove { ids } => (
            output(&workspace_command(&mut workspace, Command::Remove { ids })?),
            true,
        ),
        CliCommand::Export {
            id,
            path,
            max_size,
            quality,
        } => (
            output(&workspace_command(
                &mut workspace,
                Command::Export {
                    id,
                    path,
                    max_size,
                    quality,
                },
            )?),
            false,
        ),
        CliCommand::Run { json } => {
            let command: Command = serde_json::from_str(&json).context("invalid command JSON")?;
            let should_save = !matches!(command, Command::Open { .. } | Command::Export { .. });
            (
                output(&workspace_command(&mut workspace, command)?),
                should_save,
            )
        }
        CliCommand::Init { .. } | CliCommand::Schema => {
            unreachable!()
        }
    };

    if should_save {
        workspace.project.save(&cli.catalog)?;
    }
    Ok(json!({
        "result": result,
        "catalog": cli.catalog,
    }))
}

fn workspace_command(
    workspace: &mut Workspace,
    command: Command,
) -> Result<lumen_core::CommandOutput> {
    workspace.execute(command)
}

fn output(value: &impl Serialize) -> serde_json::Value {
    serde_json::to_value(value).unwrap()
}

fn parse_curve(value: &str) -> Result<ToneCurve> {
    let mut points = Vec::new();
    for item in value.split(';').filter(|item| !item.trim().is_empty()) {
        let (x, y) = item
            .split_once(',')
            .with_context(|| format!("invalid curve point '{item}'; expected x,y"))?;
        points.push(CurvePoint {
            x: x.trim()
                .parse()
                .with_context(|| format!("invalid curve x '{x}'"))?,
            y: y.trim()
                .parse()
                .with_context(|| format!("invalid curve y '{y}'"))?,
        });
    }
    if points.len() < 2 {
        anyhow::bail!("a tone curve needs at least two points");
    }
    Ok(ToneCurve { points }.sanitized())
}

fn schema() -> serde_json::Value {
    json!({
        "ok": true,
        "catalog_version": lumen_core::project::CATALOG_VERSION,
        "output": "JSON on stdout; structured errors on stderr; nonzero exit on failure",
        "adjustments": {
            "exposure": { "range": [-5.0, 5.0], "unit": "stops", "default": 0.0 },
            "temperature": { "range": [-100, 100], "default": 0 },
            "tint": { "range": [-100, 100], "default": 0 },
            "contrast": { "range": [-100, 100], "default": 0 },
            "highlights": { "range": [-100, 100], "default": 0 },
            "shadows": { "range": [-100, 100], "default": 0 },
            "whites": { "range": [-100, 100], "default": 0 },
            "blacks": { "range": [-100, 100], "default": 0 },
            "texture": { "range": [-100, 100], "default": 0 },
            "clarity": { "range": [-100, 100], "default": 0 },
            "dehaze": { "range": [-100, 100], "default": 0 },
            "vibrance": { "range": [-100, 100], "default": 0 },
            "saturation": { "range": [-100, 100], "default": 0 },
            "vignette": { "range": [-100, 100], "default": 0 },
            "sharpening": { "range": [0, 100], "default": 0 },
            "noise_reduction": { "range": [0, 100], "default": 0 },
            "crop": { "type": "normalized rectangle", "fields": ["x", "y", "width", "height"] },
            "straighten": { "range": [-45, 45], "unit": "degrees" },
            "hsl": { "colors": ["red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta"], "range": [-100, 100] },
            "curves": { "channels": ["master", "red", "green", "blue"], "points": "normalized x,y pairs" },
            "rotation": { "values": [0, 90, 180, 270], "unit": "degrees clockwise" },
            "flip_horizontal": { "type": "boolean" },
            "flip_vertical": { "type": "boolean" }
        },
        "raw_command_examples": [
            { "command": "adjust", "id": 1, "patch": { "exposure": 0.7, "shadows": 18 } },
            { "command": "copy-edits", "id": 1 },
            { "command": "paste-edits", "ids": [2, 3] },
            { "command": "history-back", "id": 1 },
            { "command": "export", "id": 1, "path": "output.jpg", "max_size": 2400, "quality": 92 }
        ]
    })
}

#[allow(dead_code)]
fn _assert_adjustments_are_public(_: Adjustments) {}
