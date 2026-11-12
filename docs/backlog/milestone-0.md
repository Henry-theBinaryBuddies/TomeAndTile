# Milestone 0 backlog (due 2026-11-15)

Drafts of the first GitHub issues. Once the remote exists, file each as an issue from the Task template and delete it from this file.

## Done in the initial commit

- Repository skeleton, `ARCHITECTURE.md`, ADRs 0001 to 0013
- Contracts v0 (written, **not yet linted or compiled**)
- CODEOWNERS, per-directory `CLAUDE.md`, issue and PR templates
- CI workflow and `tools/check-deps.sh`
- Onboarding and Windows setup documents

## @Henry-theBinaryBuddies

### [repo] Create the remote and protect `main`
- Create `Henry-theBinaryBuddies/TomeAndTile` on GitHub and push `main`.
- Add `LICENSE` using GitHub's "Add file > Create new file > LICENSE" picker, choosing GNU AGPL v3.0.
- Invite @aalemany00 and @Danny-Defeato with Write access.
- Branch protection on `main`: require a pull request, require review from Code Owners, require the three CI jobs, block force pushes.
- Acceptance: a direct push to `main` is refused; a PR touching `web/` requests @Danny-Defeato automatically.

### [contracts] Lint and fix contracts v0
- Install Buf, run `buf lint contracts`, fix what it reports.
- Acceptance: `buf lint contracts` exits 0 locally and in CI.

### [contracts] Code generation for Rust and TypeScript
- Add a `tt-contracts` crate generating Rust types (prost), and a TypeScript package generated with Buf for `web/`.
- Needs dependency approval: `prost`, `prost-build` or `buf` remote plugins, `@bufbuild/protobuf`.
- Acceptance: `crates/` and `integrations/` can `use tt_contracts::v1::Creature`; `web/` can import the same type.

### [server] Mock server
- A small binary that accepts a socket, answers `Hello` with a recorded `Snapshot`, and replays a recorded list of `Event`s; echoes plausible events for `MoveTokenIntent`, `ChatIntent` and `RollDiceIntent`.
- Recorded fixtures live in `contracts/fixtures/` as Protobuf JSON.
- Acceptance: `web/` can connect and render a scene with two tokens with no real server.

### [adr] Confirm or change ADR 0007 (video and voice)
- Decide between mesh-first with Cloudflare TURN (proposed) and an SFU from the start.

### [content] Pregenerated SRD characters
- Four or five level-3 characters as `Creature` JSON in `contracts/fixtures/characters/`, SRD 5.2 content only, with attribution.

## @aalemany00

### [integrations] Machine setup and onboarding
- Follow `docs/SETUP-WINDOWS.md`; watch sections 1, 2 and the map video in `docs/ONBOARDING.md`.
- Acceptance: `cargo test --workspace` and `bash tools/check-deps.sh` pass on your machine; first dev log entry written.

### [integrations] Creature Cauldron sample and field map
- Files in scope: `integrations/cauldron-import/**`.
- Get two or three real export files from @Henry-theBinaryBuddies into `tests/samples/`.
- Write `integrations/cauldron-import/MAPPING.md`: each field in the export, the `Creature` field it maps to, and every field with no home.
- Out of scope: writing the importer (blocked on code generation); editing Creature Cauldron.
- Acceptance: the mapping covers every field in the samples; gaps are filed as Contract change requests.

## @Danny-Defeato

### [web] Machine setup and onboarding
- Follow `docs/SETUP-WINDOWS.md`; watch sections 1, 3 and 4 in `docs/ONBOARDING.md`.
- Acceptance: as above; first dev log entry written.

### [web] UI shell with dockable panels
- Files in scope: `web/**` except `web/media/**` and `web/CLAUDE.md`.
- Scaffold the app (SvelteKit and TypeScript unless you propose otherwise in the issue first). Needs dependency approval.
- A full-window placeholder where the map canvas will go; empty panels for Chat, Sheet, Initiative, Video and Tools that dock, collapse and float; bottom tabs at phone width.
- Out of scope: any real data, the socket, the canvas itself.
- Acceptance: `npm run check`, `npm test` and `npm run build` pass; usable at 360 px wide with touch and with a keyboard.
