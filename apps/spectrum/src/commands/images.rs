//! Editing one image asset: its adjustments, crop, and history.
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use spectrum_assets::Service;
use spectrum_image::{
    AdjustmentPatch, Adjustments, ColorGrade, Command, CropRect, CurvePoint, SpotRemoval, ToneCurve,
};
use spectrum_library::AssetId;

#[path = "image_commands/benchmark.rs"]
mod benchmark;
#[path = "image_commands/benchmark_profile.rs"]
mod benchmark_profile;
use benchmark_profile::BenchmarkProfile;

#[derive(Parser)]
#[command(
    name = "images",
    version,
    about = "Spectrum image editing commands",
    long_about = "Edit one image asset, chosen with --asset. All successful output is JSON."
)]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Subcommand)]
enum CliCommand {
    /// Inspect the image: its source, size, camera details, and edits.
    Get,
    /// Set one or more edit values.
    Edit(EditArgs),
    /// Set or clear a normalized crop rectangle and straighten angle.
    Crop {
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
        channel: CurveChannel,
        /// Semicolon-separated points, for example: 0,0;0.4,0.55;1,1
        #[arg(long)]
        points: Option<String>,
        #[arg(long)]
        reset: bool,
    },
    /// Grade shadows, midtones, or highlights with hue, saturation, and luminance.
    Grade {
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
    /// Add or clear dust and smudge repair dabs.
    Spot {
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
    /// Rotate 90 degrees.
    Rotate {
        #[arg(long)]
        counterclockwise: bool,
    },
    /// Flip horizontally or vertically.
    Flip {
        #[arg(long, conflicts_with = "vertical")]
        horizontal: bool,
        #[arg(long)]
        vertical: bool,
    },
    /// Remove every edit.
    Reset,
    /// Show the image's revisions and where each session is.
    History,
    /// Go back one edit.
    Undo,
    /// Go forward one edit.
    Redo,
    /// Return to a revision from `history`.
    HistoryJump {
        revision: spectrum_document::RevisionId,
    },
    /// Execute image Command JSON: one object or an array.
    Run { json: String },
    /// Run image rendering and editing performance workloads.
    Benchmark {
        /// Fail when any performance budget is missed.
        #[arg(long)]
        strict: bool,
        /// Budget calibration: workstation feel or GitHub's shared Linux runner.
        #[arg(long, value_enum, default_value_t = BenchmarkProfile::Interactive)]
        profile: BenchmarkProfile,
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

pub(super) fn definition() -> clap::Command {
    use clap::CommandFactory;
    Cli::command()
}

pub(super) fn protocol() -> Value {
    schema()
}

/// Runs an editor command on one image asset.
pub(super) fn execute_target(
    matches: &clap::ArgMatches,
    service: &mut Service,
    id: AssetId,
) -> Result<Value> {
    use clap::FromArgMatches;
    let command = if matches.subcommand_name() == Some("inspect") {
        CliCommand::Get
    } else {
        CliCommand::from_arg_matches(matches)?
    };
    run(service, id, command)
}

/// Commands that need no image.
pub(super) fn execute_standalone(matches: &clap::ArgMatches) -> Option<Result<Value>> {
    use clap::FromArgMatches;
    match CliCommand::from_arg_matches(matches).ok()? {
        CliCommand::Schema => Some(Ok(schema())),
        CliCommand::Benchmark { strict, profile } => Some(benchmark::benchmark(strict, profile)),
        _ => None,
    }
}

fn run(service: &mut Service, id: AssetId, command: CliCommand) -> Result<Value> {
    let set = |service: &Service, edit: &dyn Fn(&mut Adjustments) -> Result<()>| {
        let mut adjustments = service.image(id)?.adjustments;
        edit(&mut adjustments)?;
        service.edit_image(id, vec![Command::SetAdjustments { adjustments }])
    };
    let outputs = match command {
        CliCommand::Get | CliCommand::Schema | CliCommand::Benchmark { .. } => {
            return Ok(json!({"ok": true, "asset": id, "image": service.image(id)?}));
        }
        CliCommand::History => {
            let history = service.history(id)?;
            return Ok(json!({
                "ok": true,
                "asset": id,
                "root": history.root,
                "current": history.current,
                "revisions": history.revisions,
                "sessions": history.sessions,
            }));
        }
        CliCommand::HistoryJump { revision } => {
            service.move_to(id, revision)?;
            return Ok(json!({"ok": true, "action": "history_jump", "revision": revision}));
        }
        CliCommand::Edit(arguments) => service.edit_image(
            id,
            vec![Command::Adjust {
                patch: arguments.into(),
            }],
        )?,
        CliCommand::Crop {
            x,
            y,
            width,
            height,
            straighten,
            clear,
        } => set(service, &|adjustments| {
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
            Ok(())
        })?,
        CliCommand::Hsl {
            color,
            hue,
            saturation,
            luminance,
            reset,
        } => set(service, &|adjustments| {
            let band = adjustments.hsl.band_mut(color as usize);
            if reset {
                *band = Default::default();
            }
            band.hue = hue.unwrap_or(band.hue);
            band.saturation = saturation.unwrap_or(band.saturation);
            band.luminance = luminance.unwrap_or(band.luminance);
            Ok(())
        })?,
        CliCommand::Curve {
            channel,
            points,
            reset,
        } => set(service, &|adjustments| {
            let curve = match channel {
                CurveChannel::Master => &mut adjustments.curves.master,
                CurveChannel::Red => &mut adjustments.curves.red,
                CurveChannel::Green => &mut adjustments.curves.green,
                CurveChannel::Blue => &mut adjustments.curves.blue,
            };
            if reset {
                *curve = ToneCurve::default();
            }
            if let Some(points) = &points {
                *curve = parse_curve(points)?;
            }
            Ok(())
        })?,
        CliCommand::Grade {
            range,
            hue,
            saturation,
            luminance,
            balance,
            reset,
        } => set(service, &|adjustments| {
            let grade = match range {
                GradeRange::Shadows => &mut adjustments.color_grading.shadows,
                GradeRange::Midtones => &mut adjustments.color_grading.midtones,
                GradeRange::Highlights => &mut adjustments.color_grading.highlights,
            };
            if reset {
                *grade = ColorGrade::default();
            }
            grade.hue = hue.unwrap_or(grade.hue);
            grade.saturation = saturation.unwrap_or(grade.saturation);
            grade.luminance = luminance.unwrap_or(grade.luminance);
            if let Some(value) = balance {
                adjustments.color_grading.balance = value;
            }
            Ok(())
        })?,
        CliCommand::Spot {
            x,
            y,
            radius,
            opacity,
            clear,
        } => set(service, &|adjustments| {
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
            Ok(())
        })?,
        CliCommand::Rotate { counterclockwise } => set(service, &|adjustments| {
            let turn = if counterclockwise { -90 } else { 90 };
            adjustments.rotation = (adjustments.rotation + turn).rem_euclid(360);
            Ok(())
        })?,
        CliCommand::Flip {
            horizontal,
            vertical,
        } => set(service, &|adjustments| {
            if vertical && !horizontal {
                adjustments.flip_vertical = !adjustments.flip_vertical;
            } else {
                adjustments.flip_horizontal = !adjustments.flip_horizontal;
            }
            Ok(())
        })?,
        CliCommand::Reset => service.edit_image(id, vec![Command::Reset])?,
        CliCommand::Undo => service.edit_image(id, vec![Command::Undo])?,
        CliCommand::Redo => service.edit_image(id, vec![Command::Redo])?,
        CliCommand::Run { json } => service.edit_image(id, decode_commands(&json)?)?,
    };
    Ok(json!({
        "ok": true,
        "asset": id,
        "results": outputs,
        "adjustments": service.image(id)?.adjustments,
    }))
}

/// One command object, or an array of them.
pub(super) fn decode_commands(value: &str) -> Result<Vec<Command>> {
    let commands = if value.trim_start().starts_with('[') {
        serde_json::from_str(value)?
    } else {
        vec![serde_json::from_str(value)?]
    };
    if commands.is_empty() {
        bail!("there is nothing to do");
    }
    Ok(commands)
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
        bail!("a tone curve needs at least two points");
    }
    Ok(ToneCurve { points }.sanitized())
}

fn schema() -> Value {
    json!({
        "ok": true,
        "output": "JSON on stdout; structured errors on stderr; nonzero exit on failure",
        "targeting": "spectrum images --asset <UUID> <command>; spectrum library lists asset IDs",
        "storage": "each image is one .spectrum document: its embedded source photo and a revision for every edit",
        "history": "history lists revisions; undo, redo, and history-jump move through them",
        "commands": {
            "adjust": { "action": "adjust", "patch": { "exposure": 0.7, "shadows": 18 } },
            "set_adjustments": { "action": "set_adjustments", "adjustments": Adjustments::default() },
            "reset": { "action": "reset" },
            "undo": { "action": "undo" },
            "redo": { "action": "redo" }
        },
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
        }
    })
}
