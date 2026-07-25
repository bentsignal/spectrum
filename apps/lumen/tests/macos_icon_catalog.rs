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

fn assert_centered_alpha_bounds(path: &Path) {
    let rendered = image::open(path)
        .unwrap_or_else(|error| panic!("{} should decode: {error}", path.display()))
        .into_rgba8();
    let (width, height) = rendered.dimensions();
    let mut bounds = (width, height, 0, 0);
    for (x, y, pixel) in rendered.enumerate_pixels() {
        if pixel[3] >= 8 {
            bounds.0 = bounds.0.min(x);
            bounds.1 = bounds.1.min(y);
            bounds.2 = bounds.2.max(x);
            bounds.3 = bounds.3.max(y);
        }
    }
    let content_width = bounds.2 - bounds.0 + 1;
    let content_height = bounds.3 - bounds.1 + 1;
    let width_ratio = f64::from(content_width) / f64::from(width);
    let height_ratio = f64::from(content_height) / f64::from(height);
    assert!(
        (0.80..=0.89).contains(&width_ratio),
        "{} alpha width should match the native macOS enclosure: {bounds:?}",
        path.display()
    );
    assert!(
        (0.80..=0.89).contains(&height_ratio),
        "{} alpha height should match the native macOS enclosure: {bounds:?}",
        path.display()
    );
    assert!(
        (i64::from(bounds.0 + bounds.2) - i64::from(width - 1)).abs() <= 12,
        "{} alpha should be horizontally centered: {bounds:?}",
        path.display()
    );
    assert!(
        (i64::from(bounds.1 + bounds.3) - i64::from(height - 1)).abs() <= 20,
        "{} alpha should be vertically centered: {bounds:?}",
        path.display()
    );
}

fn assert_compiled_icon(
    repository: &Path,
    app: &str,
    base_layer: &str,
    mono_layer: &str,
    source_pixels: u64,
    layer_size: &str,
    layer_position: &str,
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
            assert_eq!(string(rendition, "LayerSize"), layer_size);
            assert_eq!(string(rendition, "LayerPosition"), layer_position);
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

    let iconset = output.join(format!("{app}.iconset"));
    let extract = Command::new("iconutil")
        .args(["--convert", "iconset", "--output"])
        .arg(&iconset)
        .arg(&destination)
        .output()
        .expect("iconutil should launch");
    assert!(
        extract.status.success(),
        "iconutil failed:\n{}",
        String::from_utf8_lossy(&extract.stderr)
    );
    for rendition in ["icon_32x32@2x.png", "icon_512x512@2x.png"] {
        assert_centered_alpha_bounds(&iconset.join(rendition));
    }

    fs::remove_dir_all(output).expect("icon test directory should be removable");
}

#[test]
fn native_macos_icons_compile_centered_safe_area_variants() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_compiled_icon(
        &repository,
        "Lumen",
        "lumen-violet-final-clean",
        "lumen-violet-mono",
        1024,
        "870,870",
        "76,76",
    );
    assert_compiled_icon(
        &repository,
        "Prism",
        "cropped-prism",
        "prism-mono",
        400,
        "870,870",
        "76,76",
    );
}

#[test]
fn macos_bundle_stamp_records_exact_source_and_rejects_unsafe_targets() {
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = temporary_directory("bundle-stamp");
    let plist = output.join("Info.plist");
    fs::copy(repository.join("packaging/macos/Info.plist"), &plist).unwrap();
    let stamp = repository.join("scripts/stamp-macos-bundle.sh");
    for _ in 0..2 {
        let result = Command::new(&stamp)
            .arg(&plist)
            .output()
            .expect("bundle stamp should launch");
        assert!(
            result.status.success(),
            "bundle stamp failed:\n{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    let plist_value = |key: &str| {
        let output = Command::new("plutil")
            .args(["-extract", key, "raw", "-o", "-"])
            .arg(&plist)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    let git_value = |arguments: &[&str]| {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(&repository)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    assert_eq!(
        plist_value("CFBundleVersion"),
        git_value(&["rev-list", "--count", "HEAD"])
    );
    assert_eq!(
        plist_value("SpectrumGitRevision"),
        git_value(&["rev-parse", "--verify", "HEAD^{commit}"])
    );
    assert!(matches!(
        plist_value("SpectrumGitDirty").as_str(),
        "true" | "false"
    ));

    let malformed = output.join("not-a-plist");
    fs::write(&malformed, "not a plist").unwrap();
    assert!(
        !Command::new(&stamp)
            .arg(&malformed)
            .status()
            .unwrap()
            .success()
    );

    #[cfg(unix)]
    {
        let linked = output.join("linked.plist");
        symlink(&plist, &linked).unwrap();
        assert!(
            !Command::new(&stamp)
                .arg(&linked)
                .status()
                .unwrap()
                .success()
        );
    }

    fs::remove_dir_all(output).unwrap();
}
