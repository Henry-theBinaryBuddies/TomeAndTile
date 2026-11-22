#!/usr/bin/env bash
# Start tt-server with the variables from the repository's `.env` (see `.env.example`).
# Blank lines, comments and empty values are skipped; values are never printed.
# Extra arguments are passed to `cargo run`, e.g. `bash tools/run-server.sh --release`.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
env_file="$root/.env"
if [ -f "$env_file" ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    line="${line%$'\r'}"
    trimmed="${line#"${line%%[![:space:]]*}"}"
    case "$trimmed" in ''|'#'*) continue ;; esac
    case "$trimmed" in *=*) ;; *) continue ;; esac
    name="${trimmed%%=*}"
    value="${trimmed#*=}"
    name="${name%"${name##*[![:space:]]}"}"
    value="${value#"${value%%[![:space:]]*}"}"
    value="${value%"${value##*[![:space:]]}"}"
    [ -z "$name" ] || [ -z "$value" ] && continue
    export "$name=$value"
    echo "run-server: $name set from .env"
  done < "$env_file"
else
  echo "run-server: no .env found; using the current environment"
fi
exec cargo run --manifest-path "$root/Cargo.toml" -p tt-server "$@"
