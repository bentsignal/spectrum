use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use lumen_core::{
    DurableCatalog as LumenDurableCatalog, Project as LumenProject, engine::render_photo,
};
use prism_core::{
    BlendMode, Command, Document, LayerMask, ShapeStroke, Transform, Workspace, export_document,
};
use serde_json::{Value, json};
use spectrum_imaging::{AdjustmentPatch, RenderOptions};
use spectrum_revisions::{Actor, ActorKind, CollaborationMode, SessionId};

#[path = "prism_cli/benchmark.rs"]
mod benchmark;
use benchmark::benchmark;

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
    /// Add editable text using Prism's bundled portable font.
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
    },
}

#[derive(Subcommand)]
enum AgentCommand {
    /// Start from the current human position and return a persistent agent session.
    Start {
        #[arg(long, value_enum)]
        mode: CliAgentMode,
        #[arg(long, default_value = "Agent")]
        name: String,
        /// Choose a specific human session instead of the most recently active one.
        #[arg(long)]
        from_session: Option<SessionId>,
    },
    /// Inspect this agent session's mode, cursor, and follow status.
    Status,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliAgentMode {
    Together,
    Separate,
}

impl From<CliAgentMode> for CollaborationMode {
    fn from(value: CliAgentMode) -> Self {
        match value {
            CliAgentMode::Together => Self::Together,
            CliAgentMode::Separate => Self::Separate,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum CliBlend {
    Normal,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    DarkerColor,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl From<CliBlend> for BlendMode {
    fn from(value: CliBlend) -> Self {
        match value {
            CliBlend::Normal => Self::Normal,
            CliBlend::Darken => Self::Darken,
            CliBlend::Multiply => Self::Multiply,
            CliBlend::ColorBurn => Self::ColorBurn,
            CliBlend::LinearBurn => Self::LinearBurn,
            CliBlend::DarkerColor => Self::DarkerColor,
            CliBlend::Lighten => Self::Lighten,
            CliBlend::Screen => Self::Screen,
            CliBlend::ColorDodge => Self::ColorDodge,
            CliBlend::LinearDodge => Self::LinearDodge,
            CliBlend::LighterColor => Self::LighterColor,
            CliBlend::Overlay => Self::Overlay,
            CliBlend::SoftLight => Self::SoftLight,
            CliBlend::HardLight => Self::HardLight,
            CliBlend::VividLight => Self::VividLight,
            CliBlend::LinearLight => Self::LinearLight,
            CliBlend::PinLight => Self::PinLight,
            CliBlend::HardMix => Self::HardMix,
            CliBlend::Difference => Self::Difference,
            CliBlend::Exclusion => Self::Exclusion,
            CliBlend::Subtract => Self::Subtract,
            CliBlend::Divide => Self::Divide,
            CliBlend::Hue => Self::Hue,
            CliBlend::Saturation => Self::Saturation,
            CliBlend::Color => Self::Color,
            CliBlend::Luminosity => Self::Luminosity,
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
        CliCommand::Benchmark { strict } => benchmark(strict),
        command => {
            let mut workspace = match cli.session {
                Some(session) => Workspace::open_session(&cli.project, session)?,
                None => Workspace::open_as(&cli.project, cli_actor(), SessionId::new())?,
            };
            let outputs = match command {
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
                CliCommand::Blend { id, mode } => {
                    vec![workspace.execute(Command::SetBlendMode {
                        id,
                        blend_mode: mode.into(),
                    })?]
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

fn agent_command(path: &Path, session: Option<SessionId>, command: AgentCommand) -> Result<Value> {
    match command {
        AgentCommand::Start {
            mode,
            name,
            from_session,
        } => {
            let name = name.trim();
            if name.is_empty() {
                bail!("agent name cannot be empty");
            }
            let actor = Actor {
                id: format!("external-agent:{}", SessionId::new()),
                display_name: name.into(),
                kind: ActorKind::Agent,
            };
            let mode = CollaborationMode::from(mode);
            let collaboration = match from_session {
                Some(source) => Workspace::start_collaboration(path, Some(source), actor, mode)?,
                None => match local_gui_session_id() {
                    Some(source) => {
                        Workspace::start_collaboration(path, Some(source), actor.clone(), mode)
                            .or_else(|_| Workspace::start_collaboration(path, None, actor, mode))?
                    }
                    None => Workspace::start_collaboration(path, None, actor, mode)?,
                },
            };
            Ok(json!({
                "ok": true,
                "action": "agent_start",
                "project": path,
                "mode": collaboration.mode,
                "session": collaboration.agent_session,
                "source_session": collaboration.source_session,
                "base_revision": collaboration.base_revision,
                "status": collaboration.status,
                "use_for_every_command": [
                    "--project", path,
                    "--session", collaboration.agent_session.to_string()
                ],
                "behavior": match collaboration.mode {
                    CollaborationMode::Together => "Prism follows agent revisions until the human makes a competing edit",
                    CollaborationMode::Separate => "the human canvas stays on its own session while the agent explores",
                }
            }))
        }
        AgentCommand::Status => {
            let session = session.context("agent status requires --session <SESSION_ID>")?;
            let collaboration = Workspace::collaboration(path, session)?;
            let workspace = Workspace::open_session(path, session)?;
            let cursor = workspace
                .history()?
                .context("agent session does not have durable history")?
                .current;
            Ok(json!({
                "ok": true,
                "action": "agent_status",
                "project": path,
                "session": session,
                "cursor": cursor,
                "collaboration": collaboration,
            }))
        }
    }
}

fn local_gui_session_id() -> Option<SessionId> {
    let directory = eframe::storage_dir("Spectrum")?;
    spectrum_revisions::local_session_id(&directory).ok()
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

fn from_lumen(catalog: &Path, photo_id: u64, output: &Path) -> Result<Value> {
    let project = if LumenDurableCatalog::looks_durable(catalog)? {
        LumenDurableCatalog::load_current(catalog)?
    } else {
        LumenProject::load(catalog)?
    };
    let photo = project.photo(photo_id)?;
    let rendered = render_photo(photo, RenderOptions::default())?;
    let session = SessionId::new();
    let import_directory = std::env::temp_dir().join("spectrum-prism-imports");
    std::fs::create_dir_all(&import_directory)?;
    let asset = import_directory.join(format!("{session}.png"));
    rendered.save(&asset)?;

    let mut workspace = Workspace::new(
        Document::new(
            format!("{} — {}", project.name, photo.name),
            rendered.width(),
            rendered.height(),
        ),
        None,
    );
    workspace.document.background = [0, 0, 0, 0];
    workspace.execute(Command::AddRaster {
        path: asset.clone(),
        name: Some(photo.name.clone()),
        x: 0.0,
        y: 0.0,
    })?;
    let durable = Workspace::create_durable(workspace.document, output, cli_actor(), session);
    let _ = std::fs::remove_file(asset);
    let mut workspace = durable?;
    workspace.save(None)?;
    Ok(json!({
        "ok": true,
        "action": "from_lumen",
        "catalog": catalog,
        "photo_id": photo_id,
        "project": output
    }))
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

fn schema() -> Value {
    json!({
        "ok": true,
        "application": "Prism",
        "project_extension": ".prism",
        "legacy_project_extensions": [".mica"],
        "project_storage": {
            "container": "single transactional SQLite .prism file",
            "persistence": "each completed semantic action is an attributed durable revision",
            "batching": "run arrays commit atomically as one revision",
            "history": "immutable revision tree with session-specific cursors",
            "assets": "embedded and content-addressed"
        },
        "agent_collaboration": {
            "transport": "CLI JSON; no vendor-specific integration required",
            "start": "prism --project <path> agent start --mode <together|separate> --name <agent>",
            "continue": "pass the returned --session value to every list, edit, run, and export command",
            "status": "prism --project <path> --session <id> agent status",
            "together": "starts at the current human cursor; Prism follows until the human makes a competing edit, then both sessions continue separately",
            "separate": "starts at the current human cursor and never moves the human session",
            "agent_prompt": "before starting, ask whether the user wants to continue together or explore separately"
        },
        "command_protocol": {
            "encoding": "serde tagged JSON",
            "tag": "command",
            "examples": [
                {"command": "add_text", "text": "Hello", "name": null, "font_size": 72.0, "color": [255,255,255,255], "x": 100.0, "y": 120.0},
                {"command": "add_ellipse", "name": "Badge", "width": 320, "height": 320, "color": [247,178,102,255], "x": 100.0, "y": 120.0},
                {"command": "set_shape_stroke", "id": 1, "stroke": {"enabled": true, "width": 6.0, "color": [255,255,255,255]}},
                {"command": "rasterize_shape", "id": 1, "path": "/generated/shape.png", "scale": 2.0},
                {"command": "set_transform", "id": 1, "transform": {"x": 220.0, "y": 160.0, "scale_x": 1.2, "scale_y": 1.2, "rotation": 8.0}},
                {"command": "set_rotation", "id": 1, "degrees": 15.0},
                {"command": "set_mask", "id": 1, "mask": {"enabled": true, "x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8, "invert": false}},
                {"command": "adjust_layer", "id": 1, "patch": {"exposure": 0.5, "contrast": 12.0}}
            ]
        },
        "gui_interactions": {
            "rotate_focused_object": "Option-R on macOS or Alt-R on Windows/Linux arms the next canvas drag; Shift snaps the absolute angle to 15-degree increments; Escape cancels"
        },
        "blend_modes": [
            "normal", "darken", "multiply", "color_burn", "linear_burn", "darker_color",
            "lighten", "screen", "color_dodge", "linear_dodge", "lighter_color", "overlay",
            "soft_light", "hard_light", "vivid_light", "linear_light", "pin_light", "hard_mix",
            "difference", "exclusion", "subtract", "divide", "hue", "saturation", "color",
            "luminosity"
        ],
        "layer_types": ["raster", "text", "rectangle", "ellipse"],
        "color": "RRGGBB or RRGGBBAA",
        "coordinates": "canvas pixels; layer masks are normalized 0..1"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn colors_accept_rgb_and_rgba() {
        assert_eq!(parse_color("ae7bff").unwrap(), [174, 123, 255, 255]);
        assert_eq!(parse_color("#01020304").unwrap(), [1, 2, 3, 4]);
    }

    #[test]
    fn rotate_cli_persists_the_normalized_angle() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let project = std::env::temp_dir().join(format!("prism-rotate-cli-{stamp}.prism"));
        run(Cli {
            project: project.clone(),
            session: None,
            command: CliCommand::Init {
                name: "Rotate CLI".into(),
                width: 400,
                height: 300,
                background: "18191dff".into(),
            },
        })
        .unwrap();
        run(Cli {
            project: project.clone(),
            session: None,
            command: CliCommand::AddRectangle {
                name: None,
                width: 100,
                height: 80,
                color: "ffffffff".into(),
                radius: 0.0,
                x: 10.0,
                y: 20.0,
            },
        })
        .unwrap();
        let rotate = Cli::try_parse_from([
            "prism",
            "--project",
            project.to_str().unwrap(),
            "rotate",
            "1",
            "-15",
        ])
        .unwrap();
        run(rotate).unwrap();
        let document = Workspace::load_read_only(&project).unwrap();
        assert_eq!(document.layer(1).unwrap().transform.rotation, 345.0);
        std::fs::remove_file(project).unwrap();
    }
}
