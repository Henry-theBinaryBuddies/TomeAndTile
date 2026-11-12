# Weekly check-in reports

One file per check-in, named by its due date: `2026-11-15.md`, `2026-11-22.md`, `2026-11-29.md`, `2026-12-06.md`, `2026-12-13.md`. Check-ins are due Sundays at midnight.

Each report is compiled by the project owner's Claude from three sources, for the week ending that Sunday:

1. Pull requests merged (`git log --merges` or the GitHub list), grouped by person.
2. Each person's entries in `docs/devlog/`.
3. ADRs added or changed.

## Report format

```markdown
# Check-in 2026-11-15

## Milestone status
Which milestone was due, what landed, what did not, and why.

## By person
### @handle
- Merged: PR links with one line each
- From the dev log: what they did and learned
- Blocked on:

## Decisions this week
New or changed ADRs.

## Next week
The plan, per person.
```

Report what did not get done as plainly as what did.
