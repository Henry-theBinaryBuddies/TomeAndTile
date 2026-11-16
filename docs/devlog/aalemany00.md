# Dev log: @aalemany00

Newest entry first. Format in [README.md](README.md).

## 2026-11-15 — #33 Asset pipeline: inspect and describe uploaded images

**Did:** Added `tt-asset-pipeline`, a pure `inspect(bytes) -> Result<AssetRef, AssetError>`:
sniffs PNG/JPEG/WebP (lossy `VP8 `, lossless `VP8L`, and extended `VP8X`) from each format's own
header bytes, never decoding the full image; computes `asset_id` as the lowercase hex SHA-256 of
the file; rejects empty files, files over 20MiB, unknown formats, SVG (by content, not
extension), truncated headers, 0 or >16384 width/height, and >100,000,000 pixels. Both SHA-256
and the three header parsers are hand-written with zero dependencies, since issue #33 says to
stop and ask before adding one (`sha2`/an image-decoding crate) and the real sample files are
all well under a size where that mattered. Tests use five real PNG/JPEG/WebP samples generated
with Pillow in `tests/samples/`, plus hand-built hostile inputs (truncated headers, a header
claiming 20000x20000, all-random bytes, an SVG with an `onload`) asserting nothing panics.
**Directed Claude to:** Read issue #33, `integrations/CLAUDE.md` and ADRs 0008/0010, then build
the crate without adding a dependency or touching the root `Cargo.toml`.
**Learned:** Issue #33 says to add the crate to the root `Cargo.toml` workspace members myself,
"ask @Henry-theBinaryBuddies ... if the rules stop you" — and the rules do: root files are his
per the working agreement. Left the one-line addition for him rather than self-editing root
`Cargo.toml`, same call as the `web/media/` check/test glob gap on the #34 branch.
**Blocked on / next:** This crate isn't a workspace member yet (needs that one line from Henry),
so `cargo test -p tt-asset-pipeline` etc. can't run from the repo root until then. More importantly:
*no Rust toolchain was installed in this session at all* (no `cargo` on `PATH` anywhere I could
find), so none of `cargo fmt`, `cargo clippy` or `cargo test` actually ran — not here, not as a
standalone crate, not anywhere. Everything above is from careful manual review (including
checking the SHA-256 test vectors against Python's `hashlib` by hand) and real sample files, not
a compiler. This genuinely needs a `cargo build`, `cargo test -p tt-asset-pipeline` and
`cargo clippy -p tt-asset-pipeline --all-targets -- -D warnings` on a machine with the toolchain
before anyone trusts this compiles, let alone merges.

## 2026-11-12 — #7 Machine setup and onboarding

**Did:** Accepted the GitHub invite and cloned the repo. Installed the Windows toolchain from `docs/SETUP-WINDOWS.md`: Rustup/cargo, `buf` via npm, and the MSVC C++ build tools. `cargo test --workspace` and `bash tools/check-deps.sh` both pass on this machine.
**Directed Claude to:** Read `docs/ONBOARDING.md`, `ARCHITECTURE.md`, the root `CLAUDE.md`, my directory's `CLAUDE.md` (`integrations/`, `web/media/`), and ADRs 0002, 0007, 0010, 0012 and 0013 before touching anything, then set up the machine and record this entry.
**Learned:** The MSVC Build Tools winget package does not pull in the Windows SDK by default even with `--includeRecommended`; `cargo test --workspace` failed with `LINK : fatal error LNK1181: cannot open input file 'kernel32.lib'` until the SDK (`Microsoft.VisualStudio.Component.Windows11SDK.22621`) was added as a separate modify step. My area (`integrations/`, `web/media/`) can only depend on `contracts/` — never on `crates/server`, `crates/rules`, `crates/renderer`, or `crates/plugin-host` — enforced by `tools/check-deps.sh`. The `tt-cauldron-import` crate already exists as an empty skeleton; the importer itself is due 2026-12-06 (M3).
**Blocked on / next:** Watching the ONBOARDING.md videos (section 1, then the map-upload video in section 5) is still outstanding on my end. The field map for issue #8 is already up in a separate PR using the sample files Henry pushed to `integrations/cauldron-samples`.
