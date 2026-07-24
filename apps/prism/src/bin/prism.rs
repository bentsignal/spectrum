use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use lumen_core::{
    DurableCatalog as LumenDurableCatalog, Project as LumenProject, engine::render_photo,
};
use prism_core::{
    AlignmentReference, BlendMode, Command, Document, LayerMask, ShapeStroke, Transform, Workspace,
    export_document,
};
use serde_json::{Value, json};
use spectrum_imaging::{AdjustmentPatch, RenderOptions};
use spectrum_revisions::{Actor, ActorKind, SessionId};

#[path = "prism_cli/agent.rs"]
mod agent;
use agent::{AgentCommand, agent_command};
#[path = "prism_cli/alignment.rs"]
mod alignment;
use alignment::{CliAlignment, GuideCommand};
#[path = "prism_cli/benchmark.rs"]
mod benchmark;
use benchmark::{BenchmarkProfile, benchmark};
#[path = "prism_cli/blend.rs"]
mod blend;
use blend::CliBlend;
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
use typography::{TypographyArgs, updated_typography};
#[path = "prism_cli/transfer.rs"]
mod transfer;
use transfer::{LayerCopyArgs, LayerPasteArgs};

#[derive(Parser)]
#[command(name = "prism", version, about = "Agent-first layered image editor")]
struct Cli {
    #[arg(short, long, global = true, default_value = "untitled.prism")]
    project: PathBuf,
    /// Continue commands in an existing collaboration session.
    #[arg(long, global = true)]
    session: Option<SessionId>,
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
    /// Add, update, or clear a two-stop linear shape gradient.
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
        visible: bool,
    },
    Lock {
        id: u64,
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
    match cli.command {
        CliCommand::Init {
            name,
            width,
            height,
            background,
        } => {
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
        } => from_lumen(&catalog, photo, &output),
        CliCommand::Agent { command } => agent_command(&cli.project, cli.session, command),
        CliCommand::Schema => Ok(schema()),
        CliCommand::Benchmark { strict, profile } => benchmark(strict, profile),
        command => {
            let mut workspace = match cli.session {
                Some(session) => Workspace::open_session(&cli.project, session)?,
                None => Workspace::open_as(&cli.project, cli_actor(), SessionId::new())?,
            };
            let outputs = match command {
                CliCommand::RenameDocument { name } => {
                    vec![workspace.execute(Command::RenameDocument { name })?]
                }
                CliCommand::FontImport { path } => {
                    vec![workspace.execute(Command::ImportFont {
                        path,
                        source_name: None,
                    })?]
                }
                CliCommand::Typography(arguments) => {
                    let typography = updated_typography(&workspace.document, &arguments)?;
                    vec![workspace.execute(Command::SetTextTypography {
                        id: arguments.id,
                        typography,
                    })?]
                }
                CliCommand::LayerPaste(arguments) => {
                    vec![workspace.execute(transfer::paste_command(arguments)?)?]
                }
                CliCommand::AddImage { path, name, x, y } => {
                    vec![workspace.execute(Command::AddRaster { path, name, x, y })?]
                }
                CliCommand::AddText {
                    text,
                    name,
                    size,
                    color,
                    x,
                    y,
                } => vec![workspace.execute(Command::AddText {
                    text,
                    name,
                    font_size: size,
                    color: parse_color(&color)?,
                    x,
                    y,
                })?],
                CliCommand::AddRectangle {
                    name,
                    width,
                    height,
                    color,
                    radius,
                    x,
                    y,
                } => vec![workspace.execute(Command::AddRectangle {
                    name,
                    width,
                    height,
                    color: parse_color(&color)?,
                    corner_radius: radius,
                    x,
                    y,
                })?],
                CliCommand::AddEllipse {
                    name,
                    width,
                    height,
                    color,
                    x,
                    y,
                } => vec![workspace.execute(Command::AddEllipse {
                    name,
                    width,
                    height,
                    color: parse_color(&color)?,
                    x,
                    y,
                })?],
                CliCommand::Path(arguments) => {
                    vec![workspace.execute(match arguments.command {
                        PathCommand::Add {
                            geometry,
                            name,
                            color,
                            x,
                            y,
                        } => Command::AddPath {
                            name,
                            geometry: paths::read_geometry(geometry)?,
                            color: parse_color(&color)?,
                            x,
                            y,
                        },
                        PathCommand::Replace { id, geometry } => {
                            paths::replace_command(id, geometry)?
                        }
                    })?]
                }
                CliCommand::Paint(arguments) => {
                    vec![workspace.execute(paint::paint_command(arguments)?)?]
                }
                CliCommand::VectorMask(arguments) => {
                    vec![workspace.execute(paths::vector_mask_command(arguments)?)?]
                }
                CliCommand::EditText {
                    id,
                    text,
                    size,
                    color,
                } => vec![workspace.execute(Command::UpdateText {
                    id,
                    text,
                    font_size: size,
                    color: parse_color(&color)?,
                })?],
                CliCommand::EditRectangle {
                    id,
                    width,
                    height,
                    color,
                    radius,
                } => vec![workspace.execute(Command::UpdateRectangle {
                    id,
                    width,
                    height,
                    color: parse_color(&color)?,
                    corner_radius: radius,
                })?],
                CliCommand::EditEllipse {
                    id,
                    width,
                    height,
                    color,
                } => vec![workspace.execute(Command::UpdateEllipse {
                    id,
                    width,
                    height,
                    color: parse_color(&color)?,
                })?],
                CliCommand::Stroke {
                    id,
                    enabled,
                    width,
                    color,
                } => vec![workspace.execute(Command::SetShapeStroke {
                    id,
                    stroke: ShapeStroke {
                        enabled,
                        width,
                        color: parse_color(&color)?,
                    },
                })?],
                CliCommand::Shadow(arguments) => {
                    vec![workspace.execute(effects::shadow_command(arguments)?)?]
                }
                CliCommand::Gradient(arguments) => {
                    vec![workspace.execute(effects::gradient_command(arguments)?)?]
                }
                CliCommand::RasterizeShape { id, scale } => {
                    let layer = workspace.document.layer(id)?;
                    let scale = scale
                        .map(Ok)
                        .unwrap_or_else(|| prism_core::recommended_rasterization_scale(layer))?;
                    let asset = prism_core::rasterize_shape_asset(&workspace.document, id, scale)?;
                    vec![workspace.execute(Command::RasterizeShape {
                        id,
                        path: asset.path,
                        scale: asset.scale,
                    })?]
                }
                CliCommand::Rename { id, name } => {
                    vec![workspace.execute(Command::RenameLayer { id, name })?]
                }
                CliCommand::Delete { id } => {
                    vec![workspace.execute(Command::RemoveLayer { id })?]
                }
                CliCommand::Duplicate { id } => {
                    vec![workspace.execute(Command::DuplicateLayer { id })?]
                }
                CliCommand::Select { id } => {
                    vec![workspace.execute(Command::SelectLayer { id })?]
                }
                CliCommand::Selection(arguments) => {
                    vec![workspace.execute(selection::command(arguments)?)?]
                }
                CliCommand::Reorder { id, index } => {
                    vec![workspace.execute(Command::MoveLayer { id, index })?]
                }
                CliCommand::Visibility { id, visible } => {
                    vec![workspace.execute(Command::SetVisibility { id, visible })?]
                }
                CliCommand::Lock { id, locked } => {
                    vec![workspace.execute(Command::SetLocked { id, locked })?]
                }
                CliCommand::Opacity { id, opacity } => {
                    vec![workspace.execute(Command::SetOpacity { id, opacity })?]
                }
                CliCommand::Blend { id, mode, seed } => {
                    let blend_mode = BlendMode::from(mode);
                    if seed.is_some() && blend_mode != BlendMode::Dissolve {
                        bail!("--seed is only valid with the dissolve blend mode");
                    }
                    let mut commands = vec![Command::SetBlendMode { id, blend_mode }];
                    if let Some(seed) = seed {
                        commands.push(Command::SetDissolveSeed { id, seed });
                    }
                    workspace.execute_batch(commands)?
                }
                CliCommand::Transform {
                    id,
                    x,
                    y,
                    scale_x,
                    scale_y,
                    rotation,
                } => vec![workspace.execute(Command::SetTransform {
                    id,
                    transform: Transform {
                        x,
                        y,
                        scale_x,
                        scale_y,
                        rotation,
                    },
                })?],
                CliCommand::Rotate { id, degrees } => {
                    vec![workspace.execute(Command::SetRotation { id, degrees })?]
                }
                CliCommand::Align {
                    id,
                    alignment,
                    to_layer,
                } => vec![workspace.execute(Command::AlignLayer {
                    id,
                    alignment: alignment.into(),
                    reference: to_layer.map_or(AlignmentReference::Canvas, |id| {
                        AlignmentReference::Layer { id }
                    }),
                })?],
                CliCommand::Snapping { enabled } => {
                    vec![workspace.execute(Command::SetSnapping { enabled })?]
                }
                CliCommand::Guide { command } => vec![workspace.execute(match command {
                    GuideCommand::Add {
                        orientation,
                        position,
                    } => Command::AddGuide {
                        orientation: orientation.into(),
                        position,
                    },
                    GuideCommand::Move { id, position } => Command::MoveGuide { id, position },
                    GuideCommand::Remove { id } => Command::RemoveGuide { id },
                })?],
                CliCommand::Adjust {
                    id,
                    exposure,
                    contrast,
                    highlights,
                    shadows,
                    temperature,
                    tint,
                    vibrance,
                    saturation,
                    clarity,
                    dehaze,
                    noise_reduction,
                    sharpening,
                } => vec![workspace.execute(Command::AdjustLayer {
                    id,
                    patch: AdjustmentPatch {
                        exposure,
                        contrast,
                        highlights,
                        shadows,
                        temperature,
                        tint,
                        vibrance,
                        saturation,
                        clarity,
                        dehaze,
                        noise_reduction,
                        sharpening,
                        ..Default::default()
                    },
                })?],
                CliCommand::ResetAdjustments { id } => {
                    vec![workspace.execute(Command::ResetLayerAdjustments { id })?]
                }
                CliCommand::Mask {
                    id,
                    x,
                    y,
                    width,
                    height,
                    invert,
                    clear,
                } => vec![workspace.execute(Command::SetMask {
                    id,
                    mask: LayerMask {
                        enabled: !clear,
                        x,
                        y,
                        width,
                        height,
                        invert,
                    },
                })?],
                CliCommand::Clip { id, enabled } => {
                    vec![workspace.execute(Command::SetClipping { id, enabled })?]
                }
                CliCommand::Canvas {
                    width,
                    height,
                    background,
                } => vec![workspace.execute(Command::SetCanvas {
                    width,
                    height,
                    background: parse_color(&background)?,
                })?],
                CliCommand::Crop {
                    x,
                    y,
                    width,
                    height,
                } => vec![workspace.execute(Command::CropCanvas {
                    x,
                    y,
                    width,
                    height,
                })?],
                CliCommand::Run { json } => run_commands(&mut workspace, &json)?,
                CliCommand::Init { .. }
                | CliCommand::List
                | CliCommand::FontList { .. }
                | CliCommand::FontUsage { .. }
                | CliCommand::FontSource { .. }
                | CliCommand::FontSubsetPlan { .. }
                | CliCommand::OptimizedCopy { .. }
                | CliCommand::LayerCopy(..)
                | CliCommand::Export { .. }
                | CliCommand::FromLumen { .. }
                | CliCommand::Agent { .. }
                | CliCommand::Schema
                | CliCommand::Benchmark { .. } => unreachable!(),
            };
            workspace.save(None)?;
            Ok(json!({"ok": true, "project": cli.project, "results": outputs}))
        }
    }
}

fn session_document(path: &Path, session: Option<SessionId>) -> Result<Document> {
    match session {
        Some(session) => Ok(Workspace::open_session(path, session)?.document),
        None => Workspace::load_read_only(path),
    }
}

fn run_commands(workspace: &mut Workspace, value: &str) -> Result<Vec<prism_core::CommandOutput>> {
    if value.trim_start().starts_with('[') {
        workspace.execute_batch(serde_json::from_str::<Vec<Command>>(value)?)
    } else {
        Ok(vec![workspace.execute(serde_json::from_str(value)?)?])
    }
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
