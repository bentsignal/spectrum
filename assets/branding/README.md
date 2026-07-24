# Spectrum app icons

The approved source artwork is:

- `lumen-violet-final-clean.png` for Lumen (1024 × 1024).
- `cropped-prism.png` for Prism (400 × 400).

The `.icon` packages are the native macOS production sources. They were authored
with Apple Icon Composer and use its shared square platform enclosure so macOS
owns the mask, safe-area inset, material, and appearance rendering.

- `Lumen.icon` places the approved 1024-pixel artwork full-bleed at 100% with no
  translation.
- `Prism.icon` maps the approved 400-pixel crop to the 1024-point design canvas
  at 256% with no translation. The scale is applied to each artwork layer only;
  applying it to both the layer and its group compounds the transform and clips
  the compiled result.

Both packages retain the approved color artwork byte-for-byte. Their additional
`*-mono.png` layers are deterministic luminance masks used only for the system's
Clear and Tinted icon styles. Icon Composer opacity specializations select the
color layer for Default and Dark and the high-contrast mono layer for Tinted, so
the artwork remains identifiable instead of becoming an unmarked rounded square.

`scripts/package-macos-icon.sh` compiles each package with `actool`. It installs
both `Assets.car` for native appearance variants and a complete `.icns` fallback
for older macOS releases. Do not premask artwork before adding it to an Icon
Composer package; doing so makes macOS inset the icon twice.

The `*-app-icon.png` files remain the portable runtime/package artwork for
non-macOS surfaces.
