use anyhow::Result;
use clap::Subcommand;
use serde_json::json;
use spectrum::library::Service;
use spectrum_library::{AssetId, ProjectId};

/// Projects group library assets. Removing an asset from a project keeps it in
/// the library; assets in no project are listed by `spectrum library --unassigned`.
#[derive(Subcommand)]
pub(super) enum Projects {
    /// List projects with their asset counts.
    List,
    /// Create an empty project. Names are unique, ignoring case.
    Create {
        name: String,
    },
    /// Show a project and its assets, most recently added first.
    Show {
        project: ProjectId,
    },
    Rename {
        project: ProjectId,
        name: String,
    },
    /// Delete a project. Its assets stay in the library.
    Delete {
        project: ProjectId,
    },
    /// Add existing library assets to a project.
    Add {
        project: ProjectId,
        #[arg(required = true)]
        assets: Vec<AssetId>,
    },
    /// Remove assets from a project. They stay in the library.
    Remove {
        project: ProjectId,
        #[arg(required = true)]
        assets: Vec<AssetId>,
    },
}

fn show(service: &Service, project: ProjectId) -> Result<serde_json::Value> {
    Ok(json!({
        "project": service.library.project(project)?,
        "assets": service.library.project_assets(project)?,
    }))
}

pub(super) fn run(service: &mut Service, command: Projects) -> Result<serde_json::Value> {
    let library = &mut service.library;
    Ok(match command {
        Projects::List => serde_json::to_value(library.projects()?)?,
        Projects::Create { name } => serde_json::to_value(library.create_project(&name)?)?,
        Projects::Show { project } => show(service, project)?,
        Projects::Rename { project, name } => {
            serde_json::to_value(library.rename_project(project, &name)?)?
        }
        Projects::Delete { project } => {
            library.delete_project(project)?;
            json!({"deleted": project})
        }
        Projects::Add { project, assets } => {
            library.add_to_project(project, &assets)?;
            show(service, project)?
        }
        Projects::Remove { project, assets } => {
            library.remove_from_project(project, &assets)?;
            show(service, project)?
        }
    })
}
