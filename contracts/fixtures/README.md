# contracts/fixtures

Recorded sample data, written as Protobuf JSON of contract messages (camelCase field names, enums as strings). The mock server (`crates/mock-server`) and tests load them.

- `mock/snapshot.json`: one `Snapshot` (a campaign with one scene, two tokens and two members) that the mock server starts from.

- `characters/*.json`: five pregenerated level-3 player characters (Fighter, Cleric, Wizard, Rogue, Ranger), each the Protobuf JSON form of one `Creature`. They use 2024 rules and SRD 5.2 content only, and each file and ability carries `provenance` with the attribution below. Names are invented. `contracts/rust/tests/characters.rs` checks them. Conventions: cantrip ability ids start with `cantrip-`; spells spend `spell_slot_1` or `spell_slot_2`; `creatureId` and `ownerUserId` are empty because the server mints them.

Rule: every fixture must always parse with the generated types. A test in the consuming crate checks this; update the fixture in the same PR as any contract change that breaks it. `mock/` uses invented content only; `characters/` uses SRD 5.2 content with attribution:

> This work includes material from the System Reference Document 5.2 ("SRD 5.2") by Wizards of the Coast LLC, available at https://www.dndbeyond.com/srd. The SRD 5.2 is licensed under the Creative Commons Attribution 4.0 International License, available at https://creativecommons.org/licenses/by/4.0/legalcode.
