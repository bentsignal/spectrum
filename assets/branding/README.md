# Spectrum app icon

`Spectrum.icon` is the native macOS icon source, authored with Apple Icon
Composer on its shared square enclosure so macOS owns the mask, material, and
appearance rendering. `spectrum-mark.png` is the approved color artwork, mapped
to the 870-point safe area; `spectrum-mark-mono.png` is a luminance mask used
only for the Clear and Tinted styles. Do not premask artwork before adding it,
or macOS insets the icon twice.

`scripts/package-macos-icon.sh` compiles the package with `actool`, installing
both `Assets.car` and an `.icns` fallback. `scripts/stamp-macos-bundle.sh`
stamps the bundle's build number and Git revision into `Info.plist`.
