# Dev log: @aalemany00

Newest entry first. Format in [README.md](README.md).

## 2026-11-12 — #7 Machine setup and onboarding

**Did:** Accepted the GitHub invite and cloned the repo. Installed the Windows toolchain from `docs/SETUP-WINDOWS.md`: Rustup/cargo, `buf` via npm, and the MSVC C++ build tools. `cargo test --workspace` and `bash tools/check-deps.sh` both pass on this machine.
**Directed Claude to:** Read `docs/ONBOARDING.md`, `ARCHITECTURE.md`, the root `CLAUDE.md`, my directory's `CLAUDE.md` (`integrations/`, `web/media/`), and ADRs 0002, 0007, 0010, 0012 and 0013 before touching anything, then set up the machine and record this entry.
**Learned:** The MSVC Build Tools winget package does not pull in the Windows SDK by default even with `--includeRecommended`; `cargo test --workspace` failed with `LINK : fatal error LNK1181: cannot open input file 'kernel32.lib'` until the SDK (`Microsoft.VisualStudio.Component.Windows11SDK.22621`) was added as a separate modify step. My area (`integrations/`, `web/media/`) can only depend on `contracts/` — never on `crates/server`, `crates/rules`, `crates/renderer`, or `crates/plugin-host` — enforced by `tools/check-deps.sh`. The `tt-cauldron-import` crate already exists as an empty skeleton; the importer itself is due 2026-12-06 (M3).
**Blocked on / next:** Watching the ONBOARDING.md videos (section 1, then the map-upload video in section 5) is still outstanding on my end. The field map for issue #8 is already up in a separate PR using the sample files Henry pushed to `integrations/cauldron-samples`.
