//! Exercises `import_monster` against the five real Creature Cauldron exports in
//! `tests/samples/`. One test per sample, each checking the exact mapped values per
//! MAPPING.md and issue #105's acceptance tests; samples are read as-is and never edited.

use std::fs;
use std::path::PathBuf;

use tt_cauldron_import::{import_monster, Import};
use tt_contracts::v1::{AbilityScore, CreatureKind, CreatureSize, DamageType};

fn load(name: &str) -> Import {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/samples")
        .join(name);
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
    import_monster(&bytes).unwrap_or_else(|e| panic!("{name} did not import: {e}"))
}

#[test]
fn cinderkite_the_simplest_case() {
    let import = load("monster-cinderkite.json");
    let c = import.creature;
    assert_eq!(c.kind, CreatureKind::Monster as i32);
    assert_eq!(c.name, "Cinderkite");
    assert_eq!(c.size, CreatureSize::Small as i32);
    assert_eq!(c.armor_class, 13);
    assert_eq!(c.hp_max, 7);
    assert_eq!(c.hp_current, 7);
    assert_eq!(c.speed_feet, 10.0);
    assert_eq!(c.fly_speed_feet, 50.0);
    assert_eq!(c.swim_speed_feet, 0.0);
    assert!(!c.can_hover);
    let scores = c.scores.expect("scores");
    assert_eq!((scores.strength, scores.dexterity, scores.constitution), (6, 16, 10));
    assert_eq!((scores.intelligence, scores.wisdom, scores.charisma), (2, 14, 6));
    assert!(c.save_proficiencies.is_empty());
    assert_eq!(c.skills.len(), 1);
    assert_eq!(c.skills[0].skill, "perception");
    assert_eq!(c.skills[0].bonus, 4);
    assert!(c.vulnerabilities.is_empty());
    assert!(c.resistances.is_empty());
    assert!(c.immunities.is_empty());
    assert!(c.condition_immunities.is_empty());
    assert_eq!(c.passive_perception, 14);
    assert_eq!(c.summary, "Small beast, CR 1/8 (25 XP)");
    assert_eq!(c.proficiency_bonus, 2);
    let prov = c.provenance.expect("provenance");
    assert_eq!(prov.source, "creature-cauldron");
    assert_eq!(prov.external_id, "cinderkite-3a1f");
    assert_eq!(prov.attribution, "");
    assert!(c.abilities.is_empty(), "issue #106's job, not #105's");
    assert!(c.resources.is_empty());
    assert!(import.report.is_empty(), "nothing should need reporting for Cinderkite");
}

#[test]
fn dune_leviathan_a_high_cr_creature_with_large_numbers() {
    let import = load("monster-dune-leviathan.json");
    let c = import.creature;
    assert_eq!(c.name, "Dune Leviathan");
    assert_eq!(c.size, CreatureSize::Gargantuan as i32);
    assert_eq!(c.armor_class, 18);
    assert_eq!(c.hp_max, 350);
    assert_eq!(c.speed_feet, 40.0);
    assert_eq!(c.burrow_speed_feet, 40.0);
    assert_eq!(c.fly_speed_feet, 0.0);
    assert_eq!(c.save_proficiencies, vec![AbilityScore::Constitution as i32]);
    assert_eq!(c.condition_immunities, vec!["blinded".to_string()]);
    assert_eq!(c.passive_perception, 11);
    assert_eq!(c.summary, "Gargantuan monstrosity, CR 16 (15,000 XP)");
    // con 24 -> modifier 7, + proficiency 5 = 12, matching the printed save: no mismatch report,
    // and the no-home darkvision/blindsight/truesight senses are absent here, but tremorsense
    // has no field on Creature and should be reported.
    assert_eq!(import.report.len(), 1);
    assert_eq!(import.report[0].field, "senses.tremorsense");
}

#[test]
fn hollowvale_hexer_the_plainest_stat_block() {
    let import = load("monster-hollowvale-hexer.json");
    let c = import.creature;
    assert_eq!(c.name, "Hollowvale Hexer");
    assert_eq!(c.size, CreatureSize::Medium as i32);
    assert_eq!(c.armor_class, 12);
    assert_eq!(c.hp_max, 38);
    assert_eq!(c.speed_feet, 30.0);
    assert!(c.save_proficiencies.is_empty());
    assert!(c.skills.is_empty());
    assert_eq!(c.passive_perception, 11);
    assert_eq!(c.summary, "Medium humanoid, CR 2 (450 XP)");
    assert!(import.report.is_empty());
}

#[test]
fn stormhide_drake_multiattack_recharge_resistances() {
    let import = load("monster-stormhide-drake.json");
    let c = import.creature;
    assert_eq!(c.name, "Stormhide Drake");
    assert_eq!(c.size, CreatureSize::Large as i32);
    assert_eq!(c.armor_class, 15);
    assert_eq!(c.hp_max, 85);
    assert_eq!(c.speed_feet, 40.0);
    assert_eq!(c.fly_speed_feet, 60.0);
    assert_eq!(c.immunities, vec![DamageType::Lightning as i32]);
    assert!(c.vulnerabilities.is_empty());
    assert!(c.resistances.is_empty());
    let mut save_profs = c.save_proficiencies.clone();
    save_profs.sort();
    let mut expected = vec![AbilityScore::Dexterity as i32, AbilityScore::Constitution as i32];
    expected.sort();
    assert_eq!(save_profs, expected);
    assert_eq!(c.skills.len(), 2);
    assert!(c.skills.iter().any(|s| s.skill == "perception" && s.bonus == 3));
    assert!(c.skills.iter().any(|s| s.skill == "stealth" && s.bonus == 3));
    assert_eq!(c.passive_perception, 13);
    assert_eq!(c.summary, "Large dragon, CR 4 (1,100 XP)");
    // blindsight and darkvision both carry data with no home on Creature.
    assert_eq!(import.report.len(), 2);
    assert!(import.report.iter().any(|r| r.field == "senses.blindsight"));
    assert!(import.report.iter().any(|r| r.field == "senses.darkvision"));
}

#[test]
fn voidbloom_seer_legendary_actions_spellcasting_hover() {
    let import = load("monster-voidbloom-seer.json");
    let c = import.creature;
    assert_eq!(c.name, "Voidbloom Seer");
    assert_eq!(c.size, CreatureSize::Large as i32);
    assert_eq!(c.armor_class, 15);
    assert_eq!(c.hp_max, 114);
    assert_eq!(c.speed_feet, 0.0, "no walk speed: never borrowed from fly");
    assert_eq!(c.fly_speed_feet, 20.0);
    assert!(c.can_hover);
    assert_eq!(c.resistances, vec![DamageType::Psychic as i32]);
    assert_eq!(c.condition_immunities, vec!["prone".to_string()]);
    let mut save_profs = c.save_proficiencies.clone();
    save_profs.sort();
    let mut expected = vec![AbilityScore::Intelligence as i32, AbilityScore::Wisdom as i32];
    expected.sort();
    assert_eq!(save_profs, expected);
    assert_eq!(c.passive_perception, 13);
    assert_eq!(c.summary, "Large aberration, CR 7 (2,900 XP)");
    assert_eq!(import.report.len(), 1);
    assert_eq!(import.report[0].field, "senses.darkvision");
}

#[test]
fn every_sample_leaves_abilities_and_resources_for_issue_106() {
    for name in [
        "monster-cinderkite.json",
        "monster-dune-leviathan.json",
        "monster-hollowvale-hexer.json",
        "monster-stormhide-drake.json",
        "monster-voidbloom-seer.json",
    ] {
        let import = load(name);
        assert!(import.creature.abilities.is_empty(), "{name}");
        assert!(import.creature.resources.is_empty(), "{name}");
        assert_eq!(import.creature.creature_id, "", "{name}: server mints this");
    }
}
