$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

Push-Location $repoRoot
try {
    cargo build --release --locked -p spectrum -p lumen-photo -p prism --bins

    $destination = Join-Path $repoRoot "target/dist/Spectrum-Windows"
    Remove-Item -LiteralPath $destination -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -Path $destination -ItemType Directory -Force | Out-Null

    Copy-Item -LiteralPath (Join-Path $repoRoot "target/release/spectrum-gui.exe") -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot "target/release/spectrum.exe") -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot "target/release/lumen.exe") -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot "target/release/prism.exe") -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot "THIRD_PARTY.md") -Destination $destination
    Copy-Item -LiteralPath `
        (Join-Path $repoRoot "packaging/prism/licenses/UBUNTU-FONT-LICENCE-1.0.txt") `
        -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot "assets/branding/prism-app-icon.png") `
        -Destination (Join-Path $destination "Spectrum.png")
    $manifest = Join-Path $repoRoot "packaging/spectrum/windows/spectrum.manifest"
    foreach ($binary in "spectrum-gui.exe", "spectrum.exe", "lumen.exe", "prism.exe") {
        Copy-Item -LiteralPath $manifest -Destination (Join-Path $destination "$binary.manifest")
    }

    Write-Host "Created $destination"
}
finally {
    Pop-Location
}
