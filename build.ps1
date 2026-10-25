# Tests and builds the app and the installer, then copies the installer to dist\.
# Usage: powershell -ExecutionPolicy Bypass -File build.ps1
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

function Run($cmd) {
    Write-Host "> $cmd"
    Invoke-Expression $cmd
    if ($LASTEXITCODE -ne 0) { throw "failed: $cmd" }
}

$running = Get-Process unshufflz -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$PSScriptRoot\target\*" }
if ($running) { throw "Unshufflz is running from target\ - exit it (tray icon > Exit) before building." }

Run 'cargo test --release'
Run 'cargo build --release'
Run 'cargo build --release -p unshufflz-setup'
Run 'cargo test --release -p unshufflz-setup'

$ver = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
New-Item -ItemType Directory -Force dist | Out-Null
$out = "dist\UnshufflzSetup-$ver.exe"
Copy-Item target\release\unshufflz-setup.exe $out -Force
$hash = (Get-FileHash $out -Algorithm SHA256).Hash.ToLower()
Set-Content "$out.sha256" "$hash *$(Split-Path $out -Leaf)" -Encoding ascii -NoNewline
Write-Host "Installer: $out"
Write-Host "SHA-256:   $hash"
