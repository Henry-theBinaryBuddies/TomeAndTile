//! Exercises `import_monster` against the five real Creature Cauldron exports in
//! `tests/samples/`. One test per sample, each checking the exact mapped values per
//! MAPPING.md and issues #105/#106's acceptance tests; samples are read as-is and never edited.

use std::fs;
use std::path::PathBuf;

use tt_cauldron_import::{import_monster, Import};
use tt_contracts::v1::{effect_step, Ability, ActionCost, AbilityScore, CreatureKind, CreatureSize, DamageType, TargetShape};

fn load(name: &str) -> Import {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/samples")
        .join(name);
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
    import_monster(&bytes).unwrap_or_else(|e| panic!("{name} did not import: {e}"))
}

fn ability<'a>(abilities: &'a [Ability], name: &str) -> &'a Ability {
    abilities
        .iter()
        .find(|a| a.name == name)
        .unwrap_or_else(|| panic!("no ability named {name:?} (have {:?})", abilities.iter().map(|a| &a.name).collect::<Vec<_>>()))
}

fn attack_step(a: &Ability) -> &tt_contracts::v1::AttackStep {
    assert_eq!(a.steps.len(), 1, "{}: expected exactly one step", a.name);
    match a.steps[0].step.as_ref() {
        Some(effect_step::Step::Attack(s)) => s,
        other => panic!("{}: expected an AttackStep, got {other:?}", a.name),
    }
}

fn save_step(a: &Ability) -> &tt_contracts::v1::SaveStep {
    assert_eq!(a.steps.len(), 1, "{}: expected exactly one step", a.name);
    match a.steps[0].step.as_ref() {
        Some(effect_step::Step::Save(s)) => s,
        other => panic!("{}: expected a SaveStep, got {other:?}", a.name),
    }
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
    let scores = c.scores.as_ref().expect("scores");
    assert_eq!(
        (scores.strength, scores.dexterity, scores.constitution),
        (6, 16, 10)
    );
    assert_eq!(
        (scores.intelligence, scores.wisdom, scores.charisma),
        (2, 14, 6)
    );
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
    let prov = c.provenance.as_ref().expect("provenance");
    assert_eq!(prov.source, "creature-cauldron");
    assert_eq!(prov.external_id, "cinderkite-3a1f");
    assert_eq!(prov.attribution, "");

    // #106: one trait (passive) and one attack action.
    assert_eq!(c.abilities.len(), 2, "{:?}", c.abilities.iter().map(|a| &a.name).collect::<Vec<_>>());
    let sight = ability(&c.abilities, "Keen Sight");
    assert_eq!(sight.cost, ActionCost::Passive as i32);
    assert!(sight.steps.is_empty());
    assert!(sight.description.contains("advantage"));
    let talons = ability(&c.abilities, "Talons");
    assert_eq!(talons.ability_id, "talons");
    assert_eq!(talons.cost, ActionCost::Action as i32);
    assert_eq!(talons.provenance.as_ref().unwrap().source, "creature-cauldron");
    let step = attack_step(talons);
    assert_eq!(step.to_hit_bonus, 5);
    assert_eq!(step.damage.len(), 1);
    assert_eq!(step.damage[0].dice, "1d4+3");
    assert_eq!(step.damage[0].r#type, DamageType::Piercing as i32);
    let targeting = talons.targeting.as_ref().expect("targeting");
    assert_eq!(targeting.shape, TargetShape::Creatures as i32);
    assert_eq!(targeting.range_feet, 5.0);
    assert_eq!(targeting.max_targets, 1);
    assert!(c.resources.is_empty());

    assert!(
        import.report.is_empty(),
        "nothing should need reporting for Cinderkite: {:?}",
        import.report
    );
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
    assert_eq!(
        c.save_proficiencies,
        vec![AbilityScore::Constitution as i32]
    );
    assert_eq!(c.condition_immunities, vec!["blinded".to_string()]);
    assert_eq!(c.passive_perception, 11);
    assert_eq!(c.summary, "Gargantuan monstrosity, CR 16 (15,000 XP)");

    // Multiattack (no steps) plus a save ability, per the issue's acceptance list.
    assert_eq!(c.abilities.len(), 5, "{:?}", c.abilities.iter().map(|a| &a.name).collect::<Vec<_>>());
    let multi = ability(&c.abilities, "Multiattack");
    assert!(multi.steps.is_empty());
    assert_eq!(multi.description, "Bite once, then Tail once");
    let bite = ability(&c.abilities, "Bite");
    let step = attack_step(bite);
    assert_eq!(step.to_hit_bonus, 13);
    assert_eq!(step.damage[0].dice, "3d10+8");
    let geyser = ability(&c.abilities, "Sand Geyser");
    assert_eq!(geyser.recharge.as_ref().unwrap().min_roll, 5);
    let step = save_step(geyser);
    assert_eq!(step.ability, AbilityScore::Dexterity as i32);
    assert_eq!(step.dc, 20);
    assert_eq!(step.damage[0].dice, "10d10");
    assert!(step.half_on_success);
    let targeting = geyser.targeting.as_ref().expect("targeting");
    assert_eq!(targeting.shape, TargetShape::Line as i32);
    assert_eq!(targeting.size_feet, 60.0);
    assert_eq!(targeting.width_feet, 10.0);

    // con 24 -> modifier 7, + proficiency 5 = 12, matching the printed save: no mismatch report.
    // tremorsense has no field on Creature and is reported; multiattack is always reported.
    assert_eq!(import.report.len(), 2, "{:?}", import.report);
    assert!(import.report.iter().any(|r| r.field == "senses.tremorsense"));
    assert!(import.report.iter().any(|r| r.field == "Multiattack" && r.reason.contains("resolved by hand")));
}

#[test]
fn hollowvale_hexer_spell_slot_resources() {
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

    // 2 weapon/spell attacks, 1 passive "Spellcasting", 2 cantrips, 4 slot spells = 9.
    assert_eq!(c.abilities.len(), 9, "{:?}", c.abilities.iter().map(|a| &a.name).collect::<Vec<_>>());
    let dagger = ability(&c.abilities, "Dagger");
    let step = attack_step(dagger);
    assert_eq!(step.damage[0].dice, "1d4+1");
    let hex_bolt = ability(&c.abilities, "Hex Bolt");
    let step = attack_step(hex_bolt);
    assert_eq!(step.damage[0].dice, "2d8", "a zero bonus adds no +0 suffix");
    let targeting = hex_bolt.targeting.as_ref().expect("targeting");
    assert_eq!(targeting.range_feet, 120.0);
    assert_eq!(targeting.long_range_feet, 120.0);
    let spellcasting = ability(&c.abilities, "Spellcasting");
    assert_eq!(spellcasting.cost, ActionCost::Passive as i32);
    assert!(spellcasting.description.contains("4th-level spellcaster"));
    let charm_person = ability(&c.abilities, "charm person");
    assert_eq!(charm_person.resource_key, "spell_slot_1");
    assert_eq!(charm_person.resource_amount, 1);
    let hold_person = ability(&c.abilities, "hold person");
    assert_eq!(hold_person.resource_key, "spell_slot_2");
    let minor_illusion = ability(&c.abilities, "minor illusion");
    assert_eq!(minor_illusion.resource_key, "", "cantrips spend no resource");

    assert_eq!(c.resources.len(), 2);
    let slot1 = c.resources.iter().find(|r| r.key == "spell_slot_1").expect("spell_slot_1");
    assert_eq!((slot1.current, slot1.max), (4, 4));
    let slot2 = c.resources.iter().find(|r| r.key == "spell_slot_2").expect("spell_slot_2");
    assert_eq!((slot2.current, slot2.max), (3, 3));

    // Every one of the 6 spells (2 cantrips + 4 slotted) is reported: no mechanical effect in
    // the export. No other reports for this sample.
    assert_eq!(import.report.len(), 6, "{:?}", import.report);
    assert!(import.report.iter().all(|r| r.reason == "spell effect not in the export"));
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
    let mut expected = vec![
        AbilityScore::Dexterity as i32,
        AbilityScore::Constitution as i32,
    ];
    expected.sort();
    assert_eq!(save_profs, expected);
    assert_eq!(c.skills.len(), 2);
    assert!(c
        .skills
        .iter()
        .any(|s| s.skill == "perception" && s.bonus == 3));
    assert!(c
        .skills
        .iter()
        .any(|s| s.skill == "stealth" && s.bonus == 3));
    assert_eq!(c.passive_perception, 13);
    assert_eq!(c.summary, "Large dragon, CR 4 (1,100 XP)");

    // Static Hide (passive), Multiattack, Bite, Claw, Stormbreath.
    assert_eq!(c.abilities.len(), 5, "{:?}", c.abilities.iter().map(|a| &a.name).collect::<Vec<_>>());
    let bite = ability(&c.abilities, "Bite");
    let bite_step = attack_step(bite);
    assert_eq!(bite_step.to_hit_bonus, 6);
    assert_eq!(bite_step.damage[0].dice, "2d8+4");
    assert_eq!(bite_step.damage[0].r#type, DamageType::Piercing as i32);
    let bite_targeting = bite.targeting.as_ref().expect("targeting");
    // Same shape as the hand-written attack abilities in contracts/fixtures/characters/
    // (Longsword): TARGET_SHAPE_CREATURES, a range, exactly one target.
    assert_eq!(bite_targeting.shape, TargetShape::Creatures as i32);
    assert_eq!(bite_targeting.max_targets, 1);
    assert!(bite_targeting.range_feet > 0.0);

    let stormbreath = ability(&c.abilities, "Stormbreath");
    assert_eq!(stormbreath.recharge.as_ref().unwrap().min_roll, 5);
    let step = save_step(stormbreath);
    assert_eq!(step.ability, AbilityScore::Constitution as i32);
    assert_eq!(step.dc, 13);
    assert_eq!(step.damage.len(), 1);
    assert_eq!(step.damage[0].dice, "6d6", "a zero bonus adds no +0 suffix");
    assert_eq!(step.damage[0].r#type, DamageType::Lightning as i32);
    assert!(step.half_on_success);
    let targeting = stormbreath.targeting.as_ref().expect("targeting");
    // Same shape family as the hand-written cone save ability in
    // contracts/fixtures/characters/ (Fireball): an area shape, a size, a SaveStep.
    assert_eq!(targeting.shape, TargetShape::Line as i32);
    assert_eq!(targeting.size_feet, 30.0);
    assert_eq!(targeting.width_feet, 5.0);

    assert!(c.resources.is_empty());
    // blindsight and darkvision both carry data with no home on Creature; multiattack is
    // always reported as resolved by hand.
    assert_eq!(import.report.len(), 3, "{:?}", import.report);
    assert!(import.report.iter().any(|r| r.field == "senses.blindsight"));
    assert!(import.report.iter().any(|r| r.field == "senses.darkvision"));
    assert!(import.report.iter().any(|r| r.field == "Multiattack"));
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
    let mut expected = vec![
        AbilityScore::Intelligence as i32,
        AbilityScore::Wisdom as i32,
    ];
    expected.sort();
    assert_eq!(save_profs, expected);
    assert_eq!(c.passive_perception, 13);
    assert_eq!(c.summary, "Large aberration, CR 7 (2,900 XP)");

    // Multiattack, Mind Lash (attack), Dread Pulse (save), 3 legendary options (one of them
    // also named "Mind Lash" -- exercises ability_id uniqueness), Spellcasting, 2 at-will
    // spells, 1 per-day spell = 10.
    assert_eq!(c.abilities.len(), 10, "{:?}", c.abilities.iter().map(|a| &a.name).collect::<Vec<_>>());

    let mind_lash_attacks: Vec<&Ability> = c.abilities.iter().filter(|a| a.name == "Mind Lash").collect();
    assert_eq!(mind_lash_attacks.len(), 2, "one attack action, one legendary option, same name");
    let ids: Vec<&str> = mind_lash_attacks.iter().map(|a| a.ability_id.as_str()).collect();
    assert!(ids.contains(&"mind-lash"));
    assert!(ids.contains(&"mind-lash-2"), "the second 'Mind Lash' gets a -2 suffix: {ids:?}");

    let mind_lash_attack: &Ability = mind_lash_attacks.iter().find(|a| a.ability_id == "mind-lash").copied().unwrap();
    let step = attack_step(mind_lash_attack);
    assert_eq!(step.to_hit_bonus, 8);
    assert_eq!(step.damage[0].dice, "3d8+5");
    let targeting = mind_lash_attack.targeting.as_ref().expect("targeting");
    assert_eq!(targeting.max_targets, 0, "\"one creature\" is not \"one target\": left unset");
    assert_eq!(targeting.range_feet, 60.0);

    let mind_lash_legendary = mind_lash_attacks.iter().find(|a| a.ability_id == "mind-lash-2").unwrap();
    assert_eq!(mind_lash_legendary.cost, ActionCost::Legendary as i32);
    assert_eq!(mind_lash_legendary.resource_key, "legendary_actions");
    assert_eq!(mind_lash_legendary.resource_amount, 1);

    let unsettling = ability(&c.abilities, "Unsettling Gaze");
    assert_eq!(unsettling.resource_amount, 2);

    assert_eq!(c.resources.len(), 2);
    let legendary = c.resources.iter().find(|r| r.key == "legendary_actions").expect("legendary_actions");
    assert_eq!((legendary.current, legendary.max), (3, 3));
    let confusion_pool = c.resources.iter().find(|r| r.key == "confusion_per_day").expect("confusion_per_day");
    assert_eq!((confusion_pool.current, confusion_pool.max), (1, 1));

    let confusion = ability(&c.abilities, "confusion");
    assert_eq!(confusion.resource_key, "confusion_per_day");

    // darkvision (no home); the Mind Lash attack's "one creature" targets note; multiattack;
    // 3 spells (detect thoughts, mage hand, confusion) each "spell effect not in the export".
    assert_eq!(import.report.len(), 6, "{:?}", import.report);
    assert!(import.report.iter().any(|r| r.field == "senses.darkvision"));
    assert!(import.report.iter().any(|r| r.field == "Mind Lash.attack.targets"));
    assert!(import.report.iter().any(|r| r.field == "Multiattack"));
    assert_eq!(
        import.report.iter().filter(|r| r.reason == "spell effect not in the export").count(),
        3
    );
}

#[test]
fn every_sample_mints_no_creature_id_the_server_does_that() {
    for name in [
        "monster-cinderkite.json",
        "monster-dune-leviathan.json",
        "monster-hollowvale-hexer.json",
        "monster-stormhide-drake.json",
        "monster-voidbloom-seer.json",
    ] {
        let import = load(name);
        assert_eq!(import.creature.creature_id, "", "{name}: server mints this");
        for ability in &import.creature.abilities {
            assert!(!ability.ability_id.is_empty(), "{name}: every ability needs an id");
            assert_eq!(ability.provenance.as_ref().unwrap().source, "creature-cauldron", "{name}");
        }
    }
}
