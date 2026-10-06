mod canvas;
mod images;
mod library;
mod projects;

use anyhow::{Context, Result, bail};
use clap::{Arg, Command, CommandFactory, FromArgMatches};
use spectrum_assets::{Service, default_root};
use spectrum_library::{AssetId, AssetKind};
use std::path::PathBuf;

fn engine(domain: &str) -> Command {
    if domain == "images" {
        images::definition()
    } else {
        canvas::definition()
    }
}

fn definition() -> Command {
    let mut root = library::Cli::command().version(env!("CARGO_PKG_VERSION"));
    for domain in ["images", "canvas"] {
        let base = engine(domain);
        root = root.mut_subcommand(domain, |mut command| {
            command = command.arg(
                Arg::new("target_asset")
                    .long("asset")
                    .global(true)
                    .value_parser(clap::value_parser!(AssetId))
                    .help("The library asset an editor command works on"),
            );
            for subcommand in base.get_subcommands() {
                let name = subcommand.get_name();
                if name == "list" || name == "get" {
                    command = command.subcommand(subcommand.clone().name("inspect"));
                } else if !command.get_subcommands().any(|c| c.get_name() == name) {
                    command = command.subcommand(subcommand.clone());
                }
            }
            command
        });
    }
    root
}

pub(super) fn run() -> Result<serde_json::Value> {
    let matches = definition().get_matches();
    dispatch(matches)
}

/// Library commands, which take asset UUIDs as arguments.
const LIBRARY: [(&str, &[&str]); 2] = [
    (
        "images",
        &[
            "list",
            "import",
            "adjust",
            "apply-edits",
            "command",
            "export",
        ],
    ),
    ("canvas", &["list", "new", "place", "command", "export"]),
];

fn dispatch(matches: clap::ArgMatches) -> Result<serde_json::Value> {
    let (domain, args) = matches.subcommand().context("missing command")?;
    if domain == "schema" {
        return Ok(serde_json::json!({
            "images": images::protocol(), "canvas": canvas::protocol(),
            "library": {"asset_types": ["image", "canvas"], "references": "live",
                "copy": "independent, recursively copies referenced content"},
            "help": "spectrum images --help; spectrum canvas --help",
            "targeting": "Library commands take asset UUIDs. Editor commands take --asset UUID; canvas layer IDs are local to the canvas."
        }));
    }
    let Some((_, library_operations)) = LIBRARY.iter().find(|(name, _)| *name == domain) else {
        return library::run(library::Cli::from_arg_matches(&matches)?);
    };
    let (operation, _) = args.subcommand().context("missing editor command")?;
    let asset = args.get_one::<AssetId>("target_asset").copied();
    if library_operations.contains(&operation) {
        if asset.is_some() {
            bail!(
                "library commands take asset UUIDs as arguments; --asset belongs to editor commands"
            );
        }
        return library::run(library::Cli::from_arg_matches(&matches)?);
    }
    if operation == "schema" {
        return Ok(if domain == "images" {
            images::protocol()
        } else {
            canvas::protocol()
        });
    }
    if operation == "benchmark" {
        return if domain == "images" {
            images::execute_standalone(args).context("unknown image command")?
        } else {
            canvas::execute_standalone(args)
        };
    }
    let id = asset.context("choose the asset to edit with --asset UUID")?;
    let root = matches
        .get_one::<PathBuf>("library")
        .cloned()
        .map(Ok)
        .unwrap_or_else(default_root)?;
    let mut service = Service::agent(&root)?;
    if domain == "images" {
        images::execute_target(args, &mut service, id)
    } else {
        let (path, session) = service.editor_target(id, AssetKind::Canvas)?;
        let mut args = args.clone();
        let output = canvas::execute_target(&mut args, path, session)?;
        service.index_canvas(id)?;
        Ok(output)
    }
}
