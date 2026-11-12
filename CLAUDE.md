# Working agreement for every Claude instance in this repository

Three people work here, each with their own Claude. These rules keep one person's work from breaking another's. The `CLAUDE.md` in your own directory adds to these rules; it never loosens them.

## Before you change anything

1. Ask which contributor you are working for if it is not obvious, then read the `CLAUDE.md` in that person's directory.
2. Read the GitHub issue you were given. It lists the files in scope. If there is no issue, ask for one before writing code.
3. Read [ARCHITECTURE.md](ARCHITECTURE.md) and any ADR the issue links. Decisions in `docs/adr/` are settled; do not re-argue them in code. To change one, open an issue.

## Ownership

| Path | Owner |
|---|---|
| `contracts/`, `crates/`, `tools/`, `.github/`, `docs/adr/`, root files | @Henry-theBinaryBuddies |
| `integrations/`, `web/media/` | @aalemany00 |
| `web/` except `web/media/` | @Danny-Defeato |
| `docs/devlog/<name>.md` | that person |

- Edit only files your contributor owns.
- **If the task needs a change outside your area, stop.** Do not make the change. Write up what you need and why as a GitHub issue for the owner, tell your contributor, and carry on with whatever part of the task does not depend on it.
- A change to `contracts/` is always a separate PR, approved by @Henry-theBinaryBuddies, merged before the code that uses it.

## The dependency rule

Code in `integrations/` and `web/` may depend on `contracts/` and nothing else in this repository. Never import from `crates/server`, `crates/rules` or `crates/plugin-host`. `web/` may call the renderer only through its published WASM interface. CI runs `tools/check-deps.sh` and fails the PR otherwise.

## Git

- Never commit to `main`. Work on a branch named `<area>/<issue-number>-<short-name>`, for example `web/12-chat-panel`.
- One issue per branch, one branch per PR. Keep PRs small.
- Never force-push, never rewrite shared history, never use `--no-verify`.
- Push only your own branch, and only when your contributor tells you to.

## Before opening a PR

1. Run the checks for your area (listed in your directory's `CLAUDE.md`).
2. Run a code review on your own diff and fix what it finds.
3. Fill in the PR template honestly, including what is not done.
4. Append an entry to `docs/devlog/<name>.md` (format at the top of that file).

## Dependencies

Ask your contributor before adding, removing or upgrading any dependency. State what it is for and what it replaces.

## Secrets

Never put a key, token or password in the repository, an issue, a PR or a chat message. Local secrets go in `.env`, which is git-ignored.

## Style

- Match the surrounding code. No unrelated refactors.
- Tests go with the code they test, in the same PR.
- Player-facing game content must come from the SRD 5.1/5.2 (CC-BY-4.0) or be imported by the user. Do not copy text from any other rulebook.
