# Creature Cauldron sample exports

Real export files for the importer's tests and for the field mapping in `../../MAPPING.md`.

## What a file looks like

Creature Cauldron's "Export JSON" button on a stat block saves `monster-<name>.json`: a small envelope around one monster.

```json
{
  "app": "Creature Cauldron",
  "kind": "monster",
  "format_version": 1,
  "exported_at": "2026-11-12T22:00:00.000Z",
  "monster": { "...": "the stat block" }
}
```

An importer must check `app`, `kind` and `format_version` before reading anything else, and refuse a file it does not recognise. All monsters use the 2014 rules.

`monster.schema.ts.txt` is a snapshot of Creature Cauldron's own type definition for `monster`, copied on 2026-11-12. It is reference only: the files are the truth, and the snapshot may fall behind.

## The samples

All five are original creatures from Creature Cauldron's curated set, provided by @Henry-theBinaryBuddies. They were produced from the app's sample data with the same envelope and formatting the export button writes.

| File | CR | What it exercises |
|---|---|---|
| `monster-cinderkite.json` | 1/8 | The simplest case: one melee attack, a fly speed, a trait |
| `monster-stormhide-drake.json` | 4 | Multiattack, a save ability with a recharge and a line area, half damage on a success, resistances |
| `monster-voidbloom-seer.json` | 7 | Legendary actions, a spell attack, innate spellcasting, a ranged attack |
| `monster-hollowvale-hexer.json` | 2 | Spellcasting with spell slots, a ranged spell attack |
| `monster-dune-leviathan.json` | 16 | A high-CR creature with large numbers, multiattack and a save ability |

## Not covered yet

None of the samples has any of the following. Each needs a real export before the importer can claim to handle it:

- `reactions`
- `bonus_actions`
- an action of kind `text`
- `damage_text` (wording such as "from nonmagical weapons")
- a monster whose `source` is `srd`, `a5e`, `ai` or `reskin` (these carry `attribution`, `parent_id`, `generator` or `changes`)
- an encounter export (`kind: "encounter"`), which wraps several monsters with counts

## Rules for adding a sample

- Only add files exported by the real app. Never hand-write or edit one to make a test pass.
- Only add creatures that are original, or SRD content with its attribution intact. This repository is public.
- Add a row to the table above saying what the file exercises.
