$ErrorActionPreference = "Stop"
cargo build --release --bins --locked
$destination = "target/dist/Lumen-Windows"
Remove-Item $destination -Recurse -Force -ErrorAction SilentlyContinue
New-Item $destination -ItemType Directory -Force | Out-Null
Copy-Item target/release/lumen-gui.exe,target/release/lumen.exe,THIRD_PARTY.md $destination
Write-Host "Created $destination"
