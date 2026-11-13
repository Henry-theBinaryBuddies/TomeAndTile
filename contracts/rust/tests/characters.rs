use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use tt_contracts::v1::{effect_step, Ability, Creature, CreatureKind};

const SRD: &str = "srd-5.2";

fn characters_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/characters")
}

fn load_all() -> Vec<(String, Creature)> {
    let mut files: Vec<PathBuf> = fs::read_dir(characters_dir())
        .expect("characters directory")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 4,
        "expected at least 4 character files, found {}",
        files.len()
    );
    files
        .into_iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let text = fs::read_to_string(&path).expect("read file");
            let creature: Creature = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("{name} does not parse: {e}"));
            (name, creature)
        })
        .collect()
}

/// A spell is an ability that spends a spell slot, or a cantrip. Cantrips cost
/// no slot, so by fixture convention their ability ids start with "cantrip-".
fn is_spell(ability: &Ability) -> bool {
    ability.resource_key.starts_with("spell_slot_") || ability.ability_id.starts_with("cantrip-")
}

#[test]
fn every_character_is_a_valid_srd_player_character() {
    for (file, c) in load_all() {
        assert_eq!(c.kind, CreatureKind::PlayerCharacter as i32, "{file}: kind");
        assert!(c.hp_max > 0, "{file}: hp_max");
        assert!(c.armor_class > 0, "{file}: armor_class");
        let prov = c.provenance.as_ref().expect("creature provenance");
        assert_eq!(prov.source, SRD, "{file}: creature source");
        assert!(!prov.attribution.is_empty(), "{file}: creature attribution");

        let resource_keys: HashSet<&str> = c.resources.iter().map(|r| r.key.as_str()).collect();
        let mut ids = HashSet::new();
        for a in &c.abilities {
            assert!(!a.ability_id.is_empty(), "{file}: empty ability_id");
            assert!(
                ids.insert(a.ability_id.as_str()),
                "{file}: duplicate ability_id {}",
                a.ability_id
            );
            let p = a.provenance.as_ref().expect("ability provenance");
            assert_eq!(p.source, SRD, "{file}: {} source", a.ability_id);
            assert!(
                !p.attribution.is_empty(),
                "{file}: {} attribution",
                a.ability_id
            );
            if !a.resource_key.is_empty() {
                assert!(
                    resource_keys.contains(a.resource_key.as_str()),
                    "{file}: {} spends unknown resource {}",
                    a.ability_id,
                    a.resource_key
                );
            }
        }
    }
}

#[test]
fn party_covers_the_required_kinds_of_ability() {
    let (mut melee, mut ranged, mut spell_attack) = (false, false, false);
    let (mut half_save, mut healing, mut slots) = (false, false, false);
    for (_, c) in load_all() {
        slots |= c.resources.iter().any(|r| r.key.starts_with("spell_slot_"));
        for a in &c.abilities {
            let range = a.targeting.as_ref().map_or(0.0, |t| t.range_feet);
            for step in a.steps.iter().filter_map(|s| s.step.as_ref()) {
                match step {
                    effect_step::Step::Attack(_) => {
                        if is_spell(a) {
                            spell_attack = true;
                        } else if range <= 5.0 {
                            melee = true;
                        } else {
                            ranged = true;
                        }
                    }
                    effect_step::Step::Save(s) => half_save |= s.half_on_success,
                    effect_step::Step::Flat(f) => healing |= !f.healing_dice.is_empty(),
                    _ => {}
                }
            }
        }
    }
    assert!(melee, "no melee weapon attack");
    assert!(ranged, "no ranged weapon attack");
    assert!(spell_attack, "no spell attack");
    assert!(half_save, "no save step with half_on_success");
    assert!(healing, "no healing step");
    assert!(slots, "no spell_slot_ resource");
}

#[test]
fn every_character_roundtrips_through_json() {
    for (file, c) in load_all() {
        let json = serde_json::to_string(&c).expect("serialize");
        let back: Creature = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(c, back, "{file}: roundtrip");
    }
}
