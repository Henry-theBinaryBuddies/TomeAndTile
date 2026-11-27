# contracts/fixtures

Recorded sample data, written as Protobuf JSON of contract messages (camelCase field names, enums as strings). The mock server (`crates/mock-server`) and tests load them.

- `mock/snapshot.json`: one `Snapshot` (a campaign with one scene, two tokens and two members) that the mock server starts from.

- `characters/*.json`: five pregenerated level-3 player characters (Fighter, Cleric, Wizard, Rogue, Ranger), each the Protobuf JSON form of one `Creature`. They use 2024 rules and SRD 5.2 content only, and each file and ability carries `provenance` with the attribution below. Names are invented. `contracts/rust/tests/characters.rs` checks them. Conventions: cantrip ability ids start with `cantrip-`; spells spend `spell_slot_1` or `spell_slot_2`; `creatureId` and `ownerUserId` are empty because the server mints them.

- `anvil/*.json`: five player characters exported by Adventurer's Anvil (Fighter 3, Fighter 4 / Wizard 1, Cleric 3, Ranger 3, Warlock 3), each the Protobuf JSON form of one `Creature`, kept unchanged to test importing character sheets. They use 2014 rules and SRD 5.1 content; `provenance.source` is `adventurers-anvil`, and each SRD ability carries its SRD 5.1 attribution text. `creatureId` and `ownerUserId` are empty. `contracts/rust/tests/anvil.rs` checks them.

- `seeds/pregen-table.json`: one `Snapshot` for M2 play: everything in `mock/snapshot.json` (members, scene, grid, `token-aria`), plus the five pregenerated characters from `characters/` on the map and a monster to attack. Each pregen is copied unchanged except for a fixed `creatureId` (`creature-brannoch`, `creature-tamsin`, `creature-korvath`, `creature-maribel`, `creature-ilvaren`), and has a token (`token-brannoch`, ...) linked to it, controlled by `mock-player`, with the creature's hit points. `token-goblin` is linked to `creature-goblin`, an invented monster ("Goblin Skulker", one melee attack). Brannoch stands 5 feet from the goblin, so his longsword reaches it, and the others stand farther away, so an out-of-reach attack can be tried. `contracts/rust/tests/seeds.rs` checks the file, including that each pregen copy still matches its source. The pregens carry the SRD 5.2 attribution below; the monster is invented (`provenance.source` `tome-and-tile`).

Rule: every fixture must always parse with the generated types. A test in the consuming crate checks this; update the fixture in the same PR as any contract change that breaks it. `mock/` uses invented content only; `characters/` and `seeds/` use SRD 5.2 content with attribution (the seed's monster is invented):

> This work includes material from the System Reference Document 5.2 ("SRD 5.2") by Wizards of the Coast LLC, available at https://www.dndbeyond.com/srd. The SRD 5.2 is licensed under the Creative Commons Attribution 4.0 International License, available at https://creativecommons.org/licenses/by/4.0/legalcode.
