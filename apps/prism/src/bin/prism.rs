use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Instant,
};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use lumen_core::{Project as LumenProject, engine::render_photo};
use prism_core::{
    BlendMode, Command, Document, LayerMask, Transform, Workspace, export_document,
    render_document, render_solid_color, save_document,
};
use serde::Serialize;
use serde_json::{Value, json};
use spectrum_imaging::{AdjustmentPatch, Adjustments, RenderOptions};

#[derive(Parser)]
#[command(name = "prism", version, about = "Agent-first layered image editor")]
struct Cli {
    #[arg(short, long, global = true, default_value = "untitled.prism")]
    project: PathBuf,
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
    /// Print the machine-facing Command protocol and examples.
    Schema,
    /// Run deterministic command and compositing performance workloads.
    Benchmark {
        #[arg(long)]
        strict: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum CliBlend {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
}

impl From<CliBlend> for BlendMode {
    fn from(value: CliBlend) -> Self {
        match value {
            CliBlend::Normal => Self::Normal,
            CliBlend::Multiply => Self::Multiply,
            CliBlend::Screen => Self::Screen,
            CliBlend::Overlay => Self::Overlay,
            CliBlend::Darken => Self::Darken,
            CliBlend::Lighten => Self::Lighten,
            CliBlend::ColorDodge => Self::ColorDodge,
            CliBlend::ColorBurn => Self::ColorBurn,
            CliBlend::HardLight => Self::HardLight,
            CliBlend::SoftLight => Self::SoftLight,
            CliBlend::Difference => Self::Difference,
            CliBlend::Exclusion => Self::Exclusion,
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
            save_document(&document, &cli.project)?;
            Ok(json!({"ok": true, "action": "init", "project": cli.project, "document": document}))
        }
        CliCommand::List => {
            let workspace = Workspace::open(&cli.project)?;
            Ok(json!({"ok": true, "project": cli.project, "document": workspace.document}))
        }
        CliCommand::Export { path, quality } => {
            let workspace = Workspace::open(&cli.project)?;
            export_document(&workspace.document, &path, quality)?;
            Ok(json!({"ok": true, "action": "export", "path": path}))
        }
        CliCommand::FromLumen {
            catalog,
            photo,
            output,
        } => from_lumen(&catalog, photo, &output),
        CliCommand::Schema => Ok(schema()),
        CliCommand::Benchmark { strict } => benchmark(strict),
        command => {
            let mut workspace = Workspace::open(&cli.project)?;
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
                | CliCommand::Schema
                | CliCommand::Benchmark { .. } => unreachable!(),
            };
            workspace.save(None)?;
            Ok(json!({"ok": true, "project": cli.project, "results": outputs}))
        }
    }
}

fn run_commands(workspace: &mut Workspace, value: &str) -> Result<Vec<prism_core::CommandOutput>> {
    if value.trim_start().starts_with('[') {
        serde_json::from_str::<Vec<Command>>(value)?
            .into_iter()
            .map(|command| workspace.execute(command))
            .collect()
    } else {
        Ok(vec![workspace.execute(serde_json::from_str(value)?)?])
    }
}

fn from_lumen(catalog: &Path, photo_id: u64, output: &Path) -> Result<Value> {
    let project = LumenProject::load(catalog)?;
    let photo = project.photo(photo_id)?;
    let rendered = render_photo(photo, RenderOptions::default())?;
    let directory = output.parent().unwrap_or_else(|| Path::new("."));
    let assets = directory.join("prism-assets");
    std::fs::create_dir_all(&assets)?;
    let mut source_hash = DefaultHasher::new();
    std::fs::canonicalize(catalog)
        .unwrap_or_else(|_| catalog.to_owned())
        .hash(&mut source_hash);
    photo_id.hash(&mut source_hash);
    let asset = assets.join(format!(
        "lumen-{:016x}-{photo_id}.png",
        source_hash.finish()
    ));
    rendered.save(&asset)?;

    let mut workspace = Workspace::new(
        Document::new(
            format!("{} — {}", project.name, photo.name),
            rendered.width(),
            rendered.height(),
        ),
        Some(output.to_owned()),
    );
    workspace.document.background = [0, 0, 0, 0];
    workspace.execute(Command::AddRaster {
        path: asset,
        name: Some(photo.name.clone()),
        x: 0.0,
        y: 0.0,
    })?;
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
        "command_protocol": {
            "encoding": "serde tagged JSON",
            "tag": "command",
            "examples": [
                {"command": "add_text", "text": "Hello", "name": null, "font_size": 72.0, "color": [255,255,255,255], "x": 100.0, "y": 120.0},
                {"command": "set_transform", "id": 1, "transform": {"x": 220.0, "y": 160.0, "scale_x": 1.2, "scale_y": 1.2, "rotation": 8.0}},
                {"command": "set_mask", "id": 1, "mask": {"enabled": true, "x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8, "invert": false}},
                {"command": "adjust_layer", "id": 1, "patch": {"exposure": 0.5, "contrast": 12.0}}
            ]
        },
        "blend_modes": [
            "normal", "multiply", "screen", "overlay", "darken", "lighten",
            "color_dodge", "color_burn", "hard_light", "soft_light", "difference",
            "exclusion"
        ],
        "layer_types": ["raster", "text", "rectangle"],
        "color": "RRGGBB or RRGGBBAA",
        "coordinates": "canvas pixels; layer masks are normalized 0..1"
    })
}

#[derive(Serialize)]
struct BenchmarkMetric {
    name: &'static str,
    median_ms: f64,
    p95_ms: f64,
    budget_ms: f64,
    pass: bool,
}

fn benchmark(strict: bool) -> Result<Value> {
    let mut command_samples = Vec::new();
    let mut workspace = None;
    for _ in 0..9 {
        let mut sample = Workspace::new(Document::new("Benchmark", 1600, 1200), None);
        let started = Instant::now();
        for index in 0..24 {
            sample.execute(Command::AddRectangle {
                name: Some(format!("Layer {index}")),
                width: 720,
                height: 480,
                color: [40 + index * 6, 90, 180, 180],
                corner_radius: 24.0,
                x: (index * 17) as f32,
                y: (index * 11) as f32,
            })?;
        }
        command_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        workspace = Some(sample);
    }
    let workspace = workspace.expect("benchmark always records at least one command sample");
    let mut interaction_workspace = Workspace::new(workspace.document.clone(), None);
    let interaction_layer = interaction_workspace.document.layers.last().unwrap().id;
    interaction_workspace.begin_interaction();
    let mut interaction_samples = Vec::new();
    for frame in 0..240 {
        let started = Instant::now();
        interaction_workspace.preview(Command::SetTransform {
            id: interaction_layer,
            transform: Transform {
                x: frame as f32 * 2.0,
                y: frame as f32,
                scale_x: 1.0 + frame as f32 / 1_000.0,
                scale_y: 1.0 + frame as f32 / 1_000.0,
                rotation: 0.0,
            },
        })?;
        interaction_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    interaction_workspace.commit_interaction();
    let mut text_workspace = Workspace::new(Document::new("Text benchmark", 1600, 1200), None);
    text_workspace.execute(Command::AddText {
        text: "Prism interaction benchmark".into(),
        name: Some("Text".into()),
        font_size: 144.0,
        color: [255, 255, 255, 255],
        x: 100.0,
        y: 100.0,
    })?;
    let text_layer = text_workspace.document.selected.unwrap();
    text_workspace.begin_interaction();
    let mut text_interaction_samples = Vec::new();
    for frame in 0..240 {
        let started = Instant::now();
        text_workspace.preview(Command::SetTransform {
            id: text_layer,
            transform: Transform {
                x: 100.0 + frame as f32 * 2.0,
                y: 100.0 + frame as f32,
                ..Default::default()
            },
        })?;
        text_interaction_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    text_workspace.commit_interaction();
    let mut shape_preview_samples = Vec::new();
    for frame in 0..240 {
        let adjustments = Adjustments {
            exposure: frame as f32 / 48.0 - 2.5,
            contrast: frame as f32 % 100.0 - 50.0,
            ..Default::default()
        };
        let started = Instant::now();
        let _ = render_solid_color([93, 216, 199, 255], &adjustments);
        shape_preview_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let mut render_samples = Vec::new();
    let mut rendered = None;
    for _ in 0..7 {
        let started = Instant::now();
        rendered = Some(render_document(&workspace.document, None)?);
        render_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let (command_median, command_p95) = sample_summary(&mut command_samples);
    let (interaction_median, interaction_p95) = sample_summary(&mut interaction_samples);
    let (text_interaction_median, text_interaction_p95) =
        sample_summary(&mut text_interaction_samples);
    let (shape_median, shape_p95) = sample_summary(&mut shape_preview_samples);
    let (render_median, render_p95) = sample_summary(&mut render_samples);
    let metrics = [
        BenchmarkMetric {
            name: "flat_shape_adjustment_preview",
            median_ms: shape_median,
            p95_ms: shape_p95,
            budget_ms: 0.5,
            pass: shape_p95 <= 0.5,
        },
        BenchmarkMetric {
            name: "live_transform_preview",
            median_ms: interaction_median,
            p95_ms: interaction_p95,
            budget_ms: 8.0,
            pass: interaction_p95 <= 8.0,
        },
        BenchmarkMetric {
            name: "live_text_move_preview",
            median_ms: text_interaction_median,
            p95_ms: text_interaction_p95,
            budget_ms: 1.0,
            pass: text_interaction_p95 <= 1.0,
        },
        BenchmarkMetric {
            name: "24_layer_command_batch",
            median_ms: command_median,
            p95_ms: command_p95,
            budget_ms: 50.0,
            pass: command_p95 <= 50.0,
        },
        BenchmarkMetric {
            name: "1600x1200_24_layer_composite",
            median_ms: render_median,
            p95_ms: render_p95,
            budget_ms: 2_000.0,
            pass: render_p95 <= 2_000.0,
        },
    ];
    let passed = metrics.iter().all(|metric| metric.pass);
    if strict && !passed {
        bail!("Prism benchmark exceeded a strict regression budget");
    }
    Ok(json!({
        "ok": true,
        "action": "benchmark",
        "strict": strict,
        "passed": passed,
        "output": [rendered.as_ref().unwrap().width(), rendered.as_ref().unwrap().height()],
        "metrics": metrics
    }))
}

fn sample_summary(samples: &mut [f64]) -> (f64, f64) {
    samples.sort_by(f64::total_cmp);
    let median = samples[samples.len() / 2];
    let p95_index = ((samples.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
    (median, samples[p95_index.min(samples.len() - 1)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_accept_rgb_and_rgba() {
        assert_eq!(parse_color("ae7bff").unwrap(), [174, 123, 255, 255]);
        assert_eq!(parse_color("#01020304").unwrap(), [1, 2, 3, 4]);
    }
}
