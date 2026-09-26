mod canvas;
mod images;
mod library;

use anyhow::{Context, Result, bail};
use clap::{Arg, Command, CommandFactory, FromArgMatches};
use spectrum::library::{Service, default_root};
use spectrum_library::AssetId;
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
            for argument in base.get_arguments() {
                if !matches!(argument.get_id().as_str(), "help" | "version") {
                    let mut argument = argument.clone();
                    if matches!(argument.get_id().as_str(), "catalog" | "project") {
                        argument = argument.default_value(None::<&str>).required(false);
                    }
                    command = command.arg(argument);
                }
            }
            command = command.arg(Arg::new("target_asset").long("asset").global(true)
                .value_parser(clap::value_parser!(AssetId))
                .help("Select a library asset for editor commands; image item IDs remain document-local"));
            for subcommand in base.get_subcommands() {
                let name = subcommand.get_name();
                if name == "list" {
                    command = command.subcommand(subcommand.clone().name("inspect"));
                } else if !matches!(name, "from-lumen" | "import" | "export")
                    && !command.get_subcommands().any(|c| c.get_name() == name) {
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

fn dispatch(matches: clap::ArgMatches) -> Result<serde_json::Value> {
    let (domain, args) = matches.subcommand().context("missing command")?;
    if domain == "schema" {
        return Ok(serde_json::json!({
            "images": images::protocol(), "canvas": canvas::protocol(),
            "library": {"asset_types": ["image", "canvas"], "references": "live",
                "copy": "independent, recursively copies referenced content"},
            "help": "spectrum images --help; spectrum canvas --help",
            "targeting": "Library commands use asset UUIDs. Editor commands use --asset UUID or --document PATH; image item and canvas layer IDs are local to that document."
        }));
    }
    if matches!(domain, "images" | "canvas") {
        let (operation, _) = args.subcommand().context("missing editor command")?;
        if operation == "schema" {
            return Ok(if domain == "images" {
                images::protocol()
            } else {
                canvas::protocol()
            });
        }
        let managed = if domain == "images" {
            ["list", "import", "adjust", "command", "export"].contains(&operation)
        } else {
            ["list", "new", "place", "command", "export"].contains(&operation)
        };
        if !managed {
            let mut args = args.clone();
            let field = if domain == "images" {
                "catalog"
            } else {
                "project"
            };
            let explicit_asset = args.get_one::<AssetId>("target_asset").copied();
            if explicit_asset.is_some()
                && args.value_source(field) == Some(clap::parser::ValueSource::CommandLine)
            {
                bail!("choose --asset or --document, not both");
            }
            let path = if let Some(id) = explicit_asset {
                let root = matches
                    .get_one::<PathBuf>("library")
                    .cloned()
                    .map(Ok)
                    .unwrap_or_else(default_root)?;
                let service = Service::open(&root)?;
                let asset = service.library.get(id)?;
                let expected = if domain == "images" {
                    "image"
                } else {
                    "canvas"
                };
                if asset.kind != expected {
                    bail!("expected {expected} asset, got {}", asset.kind);
                }
                service.library.path(&asset)?
            } else {
                args.get_one::<PathBuf>(field).cloned().unwrap_or_default()
            };
            if path.as_os_str().is_empty() && !matches!(operation, "benchmark" | "live") {
                bail!(
                    "select a target with --asset UUID or --document PATH (the embedded terminal supplies its current document)"
                );
            }
            // Reparse against the engine's own command tree, retaining clap's typed values.
            // ArgMatches cannot rename a subcommand, so the adapter handles inspect explicitly.
            if domain == "images" {
                images::execute_target(&mut args, path)
            } else {
                canvas::execute_target(&mut args, path)
            }
        } else {
            if args.get_one::<AssetId>("target_asset").is_some()
                || args.value_source(if domain == "images" {
                    "catalog"
                } else {
                    "project"
                }) == Some(clap::parser::ValueSource::CommandLine)
                || args.value_source("live") == Some(clap::parser::ValueSource::CommandLine)
                || args.value_source("session") == Some(clap::parser::ValueSource::CommandLine)
            {
                bail!(
                    "library commands take asset UUIDs as positional arguments; --asset, --document, --session, and --live belong to editor commands"
                );
            }
            library::run(library::Cli::from_arg_matches(&matches)?)
        }
    } else {
        library::run(library::Cli::from_arg_matches(&matches)?)
    }
}
