$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

Push-Location $repoRoot
try {
    cargo build --release --locked -p prism --bins

    $destination = Join-Path $repoRoot "target/dist/Prism-Windows"
    Remove-Item -LiteralPath $destination -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -Path $destination -ItemType Directory -Force | Out-Null

    Copy-Item -LiteralPath (Join-Path $repoRoot "target/release/prism.exe") -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot "target/release/prism-gui.exe") -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot "THIRD_PARTY.md") -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot "assets/branding/prism-app-icon.png") `
        -Destination (Join-Path $destination "Prism.png")
    $manifest = Join-Path $repoRoot "packaging/prism/windows/prism.manifest"
    Copy-Item -LiteralPath $manifest -Destination (Join-Path $destination "prism.exe.manifest")
    Copy-Item -LiteralPath $manifest -Destination (Join-Path $destination "prism-gui.exe.manifest")

    Write-Host "Created $destination"
}
finally {
    Pop-Location
}
