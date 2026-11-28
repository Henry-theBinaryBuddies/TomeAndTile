# 0016: Per-token resources and recharge

Status: Accepted (2026-11-27)

## Context

Each token is its own creature at the table. #95 already moved hit points and conditions onto the token, because several tokens can share one `Creature` (three goblins, one stat block). Two kinds of state were still on the shared `Creature`:

- `Ability.recharge_spent`: when one goblin used a recharge ability, it was marked spent for all three goblins.
- `Creature.resources[].current`: spell slots, rage and other limited uses were one pool for every token of the sheet.

The recharge roll made it worse. `end_turn` rolled a d6 at the start of each token's turn against the shared creature, so with three goblins the shared ability got three recharge rolls per round and came back about three times as fast.

Player characters are not affected in practice: one token per sheet.

## Decision

### 1. The Creature is the template, the Token holds the live state

- `Creature.resources[].max` and each `Ability.recharge` rule stay as the sheet's definition.
- `Creature.resources[].current` and `Ability.recharge_spent` are the template (initial) values. They are not changed in play. Only a re-import of the sheet changes the creature.
- `Token` gets two fields (additive, `buf breaking` passes):
  - `repeated TokenResource resources = 20;` the token's live pools: key, label, `current`, `max`, in the creature's order. `TokenResource` mirrors `Resource` because `creature.proto` imports `scene.proto`, so `scene.proto` cannot import `Resource`.
  - `repeated string recharge_spent_ability_ids = 21;` the abilities of this token's creature that are spent and waiting to recharge.
- The server fills both (`fill_live_state`): on `CreateTokenIntent`, when a campaign is loaded from a seed or snapshot (every token with a creature), and on a re-import of the sheet (every linked token).

### 2. Rules

- **Using an ability** spends the source token's resources and marks the ability spent on the source token only. The server gives the rules engine a clone of the creature with that token's pools and spent flags written in, then writes what the engine returns for the source back to the token. The stored creature is not changed, and no `CreatureUpserted` is sent for a use. `resolve_ability` keeps its API.
- **`end_turn`** rolls a d6 only for the token whose turn starts, for each id in that token's `recharge_spent_ability_ids`. A success removes the id from that token's list. Roll visibility is unchanged (including the `GM_ONLY` rules of #133).
- **Undo** restores the token's state: the group records the source token's earlier value, and the dependency rule compares token ids. Two tokens of one creature do not block each other's undo.
- **Visibility:** a token's `resources` and `recharge_spent_ability_ids` follow the same rule as its hit points: the GM and the token's controllers, empty for everyone else (`redact_token`).
- **Re-import of a sheet** reconciles each linked token with the new sheet: a pool keeps its `current`, clamped to the new `max`; a key the sheet has gained starts at the sheet's `current`; a key the sheet no longer has is dropped; a spent id whose ability no longer exists or no longer recharges is dropped. Tokens whose state changed are sent as `TokenUpserted` in the same group as the `CreatureUpserted`. The group stays not undoable.

### 3. Two rules decided with this change (project owner, 2026-11-27)

- **Rounding:** damage halving and resistance keep rounding down per damage part, as #98 specifies.
- **Timing:** "until the start or end of the source's next turn" keeps its current rule (round R + 1), including when the effect is applied as a reaction.

## Why

- A token is the thing that acts, so it is the thing that gets tired. This is the same reasoning as hit points in #95.
- Keeping the creature as the template means a re-import has something to reconcile against and a new token always starts fresh from the sheet.
- Keeping `resolve_ability` pure and unchanged limits the change to the server's glue: the rules crate neither knows nor cares whether the creature it is handed is a template or a per-token copy.

## Consequences

- The sheet UI must show the token's pools and spent abilities (`Token.resources`, `Token.recharge_spent_ability_ids`), not `Creature.resources[].current` and `Ability.recharge_spent`. That is a `web/` follow-up.
- A client that only reads the creature will see stale initial values.
- A seed does not need to list live state on its tokens: the server fills it at load.
- The mock server mirrors per-token spending but has no recharge rolls and no re-import, and says so in its README.
- Existing tests that read the creature's pools or `recharge_spent` after an ability use now read the token.

## Rejected

- **A map of token id to state on the `Creature`:** it would put every token's secrets in a message that goes to a different set of viewers.
- **Making the rules engine take a token's state directly:** a wider API change than needed; the clone-and-write-back is small and tested.
