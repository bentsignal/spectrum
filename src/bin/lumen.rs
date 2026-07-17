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
        /// JSON object matching the tagged core Command enum.
        json: String,
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
            Self::Interactive => 20.0,
            // Catalog persistence is sub-millisecond in the median, but a
            // shared hosted runner can stall one filesystem sample. Keep the
            // workstation gate strict while allowing bounded host jitter.
            Self::HostedCi => 100.0,
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
                "catalog": cli.catalog,
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
            let command: Command = serde_json::from_str(&json).context("invalid command JSON")?;
            let should_save = !matches!(
                command,
                Command::Open { .. } | Command::Export { .. } | Command::ExportBatch { .. }
            );
            (
                output(&workspace_command(&mut workspace, command)?),
                should_save,
            )
        }
        CliCommand::Init { .. } | CliCommand::Benchmark { .. } | CliCommand::Schema => {
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

fn benchmark(
    strict: bool,
    profile: BenchmarkProfile,
    raw_import: Option<&std::path::Path>,
) -> Result<serde_json::Value> {
    const PREVIEW_WIDTH: u32 = 1800;
    const PREVIEW_HEIGHT: u32 = 1200;
    const EXPORT_WIDTH: u32 = 6000;
    const EXPORT_HEIGHT: u32 = 4000;
    let directory = std::env::temp_dir().join(format!("lumen-benchmark-{}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory)?;
    }
    fs::create_dir_all(&directory)?;
    let result = (|| -> Result<serde_json::Value> {
        let source_path = directory.join("24mp-source.jpg");
        let source = deterministic_rgb(EXPORT_WIDTH, EXPORT_HEIGHT);
        DynamicImage::ImageRgb8(source)
            .save(&source_path)
            .context("prepare deterministic 24 MP benchmark source")?;

        let import_source = directory.join("import-source.jpg");
        DynamicImage::ImageRgb8(deterministic_rgb(2400, 1600))
            .save(&import_source)
            .context("prepare deterministic import benchmark source")?;
        let mut import_paths = Vec::with_capacity(12);
        for index in 0..12 {
            let path = directory.join(format!("import-{index:02}.jpg"));
            fs::copy(&import_source, &path)?;
            import_paths.push(path);
        }
        let mut import_samples = Vec::with_capacity(6);
        for _ in 0..6 {
            let started = Instant::now();
            let mut import_project = Project::new("Import benchmark");
            std::hint::black_box(import_project.import(&import_paths)?);
            import_samples.push(started.elapsed());
        }

        let mut project = Project::new("Performance benchmark");
        let mut photo = Photo::new(
            1,
            source_path.clone(),
            "24mp-source.jpg".into(),
            EXPORT_WIDTH,
            EXPORT_HEIGHT,
        );
        photo.format = "jpg".into();
        project.photos.push(photo);
        project.selected = Some(1);
        let catalog_path = directory.join("benchmark.lumencatalog");
        project.save(&catalog_path)?;
        let mut workspace = Workspace::new(project, Some(catalog_path.clone()));

        let mut command_samples = Vec::with_capacity(12);
        for iteration in 0..12 {
            let started = Instant::now();
            let project = Project::load(&catalog_path)?;
            let mut invocation = Workspace::new(project, Some(catalog_path.clone()));
            let mut adjustments = invocation.project.photo(1)?.adjustments.clone();
            let y = if iteration % 2 == 0 { 0.42 } else { 0.58 };
            adjustments.curves.master = ToneCurve {
                points: vec![
                    CurvePoint { x: 0.0, y: 0.0 },
                    CurvePoint { x: 0.5, y },
                    CurvePoint { x: 1.0, y: 1.0 },
                ],
            };
            invocation.execute(Command::SetAdjustments { id: 1, adjustments })?;
            invocation.project.save(&catalog_path)?;
            command_samples.push(started.elapsed());
            workspace = invocation;
        }

        let preview = DynamicImage::ImageRgb8(deterministic_rgb(PREVIEW_WIDTH, PREVIEW_HEIGHT));
        let preview_adjustments = Adjustments {
            exposure: 0.35,
            contrast: 12.0,
            shadows: 18.0,
            vibrance: 8.0,
            curves: ToneCurves {
                master: ToneCurve {
                    points: vec![
                        CurvePoint { x: 0.0, y: 0.0 },
                        CurvePoint { x: 0.4, y: 0.35 },
                        CurvePoint { x: 1.0, y: 1.0 },
                    ],
                },
                ..Default::default()
            },
            ..Default::default()
        };
        std::hint::black_box(render_image(
            preview.clone(),
            preview_adjustments.clone(),
            RenderOptions::default(),
        ));
        let mut preview_samples = Vec::with_capacity(8);
        for _ in 0..8 {
            let started = Instant::now();
            std::hint::black_box(render_image(
                preview.clone(),
                preview_adjustments.clone(),
                RenderOptions::default(),
            ));
            preview_samples.push(started.elapsed());
        }

        let export_path = directory.join("24mp-export.jpg");
        let started = Instant::now();
        workspace.execute(Command::Export {
            id: 1,
            path: export_path.clone(),
            max_size: None,
            quality: 90,
        })?;
        let export_duration = started.elapsed();
        let export_size = fs::metadata(&export_path)?.len();

        let command = latency_metric(
            "tone_curve_command",
            "Catalog load, core command, and atomic persistence",
            &command_samples,
            8.0,
            profile.command_budget_ms(),
        );
        let preview_metric = latency_metric(
            "tone_curve_preview",
            "1800x1200 developed preview frame",
            &preview_samples,
            16.7,
            profile.preview_budget_ms(),
        );
        let export_ms = duration_ms(export_duration);
        let export_pass = export_ms <= 5000.0;
        let export = json!({
            "name": "jpeg_export_24mp",
            "workload": "6000x4000 JPEG decode, develop, and quality-90 encode",
            "elapsed_ms": rounded(export_ms),
            "megapixels_per_second": rounded(24.0 / export_duration.as_secs_f64()),
            "output_bytes": export_size,
            "target_ms": 2000.0,
            "budget_ms": 5000.0,
            "pass": export_pass,
        });
        let jpeg_import = latency_metric(
            "jpeg_batch_import",
            "12 deterministic 2400x1600 JPEG files: validate, dimensions, and catalog records",
            &import_samples,
            75.0,
            250.0,
        );
        let raw_import_metric = raw_import.map(|path| -> Result<serde_json::Value> {
            let mut samples = Vec::with_capacity(3);
            for _ in 0..3 {
                let started = Instant::now();
                let mut project = Project::new("RAW import benchmark");
                std::hint::black_box(project.import(&[path.to_owned()])?);
                samples.push(started.elapsed());
            }
            Ok(latency_metric(
                "raw_metadata_import",
                "Supplied RAW: container validation, dimensions, EXIF, and catalog record (no development)",
                &samples,
                250.0,
                1500.0,
            ))
        }).transpose()?;
        let passed = command["pass"].as_bool() == Some(true)
            && preview_metric["pass"].as_bool() == Some(true)
            && jpeg_import["pass"].as_bool() == Some(true)
            && raw_import_metric
                .as_ref()
                .is_none_or(|metric| metric["pass"].as_bool() == Some(true))
            && export_pass;
        let mut metrics = vec![command, preview_metric, jpeg_import, export];
        if let Some(metric) = raw_import_metric {
            metrics.push(metric);
        }
        let report = json!({
            "ok": true,
            "action": "benchmark",
            "strict": strict,
            "profile": profile.name(),
            "passed": passed,
            "budgets": "Targets describe excellent feel; budgets are CI regression limits.",
            "metrics": metrics,
        });
        if strict && !passed {
            anyhow::bail!(
                "performance budget missed: {}",
                serde_json::to_string(&report)?
            );
        }
        Ok(report)
    })();
    let _ = fs::remove_dir_all(&directory);
    result
}

fn deterministic_rgb(width: u32, height: u32) -> RgbImage {
    RgbImage::from_fn(width, height, |x, y| {
        Rgb([
            ((x * 13 + y * 3) % 256) as u8,
            ((x * 5 + y * 11) % 256) as u8,
            ((x * 7 + y * 17) % 256) as u8,
        ])
    })
}

fn latency_metric(
    name: &str,
    workload: &str,
    samples: &[Duration],
    target_ms: f64,
    budget_ms: f64,
) -> serde_json::Value {
    let mut milliseconds: Vec<_> = samples.iter().copied().map(duration_ms).collect();
    milliseconds.sort_by(f64::total_cmp);
    let median = percentile(&milliseconds, 0.5);
    let p95 = percentile(&milliseconds, 0.95);
    json!({
        "name": name,
        "workload": workload,
        "samples": milliseconds.len(),
        "median_ms": rounded(median),
        "p95_ms": rounded(p95),
        "target_ms": target_ms,
        "budget_ms": budget_ms,
        "pass": p95 <= budget_ms,
    })
}

fn percentile(sorted: &[f64], quantile: f64) -> f64 {
    let index = ((sorted.len().saturating_sub(1)) as f64 * quantile).ceil() as usize;
    sorted[index]
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn rounded(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
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
            "color_grading": { "ranges": ["shadows", "midtones", "highlights"], "hue": [0, 360], "saturation": [0, 100], "luminance": [-100, 100], "balance": [-100, 100] },
            "spots": { "type": "normalized repair dabs", "fields": ["x", "y", "radius", "opacity"] },
            "rotation": { "values": [0, 90, 180, 270], "unit": "degrees clockwise" },
            "flip_horizontal": { "type": "boolean" },
            "flip_vertical": { "type": "boolean" }
        },
        "raw_command_examples": [
            { "command": "adjust", "id": 1, "patch": { "exposure": 0.7, "shadows": 18 } },
            { "command": "copy-edits", "id": 1 },
            { "command": "paste-edits", "ids": [2, 3] },
            { "command": "history-back", "id": 1 },
            { "command": "set-pick", "ids": [1, 2], "state": "keep" },
            { "command": "rename-batch", "id": 1, "name": "Night walk" },
            { "command": "save-preset", "name": "Warm portrait", "from_id": 1 },
            { "command": "apply-preset", "preset_id": 1, "ids": [2, 3] },
            { "command": "export-batch", "ids": [1, 2], "directory": "finished", "format": "jpeg", "max_size": 3000, "quality": 90 },
            { "command": "export", "id": 1, "path": "output.jpg", "max_size": 2400, "quality": 92 }
        ]
    })
}

#[allow(dead_code)]
fn _assert_adjustments_are_public(_: Adjustments) {}
