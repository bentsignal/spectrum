use std::{
    fs,
    path::PathBuf,
    process::ExitCode,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use image::{DynamicImage, Rgb, RgbImage};
use lumen_core::{
    AdjustmentPatch, Adjustments, ColorGrade, Command, CropRect, CurvePoint, ExportFormat, Photo,
    PickState, Project, SpotRemoval, ToneCurve, ToneCurves, Workspace,
    engine::{RenderOptions, render_image},
    preview::{
        PreparedPreview, PreviewCompletionDisposition, PreviewPipeline, PreviewRequestDecision,
        PreviewWorker,
    },
};
use serde::Serialize;
use serde_json::json;
use spectrum_revisions::{CollaborationMode, SessionId};

#[path = "lumen_cli/benchmark.rs"]
mod benchmark;
#[path = "lumen_cli/collaboration.rs"]
mod collaboration;
#[path = "lumen_cli/schema.rs"]
mod schema;
use benchmark::benchmark;
use collaboration::*;
use schema::schema;

#[derive(Parser)]
#[command(
    name = "lumen",
    version,
    about = "CLI-first nondestructive photo editor",
    long_about = "Lumen's CLI exposes the same command engine as its native GUI. All successful output is JSON."
)]
struct Cli {
    /// Portable Lumen project used by this command.
    #[arg(short, long, global = true, default_value = "untitled.lumen")]
    catalog: PathBuf,

    /// Continue commands in an existing collaboration session.
    #[arg(long, global = true)]
    session: Option<SessionId>,

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
    /// Grade shadows, midtones, or highlights with hue, saturation, and luminance.
    Grade {
        id: u64,
        range: GradeRange,
        #[arg(long)]
        hue: Option<f32>,
        #[arg(long)]
        saturation: Option<f32>,
        #[arg(long, allow_hyphen_values = true)]
        luminance: Option<f32>,
        #[arg(long, allow_hyphen_values = true)]
        balance: Option<f32>,
        #[arg(long)]
        reset: bool,
    },
    /// Add or clear nondestructive dust/smudge repair dabs.
    Spot {
        id: u64,
        #[arg(long)]
        x: Option<f32>,
        #[arg(long)]
        y: Option<f32>,
        #[arg(long, default_value_t = 0.025)]
        radius: f32,
        #[arg(long, default_value_t = 1.0)]
        opacity: f32,
        #[arg(long)]
        clear: bool,
    },
    /// Mark photos as keeps, rejects, or unmarked for fast culling.
    Pick {
        #[arg(num_args = 1..)]
        ids: Vec<u64>,
        #[arg(long, value_enum)]
        state: CliPickState,
    },
    /// Rename a chronological shoot batch in the catalog library.
    BatchRename { id: u64, name: String },
    /// Show one photo's immutable revision tree and session cursors.
    History { id: u64 },
    /// Move this session one photo edit backward.
    HistoryBack { id: u64 },
    /// Move this session one photo edit forward.
    HistoryForward { id: u64 },
    /// Jump this session to a specific revision of one photo.
    HistoryJump {
        id: u64,
        revision: spectrum_revisions::RevisionId,
    },
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
    /// Render multiple photos into a directory with generated filenames.
    ExportBatch {
        #[arg(num_args = 1..)]
        ids: Vec<u64>,
        #[arg(long)]
        directory: PathBuf,
        #[arg(long, value_enum, default_value_t = CliExportFormat::Jpeg)]
        format: CliExportFormat,
        #[arg(long)]
        max_size: Option<u32>,
        #[arg(long, default_value_t = 92, value_parser = clap::value_parser!(u8).range(1..=100))]
        quality: u8,
    },
    /// List saved development presets.
    PresetList,
    /// Save a photo's reusable development settings as a preset.
    PresetSave {
        name: String,
        #[arg(long)]
        from: u64,
    },
    /// Apply a saved preset to one or more photos.
    PresetApply {
        preset_id: u64,
        #[arg(num_args = 1..)]
        ids: Vec<u64>,
    },
    /// Delete a saved preset.
    PresetDelete { preset_id: u64 },
    /// Execute a serialized core command. Useful for agents and integrations.
    Run {
        /// JSON object or array matching the tagged core Command enum.
        json: String,
    },
    /// Start or inspect a CLI-first agent collaboration.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Run deterministic tone-curve and 24 MP export performance workloads.
    Benchmark {
        /// Return a failure when any user-experience performance budget is missed.
        #[arg(long)]
        strict: bool,
        /// Budget calibration: workstation feel or GitHub's shared Linux runner.
        #[arg(long, value_enum, default_value_t = BenchmarkProfile::Interactive)]
        profile: BenchmarkProfile,
        /// Optional real RAW file used for an import-only metadata benchmark.
        #[arg(long)]
        raw_import: Option<PathBuf>,
    },
    /// Print the JSON command protocol and adjustment ranges.
    Schema,
}

#[derive(Clone, Subcommand)]
enum AgentCommand {
    /// Start from the current human position and return a persistent agent session.
    Start {
        /// Photo whose history this agent may extend.
        photo_id: u64,
        #[arg(long, value_enum)]
        mode: CliAgentMode,
        #[arg(long, default_value = "Agent")]
        name: String,
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

#[derive(Clone, Copy, ValueEnum)]
enum GradeRange {
    Shadows,
    Midtones,
    Highlights,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliPickState {
    Unmarked,
    Keep,
    Reject,
}

impl From<CliPickState> for PickState {
    fn from(value: CliPickState) -> Self {
        match value {
            CliPickState::Unmarked => Self::Unmarked,
            CliPickState::Keep => Self::Keep,
            CliPickState::Reject => Self::Reject,
        }
    }
}

#[derive(Clone, Copy, Default, ValueEnum)]
enum CliExportFormat {
    #[default]
    Jpeg,
    Png,
    Tiff,
    Webp,
}

#[derive(Clone, Copy, Default, ValueEnum)]
enum BenchmarkProfile {
    #[default]
    Interactive,
    HostedCi,
}

impl BenchmarkProfile {
    fn name(self) -> &'static str {
        match self {
            Self::Interactive => "interactive-workstation",
            Self::HostedCi => "github-hosted-linux",
        }
    }

    fn preview_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 50.0,
            // The shared two-core Linux runner measured 97 ms p95 at the
            // workstation-passing baseline. Keep 29% headroom for host jitter.
            Self::HostedCi => 125.0,
        }
    }

    fn command_budget_ms(self) -> f64 {
        match self {
            // A durable command atomically republishes a portable project that
            // contains a 24 MP source asset. It runs after interaction preview,
            // so this gate protects completion latency rather than frame time.
            Self::Interactive => 100.0,
            Self::HostedCi => 175.0,
        }
    }

    fn switch_dispatch_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 4.0,
            Self::HostedCi => 12.0,
        }
    }

    fn prefetched_switch_ready_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 35.0,
            Self::HostedCi => 75.0,
        }
    }

    fn cold_switch_ready_budget_ms(self) -> f64 {
        match self {
            // The deterministic 2400x1600 JPEG path measured 120 ms p95
            // locally. Keep 46% workstation and 150% hosted-runner headroom
            // without allowing a regression into visibly sluggish switching.
            Self::Interactive => 175.0,
            Self::HostedCi => 300.0,
        }
    }
}

impl From<CliExportFormat> for ExportFormat {
    fn from(value: CliExportFormat) -> Self {
        match value {
            CliExportFormat::Jpeg => Self::Jpeg,
            CliExportFormat::Png => Self::Png,
            CliExportFormat::Tiff => Self::Tiff,
            CliExportFormat::Webp => Self::Webp,
        }
    }
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
    if let CliCommand::Benchmark {
        strict,
        profile,
        raw_import,
    } = &cli.command
    {
        return benchmark(*strict, *profile, raw_import.as_deref());
    }

    if let CliCommand::Agent { command } = &cli.command {
        return agent_command(&cli.catalog, cli.session, command.clone());
    }

    if let CliCommand::Init { name, force } = &cli.command {
        if cli.catalog.exists() && !force {
            anyhow::bail!(
                "catalog {} already exists; pass --force to replace it",
                cli.catalog.display()
            );
        }
        if cli.catalog.exists() {
            fs::remove_file(&cli.catalog)
                .with_context(|| format!("could not replace {}", cli.catalog.display()))?;
        }
        let project = Project::new(name.clone());
        let workspace =
            Workspace::create_durable(project, &cli.catalog, cli_actor(), SessionId::new())?;
        return Ok(json!({
            "ok": true,
            "action": "init",
            "catalog": cli.catalog,
            "project": workspace.project,
        }));
    }

    let mut workspace = match cli.session {
        Some(session) => Workspace::open_session(&cli.catalog, session),
        None => Workspace::open_as(&cli.catalog, cli_actor(), SessionId::new()),
    }
    .with_context(|| {
        format!(
            "open {} or create it first with `lumen init`",
            cli.catalog.display()
        )
    })?;
    let active_catalog = workspace
        .catalog_path
        .clone()
        .unwrap_or_else(|| cli.catalog.clone());

    let (result, _should_save) = match cli.command {
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
                "catalog": active_catalog,
                "project": workspace.project,
            }));
        }
        CliCommand::Get { id } => {
            return Ok(json!({
                "ok": true,
                "catalog": active_catalog,
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
        CliCommand::Grade {
            id,
            range,
            hue,
            saturation,
            luminance,
            balance,
            reset,
        } => {
            let mut adjustments = workspace.project.photo(id)?.adjustments.clone();
            let grade = match range {
                GradeRange::Shadows => &mut adjustments.color_grading.shadows,
                GradeRange::Midtones => &mut adjustments.color_grading.midtones,
                GradeRange::Highlights => &mut adjustments.color_grading.highlights,
            };
            if reset {
                *grade = ColorGrade::default();
            }
            if let Some(value) = hue {
                grade.hue = value;
            }
            if let Some(value) = saturation {
                grade.saturation = value;
            }
            if let Some(value) = luminance {
                grade.luminance = value;
            }
            if let Some(value) = balance {
                adjustments.color_grading.balance = value;
            }
            (
                output(&workspace_command(
                    &mut workspace,
                    Command::SetAdjustments { id, adjustments },
                )?),
                true,
            )
        }
        CliCommand::Spot {
            id,
            x,
            y,
            radius,
            opacity,
            clear,
        } => {
            let mut adjustments = workspace.project.photo(id)?.adjustments.clone();
            if clear {
                adjustments.spots.clear();
            } else {
                adjustments.spots.push(SpotRemoval {
                    x: x.context("--x is required unless --clear is used")?,
                    y: y.context("--y is required unless --clear is used")?,
                    radius,
                    opacity,
                });
            }
            (
                output(&workspace_command(
                    &mut workspace,
                    Command::SetAdjustments { id, adjustments },
                )?),
                true,
            )
        }
        CliCommand::Pick { ids, state } => (
            output(&workspace_command(
                &mut workspace,
                Command::SetPick {
                    ids,
                    state: state.into(),
                },
            )?),
            true,
        ),
        CliCommand::BatchRename { id, name } => (
            output(&workspace_command(
                &mut workspace,
                Command::RenameBatch { id, name },
            )?),
            true,
        ),
        CliCommand::History { id } => {
            return Ok(json!({
                "ok": true,
                "project": active_catalog,
                "history": workspace.history_for(id)?.context("photo history is unavailable")?,
            }));
        }
        CliCommand::HistoryBack { id } => {
            workspace.execute(Command::Select { id })?;
            (
                output(&workspace_command(&mut workspace, Command::Undo)?),
                true,
            )
        }
        CliCommand::HistoryForward { id } => {
            workspace.execute(Command::Select { id })?;
            (
                output(&workspace_command(&mut workspace, Command::Redo)?),
                true,
            )
        }
        CliCommand::HistoryJump { id, revision } => {
            workspace.move_photo_to_revision(id, revision)?;
            (
                json!({"ok": true, "action": "history_jump", "photo_id": id, "revision": revision}),
                true,
            )
        }
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
        CliCommand::ExportBatch {
            ids,
            directory,
            format,
            max_size,
            quality,
        } => (
            output(&workspace_command(
                &mut workspace,
                Command::ExportBatch {
                    ids,
                    directory,
                    format: format.into(),
                    max_size,
                    quality,
                },
            )?),
            false,
        ),
        CliCommand::PresetList => {
            return Ok(json!({
                "ok": true,
                "catalog": active_catalog,
                "presets": workspace.project.presets,
            }));
        }
        CliCommand::PresetSave { name, from } => (
            output(&workspace_command(
                &mut workspace,
                Command::SavePreset {
                    name,
                    from_id: from,
                },
            )?),
            true,
        ),
        CliCommand::PresetApply { preset_id, ids } => (
            output(&workspace_command(
                &mut workspace,
                Command::ApplyPreset { preset_id, ids },
            )?),
            true,
        ),
        CliCommand::PresetDelete { preset_id } => (
            output(&workspace_command(
                &mut workspace,
                Command::DeletePreset { id: preset_id },
            )?),
            true,
        ),
        CliCommand::Run { json } => {
            let outputs = run_commands(&mut workspace, &json)?;
            (serde_json::to_value(outputs)?, true)
        }
        CliCommand::Init { .. }
        | CliCommand::Agent { .. }
        | CliCommand::Benchmark { .. }
        | CliCommand::Schema => {
            unreachable!()
        }
    };

    workspace.checkpoint()?;
    Ok(json!({
        "result": result,
        "catalog": active_catalog,
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
