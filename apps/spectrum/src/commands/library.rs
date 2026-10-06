use anyhow::Result;
use clap::{Parser, Subcommand};
use spectrum_assets::{Service, default_root};
use spectrum_library::{AssetId, AssetKind, ProjectId};
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
    /// Rename an image or canvas.
    Rename { asset: AssetId, name: String },
    /// Move an asset to the trash for 30 days. Canvases that use it show a
    /// same-sized placeholder until it is restored.
    Delete { asset: AssetId },
    /// List, restore, or permanently empty the trash.
    Trash {
        #[command(subcommand)]
        command: Trash,
    },
    /// Make an independent copy, including referenced images for a canvas.
    Copy { asset: AssetId },
    /// Show typed JSON command formats for editor automation.
    Schema,
}
#[derive(Subcommand)]
enum Trash {
    /// List trashed assets, most recently deleted first.
    List,
    /// Return an asset to the library, its projects, and its canvases.
    Restore { asset: AssetId },
    /// Permanently delete everything in the trash now.
    Empty,
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
    /// Give images every edit from another image, crop included.
    ApplyEdits {
        from: AssetId,
        #[arg(required = true)]
        to: Vec<AssetId>,
    },
    /// Execute image Command JSON: one object or an array.
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
        /// Background color as RRGGBB or RRGGBBAA.
        #[arg(long, default_value = "18191dff")]
        background: String,
        /// Add the new canvas to this project.
        #[arg(long = "project", id = "project_id")]
        project: Option<ProjectId>,
    },
    Place {
        canvas: AssetId,
        image: AssetId,
    },
    /// Execute canvas Command JSON: one object, or an array applied as one edit.
    Command {
        asset: AssetId,
        json: String,
    },
    Export {
        asset: AssetId,
        path: PathBuf,
        #[arg(long, default_value_t = 92, value_parser = clap::value_parser!(u8).range(1..=100))]
        quality: u8,
        /// Longest edge in pixels; the full canvas size when omitted.
        #[arg(long)]
        max_size: Option<u32>,
    },
}
pub(super) fn run(cli: Cli) -> Result<serde_json::Value> {
    let mut service = Service::agent(&cli.library.map(Ok).unwrap_or_else(default_root)?)?;
    service.purge_expired()?;
    let value = match cli.command {
        Domain::Rename { asset, name } => serde_json::to_value(service.rename(asset, &name)?)?,
        Domain::Delete { asset } => serde_json::to_value(service.delete(asset)?)?,
        Domain::Trash {
            command: Trash::List,
        } => serde_json::to_value(service.library.trashed()?)?,
        Domain::Trash {
            command: Trash::Restore { asset },
        } => serde_json::to_value(service.restore(asset)?)?,
        Domain::Trash {
            command: Trash::Empty,
        } => {
            let trashed = service.library.trashed()?;
            for entry in &trashed {
                service.purge(entry.asset.id)?;
            }
            serde_json::json!({"purged": trashed.iter().map(|t| t.asset.id).collect::<Vec<_>>()})
        }
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
            serde_json::json!({"images":{"adjust":{"exposure":1.0},"command":{"action":"set_adjustments","adjustments":spectrum_image::Adjustments::default()}},"canvas":{"command":{"command":"add_text","text":"Hello","name":null,"font_size":48,"color":[255,255,255,255],"x":0,"y":0}},"asset_types":["image","canvas"],"references":"live","copy":"independent, recursively copies referenced content"})
        }
        Domain::Images {
            command: Images::List,
        } => serde_json::to_value(
            service
                .library
                .list()?
                .into_iter()
                .filter(|a| a.kind == AssetKind::Image)
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
        }
        | Domain::Canvas {
            command:
                Canvas::Export {
                    asset,
                    path,
                    quality,
                    max_size,
                },
        } => {
            let options = spectrum_assets::ExportOptions { quality, max_size };
            service.export_with(asset, &path, options)?;
            serde_json::json!({"exported":path})
        }
        Domain::Images {
            command: Images::ApplyEdits { from, to },
        } => {
            service.apply_edits(from, &to)?;
            serde_json::json!({"from": from, "to": to})
        }
        Domain::Images {
            command: Images::Adjust { asset, patch },
        } => {
            service.adjust(asset, serde_json::from_str(&patch)?)?;
            serde_json::json!({"asset": asset, "adjustments": service.image(asset)?.adjustments})
        }
        Domain::Images {
            command: Images::Command { asset, json },
        } => {
            let outputs = service.edit_image(asset, super::images::decode_commands(&json)?)?;
            serde_json::json!({"asset": asset, "results": outputs})
        }
        Domain::Canvas {
            command: Canvas::List,
        } => serde_json::to_value(
            service
                .library
                .list()?
                .into_iter()
                .filter(|a| a.kind == AssetKind::Canvas)
                .collect::<Vec<_>>(),
        )?,
        Domain::Canvas {
            command:
                Canvas::New {
                    name,
                    width,
                    height,
                    background,
                    project,
                },
        } => {
            if let Some(project) = project {
                service.library.project(project)?;
            }
            let mut document = spectrum_canvas::Document::new(name, width, height);
            document.background = super::canvas::parse_color(&background)?;
            let canvas = service.create_canvas_from(document)?;
            if let Some(project) = project {
                service.library.add_to_project(project, &[canvas.id])?;
            }
            serde_json::to_value(canvas)?
        }
        Domain::Canvas {
            command: Canvas::Place { canvas, image },
        } => serde_json::json!({"layer":service.place(canvas,image)?}),
        Domain::Canvas {
            command: Canvas::Command { asset, json },
        } => {
            let commands = super::canvas::decode_commands(&json)?;
            serde_json::json!({"asset": asset, "results": service.edit_canvas(asset, commands)?})
        }
    };
    Ok(value)
}
