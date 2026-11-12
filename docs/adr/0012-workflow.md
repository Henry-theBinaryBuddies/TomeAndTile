# 0012: Agentic workflow, review and records

Status: Accepted (2026-11-12)

## Context

This is a project for an Agentic Development class with labor-based grading. Each of three contributors directs their own Claude instance. The workflow is as much the deliverable as the code.

## Decision

### Tasks
- Work is handed out as GitHub issues written by the project owner from the task template: goal, files in scope, contract messages involved, acceptance tests, out of scope.
- A contributor points their Claude at one issue. Claude works on a branch named `<area>/<issue-number>-<short-name>` and opens one PR.

### Review, in order
1. CI: build, tests, `tools/check-deps.sh`, and `buf breaking` for contracts.
2. The author runs a Claude code review on their own PR and fixes the findings.
3. The code owner approves. For `integrations/` and `web/` PRs, the project owner also reviews until the boundaries have proved themselves.

### Branching
- `main` is protected: no direct pushes, PR and owner approval required, CI must pass.
- Branches are short-lived. Merge small and often.

### Records
- **ADRs** in `docs/adr/` for decisions.
- **A dev log per person** in `docs/devlog/`. Claude appends a dated entry at the end of every working session: what was done, what was learned, what is blocked.
- **A weekly check-in report** in `docs/checkins/`, compiled each Sunday from the week's merged PRs, dev log entries and new ADRs.

### Check-ins
- Weekly, due Sundays at midnight: 2026-11-15, 11-22, 11-29, 12-06, and the final on 12-13.

## Why

- Issues with explicit file scope are how the project owner controls what each Claude touches.
- Dev logs are direct evidence of each person's labor and give the next Claude session its context.

## Rejected

- **Task files in the repository:** merge conflicts, no discussion thread.
- **Contributors defining their own tasks:** drift from the architecture.
- **Peer review among beginners as the only gate:** little protection.
