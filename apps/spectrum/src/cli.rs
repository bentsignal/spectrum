use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use spectrum::library::{Service, default_root};
use spectrum_library::AssetId;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "spectrum", about = "Spectrum's shared creative library")]
struct Cli {
    #[arg(long, env = "SPECTRUM_LIBRARY")]
    library: Option<PathBuf>,
    #[command(subcommand)]
    command: Domain,
}
#[derive(Subcommand)]
enum Domain {
    /// List all assets and inspect the library location.
    Library,
    Images {
        #[command(subcommand)]
        command: Images,
    },
    Canvas {
        #[command(subcommand)]
        command: Canvas,
    },
    /// Make an independent copy, including referenced images for a canvas.
    Copy { asset: AssetId },
    /// Show typed JSON command formats for editor automation.
    Schema,
}
#[derive(Subcommand)]
enum Images {
    List,
    Import {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// Apply an adjustment patch, e.g. '{"exposure":1.0}'.
    Adjust {
        asset: AssetId,
        patch: String,
    },
    /// Execute any image engine command as JSON. Photo IDs are local to the asset's document.
    Command {
        asset: AssetId,
        json: String,
    },
    Export {
        asset: AssetId,
        path: PathBuf,
    },
}
#[derive(Subcommand)]
enum Canvas {
    List,
    New {
        name: String,
        #[arg(long, default_value_t = 1920)]
        width: u32,
        #[arg(long, default_value_t = 1080)]
        height: u32,
    },
    Place {
        canvas: AssetId,
        image: AssetId,
    },
    /// Execute any canvas engine command as JSON.
    Command {
        asset: AssetId,
        json: String,
    },
    Export {
        asset: AssetId,
        path: PathBuf,
    },
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut service = Service::open(&cli.library.map(Ok).unwrap_or_else(default_root)?)?;
    service.ensure_catalog()?;
    service.scan()?;
    let value = match cli.command {
        Domain::Library => {
            serde_json::json!({"root":service.library.root(),"assets":service.library.list()?})
        }
        Domain::Copy { asset } => serde_json::to_value(service.copy(asset)?)?,
        Domain::Schema => {
            serde_json::json!({"images":{"adjust":{"exposure":1.0},"command":{"command":"set-adjustments","id":1,"adjustments":lumen_core::Adjustments::default()}},"canvas":{"command":{"command":"add_text","text":"Hello","name":null,"font_size":48,"color":[255,255,255,255],"x":0,"y":0}},"asset_types":["image","canvas"],"references":"live","copy":"independent, recursively copies referenced content"})
        }
        Domain::Images {
            command: Images::List,
        } => serde_json::to_value(
            service
                .library
                .list()?
                .into_iter()
                .filter(|a| a.kind == "image")
                .collect::<Vec<_>>(),
        )?,
        Domain::Images {
            command: Images::Import { paths },
        } => serde_json::to_value(service.import(paths)?)?,
        Domain::Images {
            command: Images::Export { asset, path },
        } => {
            service.check_export(&path)?;
            lumen_core::engine::render_photo(&service.image(asset)?, Default::default())?
                .save(&path)?;
            serde_json::json!({"exported":path})
        }
        Domain::Images { command } => {
            let (id, raw, patch) = match command {
                Images::Adjust { asset, patch } => (asset, None, Some(patch)),
                Images::Command { asset, json } => (asset, Some(json), None),
                _ => unreachable!(),
            };
            let asset = service.library.get(id)?;
            if asset.kind != "image" {
                bail!("expected image");
            }
            let command = if let Some(raw) = raw {
                serde_json::from_str(&raw)?
            } else {
                lumen_core::Command::Adjust {
                    id: asset.item.context("missing image item")?,
                    patch: serde_json::from_str(&patch.unwrap())?,
                }
            };
            if matches!(
                command,
                lumen_core::Command::New { .. }
                    | lumen_core::Command::Open { .. }
                    | lumen_core::Command::Save { .. }
            ) {
                bail!("library owns document locations");
            }
            spectrum::library::live::image(
                &service.library.path(&asset)?,
                asset.item.context("missing image item")?,
                command,
            )?
        }
        Domain::Canvas {
            command: Canvas::List,
        } => serde_json::to_value(
            service
                .library
                .list()?
                .into_iter()
                .filter(|a| a.kind == "canvas")
                .collect::<Vec<_>>(),
        )?,
        Domain::Canvas {
            command:
                Canvas::New {
                    name,
                    width,
                    height,
                },
        } => serde_json::to_value(service.create_canvas(name, width, height)?)?,
        Domain::Canvas {
            command: Canvas::Place { canvas, image },
        } => serde_json::json!({"layer":service.place(canvas,image)?}),
        Domain::Canvas {
            command: Canvas::Export { asset, path },
        } => {
            let asset = service.library.get(asset)?;
            if asset.kind != "canvas" {
                bail!("expected canvas");
            }
            let doc = prism_core::Workspace::load_read_only(&service.library.path(&asset)?)?;
            service.export_canvas(&doc, &path)?;
            serde_json::json!({"exported":path})
        }
        Domain::Canvas {
            command: Canvas::Command { asset, json },
        } => {
            let asset = service.library.get(asset)?;
            if asset.kind != "canvas" {
                bail!("expected canvas");
            }
            spectrum::library::live::canvas(
                &service.library.path(&asset)?,
                vec![serde_json::from_str(&json)?],
            )?
        }
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
