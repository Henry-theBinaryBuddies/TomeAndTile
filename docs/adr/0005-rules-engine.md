# 0005: 5e rules engine and automation

Status: Accepted (2026-11-12)

## Decision

- **System:** D&D 5e, 2014 and 2024 rules, data-driven. Other systems are not a goal, but rules live in data so the core is not hard-wired.
- **Ability model:** each ability is declarative data: target type, range, attack roll or saving throw (ability and DC), damage dice and type, half-on-save, conditions applied and duration. The engine runs those steps. What the data cannot express becomes a WASM plugin or a manual chat card.
- **Resolution:** a pausable state machine: `Targeting -> AttackOrSave -> Damage -> Effects -> Done`. An `AwaitingReaction` state is in the contracts now; reactions (Shield, Counterspell, opportunity attacks) are built later.
- **Automation level:** fully automatic. Click the ability, click the target; the server rolls the attack or the target's save, applies resistances and deducts damage. The GM can undo or edit any result.
- **Enforcement:** track and warn, never block. Turn order, action/bonus action/reaction use, spell slots, movement and range are tracked. A violation attaches a warning; the action still goes through.
- **Initiative:** the server rolls for every combatant when the GM starts combat. The GM can edit any result and reorder, which covers abilities that roll twice or grant a second turn.
- **0 HP and death:** not automated. The engine marks a creature as down at 0 HP and stops there. Death saves are rolled by hand by the player; the GM decides the fate of monsters.
- **Control handover:** the GM can assign control of any token to a player (summons, familiars, companions).
- **Designed in now, built later:** conditions and timed effects, reactions and interrupts.
- **Purity:** `crates/rules` has no I/O, no clock and no network. Randomness is an injected roller, so tests are deterministic.

## Why

- Declarative data lets importers produce abilities from Creature Cauldron and Adventurer's Anvil without generating code.
- Warn-not-block keeps tables with house rules playable and keeps an incomplete engine from locking a game.
- A pure engine is the only practical way to test hundreds of rule interactions.

## Rejected

- **A script per ability:** every ability needs code and importers would have to emit scripts.
- **Hard-coded Rust per ability:** does not scale to imported homebrew.
- **GM confirms every damage application:** slower at the table; undo covers the edge cases.
- **Hard enforcement:** frustrating wherever the engine is incomplete.
- **Automated death saves:** rejected by the project owner.
