# Start tt-server with the variables from the repository's `.env` (see `.env.example`).
# Blank lines, comments and empty values are skipped; values are never printed.
# Extra arguments are passed to `cargo run`, e.g. `tools/run-server.ps1 --release`.
$root = Split-Path -Parent $PSScriptRoot
$envFile = Join-Path $root '.env'
if (Test-Path $envFile) {
    foreach ($line in Get-Content $envFile) {
        $text = $line.Trim()
        if ($text -eq '' -or $text.StartsWith('#')) { continue }
        $eq = $text.IndexOf('=')
        if ($eq -lt 1) { continue }
        $name = $text.Substring(0, $eq).Trim()
        $value = $text.Substring($eq + 1).Trim()
        if ($value -eq '') { continue }
        Set-Item -Path "Env:$name" -Value $value
        Write-Host "run-server: $name set from .env"
    }
} else {
    Write-Host "run-server: no .env found; using the current environment"
}
& cargo run --manifest-path (Join-Path $root 'Cargo.toml') -p tt-server @args
exit $LASTEXITCODE
