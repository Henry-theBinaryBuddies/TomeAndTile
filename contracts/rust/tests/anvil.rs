//! Checks `fixtures/anvil/*.json`: five player characters exported by
//! Adventurer's Anvil as `Creature` Protobuf JSON, kept to test importing
//! character sheets.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use tt_contracts::v1::{effect_step, Creature, CreatureKind};

const FILES: [&str; 5] = [
    "aldric-thorne-fighter-3.json",
    "borin-stonehelm-fighter-4-wizard-1.json",
    "bronna-ironhearth-cleric-3.json",
    "kael-wren-ranger-3.json",
    "mordecai-ash-warlock-3.json",
];

fn load(name: &str) -> Creature {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/anvil")
        .join(name);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} does not parse: {e}"))
}

#[test]
fn every_anvil_character_parses_as_a_player_character() {
    for name in FILES {
        let c = load(name);
        assert_eq!(c.kind, CreatureKind::PlayerCharacter as i32, "{name}");
        assert!(!c.name.is_empty(), "{name} has no name");
        assert!(c.hp_max > 0 && c.hp_current == c.hp_max, "{name} HP");
        assert!(c.armor_class > 0, "{name} AC");
    }
}

#[test]
fn ids_are_left_for_the_server_to_mint() {
    for name in FILES {
        let c = load(name);
        assert!(c.creature_id.is_empty(), "{name} has a creature_id");
        assert!(c.owner_user_id.is_empty(), "{name} has an owner_user_id");
    }
}

#[test]
fn ability_ids_are_unique_within_each_character() {
    for name in FILES {
        let c = load(name);
        let mut seen = HashSet::new();
        for a in &c.abilities {
            assert!(!a.ability_id.is_empty(), "{name}: an ability has no id");
            assert!(seen.insert(&a.ability_id), "{name}: {} twice", a.ability_id);
        }
    }
}

#[test]
fn provenance_names_anvil_and_srd_content_carries_its_attribution() {
    for name in FILES {
        let c = load(name);
        let p = c
            .provenance
            .as_ref()
            .unwrap_or_else(|| panic!("{name}: no provenance"));
        assert_eq!(p.source, "adventurers-anvil", "{name}");
        for a in &c.abilities {
            let ap = a.provenance.as_ref();
            if ap.is_some_and(|p| p.source.starts_with("srd")) {
                assert!(
                    ap.is_some_and(|p| !p.attribution.is_empty()),
                    "{name}: {} is SRD content with no attribution",
                    a.ability_id
                );
            }
        }
    }
}

#[test]
fn every_damage_and_healing_step_names_its_dice() {
    for name in FILES {
        let c = load(name);
        for a in &c.abilities {
            for s in &a.steps {
                let dice: Vec<&str> = match &s.step {
                    Some(effect_step::Step::Attack(x)) => {
                        x.damage.iter().map(|d| d.dice.as_str()).collect()
                    }
                    Some(effect_step::Step::Save(x)) => {
                        x.damage.iter().map(|d| d.dice.as_str()).collect()
                    }
                    Some(effect_step::Step::Flat(x)) => x
                        .damage
                        .iter()
                        .map(|d| d.dice.as_str())
                        .chain((!x.healing_dice.is_empty()).then_some(x.healing_dice.as_str()))
                        .collect(),
                    _ => continue,
                };
                assert!(
                    dice.iter().all(|d| !d.trim().is_empty()),
                    "{name}: {} has a step with empty dice",
                    a.ability_id
                );
            }
        }
    }
}
