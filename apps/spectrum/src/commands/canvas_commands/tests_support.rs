use super::*;

pub(super) fn tree_snapshot(root: &Path) -> Vec<(PathBuf, bool, Vec<u8>)> {
    fn visit(root: &Path, directory: &Path, snapshot: &mut Vec<(PathBuf, bool, Vec<u8>)>) {
        let mut entries = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            let relative = path.strip_prefix(root).unwrap().to_owned();
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                snapshot.push((relative, true, Vec::new()));
                visit(root, &path, snapshot);
            } else {
                snapshot.push((relative, false, std::fs::read(path).unwrap()));
            }
        }
    }

    let mut snapshot = Vec::new();
    visit(root, root, &mut snapshot);
    snapshot
}

pub(super) fn initialize_rectangle_project(project: &Path) {
    run(Cli {
        project: project.to_owned(),
        session: None,
        live: None,
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
        live: None,
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
