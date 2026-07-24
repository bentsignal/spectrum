#![cfg(target_os = "macos")]

use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn temporary_directory(app: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "spectrum-{app}-icon-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&directory).expect("icon test directory should be created");
    directory
}

fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} should be a string in {value:#?}"))
}

fn number(value: &Value, key: &str) -> f64 {
    value[key]
        .as_f64()
        .unwrap_or_else(|| panic!("{key} should be numeric in {value:#?}"))
}

fn assert_compiled_icon(
    repository: &Path,
    app: &str,
    base_layer: &str,
    mono_layer: &str,
    source_pixels: u64,
    positions: &[&str],
) {
    let output = temporary_directory(app);
    let source = repository.join(format!("assets/branding/{app}.icon"));
    let destination = output.join(format!("{app}.icns"));
    let package = Command::new(repository.join("scripts/package-macos-icon.sh"))
        .arg(&source)
        .arg(&destination)
        .output()
        .expect("macOS icon compiler should launch");
    assert!(
        package.status.success(),
        "macOS icon compiler failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&package.stdout),
        String::from_utf8_lossy(&package.stderr)
    );

    let catalog = Command::new("xcrun")
        .args(["--sdk", "macosx", "assetutil", "--info"])
        .arg(output.join("Assets.car"))
        .output()
        .expect("assetutil should launch");
    assert!(
        catalog.status.success(),
        "assetutil failed:\n{}",
        String::from_utf8_lossy(&catalog.stderr)
    );
    let renditions: Vec<Value> =
        serde_json::from_slice(&catalog.stdout).expect("assetutil should emit JSON");
    let groups: Vec<_> = renditions
        .iter()
        .filter(|rendition| rendition["AssetType"] == "IconGroup")
        .collect();
    assert_eq!(
        groups.len(),
        3,
        "{app} should compile all three appearances"
    );

    let expected_opacity = BTreeMap::from([
        ("NSAppearanceNameAqua", (1.0, 0.0)),
        ("NSAppearanceNameDarkAqua", (1.0, 0.0)),
        ("ISAppearanceTintable", (0.0, 1.0)),
    ]);
    for group in groups {
        let appearance = string(group, "Appearance");
        let &(base_opacity, mono_opacity) = expected_opacity
            .get(appearance)
            .unwrap_or_else(|| panic!("unexpected {app} appearance {appearance}"));
        let layers = group["Layers"]
            .as_array()
            .expect("icon group should contain layers");
        assert_eq!(layers.len(), 2);
        let layer = |name: &str| {
            layers
                .iter()
                .find(|layer| string(layer, "Name").ends_with(name))
                .unwrap_or_else(|| panic!("{app} should contain compiled layer {name}"))
        };
        for (name, opacity) in [(base_layer, base_opacity), (mono_layer, mono_opacity)] {
            let rendition = layer(name);
            assert_eq!(number(rendition, "LayerOpacity"), opacity);
            assert_eq!(string(rendition, "LayerSize"), "1024,1024");
            assert!(
                positions.contains(&string(rendition, "LayerPosition")),
                "unexpected {app} layer position: {rendition:#?}"
            );
            assert_eq!(rendition["PixelWidth"].as_u64(), Some(source_pixels));
            assert_eq!(rendition["PixelHeight"].as_u64(), Some(source_pixels));
            assert!(
                rendition["SizeOnDisk"].as_u64().unwrap_or_default() > 10_000,
                "{app} layer {name} must compile real pixel content"
            );
            if appearance == "ISAppearanceTintable" && name == mono_layer {
                assert_eq!(
                    rendition["Opaque"], false,
                    "{app} mono artwork must retain its contrast mask"
                );
                assert!(
                    rendition.get("LayerHasLightingEffects").is_none(),
                    "{app} mono artwork must not be washed out by glass lighting"
                );
            }
        }
    }

    let stacks: Vec<_> = renditions
        .iter()
        .filter(|rendition| rendition["AssetType"] == "IconImageStack")
        .collect();
    assert_eq!(
        stacks.len(),
        3,
        "{app} should compile a stack per appearance"
    );
    assert!(
        stacks
            .iter()
            .all(|stack| stack["CanvasWidth"] == 1024 && stack["CanvasHeight"] == 1024)
    );

    fs::remove_dir_all(output).expect("icon test directory should be removable");
}

#[test]
fn native_macos_icons_compile_visible_runtime_variants_at_canvas_size() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_compiled_icon(
        &repository,
        "Lumen",
        "lumen-violet-final-clean",
        "lumen-violet-mono",
        1024,
        &["0,0"],
    );
    assert_compiled_icon(
        &repository,
        "Prism",
        "cropped-prism",
        "prism-mono",
        400,
        &["-1,-1", "0,0"],
    );
}
