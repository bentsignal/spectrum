$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

Push-Location $repoRoot
try {
    cargo build --release --locked -p mica --bins

    $destination = Join-Path $repoRoot "target/dist/Mica-Windows"
    Remove-Item -LiteralPath $destination -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -Path $destination -ItemType Directory -Force | Out-Null

    Copy-Item -LiteralPath (Join-Path $repoRoot "target/release/mica.exe") -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot "target/release/mica-gui.exe") -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot "THIRD_PARTY.md") -Destination $destination
    $manifest = Join-Path $repoRoot "packaging/mica/windows/mica.manifest"
    Copy-Item -LiteralPath $manifest -Destination (Join-Path $destination "mica.exe.manifest")
    Copy-Item -LiteralPath $manifest -Destination (Join-Path $destination "mica-gui.exe.manifest")

    Write-Host "Created $destination"
}
finally {
    Pop-Location
}
