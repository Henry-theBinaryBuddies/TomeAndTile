# contracts/ — rules for Claude

Owner: @Henry-theBinaryBuddies. Only the owner's Claude edits this directory. If you are working for anyone else and need a change here, stop and file a "Contract change request" issue.

This directory is the only thing every component shares. A careless change here breaks all three people at once.

## Rules

- Files live in `proto/tomeandtile/v1/`. Package is `tomeandtile.v1`.
- **Never renumber, reuse or change the type of an existing field.** To remove one, delete it and add `reserved <number>;` and `reserved "<name>";`.
- Adding a field, a message, an enum value or a `oneof` case is allowed. Changing the meaning of an existing one is not.
- Every enum starts with `<NAME>_UNSPECIFIED = 0`.
- Every message and every non-obvious field gets a comment saying what it means and who sets it.
- Viewer-filtered fields (things a player may not see) are `optional` or documented as "unset means unknown".
- There must never be a message that lets a client supply a dice result or a state change. Clients send intents only.
- A contracts PR contains only contracts changes. Code that uses the change comes in a later PR.

## Checks

```bash
buf lint contracts
```

```bash
buf breaking contracts --against ".git#branch=main,subdir=contracts"
```

Both must pass. If `buf breaking` fails, the change is breaking: redesign it as an addition, or raise it with the owner as a `v2` question.
