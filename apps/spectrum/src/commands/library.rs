use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use spectrum::library::{Service, default_root};
use spectrum_library::{AssetId, ProjectId};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "spectrum", about = "Spectrum's shared creative library")]
pub(super) struct Cli {
    #[arg(long, global = true, env = "SPECTRUM_LIBRARY")]
    library: Option<PathBuf>,
    #[command(subcommand)]
    command: Domain,
}
#[derive(Subcommand)]
enum Domain {
    /// List all assets and inspect the library location.
    Library {
        /// List only assets that belong to no project.
        #[arg(long)]
        unassigned: bool,
    },
    /// Create projects and organize library assets into them.
    Projects {
        #[command(subcommand)]
        command: super::projects::Projects,
    },
    /// List import batches, newest first. Each lists the assets imported together.
    Imports,
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
    /// Import image files as one batch, optionally into a project.
    Import {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Add the imported assets to this project.
        #[arg(long = "project", id = "project_id")]
        project: Option<ProjectId>,
        /// Create a project with this name and add the imported assets to it.
        #[arg(long, conflicts_with = "project_id")]
        new_project: Option<String>,
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
        #[arg(long, default_value_t = 92, value_parser = clap::value_parser!(u8).range(1..=100))]
        quality: u8,
        #[arg(long)]
        max_size: Option<u32>,
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
        /// Add the new canvas to this project.
        #[arg(long = "project", id = "project_id")]
        project: Option<ProjectId>,
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
        #[arg(long, default_value_t = 92, value_parser = clap::value_parser!(u8).range(1..=100))]
        quality: u8,
    },
}
pub(super) fn run(cli: Cli) -> Result<serde_json::Value> {
    let mut service = Service::open(&cli.library.map(Ok).unwrap_or_else(default_root)?)?;
    service.ensure_catalog()?;
    service.scan()?;
    let value = match cli.command {
        Domain::Library { unassigned } => {
            let assets = if unassigned {
                service.library.unassigned()?
            } else {
                service.library.list()?
            };
            serde_json::json!({"root":service.library.root(),"assets":assets})
        }
        Domain::Projects { command } => super::projects::run(&mut service, command)?,
        Domain::Imports => serde_json::to_value(service.library.imports()?)?,
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
            command:
                Images::Import {
                    paths,
                    project,
                    new_project,
                },
        } => {
            let created = new_project
                .map(|name| service.library.create_project(&name))
                .transpose()?
                .map(|p| p.id);
            let project = created.or(project);
            let (batch, assets) = match service.import_into(paths, project) {
                Ok(imported) => imported,
                Err(error) => {
                    if let Some(id) = created {
                        service.library.delete_project(id)?;
                    }
                    return Err(error);
                }
            };
            serde_json::json!({"batch": batch.id, "project": project, "assets": assets})
        }
        Domain::Images {
            command:
                Images::Export {
                    asset,
                    path,
                    quality,
                    max_size,
                },
        } => {
            service.check_export(&path)?;
            lumen_core::engine::export_photo(
                &service.image(asset)?,
                &path,
                spectrum_imaging::RenderOptions { max_size },
                quality,
            )?;
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
                    project,
                },
        } => {
            if let Some(project) = project {
                service.library.project(project)?;
            }
            let canvas = service.create_canvas(name, width, height)?;
            if let Some(project) = project {
                service.library.add_to_project(project, &[canvas.id])?;
            }
            serde_json::to_value(canvas)?
        }
        Domain::Canvas {
            command: Canvas::Place { canvas, image },
        } => serde_json::json!({"layer":service.place(canvas,image)?}),
        Domain::Canvas {
            command:
                Canvas::Export {
                    asset,
                    path,
                    quality,
                },
        } => {
            let asset = service.library.get(asset)?;
            if asset.kind != "canvas" {
                bail!("expected canvas");
            }
            let mut doc = prism_core::Workspace::load_read_only(&service.library.path(&asset)?)?;
            service.check_export(&path)?;
            service.resolve(&mut doc)?;
            prism_core::export_document(&doc, &path, quality)?;
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
    Ok(value)
}
