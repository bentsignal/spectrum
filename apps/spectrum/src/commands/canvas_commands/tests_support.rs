use super::*;
use std::path::Path;

pub(super) fn initialize_rectangle_project(project: &Path) {
    run(Cli {
        project: project.to_owned(),
        session: None,
        command: CliCommand::Init {
            name: "CLI test".into(),
            width: 400,
            height: 300,
            background: "18191dff".into(),
        },
    })
    .unwrap();
    run(Cli {
        project: project.to_owned(),
        session: None,
        command: CliCommand::AddRectangle {
            name: None,
            width: 100,
            height: 80,
            color: "ffffffff".into(),
            radius: 0.0,
            x: 10.0,
            y: 20.0,
        },
    })
    .unwrap();
}
