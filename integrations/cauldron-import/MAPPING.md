# Creature Cauldron → `Creature` field map

Field-by-field map from a Creature Cauldron monster export to the `Creature` contract
(`contracts/proto/tomeandtile/v1/creature.proto`). Built against the five files in
[`tests/samples/`](tests/samples/) and the field list in
[`tests/samples/monster.schema.ts.txt`](tests/samples/monster.schema.ts.txt). No importer
code exists yet (blocked on the contracts code-generation issue); this is the design the
importer will follow.

Every row either names a `Creature`/`Ability`/`EffectStep` field, or says **no home**. See
"Gaps pending a Contract change" at the end for what that means for each no-home field.

## Envelope

| Export field | Maps to | Conversion / rule |
|---|---|---|
| `app` | — | Must equal `"Creature Cauldron"`. Any other value: reject the file with an error, read nothing further. |
| `kind` | — | Must equal `"monster"`. `"encounter"` (seen in the README's "not covered yet" list) is a different, unhandled shape for now — reject with an error naming it, don't try to unwrap it. |
| `format_version` | — | Must equal `1`. A higher version: reject with an error asking for an importer update, never guess at the shape. A lower version: not expected yet; same rejection. |
| `exported_at` | — | No home. Informational only; not part of game state. Safely dropped (see below). |
| `monster` | `Creature` | The envelope's payload; everything below reads from here. |

## Identity and provenance

| Export field | Maps to | Conversion / rule |
|---|---|---|
| `monster.id` | `Creature.provenance.external_id` *(pending contracts change)* | `Creature.creature_id` is minted by the server (contracts comment: "Identifiers are opaque strings minted by the server") and is not a home for Cauldron's own id. An id doesn't belong in `attribution` either — attribution is licence-credit text, not a database key. @Henry-theBinaryBuddies is adding an `external_id` field to `Provenance`; map `monster.id` there so a re-import is traceable. Until that field merges, this mapping is blocked on it rather than stashed in `attribution`. |
| `monster.schema_version` | — | No home. Internal to Creature Cauldron's own schema, not ours. Safely dropped. |
| `monster.name` | `Creature.name` | Direct copy. |
| `monster.source` | `Creature.provenance.source` | Always set `Provenance.source = "creature-cauldron"` regardless of Cauldron's own `source` value — `Provenance.source` names *our* importer, not Cauldron's internal taxonomy. Cauldron's `source` ("srd" / "a5e" / "curated" / "ai" / "reskin") instead feeds the Cauldron-metadata field(s) below. **None of the five samples has a `source` other than `"curated"`** — `srd`, `a5e`, `ai` and `reskin` need a real sample before this is more than a guess (flagged in the sample README's "Not covered yet"). |
| `monster.validated` | — | No home. Cauldron-internal QA flag, not game state. Safely dropped. |
| `monster.attribution` | `Creature.provenance.attribution` | Copied **word for word** when present, never rephrased or combined with other fields — SRD's CC-BY-4.0 requires the attribution text exactly as given. All five samples have `attribution: null`, so this is untested against a real `srd`/`reskin`/`ai` sample. |
| `monster.parent_id`, `monster.generator`, `monster.changes` | *(pending contracts change)* | Cauldron's reskin/generation metadata, distinct from the licence-attribution string above. @Henry-theBinaryBuddies is making room for these on the contract (not `attribution`); until that field exists, this mapping is blocked on it rather than folding them into `attribution`. All three are `null` in every sample we have, so untested either way. |

## Core stat block

| Export field | Maps to | Conversion / rule |
|---|---|---|
| `monster.size` | `Creature.size` | Maps to `CreatureSize` (defined in `scene.proto`). Confirmed with @Henry-theBinaryBuddies: the enum covers tiny through gargantuan, so this is a direct 1:1 mapping of Cauldron's size strings. |
| `monster.type`, `monster.subtypes` | — | No home. `Creature` has no creature-type/subtype field (only `summary`, display-only). File as a contract change if combat or targeting ever needs to key off type (e.g. "undead"); until then, fold into `summary`. |
| `monster.alignment` | — | No home. Flavor only. Fold into `summary` or drop. |
| `monster.armor_class.value` | `Creature.armor_class` | Direct copy. |
| `monster.armor_class.note` | — | No home as structured data. Fold into `summary` if kept at all; typically droppable. |
| `monster.hit_points.average` | `Creature.hp_max` (and `hp_current` at import time) | Direct copy to both — a freshly imported creature starts at full HP. |
| `monster.hit_points.dice` | — | No home. `Creature` stores only the rolled/average max, not the formula. Droppable (the average is what matters for play; nothing re-rolls max HP later). |
| `monster.speeds.walk` | `Creature.speed_feet` | Direct copy. If `speeds.walk` is absent (Voidbloom Seer has only `fly` and `hover`, no `walk`), set `speed_feet = 0` — never borrow a non-walk speed as a stand-in for walking. |
| `monster.speeds.fly` / `.swim` / `.climb` / `.burrow` / `.hover` | — | **No home.** `Creature` has one `speed_feet`, not a per-mode map. This is a real gap — four of five samples use a non-walk speed (Cinderkite and Stormhide Drake fly, Dune Leviathan burrows, Voidbloom Seer flies and hovers) — **pending contracts change**: @Henry-theBinaryBuddies is covering this in a `contracts/` PR (per-mode speeds and hover). Do not silently use the walk speed as a stand-in. |
| `monster.abilities` (`str`..`cha`) | `Creature.scores` (`AbilityScores`) | Direct 1:1 copy, field by field. |
| `monster.saves` | `Creature.save_proficiencies` | `Creature.save_proficiencies` is a list of `AbilityScore` (which abilities are proficient), not a bonus map. The *list of keys present* in `monster.saves` maps across. **No contract change**: in all five samples the printed bonus equals ability modifier plus proficiency bonus, confirming ADR 0005's data-driven design. The importer should compute the save bonus from `scores` + `proficiency_bonus`, compare it against the printed `monster.saves` bonus, and report a mismatch instead of silently dropping a bonus that doesn't match. |
| `monster.skills` | — | **No home.** No skills/skill-bonus field anywhere on `Creature`. Two of five samples use this (Cinderkite, Stormhide Drake — Perception, Stealth). **Pending contracts change** — @Henry-theBinaryBuddies is covering this, needed at minimum for passive checks the GM calls for, together with `senses.passive_perception` below. |
| `monster.vulnerabilities` | `Creature.vulnerabilities` | List of `DamageType`; Cauldron's string names map 1:1 to the `DamageType` enum values (case/shape to confirm when the importer is written). |
| `monster.resistances` | `Creature.resistances` | Same mapping as vulnerabilities. |
| `monster.immunities` | `Creature.immunities` | Same mapping as vulnerabilities. |
| `monster.damage_text` | — | No home, by design — `DamagePart`/`DamageType` are typed lists, not prose (contract favors structured data). None of the five samples has this field set, so it's untested either way; when a sample with it shows up, the wording is dropped and only the type lists in `vulnerabilities`/`resistances`/`immunities` carry forward. Safely dropped. |
| `monster.condition_immunities` | — | **No home.** `Creature.conditions` holds *active* condition ids, not immunities. **Pending contracts change** — two of five samples set this (`prone`, `blinded`). |
| `monster.senses.*` (`darkvision`, `blindsight`, `tremorsense`, `truesight`) | — | No home. `Creature` has no senses field. Low risk for the class scope (no fog/dynamic lighting yet per ARCHITECTURE.md "designed into the contracts but built later"); deferred, not part of the current contracts change. |
| `monster.senses.passive_perception` | — | No home today for the same reason as skills above; carried by the same `skills` contracts change. |
| `monster.languages` | — | No home. Flavor/roleplay only, never read by the rules engine. Droppable. |
| `monster.challenge.cr`, `.cr_value`, `.xp` | — | No home. Display/GM-reference only; `Creature` has no CR/XP field. Fold into `summary` (this is exactly what `summary` is for: "display only"). |
| `monster.challenge.proficiency_bonus` | `Creature.proficiency_bonus` | Direct copy. |

## Traits, actions, and the hybrid action model

`Creature.abilities` (`repeated Ability`) is the single list the sheet clicks on. Cauldron
splits the same idea across `traits`, `actions`, `bonus_actions`, `reactions` and
`legendary_actions.options` — all of those flatten into `Creature.abilities`, each tagged
with the right `ActionCost`.

| Export field | Maps to | Conversion / rule |
|---|---|---|
| `monster.traits[]` (always `kind: "text"`) | `Ability` with a passive cost *(pending contracts change)* | `ACTION_COST_FREE` is wrong here — traits are passive (nothing happens when you click them), not a free clickable action. @Henry-theBinaryBuddies is adding a passive `ActionCost` value; traits map to that once it merges. `name` → `Ability.name`; `text` → `Ability.description`; no `EffectStep` (traits are descriptive, not clickable effects) — `Ability.steps` stays empty. |
| `monster.actions[]` | `Ability` with `cost = ACTION_COST_ACTION` | See action-kind table below for `steps`. |
| `monster.bonus_actions[]` | `Ability` with `cost = ACTION_COST_BONUS_ACTION` | Same per-kind handling as `actions[]`. **No sample has a non-empty `bonus_actions`** — untested until one arrives (flagged in the sample README). |
| `monster.reactions[]` | `Ability` with `cost = ACTION_COST_REACTION` | Same per-kind handling. **No sample has a non-empty `reactions`** — untested, flagged in the sample README. Also note ARCHITECTURE.md: the rules engine's `AwaitingReaction` state is reserved but not necessarily built yet — confirm the engine actually accepts a reaction-cost ability before the importer assumes it's playable. |
| `monster.legendary_actions.options[]` | `Ability` with `cost = ACTION_COST_LEGENDARY` | `name` → `Ability.name`; `text` → `Ability.description`; `cost` → `Ability.resource_amount` (this is legendary-action *point* cost, not the resource system below); `resource_key = "legendary_actions"` (matches the `Resource.key` synthesized below, so the cost is actually spent from that pool — previously left unset); no structured `EffectStep` — legendary options are prose only in every sample, so `steps` stays empty (same as `text`-kind actions). |
| `monster.legendary_actions.per_round` | `Creature.resources` | One synthesized `Resource`: `key = "legendary_actions"`, `label = "Legendary Actions"`, `max = per_round`, `current = per_round`. |

### Action kind → `EffectStep`

| Cauldron `kind` | `EffectStep` variant | Conversion |
|---|---|---|
| `"attack"` | `attack` (`AttackStep`) | `attack.to_hit` → `to_hit_bonus`. `damage[]` → `damage[]` (`DamagePart`): each `{dice, bonus}` combines into one dice string: `bonus > 0` → `"<dice>+<bonus>"` (e.g. `dice:"1d4", bonus:3` → `"1d4+3"`); `bonus < 0` → `"<dice><bonus>"`, the negative sign carries its own `-` (e.g. `dice:"1d4", bonus:-1` → `"1d4-1"`); `bonus == 0` → `"<dice>"` with no `+0` suffix. `type` → `DamagePart.type`. `attack.mode`, `attack.ability`, `attack.reach`/`range`, `attack.targets` → folded into `Ability.targeting` (`Targeting.shape = TARGET_SHAPE_CREATURES`, `range_feet` from `reach` or `range.normal`, `long_range_feet` from `range.long` or `0`); `attack.mode`/`attack.ability` themselves have **no home** on `AttackStep` (the contract bakes ability + proficiency into `to_hit_bonus` already, so the raw ability/mode is redundant for play — safely dropped) once `to_hit_bonus` is copied. `effects_text` (free text, e.g. "target is grappled") has **no home** as a structured `ApplyEffectStep` — none of the attack actions in the five samples sets it, so left unhandled for now; a sample that uses it should become its own `ApplyEffectStep` step or get filed as a gap. |
| `"save"` | `save` (`SaveStep`) | `save.ability` → `SaveStep.ability` (`AbilityScore`). `save.dc` → `dc`. `on_fail[]` → `damage[]`, same dice+bonus combining as above (including the `bonus == 0` / negative-`bonus` rules). `on_success` (`"half"` / `"none"`) → `half_on_success` (`true` / `false`). `area.shape`/`size`/`width` → `Ability.targeting` (`TargetShape` from `shape`: `cone`→`TARGET_SHAPE_CONE`, `line`→`TARGET_SHAPE_LINE`, `radius`→`TARGET_SHAPE_SPHERE` (confirmed with @Henry-theBinaryBuddies — Cauldron's "radius" is a sphere/circle on the ground, not `TARGET_SHAPE_CUBE`), `cube`→`TARGET_SHAPE_CUBE`, `sphere`→`TARGET_SHAPE_SPHERE`; `size`→`size_feet`; `width`→`width_feet`). `recharge.min` → **no home** on `Resource` — **pending contracts change**: @Henry-theBinaryBuddies is adding recharge as a field on `Ability` directly, not modeled as a `Resource` (every multiattack-adjacent monster in the samples has a recharge ability, so this will matter quickly). `effects_text` → same no-home note as the attack case. |
| `"multiattack"` | — (no `EffectStep`, for now) | **No home as structured data yet.** `sequence: [{action, count}]` (e.g. "Bite ×1, Claw ×2") describes *which other abilities to use and how many times*, which `EffectStep`'s `oneof` can't express today. Three of five samples use multiattack. Decision from @Henry-theBinaryBuddies: needs a `MultiattackStep`-shaped addition (option (b)), but that depends on the rules engine's resolution state machine and will be designed in Milestone 2, not as a simple contract tweak. **Until then**: import `multiattack` as an `Ability` with the plain-English `sequence` description in `Ability.description` and no `steps` (the GM resolves it by hand in the interim — accepted as a known gap, not silently dropped). |
| `"text"` | — (no `EffectStep`) | Same shape as a trait: `text` → `Ability.description`, no `steps`. **No sample exercises a `text`-kind action** (all five put their prose-only abilities in `traits`, not `actions`) — flagged in the sample README; this row is inferred from the schema, not yet confirmed against a real file. |

## Spellcasting

| Export field | Maps to | Conversion / rule |
|---|---|---|
| `monster.spellcasting.ability` | — | No home on `Creature` directly; used only to compute `attack_bonus`/`dc` below, both already precomputed by Cauldron. |
| `monster.spellcasting.dc` | Each spell-derived `Ability`'s implicit `SaveStep.dc` | Not a `Creature`-level field — applies per spell-as-ability. |
| `monster.spellcasting.attack_bonus` | Each spell-derived `Ability`'s implicit `AttackStep.to_hit_bonus` | Same, per spell. |
| `monster.spellcasting.innate` | — | No home structurally; affects only `intro` wording (see below). |
| `monster.spellcasting.intro`, `.footer` | — | No home. Narrative sentence(s); fold into `Creature.summary` or a synthesized "Spellcasting" trait's `Ability.description` if kept at all. Likely droppable once the spells below are each their own `Ability`. |
| `monster.spellcasting.cantrips[]`, `.at_will[]` | `Ability` per spell, `cost = ACTION_COST_ACTION`, `resource_key` unset | Each `SpellRef.name` → `Ability.name`; **no `EffectStep`** — a spell's actual mechanical effect (its damage, save, etc.) is not in the export at all, only its name. This is a real gap: importing "confusion" or "mage hand" by name gives no playable effect, only a label. **File as a Contract change request is the wrong fix here** — the gap is Creature Cauldron's export, not our contract (it exports spell *names*, not spell *definitions*). Flag this to @Henry-theBinaryBuddies as a product question: either the SRD spell list needs to live in our own data (keyed by name) and the importer looks up the effect there, or spellcasting monsters are out of scope for the importer until that exists. Not filing as a Contract change request since `Creature`/`Ability`/`EffectStep` already have everything needed *if* the spell's mechanics were known. |
| `monster.spellcasting.slots[]` (`level`, `slots`, `spells[]`) | `Creature.resources` (one `Resource` per level) + `Ability` per spell | `Resource{key:"spell_slot_<level>", label:"Level <level> Slots", max:slots, current:slots}`. Each `SpellRef` in `spells[]` → `Ability` with `resource_key = "spell_slot_<level>"`, `resource_amount = 1`; same no-effect gap as cantrips above. |
| `monster.spellcasting.per_day[]` (`uses`, `each`, `spells[]`) | `Creature.resources` + `Ability` per spell | If `each = true` (e.g. "3/day each"): one `Resource` per spell, `key = "<slug(spell-name)>_per_day"`, `max = uses`. If `each = false` (shared pool, e.g. "1/day" split across a list): one shared `Resource`, `max = uses`, referenced by every spell in the list. Slug rule for `<slug(spell-name)>`: lower-case the spell's `name`, then replace every run of non-alphanumeric characters with a single `_` (e.g. `"Mage Hand"` → `"mage_hand"`) — this keeps two differently-cased or differently-punctuated spell names from colliding on the same resource key. Same no-effect gap as above. |

## Discovery tags and presentation

| Export field | Maps to | Conversion / rule |
|---|---|---|
| `monster.environments`, `.role`, `.damage_themes`, `.features` | — | No home. These are Creature Cauldron's own browse/filter tags, meaningless once the monster is in a campaign. Safely dropped — nothing in `Creature` or the rules engine needs them. |
| `monster.description` | — | No home as a dedicated field; fold into `Creature.summary` if there's room, otherwise dropped. Lower priority than the CR/class-level info `summary` is meant for (per the contract comment: "Class and level, or creature type and CR; display only"). |

## Not covered by any current sample

Per `tests/samples/README.md`, these need a real export before this mapping (or the importer)
can claim to handle them, and are not reflected above with confidence:

- `reactions` with real content
- `bonus_actions` with real content
- an action of `kind: "text"`
- `damage_text` set to a non-null value
- a monster with `source` other than `"curated"` (`srd`, `a5e`, `ai`, `reskin`), which carries `attribution`/`parent_id`/`generator`/`changes`
- an `encounter`-kind export wrapping several monsters

## Gaps pending a Contract change

@Henry-theBinaryBuddies owns `contracts/` and is opening one `contracts/` pull request covering
all of the items below, plus the `Provenance.external_id` field, the passive `ActionCost` value,
and room for Cauldron's `parent_id`/`generator`/`changes` metadata (see the relevant rows above).
This mapping does not file separate issues for them. Rows above that depend on a field from that
PR are marked **pending contracts change** until it merges; this list is the standing summary:

1. **Multiple movement speeds** (`speeds.fly`/`.swim`/`.climb`/`.burrow`/`.hover`) — `Creature` has only one `speed_feet`. Contract addition: per-mode speeds and hover.
2. **Skills and passive scores** (`skills`, `senses.passive_perception`) — no field on `Creature` for either. Contract addition.
3. **Condition immunities** (`condition_immunities`) — `Creature.conditions` is active-conditions-only, not an immunity list. Contract addition.
4. **Ability recharge** (`recharge.min`, e.g. "recharges on 5–6") — no clean fit in `Resource`. Contract addition on `Ability` directly, not modeled as a `Resource`.
5. **Multiattack as structured data** — `EffectStep` has no "invoke these other abilities N times" step. Agreed it needs structure (a `MultiattackStep`-shaped addition referencing other `ability_id`s by name and count), but that's rules-engine work for Milestone 2, not a simple contract tweak — until then, multiattack imports as an `Ability` with plain-English `description` and no `steps`.

Not a contract gap, decided separately:

- **Save bonuses** — no contract change; the importer computes the save bonus from `scores` + `proficiency_bonus` and reports a mismatch against the printed `monster.saves` bonus instead of dropping it.
- **Spell names without mechanical effects** (`cantrips`/`at_will`/`slots`/`per_day`) — not a contract gap, since `Creature`/`Ability`/`EffectStep` already have everything needed *if* the spell's mechanics were known. This is a Creature Cauldron export gap (it exports spell names, not definitions) or a future SRD spell-library question, flagged to @Henry-theBinaryBuddies as a product question rather than filed as a contract change.
- **Senses, creature type and subtypes** — deferred (folded into `summary` for now), not part of the current contracts change.

Fields judged **safely dropped** (no contract change proposed): `exported_at`, Cauldron's own
`schema_version`/`validated`, `type`/`subtypes`/`alignment` (fold into `summary` instead),
`hit_points.dice`, `armor_class.note`, `damage_text`, `languages`, `challenge.cr`/`cr_value`/`xp`
(fold into `summary`), `environments`/`role`/`damage_themes`/`features`, `description` (fold into
`summary` if space allows), and per-ability `effects_text` until a sample exercises it. `monster.id`
is not dropped — see "Identity and provenance" above, it maps to `Provenance.external_id` pending
the contracts change.
