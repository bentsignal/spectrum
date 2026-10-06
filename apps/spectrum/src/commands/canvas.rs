use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use spectrum_canvas::Workspace;
#[cfg(test)]
use spectrum_canvas::{Command, Document};
use spectrum_document::{Actor, ActorKind, SessionId};

#[path = "canvas_commands/alignment.rs"]
mod alignment;
use alignment::{CliAlignment, GuideCommand};
#[path = "canvas_commands/benchmark.rs"]
mod benchmark;
use benchmark::{BenchmarkProfile, benchmark};
#[path = "canvas_commands/blend.rs"]
mod blend;
use blend::CliBlend;
#[path = "canvas_commands/dispatch.rs"]
mod dispatch;
pub(super) use dispatch::decode_commands;
#[path = "canvas_commands/effects.rs"]
mod effects;
use effects::{GradientArgs, ShadowArgs};
#[path = "canvas_commands/style_effects.rs"]
mod style_effects;
use style_effects::EffectArgs;
#[path = "canvas_commands/paths.rs"]
mod paths;
use paths::{PathArgs, PathCommand, VectorMaskArgs};
#[path = "canvas_commands/paint.rs"]
mod paint;
use paint::PaintArgs;
#[path = "canvas_commands/schema.rs"]
mod schema;
use schema::schema;
#[path = "canvas_commands/selection.rs"]
mod selection;
use selection::SelectionArgs;
#[path = "canvas_commands/typography.rs"]
mod typography;
use typography::{CliTextLayout, TypographyArgs, text_shaping, updated_typography};
#[path = "canvas_commands/transfer.rs"]
mod transfer;
use transfer::{LayerCopyArgs, LayerPasteArgs};

#[derive(Parser)]
#[command(name = "canvas", version, about = "Spectrum canvas editing commands")]
struct Cli {
    /// The canvas's document, found from `--asset` in the library.
    #[arg(skip)]
    project: PathBuf,
    /// The session edits are made in; a fresh one when unset.
    #[arg(skip)]
    session: Option<SessionId>,
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Subcommand)]
enum CliCommand {
    /// Creates a canvas document outside a library, for tests.
    #[cfg(test)]
    Init {
        name: String,
        #[arg(long, default_value_t = 1920)]
        width: u32,
        #[arg(long, default_value_t = 1080)]
        height: u32,
        #[arg(long, default_value = "18191dff")]
        background: String,
    },
    /// Inspect the complete layered document.
    List,
    /// Rename the canvas's document title.
    RenameDocument {
        name: String,
    },
    /// Add an immutable image source as a raster layer.
    AddImage {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value_t = 0.0)]
        x: f32,
        #[arg(long, default_value_t = 0.0)]
        y: f32,
    },
    /// Add editable text in the bundled Ubuntu Light font.
    AddText {
        text: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value_t = 72.0)]
        size: f32,
        #[arg(long, default_value = "ffffffff")]
        color: String,
        #[arg(long, default_value_t = 0.0)]
        x: f32,
        #[arg(long, default_value_t = 0.0)]
        y: f32,
        /// Permanent layout engine for the new text.
        #[arg(long, value_enum, default_value_t = CliTextLayout::HarfbuzzV1)]
        layout: CliTextLayout,
        /// Canonical BCP-47 shaping language; omitted means und.
        #[arg(long)]
        language: Option<String>,
    },
    /// Embed an OpenType font in this canvas.
    FontImport {
        path: PathBuf,
    },
    /// Search bundled and embedded font faces.
    FontList {
        #[arg(long)]
        query: Option<String>,
        /// Also list fonts installed on this computer, which `font-import`
        /// can embed by path.
        #[arg(long)]
        system: bool,
    },
    /// Analyze current embedded-font character usage and cmap coverage without modifying bytes.
    FontUsage {
        /// Limit analysis to one embedded font asset.
        #[arg(long)]
        font_id: Option<u64>,
    },
    /// Update one text layer's font, paragraph metrics, and effects.
    Typography(TypographyArgs),
    /// Serialize one layer and its referenced font for cross-document transfer.
    LayerCopy(LayerCopyArgs),
    /// Insert a layer transfer as one durable edit.
    LayerPaste(LayerPasteArgs),
    /// Add an editable vector-style rectangle layer.
    AddRectangle {
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value_t = 640)]
        width: u32,
        #[arg(long, default_value_t = 360)]
        height: u32,
        #[arg(long, default_value = "ae7bffff")]
        color: String,
        #[arg(long, default_value_t = 0.0)]
        radius: f32,
        #[arg(long, default_value_t = 0.0)]
        x: f32,
        #[arg(long, default_value_t = 0.0)]
        y: f32,
    },
    /// Add an editable vector ellipse layer.
    AddEllipse {
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value_t = 360)]
        width: u32,
        #[arg(long, default_value_t = 360)]
        height: u32,
        #[arg(long, default_value = "f7b266ff")]
        color: String,
        #[arg(long, default_value_t = 0.0)]
        x: f32,
        #[arg(long, default_value_t = 0.0)]
        y: f32,
    },
    /// Add or replace editable cubic paths.
    Path(PathArgs),
    /// Add Paint layers or append nondestructive Brush/Eraser strokes.
    Paint(PaintArgs),
    /// Apply or clear one reusable closed vector mask.
    VectorMask(VectorMaskArgs),
    EditText {
        id: u64,
        text: String,
        #[arg(long, default_value_t = 72.0)]
        size: f32,
        #[arg(long, default_value = "ffffffff")]
        color: String,
    },
    EditRectangle {
        id: u64,
        #[arg(long)]
        width: u32,
        #[arg(long)]
        height: u32,
        #[arg(long, default_value = "ae7bffff")]
        color: String,
        #[arg(long, default_value_t = 0.0)]
        radius: f32,
    },
    EditEllipse {
        id: u64,
        #[arg(long)]
        width: u32,
        #[arg(long)]
        height: u32,
        #[arg(long, default_value = "f7b266ff")]
        color: String,
    },
    Stroke {
        id: u64,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        enabled: bool,
        #[arg(long, default_value_t = 4.0)]
        width: f32,
        #[arg(long, default_value = "ffffffff")]
        color: String,
    },
    /// Add, update, or clear a portable layer drop shadow.
    Shadow(ShadowArgs),
    /// Add, update, or clear a layer style: stroke, outer or inner glow,
    /// inner shadow, or color overlay. Other styles on the layer stay.
    Effect(EffectArgs),
    /// Add, update, or clear a bounded multi-stop shape gradient.
    Gradient(GradientArgs),
    /// Freeze an editable shape into an embedded raster asset.
    RasterizeShape {
        id: u64,
        /// Raster pixels per shape unit. Defaults to the current transform scale.
        #[arg(long)]
        scale: Option<f32>,
    },
    Rename {
        id: u64,
        name: String,
    },
    Delete {
        id: u64,
    },
    Duplicate {
        id: u64,
    },
    Select {
        id: Option<u64>,
    },
    /// Create, clear, color-select, crop, fill, or nondestructively delete pixels.
    Selection(SelectionArgs),
    Reorder {
        id: u64,
        index: usize,
    },
    Visibility {
        id: u64,
        #[arg(action = clap::ArgAction::Set)]
        visible: bool,
    },
    Lock {
        id: u64,
        #[arg(action = clap::ArgAction::Set)]
        locked: bool,
    },
    Opacity {
        id: u64,
        opacity: f32,
    },
    Blend {
        id: u64,
        mode: CliBlend,
        /// Stable 32-bit pattern seed for Dissolve.
        #[arg(long)]
        seed: Option<u32>,
    },
    Transform {
        id: u64,
        #[arg(long)]
        x: f32,
        #[arg(long)]
        y: f32,
        #[arg(long, default_value_t = 1.0)]
        scale_x: f32,
        #[arg(long, default_value_t = 1.0)]
        scale_y: f32,
        #[arg(long, default_value_t = 0.0)]
        rotation: f32,
    },
    /// Set one layer's absolute clockwise rotation in degrees.
    Rotate {
        id: u64,
        #[arg(allow_negative_numbers = true)]
        degrees: f32,
    },
    /// Align a layer's transformed visual bounds to the canvas or another layer.
    Align {
        id: u64,
        #[arg(value_enum)]
        alignment: CliAlignment,
        #[arg(long)]
        to_layer: Option<u64>,
    },
    /// Enable or disable object and guide snapping for the document.
    Snapping {
        #[arg(action = clap::ArgAction::Set)]
        enabled: bool,
    },
    /// Add, move, or remove a persistent document guide.
    Guide {
        #[command(subcommand)]
        command: GuideCommand,
    },
    Adjust {
        id: u64,
        #[arg(long)]
        exposure: Option<f32>,
        #[arg(long)]
        contrast: Option<f32>,
        #[arg(long)]
        highlights: Option<f32>,
        #[arg(long)]
        shadows: Option<f32>,
        #[arg(long)]
        temperature: Option<f32>,
        #[arg(long)]
        tint: Option<f32>,
        #[arg(long)]
        vibrance: Option<f32>,
        #[arg(long)]
        saturation: Option<f32>,
        #[arg(long)]
        clarity: Option<f32>,
        #[arg(long)]
        dehaze: Option<f32>,
        #[arg(long)]
        noise_reduction: Option<f32>,
        #[arg(long)]
        sharpening: Option<f32>,
    },
    ResetAdjustments {
        id: u64,
    },
    Mask {
        id: u64,
        #[arg(long, default_value_t = 0.0)]
        x: f32,
        #[arg(long, default_value_t = 0.0)]
        y: f32,
        #[arg(long, default_value_t = 1.0)]
        width: f32,
        #[arg(long, default_value_t = 1.0)]
        height: f32,
        #[arg(long)]
        invert: bool,
        #[arg(long)]
        clear: bool,
    },
    Clip {
        id: u64,
        #[arg(action = clap::ArgAction::Set)]
        enabled: bool,
    },
    Canvas {
        width: u32,
        height: u32,
        #[arg(long, default_value = "18191dff")]
        background: String,
    },
    Crop {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    /// Read the composited color at a canvas pixel, as the eyedropper does.
    Sample {
        x: u32,
        y: u32,
    },
    /// Show the canvas's revisions and where each session is.
    History,
    /// Return to a revision from `history`.
    HistoryJump {
        revision: spectrum_document::RevisionId,
    },
    /// Execute one Command JSON object, or an array applied as one edit.
    Run {
        json: String,
    },
    /// Print the machine-facing Command protocol and examples.
    Schema,
    /// Run deterministic command and compositing performance workloads.
    Benchmark {
        #[arg(long)]
        strict: bool,
        /// Budget calibration: workstation interaction or GitHub's shared Linux runner.
        #[arg(long, value_enum, default_value_t = BenchmarkProfile::Interactive)]
        profile: BenchmarkProfile,
    },
}

pub(super) fn definition() -> clap::Command {
    use clap::CommandFactory;
    Cli::command()
}

/// Runs an editor command on a canvas document in `session`.
pub(super) fn execute_target(
    matches: &mut clap::ArgMatches,
    path: PathBuf,
    session: SessionId,
) -> Result<serde_json::Value> {
    use clap::FromArgMatches;
    let command = if matches.subcommand_name() == Some("inspect") {
        CliCommand::List
    } else {
        CliCommand::from_arg_matches(matches)?
    };
    run(Cli {
        project: path,
        session: Some(session),
        command,
    })
}

/// Runs a command that needs no canvas, such as the benchmark.
pub(super) fn execute_standalone(matches: &clap::ArgMatches) -> Result<Value> {
    use clap::FromArgMatches;
    run(Cli {
        project: PathBuf::new(),
        session: None,
        command: CliCommand::from_arg_matches(matches)?,
    })
}

pub(super) fn protocol() -> serde_json::Value {
    schema()
}

fn run(cli: Cli) -> Result<Value> {
    match cli.command {
        #[cfg(test)]
        CliCommand::Init {
            name,
            width,
            height,
            background,
        } => {
            let mut document = Document::new(name, width, height);
            document.background = parse_color(&background)?;
            let workspace =
                Workspace::create(&cli.project, document, cli_actor(), SessionId::new())?;
            Ok(json!({"ok": true, "action": "init", "document": workspace.document}))
        }
        CliCommand::List => {
            let document = Workspace::read(&cli.project)?;
            Ok(json!({"ok": true, "document": document}))
        }
        CliCommand::FontList { query, system } => Ok(typography::font_list(
            &Workspace::read(&cli.project)?,
            query,
            system,
        )),
        CliCommand::FontUsage { font_id } => {
            typography::font_usage(&Workspace::read(&cli.project)?, font_id)
        }
        CliCommand::LayerCopy(arguments) => {
            transfer::copy_layer(&Workspace::read(&cli.project)?, arguments)
        }
        CliCommand::Sample { x, y } => {
            let document = Workspace::read(&cli.project)?;
            let [r, g, b, a] = spectrum_canvas::sample_document_color(&document, x, y)?;
            Ok(
                json!({"ok": true, "x": x, "y": y, "color": format!("{r:02x}{g:02x}{b:02x}{a:02x}")}),
            )
        }
        CliCommand::History => {
            let workspace =
                Workspace::open_newest(&cli.project, cli_actor(), cli.session.unwrap_or_default())?;
            let history = workspace.history()?.context("the canvas has no history")?;
            Ok(json!({
                "ok": true,
                "root": history.root,
                "current": history.current,
                "revisions": history.revisions,
                "sessions": history.sessions,
            }))
        }
        CliCommand::HistoryJump { revision } => {
            let mut workspace =
                Workspace::open_newest(&cli.project, cli_actor(), cli.session.unwrap_or_default())?;
            workspace.move_to(revision)?;
            Ok(json!({"ok": true, "action": "history_jump", "revision": revision}))
        }
        CliCommand::Schema => Ok(schema()),
        CliCommand::Benchmark { strict, profile } => benchmark(strict, profile),
        command => {
            let session = cli.session.unwrap_or_default();
            let mut workspace = Workspace::open_newest(&cli.project, cli_actor(), session)?;
            let plan = dispatch::semantic_commands(command, &workspace.document)?;
            let outputs = match plan.commands.as_slice() {
                [only] if !plan.atomic_batch => vec![workspace.execute(only.clone())?],
                _ => workspace.execute_batch(plan.commands)?,
            };
            if let Some(error) = workspace.pending_publish_error() {
                bail!("the edit was saved but not published: {error}");
            }
            Ok(json!({"ok": true, "results": outputs}))
        }
    }
}

fn cli_actor() -> Actor {
    Actor {
        id: "agent:spectrum-cli".into(),
        display_name: "Spectrum CLI".into(),
        kind: ActorKind::Agent,
    }
}

pub(super) fn parse_color(value: &str) -> Result<[u8; 4]> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 && value.len() != 8 {
        bail!("colors use RRGGBB or RRGGBBAA hex");
    }
    let channel = |offset| u8::from_str_radix(&value[offset..offset + 2], 16);
    Ok([
        channel(0)?,
        channel(2)?,
        channel(4)?,
        if value.len() == 8 { channel(6)? } else { 255 },
    ])
}

#[cfg(test)]
#[path = "canvas_commands/test_modules.rs"]
mod test_modules;
