//! Workspace-wide architectural checks exercised by `cargo test --workspace`.

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    const MAX_RUST_LINES: usize = 1_000;

    fn rust_sources(directory: &Path, output: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(directory).expect("source directory should be readable") {
            let path = entry.expect("source entry should be readable").path();
            if path.is_dir() {
                rust_sources(&path, output);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                output.push(path);
            }
        }
    }

    #[test]
    fn workspace_rust_sources_stay_within_the_maintainability_budget() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace = manifest
            .parent()
            .and_then(Path::parent)
            .expect("guardrails crate should live under tools/");
        let mut files = Vec::new();
        for directory in ["apps", "crates", "tools"] {
            rust_sources(&workspace.join(directory), &mut files);
        }

        let oversized: Vec<_> = files
            .into_iter()
            .filter_map(|path| {
                let lines = fs::read_to_string(&path).ok()?.lines().count();
                (lines > MAX_RUST_LINES).then_some((path, lines))
            })
            .collect();

        assert!(
            oversized.is_empty(),
            "split Rust sources that exceed {MAX_RUST_LINES} lines: {oversized:#?}"
        );
    }
}
