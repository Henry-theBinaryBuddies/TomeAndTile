#!/usr/bin/env bash
# Enforces the dependency rule from ARCHITECTURE.md:
#   integrations/ and web/ may depend on contracts and nothing else in this
#   repository; rules and renderer must not depend on the server.
# Exits non-zero and lists every violation.
set -euo pipefail

cd "$(dirname "$0")/.."
fail=0

report() {
  echo "BOUNDARY VIOLATION: $1"
  fail=1
}

# Crates that code outside crates/ must never use.
internal='tt-server|tt-rules|tt-renderer|tt-plugin-host'

# 1. Cargo manifests under integrations/ must not name an internal crate or
#    point a path dependency into crates/.
while IFS= read -r manifest; do
  if grep -nE "^\s*(${internal})\s*=" "$manifest" >/dev/null; then
    report "$manifest depends on an internal crate"
  fi
  if grep -nE 'path\s*=\s*"[^"]*crates/' "$manifest" >/dev/null; then
    report "$manifest has a path dependency into crates/"
  fi
done < <(find integrations -name Cargo.toml 2>/dev/null)

# 2. Rust source under integrations/ must not import an internal crate.
if grep -rnE "\b(tt_server|tt_rules|tt_renderer|tt_plugin_host)::" \
    --include='*.rs' integrations 2>/dev/null; then
  report "integrations/ imports an internal crate (lines above)"
fi

# 3. web/ must not import from crates/ or integrations/ by relative path.
if [ -d web ]; then
  if grep -rnE "from\s+['\"][^'\"]*(crates|integrations)/" \
      --include='*.ts' --include='*.svelte' --include='*.js' \
      --exclude-dir=node_modules web 2>/dev/null; then
    report "web/ imports from crates/ or integrations/ (lines above)"
  fi
  # 4. Panel code must reach web/media only through its index.
  if grep -rnE "from\s+['\"][^'\"]*media/[^'\"]+/[^'\"]+['\"]" \
      --include='*.ts' --include='*.svelte' --include='*.js' \
      --exclude-dir=node_modules --exclude-dir=media web 2>/dev/null; then
    report "web/ reaches into web/media internals (lines above)"
  fi
fi

# 5. rules and renderer must not depend on the server.
for crate in rules renderer plugin-host; do
  manifest="crates/${crate}/Cargo.toml"
  if [ -f "$manifest" ] && grep -nE '^\s*tt-server\s*=' "$manifest" >/dev/null; then
    report "$manifest depends on tt-server"
  fi
done

if [ "$fail" -ne 0 ]; then
  echo
  echo "See ARCHITECTURE.md, 'The dependency rule'."
  exit 1
fi
echo "Dependency boundaries OK."
