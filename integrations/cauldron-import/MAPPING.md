# Creature Cauldron → `Creature` field map

Field-by-field map from a Creature Cauldron monster export to the `Creature` contract
(`contracts/proto/tomeandtile/v1/creature.proto`). Built against the five files in
[`tests/samples/`](tests/samples/) and the field list in
[`tests/samples/monster.schema.ts.txt`](tests/samples/monster.schema.ts.txt). No importer
code exists yet (blocked on the contracts code-generation issue); this is the design the
importer will follow.

Every row either names a `Creature`/`Ability`/`EffectStep` field, or says **no home**. See
"Gaps filed as contract changes" at the end for what that means for each no-home field.

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
| `monster.id` | — | No home as a stored id. `Creature.creature_id` is minted by the server (contracts comment: "Identifiers are opaque strings minted by the server"). Keep the Cauldron `id` only inside `Provenance.attribution` text so a re-import is traceable, never as `creature_id`. |
| `monster.schema_version` | — | No home. Internal to Creature Cauldron's own schema, not ours. Safely dropped. |
| `monster.name` | `Creature.name` | Direct copy. |
| `monster.source` | `Creature.provenance.source` | Always set `Provenance.source = "creature-cauldron"` regardless of Cauldron's own `source` value — `Provenance.source` names *our* importer, not Cauldron's internal taxonomy. Cauldron's `source` ("srd" / "a5e" / "curated" / "ai" / "reskin") instead feeds `attribution` below. **None of the five samples has a `source` other than `"curated"`** — `srd`, `a5e`, `ai` and `reskin` need a real sample before this is more than a guess (flagged in the sample README's "Not covered yet"). |
| `monster.validated` | — | No home. Cauldron-internal QA flag, not game state. Safely dropped. |
| `monster.parent_id`, `monster.generator`, `monster.changes`, `monster.attribution` | `Creature.provenance.attribution` | Fold into one free-text string when present, e.g. `"reskin of <parent_id> via <generator>; changes: <changes>"` or the raw `attribution` text for `srd` content. All four are `null` in every sample we have — the exact phrasing is a guess until a real `srd`/`reskin`/`ai` sample arrives (same gap as `source` above). |

## Core stat block

| Export field | Maps to | Conversion / rule |
|---|---|---|
| `monster.size` | `Creature.size` | Maps to `CreatureSize` (defined in `scene.proto`, not shown here — confirm its enum values cover Cauldron's `"small"` / `"large"` / `"medium"` / `"gargantuan"` etc. before writing code). |
| `monster.type`, `monster.subtypes` | — | No home. `Creature` has no creature-type/subtype field (only `summary`, display-only). File as a contract change if combat or targeting ever needs to key off type (e.g. "undead"); until then, fold into `summary`. |
| `monster.alignment` | — | No home. Flavor only. Fold into `summary` or drop. |
| `monster.armor_class.value` | `Creature.armor_class` | Direct copy. |
| `monster.armor_class.note` | — | No home as structured data. Fold into `summary` if kept at all; typically droppable. |
| `monster.hit_points.average` | `Creature.hp_max` (and `hp_current` at import time) | Direct copy to both — a freshly imported creature starts at full HP. |
| `monster.hit_points.dice` | — | No home. `Creature` stores only the rolled/average max, not the formula. Droppable (the average is what matters for play; nothing re-rolls max HP later). |
| `monster.speeds.walk` | `Creature.speed_feet` | Direct copy. |
| `monster.speeds.fly` / `.swim` / `.climb` / `.burrow` / `.hover` | — | **No home.** `Creature` has one `speed_feet`, not a per-mode map. This is a real gap for a flying/burrowing/swimming monster (three of five samples use a non-walk speed) — **file as a Contract change request**: `Creature` needs either a `speeds` map or separate fields, or the rules engine can't tell a flying monster from a grounded one. Do not silently use the walk speed as a stand-in. |
| `monster.abilities` (`str`..`cha`) | `Creature.scores` (`AbilityScores`) | Direct 1:1 copy, field by field. |
| `monster.saves` | `Creature.save_proficiencies` | `Creature.save_proficiencies` is a list of `AbilityScore` (which abilities are proficient), not a bonus map. The *list of keys present* in `monster.saves` maps across; **the bonus number itself has no home** — `Creature` expects the engine to compute the save bonus from `scores` + `proficiency_bonus`, so a save bonus that doesn't match that formula (common for monster stat blocks, which hand-tune saves) is silently lost. Worth a Contract change request if faithfulness to the printed bonus matters; noting it here as a gap rather than filing yet, since it may be intentional per ADR 0005 ("data-driven" engine deriving from scores). |
| `monster.skills` | — | **No home.** No skills/skill-bonus field anywhere on `Creature`. All five samples use this (Perception, Stealth). **File as a Contract change request** — needed at minimum for passive checks the GM calls for, separate from `senses.passive_perception` below. |
| `monster.vulnerabilities` | `Creature.vulnerabilities` | List of `DamageType`; Cauldron's string names map 1:1 to the `DamageType` enum values (case/shape to confirm when the importer is written). |
| `monster.resistances` | `Creature.resistances` | Same mapping as vulnerabilities. |
| `monster.immunities` | `Creature.immunities` | Same mapping as vulnerabilities. |
| `monster.damage_text` | — | No home, by design — `DamagePart`/`DamageType` are typed lists, not prose (contract favors structured data). None of the five samples has this field set, so it's untested either way; when a sample with it shows up, the wording is dropped and only the type lists in `vulnerabilities`/`resistances`/`immunities` carry forward. Safely dropped. |
| `monster.condition_immunities` | — | **No home.** `Creature.conditions` holds *active* condition ids, not immunities. **File as a Contract change request** — two of five samples set this (`prone`, `blinded`). |
| `monster.senses.*` (`darkvision`, `blindsight`, `tremorsense`, `truesight`) | — | No home. `Creature` has no senses field. Low risk for the class scope (no fog/dynamic lighting yet per ARCHITECTURE.md "designed into the contracts but built later"), but worth a future Contract change request once lighting exists. |
| `monster.senses.passive_perception` | — | No home today for the same reason as skills above; same Contract change request as `skills` should probably carry this too. |
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
| `monster.traits[]` (always `kind: "text"`) | `Ability` with `cost = ACTION_COST_FREE` | `name` → `Ability.name`; `text` → `Ability.description`; no `EffectStep` (traits are descriptive, not clickable effects) — `Ability.steps` stays empty. |
| `monster.actions[]` | `Ability` with `cost = ACTION_COST_ACTION` | See action-kind table below for `steps`. |
| `monster.bonus_actions[]` | `Ability` with `cost = ACTION_COST_BONUS_ACTION` | Same per-kind handling as `actions[]`. **No sample has a non-empty `bonus_actions`** — untested until one arrives (flagged in the sample README). |
| `monster.reactions[]` | `Ability` with `cost = ACTION_COST_REACTION` | Same per-kind handling. **No sample has a non-empty `reactions`** — untested, flagged in the sample README. Also note ARCHITECTURE.md: the rules engine's `AwaitingReaction` state is reserved but not necessarily built yet — confirm the engine actually accepts a reaction-cost ability before the importer assumes it's playable. |
| `monster.legendary_actions.options[]` | `Ability` with `cost = ACTION_COST_LEGENDARY` | `name` → `Ability.name`; `text` → `Ability.description`; `cost` → `Ability.resource_amount` (this is legendary-action *point* cost, not the resource system below); no structured `EffectStep` — legendary options are prose only in every sample, so `steps` stays empty (same as `text`-kind actions). |
| `monster.legendary_actions.per_round` | `Creature.resources` | One synthesized `Resource`: `key = "legendary_actions"`, `label = "Legendary Actions"`, `max = per_round`, `current = per_round`. |

### Action kind → `EffectStep`

| Cauldron `kind` | `EffectStep` variant | Conversion |
|---|---|---|
| `"attack"` | `attack` (`AttackStep`) | `attack.to_hit` → `to_hit_bonus`. `damage[]` → `damage[]` (`DamagePart`): each `{dice, bonus}` combines into one dice string (e.g. `dice:"1d4", bonus:3` → `"1d4+3"`); `type` → `DamagePart.type`. `attack.mode`, `attack.ability`, `attack.reach`/`range`, `attack.targets` → folded into `Ability.targeting` (`Targeting.shape = TARGET_SHAPE_CREATURES`, `range_feet` from `reach` or `range.normal`, `long_range_feet` from `range.long` or `0`); `attack.mode`/`attack.ability` themselves have **no home** on `AttackStep` (the contract bakes ability + proficiency into `to_hit_bonus` already, so the raw ability/mode is redundant for play — safely dropped) once `to_hit_bonus` is copied. `effects_text` (free text, e.g. "target is grappled") has **no home** as a structured `ApplyEffectStep` — none of the attack actions in the five samples sets it, so left unhandled for now; a sample that uses it should become its own `ApplyEffectStep` step or get filed as a gap. |
| `"save"` | `save` (`SaveStep`) | `save.ability` → `SaveStep.ability` (`AbilityScore`). `save.dc` → `dc`. `on_fail[]` → `damage[]`, same dice+bonus combining as above. `on_success` (`"half"` / `"none"`) → `half_on_success` (`true` / `false`). `area.shape`/`size`/`width` → `Ability.targeting` (`TargetShape` from `shape`: `cone`→`TARGET_SHAPE_CONE`, `line`→`TARGET_SHAPE_LINE`, `radius`→`TARGET_SHAPE_SPHERE` *(name mismatch — Cauldron's "radius" is a sphere/circle on the ground; confirm this is the intended `Targeting` shape and not `TARGET_SHAPE_CUBE` before writing code)*, `cube`→`TARGET_SHAPE_CUBE`, `sphere`→`TARGET_SHAPE_SPHERE`; `size`→`size_feet`; `width`→`width_feet`). `recharge.min` → **no home**: `Resource` could model "recharges on 5–6" as a resource with `max=1`, but that's a stretch fit, not a clean one — **file as a Contract change request** if per-ability recharge needs real tracking (every multiattack-adjacent monster in the samples has a recharge ability, so this will matter quickly). `effects_text` → same no-home note as the attack case. |
| `"multiattack"` | — (no `EffectStep`) | **No home as structured data.** `sequence: [{action, count}]` (e.g. "Bite ×1, Claw ×2") describes *which other abilities to use and how many times*, which `EffectStep`'s `oneof` can't express (it has no "run these other abilities" step). Three of five samples use multiattack. Two options, neither clean: (a) put the plain English in `Ability.description` only and let the GM click the listed abilities by hand (loses automation — contradicts ADR 0005 "fully automatic"), or (b) **file as a Contract change request** for a `MultiattackStep` referencing other `ability_id`s by name and count. Recommending (b); not filing yet pending a decision from @Henry-theBinaryBuddies since it affects the rules engine's resolution state machine, not just this importer. |
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
| `monster.spellcasting.per_day[]` (`uses`, `each`, `spells[]`) | `Creature.resources` + `Ability` per spell | If `each = true` (e.g. "3/day each"): one `Resource` per spell, `key = "<spell-name>_per_day"`, `max = uses`. If `each = false` (shared pool, e.g. "1/day" split across a list): one shared `Resource`, `max = uses`, referenced by every spell in the list. Same no-effect gap as above. |

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

## Gaps filed as Contract change requests

Not yet filed as GitHub issues — listed here first per the task's acceptance criteria, to be
opened as "Contract change request" issues and linked back here before any importer code is
written:

1. **Multiple movement speeds** (`speeds.fly`/`.swim`/`.climb`/`.burrow`/`.hover`) — `Creature` has only one `speed_feet`.
2. **Skills and passive scores** (`skills`, `senses.passive_perception`) — no field on `Creature` for either.
3. **Condition immunities** (`condition_immunities`) — `Creature.conditions` is active-conditions-only, not an immunity list.
4. **Multiattack as structured data** — `EffectStep` has no "invoke these other abilities N times" step; needs rules-engine input, not just a contract tweak.
5. **Ability recharge** (`recharge.min`, e.g. "recharges on 5–6") — no clean fit in `Resource` or elsewhere.

Fields judged **safely dropped** (no contract change proposed): `exported_at`, Cauldron's own
`id`/`schema_version`/`validated`, `type`/`subtypes`/`alignment` (fold into `summary` instead),
`hit_points.dice`, `armor_class.note`, `damage_text`, `languages`, `challenge.cr`/`cr_value`/`xp`
(fold into `summary`), `environments`/`role`/`damage_themes`/`features`, `description` (fold into
`summary` if space allows), and per-ability `effects_text` until a sample exercises it.
