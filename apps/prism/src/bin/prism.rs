use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use lumen_core::{
    DurableCatalog as LumenDurableCatalog, Project as LumenProject, engine::render_photo,
};
use prism_core::{Command, Document, Workspace, export_document};
use serde_json::{Value, json};
use spectrum_imaging::RenderOptions;
use spectrum_revisions::{Actor, ActorKind, SessionId};

#[path = "prism_cli/agent.rs"]
mod agent;
use agent::{AgentCommand, agent_command};
#[path = "prism_cli/live_bridge.rs"]
mod live_bridge;
use live_bridge::{
    CliLiveMode, LiveCommand, live_command, live_execute_prepared, prepare_live_semantic,
    resolved_live_mode,
};
#[path = "prism_cli/alignment.rs"]
mod alignment;
use alignment::{CliAlignment, GuideCommand};
#[path = "prism_cli/benchmark.rs"]
mod benchmark;
use benchmark::{BenchmarkProfile, benchmark};
#[path = "prism_cli/blend.rs"]
mod blend;
use blend::CliBlend;
#[path = "prism_cli/dispatch.rs"]
mod dispatch;
#[path = "prism_cli/effects.rs"]
mod effects;
use effects::{GradientArgs, ShadowArgs};
#[path = "prism_cli/from_lumen.rs"]
mod from_lumen;
use from_lumen::from_lumen;
#[path = "prism_cli/paths.rs"]
mod paths;
use paths::{PathArgs, PathCommand, VectorMaskArgs};
#[path = "prism_cli/paint.rs"]
mod paint;
use paint::PaintArgs;
#[path = "prism_cli/schema.rs"]
mod schema;
use schema::schema;
#[path = "prism_cli/selection.rs"]
mod selection;
use selection::SelectionArgs;
#[path = "prism_cli/typography.rs"]
mod typography;
use typography::{CliTextLayout, TypographyArgs, text_shaping, updated_typography};
#[path = "prism_cli/transfer.rs"]
mod transfer;
use transfer::{LayerCopyArgs, LayerPasteArgs};

#[derive(Parser)]
#[command(name = "prism", version, about = "Agent-first layered image editor")]
struct Cli {
    #[arg(
        short,
        long,
        global = true,
        env = "PRISM_PROJECT",
        default_value = "untitled.prism"
    )]
    project: PathBuf,
    /// Continue commands in an existing collaboration session.
    #[arg(long, global = true, env = "PRISM_SESSION")]
    session: Option<SessionId>,
    /// Choose direct project access or require the authenticated running GUI.
    #[arg(long, global = true, value_enum)]
    live: Option<CliLiveMode>,
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Subcommand)]
enum CliCommand {
    /// Create a new editable canvas.
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
    /// Rename document metadata without changing the .prism file path.
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
    /// Add editable text using Prism's bundled Ubuntu Light font.
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
    /// Embed an OpenType font in this portable Prism project.
    FontImport {
        path: PathBuf,
    },
    /// Search bundled and embedded font faces.
    FontList {
        #[arg(long)]
        query: Option<String>,
    },
    /// Analyze current embedded-font character usage and cmap coverage without modifying bytes.
    FontUsage {
        /// Limit analysis to one embedded font asset.
        #[arg(long)]
        font_id: Option<u64>,
    },
    /// Verify and inspect one immutable embedded source-font snapshot.
    FontSource {
        font_id: u64,
    },
    /// Prove an in-memory subset candidate and report why physical replacement is not yet safe.
    FontSubsetPlan {
        font_id: u64,
    },
    /// Create a smaller project by safely rewriting linear history with font subsets.
    OptimizedCopy {
        #[arg(long)]
        output: PathBuf,
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
    /// Flatten the current document into PNG or JPEG.
    Export {
        path: PathBuf,
        #[arg(long, default_value_t = 92)]
        quality: u8,
    },
    /// Create a Prism project from a developed Lumen catalog photo.
    FromLumen {
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        photo: u64,
        #[arg(long)]
        output: PathBuf,
    },
    /// Execute one Command JSON object or an array of commands.
    Run {
        json: String,
    },
    /// Start or inspect a CLI-first agent collaboration.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Discover, inspect, mutate, and subscribe through the running Prism GUI.
    Live {
        #[command(subcommand)]
        command: LiveCommand,
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

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(value) => {
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!(
                "{}",
                serde_json::to_string_pretty(&json!({"ok": false, "error": format!("{error:#}")}))
                    .unwrap()
            );
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<Value> {
    let live_mode = resolved_live_mode(cli.live)?;
    match cli.command {
        CliCommand::Init {
            name,
            width,
            height,
            background,
        } => {
            require_direct_mode(live_mode, "init")?;
            let mut document = Document::new(name, width, height);
            document.background = parse_color(&background)?;
            let mut workspace =
                Workspace::create_durable(document, &cli.project, cli_actor(), SessionId::new())?;
            workspace.save(None)?;
            Ok(
                json!({"ok": true, "action": "init", "project": cli.project, "document": workspace.document}),
            )
        }
        CliCommand::List => {
            let document = session_document(&cli.project, cli.session)?;
            Ok(json!({"ok": true, "project": cli.project, "document": document}))
        }
        CliCommand::FontList { query } => Ok(typography::font_list(
            &session_document(&cli.project, cli.session)?,
            query,
        )),
        CliCommand::FontUsage { font_id } => {
            typography::font_usage(&session_document(&cli.project, cli.session)?, font_id)
        }
        CliCommand::FontSource { font_id } => {
            typography::font_source_command(&cli.project, cli.session, font_id)
        }
        CliCommand::FontSubsetPlan { font_id } => {
            typography::font_subset_plan_command(&cli.project, cli.session, font_id)
        }
        CliCommand::OptimizedCopy { output } => {
            require_direct_mode(live_mode, "optimized-copy")?;
            if cli.session.is_some() {
                bail!("optimized-copy does not accept --session");
            }
            let report = prism_core::create_optimized_font_copy(&cli.project, &output)?;
            Ok(json!({"ok": true, "action": "optimized_copy", "report": report}))
        }
        CliCommand::LayerCopy(arguments) => {
            transfer::copy_layer(&session_document(&cli.project, cli.session)?, arguments)
        }
        CliCommand::Export { path, quality } => {
            let document = session_document(&cli.project, cli.session)?;
            export_document(&document, &path, quality)?;
            Ok(json!({"ok": true, "action": "export", "path": path}))
        }
        CliCommand::FromLumen {
            catalog,
            photo,
            output,
        } => {
            require_direct_mode(live_mode, "from-lumen")?;
            from_lumen(&catalog, photo, &output)
        }
        CliCommand::Agent { command } => agent_command(&cli.project, cli.session, command),
        CliCommand::Live { command } => live_command(&cli.project, cli.session, command),
        CliCommand::Schema => Ok(schema()),
        CliCommand::Benchmark { strict, profile } => benchmark(strict, profile),
        command => {
            enum Target {
                Direct(Box<Workspace>),
                Live(Box<live_bridge::PreparedLiveSemantic>),
            }
            let target = match live_mode {
                CliLiveMode::Off => Target::Direct(Box::new(match cli.session {
                    Some(session) => Workspace::open_session(&cli.project, session)?,
                    None => Workspace::open_as(&cli.project, cli_actor(), SessionId::new())?,
                })),
                CliLiveMode::Required => {
                    Target::Live(Box::new(prepare_live_semantic(&cli.project, cli.session)?))
                }
            };
            let document = match &target {
                Target::Direct(workspace) => &workspace.document,
                Target::Live(prepared) => &prepared.document,
            };
            let plan = dispatch::semantic_commands(command, document)?;
            match target {
                Target::Direct(mut workspace) => {
                    let outputs = if plan.atomic_batch || plan.commands.len() != 1 {
                        workspace.execute_batch(plan.commands)?
                    } else {
                        vec![
                            workspace.execute(
                                plan.commands
                                    .into_iter()
                                    .next()
                                    .expect("single semantic command"),
                            )?,
                        ]
                    };
                    workspace.save(None)?;
                    Ok(json!({"ok": true, "project": cli.project, "results": outputs}))
                }
                Target::Live(prepared) => live_execute_prepared(*prepared, plan.commands),
            }
        }
    }
}

fn session_document(path: &Path, session: Option<SessionId>) -> Result<Document> {
    match session {
        Some(session) => Ok(Workspace::open_session(path, session)?.document),
        None => Workspace::load_read_only(path),
    }
}

fn require_direct_mode(mode: CliLiveMode, command: &str) -> Result<()> {
    if mode == CliLiveMode::Required {
        bail!(
            "{command} creates a standalone artifact and is unavailable when live mode is required"
        );
    }
    Ok(())
}

fn cli_actor() -> Actor {
    Actor {
        id: "local:prism-cli".into(),
        display_name: "Prism CLI".into(),
        kind: ActorKind::Agent,
    }
}

fn parse_color(value: &str) -> Result<[u8; 4]> {
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
#[path = "prism_cli/test_modules.rs"]
mod test_modules;
