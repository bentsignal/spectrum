$ErrorActionPreference = "Stop"
cargo build --release --locked -p lumen-photo --bins
$destination = "target/dist/Lumen-Windows"
Remove-Item $destination -Recurse -Force -ErrorAction SilentlyContinue
New-Item $destination -ItemType Directory -Force | Out-Null
Copy-Item target/release/lumen-gui.exe,target/release/lumen.exe,THIRD_PARTY.md $destination
Copy-Item assets/branding/lumen-violet-final-clean.png (Join-Path $destination "Lumen.png")
Write-Host "Created $destination"
