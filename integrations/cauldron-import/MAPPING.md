# Creature Cauldron → `Creature` field map

Field-by-field map from a Creature Cauldron monster export to the `Creature` contract
(`contracts/proto/tomeandtile/v1/creature.proto`). Built against the five files in
[`tests/samples/`](tests/samples/) and the field list in
[`tests/samples/monster.schema.ts.txt`](tests/samples/monster.schema.ts.txt). No importer
code exists yet (blocked on the contracts code-generation issue); this is the design the
importer will follow.

Every row either names a `Creature`/`Ability`/`EffectStep` field, or says **no home**. See
"Gaps and the contract change" at the end for what that means for each no-home field.

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
| `monster.id` | `Creature.provenance.external_id` | `Creature.creature_id` is minted by the server (contracts comment: "Identifiers are opaque strings minted by the server") and is not a home for Cauldron's own id. An id doesn't belong in `attribution` either — attribution is licence-credit text, not a database key. `Provenance.external_id` (added in [#16](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/16)) is exactly for this: "the id this content has in the system it came from... so a re-import can be matched to the earlier one." |
| `monster.schema_version` | — | No home. Internal to Creature Cauldron's own schema, not ours. Safely dropped. |
| `monster.name` | `Creature.name` | Direct copy. |
| `monster.source` | `Creature.provenance.source` | Always set `Provenance.source = "creature-cauldron"` regardless of Cauldron's own `source` value — `Provenance.source` names *our* importer, not Cauldron's internal taxonomy. Cauldron's `source` ("srd" / "a5e" / "curated" / "ai" / "reskin") instead feeds the Cauldron-metadata field(s) below. **None of the five samples has a `source` other than `"curated"`** — `srd`, `a5e`, `ai` and `reskin` need a real sample before this is more than a guess (flagged in the sample README's "Not covered yet"). |
| `monster.validated` | — | No home. Cauldron-internal QA flag, not game state. Safely dropped. |
| `monster.attribution` | `Creature.provenance.attribution` | Copied **word for word** when present, never rephrased or combined with other fields — SRD's CC-BY-4.0 requires the attribution text exactly as given. All five samples have `attribution: null`, so this is untested against a real `srd`/`reskin`/`ai` sample. |
| `monster.parent_id`, `monster.generator`, `monster.changes` | `Creature.provenance.origin_notes` | Cauldron's reskin/generation metadata, distinct from the licence-attribution string above. `Provenance.origin_notes` (added in [#16](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/16)) is "the source system's own notes on where the content came from: its category..., parent, generator, list of changes. Free text for display." Fold the three into one free-text string when present, e.g. `"reskin of <parent_id> via <generator>; changes: <changes>"`. All three are `null` in every sample we have, so the exact phrasing is a guess until a real `reskin`/`ai` sample arrives (same gap as `source` above). |

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
| `monster.speeds.fly` / `.swim` / `.climb` / `.burrow` | `Creature.fly_speed_feet` / `.swim_speed_feet` / `.climb_speed_feet` / `.burrow_speed_feet` | Direct copy per mode (added in [#16](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/16); 0 means the creature lacks that mode, same convention as `speed_feet`). Four of five samples use a non-walk speed (Cinderkite and Stormhide Drake fly, Dune Leviathan burrows, Voidbloom Seer flies and hovers) — this was a real gap before #16; do not silently use the walk speed as a stand-in. |
| `monster.speeds.hover` | `Creature.can_hover` | Direct copy (`bool`, added in #16): "true if the creature can hover: it stays aloft when its speed is 0." Voidbloom Seer sets this. |
| `monster.abilities` (`str`..`cha`) | `Creature.scores` (`AbilityScores`) | Direct 1:1 copy, field by field. |
| `monster.saves` | `Creature.save_proficiencies` | `Creature.save_proficiencies` is a list of `AbilityScore` (which abilities are proficient), not a bonus map. The *list of keys present* in `monster.saves` maps across. **No contract change**: in all five samples the printed bonus equals ability modifier plus proficiency bonus, confirming ADR 0005's data-driven design. The importer should compute the save bonus from `scores` + `proficiency_bonus`, compare it against the printed `monster.saves` bonus, and report a mismatch instead of silently dropping a bonus that doesn't match. |
| `monster.skills` | `Creature.skills` (`repeated SkillBonus`) | Each `{skill, bonus}` pair maps to a `SkillBonus{skill, bonus}` (added in [#16](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/16)). `SkillBonus.skill` is "lower-case skill id with underscores" (e.g. `"perception"`, `"sleight_of_hand"`); confirmed in the importer (`SKILL_IDS`, one entry per SRD skill, matched against Cauldron's exact title-case name) — a name outside the 18 SRD skills is reported and left out; a skill not listed uses the plain ability modifier, so no default row is needed for skills Cauldron omits. Two of five samples use this (Cinderkite, Stormhide Drake — Perception, Stealth). |
| `monster.vulnerabilities` | `Creature.vulnerabilities` | List of `DamageType`; Cauldron's string names map 1:1 to the `DamageType` enum values. Confirmed in the importer (`damage_type`, case-insensitive match on the lower-case English name); an unrecognized name is reported and left out, never mapped to a near match. |
| `monster.resistances` | `Creature.resistances` | Same mapping as vulnerabilities. |
| `monster.immunities` | `Creature.immunities` | Same mapping as vulnerabilities. |
| `monster.damage_text` | — | No home, by design — `DamagePart`/`DamageType` are typed lists, not prose (contract favors structured data). None of the five samples has this field set, so it's untested either way; when a sample with it shows up, the wording is dropped and only the type lists in `vulnerabilities`/`resistances`/`immunities` carry forward. Safely dropped. |
| `monster.condition_immunities` | `Creature.condition_immunities` | Direct copy (`repeated string`, added in [#16](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/16)): "condition ids the creature cannot be given... `conditions` holds the ones currently active" — distinct from `Creature.conditions`, which stays active-conditions-only. Two of five samples set this (`prone`, `blinded`). |
| `monster.senses.*` (`darkvision`, `blindsight`, `tremorsense`, `truesight`) | — | No home. `Creature` has no senses field. Low risk for the class scope (no fog/dynamic lighting yet per ARCHITECTURE.md "designed into the contracts but built later"); deferred, not part of the current contracts change. |
| `monster.senses.passive_perception` | `Creature.passive_perception` | Direct copy (`int32`, added in #16): "Passive Wisdom (Perception) score. 0 means not stated." |
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
| `monster.traits[]` (always `kind: "text"`) | `Ability` with `cost = ACTION_COST_PASSIVE` | `ACTION_COST_FREE` was wrong here — traits are passive (nothing happens when you click them), not a free clickable action. `ACTION_COST_PASSIVE` (added in [#16](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/16)) is exactly this: "always on; cannot be activated... has no steps and nothing happens when it is clicked." `name` → `Ability.name`; `text` → `Ability.description`; no `EffectStep` — `Ability.steps` stays empty. |
| `monster.actions[]` | `Ability` with `cost = ACTION_COST_ACTION` | See action-kind table below for `steps`. |
| `monster.bonus_actions[]` | `Ability` with `cost = ACTION_COST_BONUS_ACTION` | Same per-kind handling as `actions[]`. **No sample has a non-empty `bonus_actions`** — untested until one arrives (flagged in the sample README). |
| `monster.reactions[]` | `Ability` with `cost = ACTION_COST_REACTION` | Same per-kind handling. **No sample has a non-empty `reactions`** — untested, flagged in the sample README. Also note ARCHITECTURE.md: the rules engine's `AwaitingReaction` state is reserved but not necessarily built yet — confirm the engine actually accepts a reaction-cost ability before the importer assumes it's playable. |
| `monster.legendary_actions.options[]` | `Ability` with `cost = ACTION_COST_LEGENDARY` | `name` → `Ability.name`; `text` → `Ability.description`; `cost` → `Ability.resource_amount` (this is legendary-action *point* cost, not the resource system below); `resource_key = "legendary_actions"` (matches the `Resource.key` synthesized below, so the cost is actually spent from that pool — previously left unset); no structured `EffectStep` — legendary options are prose only in every sample, so `steps` stays empty (same as `text`-kind actions). |
| `monster.legendary_actions.per_round` | `Creature.resources` | One synthesized `Resource`: `key = "legendary_actions"`, `label = "Legendary Actions"`, `max = per_round`, `current = per_round`. |

### Action kind → `EffectStep`

| Cauldron `kind` | `EffectStep` variant | Conversion |
|---|---|---|
| `"attack"` | `attack` (`AttackStep`) | `attack.to_hit` → `to_hit_bonus`. `damage[]` → `damage[]` (`DamagePart`): each `{dice, bonus}` combines into one dice string: `bonus > 0` → `"<dice>+<bonus>"` (e.g. `dice:"1d4", bonus:3` → `"1d4+3"`); `bonus < 0` → `"<dice><bonus>"`, the negative sign carries its own `-` (e.g. `dice:"1d4", bonus:-1` → `"1d4-1"`); `bonus == 0` → `"<dice>"` with no `+0` suffix. `type` → `DamagePart.type`. `attack.mode`, `attack.ability`, `attack.reach`/`range`, `attack.targets` → folded into `Ability.targeting` (`Targeting.shape = TARGET_SHAPE_CREATURES`, `range_feet` from `reach` or `range.normal`, `long_range_feet` from `range.long` or `0`); `attack.mode`/`attack.ability` themselves have **no home** on `AttackStep` (the contract bakes ability + proficiency into `to_hit_bonus` already, so the raw ability/mode is redundant for play — safely dropped) once `to_hit_bonus` is copied. `effects_text` (free text, e.g. "target is grappled") has **no home** as a structured `ApplyEffectStep`: Creature Cauldron's schema has no field that states a condition as data (only this free-text string), so the importer never produces an `ApplyEffectStep` — when present, `effects_text` is appended to `Ability.description` and reported as "effect not imported" instead. None of the five samples sets it, so this is unconfirmed against a real export; ask @Henry-theBinaryBuddies for one that does before trusting this path. |
| `"save"` | `save` (`SaveStep`) | `save.ability` → `SaveStep.ability` (`AbilityScore`). `save.dc` → `dc`. `on_fail[]` → `damage[]`, same dice+bonus combining as above (including the `bonus == 0` / negative-`bonus` rules). `on_success` (`"half"` / `"none"`) → `half_on_success` (`true` / `false`). `area.shape`/`size`/`width` → `Ability.targeting` (`TargetShape` from `shape`: `cone`→`TARGET_SHAPE_CONE`, `line`→`TARGET_SHAPE_LINE`, `radius`→`TARGET_SHAPE_SPHERE` (confirmed with @Henry-theBinaryBuddies — Cauldron's "radius" is a sphere/circle on the ground, not `TARGET_SHAPE_CUBE`), `cube`→`TARGET_SHAPE_CUBE`, `sphere`→`TARGET_SHAPE_SPHERE`; `size`→`size_feet`; `width`→`width_feet`). `recharge.min` → `Ability.recharge` (`Recharge{min_roll}`, added in [#16](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/16): "after use, the ability is unavailable until a d6 rolled at the start of the creature's turn shows `min_roll` or higher"), on the `Ability` directly rather than as a `Resource`. Leave `Ability.recharge_spent` false on import — "set by the server; importers leave it false." Every multiattack-adjacent monster in the samples has a recharge ability. `effects_text` → same no-home note as the attack case. Confirmed against the three recharge save abilities in the samples (Stormhide Drake, Dune Leviathan, Voidbloom Seer): none sets `effects_text`. |
| `"multiattack"` | — (no `EffectStep`, for now) | **No home as structured data yet.** `sequence: [{action, count}]` (e.g. "Bite ×1, Claw ×2") describes *which other abilities to use and how many times*, which `EffectStep`'s `oneof` can't express today. Three of five samples use multiattack. Decision from @Henry-theBinaryBuddies: needs a `MultiattackStep`-shaped addition (option (b)), but that depends on the rules engine's resolution state machine and will be designed in Milestone 2, not as a simple contract tweak. **Until then**: import `multiattack` as an `Ability` with the plain-English `sequence` description in `Ability.description` and no `steps` (the GM resolves it by hand in the interim — accepted as a known gap, not silently dropped, and always reported as "multiattack resolved by hand"). Phrasing implemented as `"<action> <once/twice/N times>"` per entry, joined with `", then "` (e.g. `"Bite once, then Claw twice"`, confirmed against Stormhide Drake and Dune Leviathan's samples). |
| `"text"` | — (no `EffectStep`) | Same shape as a trait: `text` → `Ability.description`, no `steps`; a `recharge` on a text-kind action is copied the same way as the save case. Implemented identically to a trait entry. **Still no sample exercises a `text`-kind action** (all five put their prose-only abilities in `traits`, not `actions`) — flagged in the sample README; this row is implemented from the schema, not yet confirmed against a real file. |

## Spellcasting

| Export field | Maps to | Conversion / rule |
|---|---|---|
| `monster.spellcasting.ability` | — | No home on `Creature` directly; used only to compute `attack_bonus`/`dc` below, both already precomputed by Cauldron. |
| `monster.spellcasting.dc` | Each spell-derived `Ability`'s implicit `SaveStep.dc` | Not a `Creature`-level field — applies per spell-as-ability. |
| `monster.spellcasting.attack_bonus` | Each spell-derived `Ability`'s implicit `AttackStep.to_hit_bonus` | Same, per spell. |
| `monster.spellcasting.innate` | — | No home structurally; affects only `intro` wording (see below). |
| `monster.spellcasting.intro`, `.footer` | A synthesized passive "Spellcasting" `Ability` | Implemented: `intro` (and `footer`, when non-empty, separated by a blank line) becomes the `description` of one `cost = ACTION_COST_PASSIVE` ability named "Spellcasting", no `steps`. Only added when `spellcasting` is present and `intro` is non-empty (true of every sample that has spellcasting). |
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

## Gaps and the contract change

@Henry-theBinaryBuddies's [#16](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/16)
("Contracts: additive monster fields from the Creature Cauldron field map") has merged and
closes every gap below except multiattack. The rows above now map directly to the real fields
it added — `Provenance.external_id`, `Provenance.origin_notes`, `ACTION_COST_PASSIVE`,
`Ability.recharge`, `Creature.fly_speed_feet`/`swim_speed_feet`/`climb_speed_feet`/
`burrow_speed_feet`/`can_hover`, `Creature.skills`, `Creature.passive_perception`,
`Creature.condition_immunities` — rather than saying "pending contracts change." This list is now
the standing summary of what #16 added, kept for context:

1. **Multiple movement speeds** (`speeds.fly`/`.swim`/`.climb`/`.burrow`/`.hover`) — resolved: `fly_speed_feet`, `swim_speed_feet`, `climb_speed_feet`, `burrow_speed_feet`, `can_hover`.
2. **Skills and passive scores** (`skills`, `senses.passive_perception`) — resolved: `Creature.skills` (`repeated SkillBonus`), `Creature.passive_perception`.
3. **Condition immunities** (`condition_immunities`) — resolved: `Creature.condition_immunities`.
4. **Ability recharge** (`recharge.min`, e.g. "recharges on 5–6") — resolved: `Ability.recharge` (`Recharge{min_roll}`) plus `Ability.recharge_spent`, on `Ability` directly rather than modeled as a `Resource`.
5. **Multiattack as structured data** — still open. `EffectStep` has no "invoke these other abilities N times" step, and #16 doesn't add one. Agreed it needs structure (a `MultiattackStep`-shaped addition referencing other `ability_id`s by name and count), but that's rules-engine work for Milestone 2, not a simple contract tweak — until then, multiattack imports as an `Ability` with plain-English `description` and no `steps`.

Not a contract gap, decided separately:

- **Save bonuses** — no contract change; the importer computes the save bonus from `scores` + `proficiency_bonus` and reports a mismatch against the printed `monster.saves` bonus instead of dropping it.
- **Spell names without mechanical effects** (`cantrips`/`at_will`/`slots`/`per_day`) — not a contract gap, since `Creature`/`Ability`/`EffectStep` already have everything needed *if* the spell's mechanics were known. This is a Creature Cauldron export gap (it exports spell names, not definitions) or a future SRD spell-library question, flagged to @Henry-theBinaryBuddies as a product question rather than filed as a contract change.
- **Senses, creature type and subtypes** — deferred (folded into `summary` for now), not part of the current contracts change.

Fields judged **safely dropped** (no contract change proposed): `exported_at`, Cauldron's own
`schema_version`/`validated`, `type`/`subtypes`/`alignment` (fold into `summary` instead),
`hit_points.dice`, `armor_class.note`, `damage_text`, `languages`, `challenge.cr`/`cr_value`/`xp`
(fold into `summary`), `environments`/`role`/`damage_themes`/`features`, `description` (fold into
`summary` if space allows), and per-ability `effects_text` until a sample exercises it. `monster.id`
is not dropped — see "Identity and provenance" above, it maps to `Provenance.external_id`.
