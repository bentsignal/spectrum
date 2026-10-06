use super::*;

#[path = "effect_cli_tests.rs"]
mod effect_cli_tests;
#[path = "gradient_tests.rs"]
mod gradient_tests;
#[path = "lasso_tests.rs"]
mod lasso_tests;
#[path = "paint_tests.rs"]
mod paint_tests;
#[path = "schema_tests.rs"]
mod schema_tests;
#[path = "tests.rs"]
mod tests;

/// Parses canvas editor arguments, where `--document <path>` names the
/// test's canvas document in place of a library asset.
fn parse_cli<'a>(arguments: impl IntoIterator<Item = &'a str>) -> Result<Cli, clap::Error> {
    let mut arguments: Vec<&str> = arguments.into_iter().collect();
    let project = match arguments
        .iter()
        .position(|argument| *argument == "--document")
    {
        Some(index) => {
            let project = arguments.get(index + 1).copied().unwrap_or_default();
            arguments.drain(index..(index + 2).min(arguments.len()));
            PathBuf::from(project)
        }
        None => PathBuf::new(),
    };
    let mut cli = Cli::try_parse_from(arguments)?;
    cli.project = project;
    Ok(cli)
}
