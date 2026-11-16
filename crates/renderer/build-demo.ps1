# Build the renderer to WASM and generate the JS glue into demo\pkg\.
# Usage: crates\renderer\build-demo.ps1 [-Dev]   (default: release)
# Needs the wasm32 target and wasm-bindgen-cli at EXACTLY the version of the
# wasm-bindgen crate in Cargo.lock:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version <that version> --locked
param([switch]$Dev)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
Set-Location $root
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

$profileDir = 'release'; $flag = @('--release')
if ($Dev) { $profileDir = 'debug'; $flag = @() }

$lock = Get-Content Cargo.lock
$i = [array]::IndexOf($lock, 'name = "wasm-bindgen"')
$want = ($lock[$i + 1] -replace '[^0-9.]', '')
$have = ''
try { $have = ((wasm-bindgen --version) -split ' ')[1] } catch {}
if ($want -ne $have) {
    Write-Error "wasm-bindgen CLI is '$have' but Cargo.lock wants $want. Run: cargo install wasm-bindgen-cli --version $want --locked"
}

cargo build -p tt-renderer --target wasm32-unknown-unknown @flag
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
wasm-bindgen --target web --no-typescript --out-dir crates/renderer/demo/pkg "target/wasm32-unknown-unknown/$profileDir/tt_renderer.wasm"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Get-ChildItem crates\renderer\demo\pkg | Format-Table Name, Length
