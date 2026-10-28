# Builds "dist\Handy Shortcut.exe". Needs Rust (rustup) and the Visual Studio
# C++ Build Tools. Pass -Test to run the unit tests first.
param([switch]$Test)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $cargo) { $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path" }

if ($Test) { cargo test --release; if ($LASTEXITCODE) { exit $LASTEXITCODE } }
cargo build --release
if ($LASTEXITCODE) { exit $LASTEXITCODE }

New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item target\release\handy-shortcut.exe "dist\Handy Shortcut.exe" -Force
$size = [math]::Round((Get-Item "dist\Handy Shortcut.exe").Length / 1MB, 1)
Write-Host "Built dist\Handy Shortcut.exe ($size MB)"
