//! Icons the component library does not ship (Lucide's, ISC licensed, and a
//! swap arrow drawn to match), served beside its own assets.
use gpui::{AssetSource, SharedString};
use gpui_component::Icon;
use gpui_component_assets::Assets;
use std::borrow::Cow;

const ICONS: [(&str, &[u8]); 15] = [
    (
        "spectrum/icons/pipette.svg",
        include_bytes!("../assets/icons/pipette.svg"),
    ),
    (
        "spectrum/icons/brush.svg",
        include_bytes!("../assets/icons/brush.svg"),
    ),
    (
        "spectrum/icons/eraser.svg",
        include_bytes!("../assets/icons/eraser.svg"),
    ),
    (
        "spectrum/icons/lasso.svg",
        include_bytes!("../assets/icons/lasso.svg"),
    ),
    (
        "spectrum/icons/wand.svg",
        include_bytes!("../assets/icons/wand.svg"),
    ),
    (
        "spectrum/icons/pen.svg",
        include_bytes!("../assets/icons/pen.svg"),
    ),
    (
        "spectrum/icons/swap.svg",
        include_bytes!("../assets/icons/swap.svg"),
    ),
    (
        "spectrum/icons/history.svg",
        include_bytes!("../assets/icons/history.svg"),
    ),
    (
        "spectrum/icons/play.svg",
        include_bytes!("../assets/icons/play.svg"),
    ),
    (
        "spectrum/icons/pause.svg",
        include_bytes!("../assets/icons/pause.svg"),
    ),
    (
        "spectrum/icons/volume.svg",
        include_bytes!("../assets/icons/volume.svg"),
    ),
    (
        "spectrum/icons/volume-off.svg",
        include_bytes!("../assets/icons/volume-off.svg"),
    ),
    (
        "spectrum/icons/lock.svg",
        include_bytes!("../assets/icons/lock.svg"),
    ),
    (
        "spectrum/icons/lock-open.svg",
        include_bytes!("../assets/icons/lock-open.svg"),
    ),
    (
        "spectrum/icons/scissors.svg",
        include_bytes!("../assets/icons/scissors.svg"),
    ),
];

/// The library's assets plus Spectrum's own icons.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        match ICONS.iter().find(|(name, _)| *name == path) {
            Some((_, bytes)) => Ok(Some(Cow::Borrowed(bytes))),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(
            ICONS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        Ok(paths)
    }
}

/// One of Spectrum's icons by name, such as "pipette".
pub fn icon(name: &str) -> Icon {
    Icon::empty().path(format!("spectrum/icons/{name}.svg"))
}
