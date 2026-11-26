//! Traits, actions, bonus actions, reactions, legendary actions and spellcasting, mapped to
//! `Creature.abilities`/`Creature.resources` (issue #106). Everything the contract cannot
//! express -- a condition stated only as prose, a structured multiattack step, a spell's actual
//! mechanical effect -- is reported, never guessed; see `MAPPING.md`'s "Traits, actions, and the
//! hybrid action model" and "Spellcasting" sections.
//!
//! Creature Cauldron's export schema has no structured field for "this attack also applies a
//! condition" -- only the free-text `effects_text`, which none of the five samples sets. So this
//! module never produces an `ApplyEffectStep`: there is nothing in the schema to read a
//! condition from as data. If that turns out to be wrong, it needs a real export that proves it,
//! not a guess (see the PR description).

use std::collections::HashMap;

use serde_json::Value;
use tt_contracts::v1::{
    effect_step, Ability, ActionCost, AttackStep, DamagePart, Provenance, Recharge, Resource,
    SaveStep, TargetShape, Targeting,
};

use crate::{
    damage_type, optional_bool, optional_f32, optional_i32, optional_str, ReportEntry,
    SIX_ABILITIES,
};

/// Builds `Creature.abilities` and `Creature.resources` from one monster payload, in file order:
/// traits, then actions, bonus actions, reactions, legendary actions and spellcasting.
pub(crate) fn build(
    monster: &Value,
    report: &mut Vec<ReportEntry>,
) -> (Vec<Ability>, Vec<Resource>) {
    let mut ids: HashMap<String, u32> = HashMap::new();
    let mut abilities = Vec::new();
    let mut resources = Vec::new();

    for (array_key, cost) in [
        ("traits", ActionCost::Passive),
        ("actions", ActionCost::Action),
        ("bonus_actions", ActionCost::BonusAction),
        ("reactions", ActionCost::Reaction),
    ] {
        for entry in array(monster, array_key) {
            if let Some(a) = action_ability(entry, cost, &mut ids, report, array_key) {
                abilities.push(a);
            }
        }
    }

    if let Some(legendary) = monster.get("legendary_actions").filter(|v| !v.is_null()) {
        legendary_actions(legendary, &mut ids, report, &mut abilities, &mut resources);
    }
    if let Some(spellcasting) = monster.get("spellcasting").filter(|v| !v.is_null()) {
        spellcasting_abilities(spellcasting, &mut ids, report, &mut abilities, &mut resources);
    }

    (abilities, resources)
}

fn array<'a>(monster: &'a Value, key: &str) -> &'a [Value] {
    monster
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// A slug of `name`: lower case, with every run of non-alphanumeric characters collapsed to one
/// `-` (leading/trailing dashes trimmed). Matches the convention already used in
/// `contracts/fixtures/characters/` ("Second Wind" -> "second-wind", "Fighting Style: Defense" ->
/// "fighting-style-defense").
fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// A spell name to a resource key fragment: lower case, runs of other characters collapsed to a
/// single `_` (MAPPING.md: `"Mage Hand"` -> `"mage_hand"`). Distinct from [`slug`]'s dashes,
/// which are for `ability_id` only -- a resource key is `spell_slot_1`/`legendary_actions`
/// shaped, underscore throughout.
fn resource_slug(name: &str) -> String {
    let mut out = String::new();
    let mut prev_underscore = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore && !out.is_empty() {
            out.push('_');
            prev_underscore = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

/// `ability_id`, unique within the creature: a second "Claw" gets `claw-2`, a third `claw-3`.
fn unique_id(ids: &mut HashMap<String, u32>, name: &str) -> String {
    let base = slug(name);
    let count = ids.entry(base.clone()).or_insert(0);
    *count += 1;
    if *count == 1 {
        base
    } else {
        format!("{base}-{}", *count)
    }
}

fn provenance() -> Provenance {
    Provenance {
        source: "creature-cauldron".to_string(),
        ..Default::default()
    }
}

fn required_name<'a>(
    entry: &'a Value,
    report: &mut Vec<ReportEntry>,
    array_key: &str,
) -> Option<&'a str> {
    match optional_str(entry, "name") {
        Some(n) => Some(n),
        None => {
            report.push(ReportEntry {
                field: format!("{array_key}[]"),
                reason: "entry has no name; skipped".to_string(),
            });
            None
        }
    }
}

fn recharge(entry: &Value) -> Option<Recharge> {
    entry
        .get("recharge")
        .and_then(|r| optional_i32(r, "min"))
        .map(|min| Recharge {
            min_roll: min.max(0) as u32,
        })
}

/// Prose in `effects_text`, when present, is never parsed into a condition: it is appended to
/// the ability's description and reported as not imported.
fn effects_text_description(entry: &Value, name: &str, report: &mut Vec<ReportEntry>) -> String {
    match optional_str(entry, "effects_text") {
        Some(text) if !text.is_empty() => {
            report.push(ReportEntry {
                field: format!("{name}.effects_text"),
                reason: "effect not imported".to_string(),
            });
            text.to_string()
        }
        _ => String::new(),
    }
}

/// `{dice}+{bonus}` / `{dice}{bonus}` / `{dice}`, per MAPPING.md: a positive bonus gets a `+`
/// sign, a negative one keeps its own `-`, and a zero bonus adds no suffix at all.
fn dice_string(dice: &str, bonus: i32) -> String {
    match bonus.cmp(&0) {
        std::cmp::Ordering::Greater => format!("{dice}+{bonus}"),
        std::cmp::Ordering::Less => format!("{dice}{bonus}"),
        std::cmp::Ordering::Equal => dice.to_string(),
    }
}

fn damage_parts(
    value: Option<&Value>,
    name: &str,
    report: &mut Vec<ReportEntry>,
) -> Vec<DamagePart> {
    let Some(Value::Array(list)) = value else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|roll| {
            let dice = roll.get("dice").and_then(Value::as_str)?;
            let bonus = optional_i32(roll, "bonus").unwrap_or(0);
            let type_name = roll.get("type").and_then(Value::as_str).unwrap_or("");
            match damage_type(type_name) {
                Some(d) => Some(DamagePart {
                    dice: dice_string(dice, bonus),
                    r#type: d as i32,
                }),
                None => {
                    report.push(ReportEntry {
                        field: format!("{name}.damage"),
                        reason: format!("{type_name} is not a known damage type"),
                    });
                    None
                }
            }
        })
        .collect()
}

/// `Targeting` for an attack action: always `TARGET_SHAPE_CREATURES`. `max_targets` is 1 only
/// when Cauldron's wording is exactly "one target"; any other wording is reported and left
/// unset (0) rather than guessed at.
fn attack_targeting(attack: &Value, name: &str, report: &mut Vec<ReportEntry>) -> Targeting {
    let range_feet = optional_f32(attack, "reach")
        .or_else(|| attack.get("range").and_then(|r| optional_f32(r, "normal")))
        .unwrap_or(0.0);
    let long_range_feet = attack
        .get("range")
        .and_then(|r| optional_f32(r, "long"))
        .unwrap_or(0.0);
    let targets = optional_str(attack, "targets").unwrap_or("");
    let max_targets = if targets == "one target" {
        1
    } else {
        report.push(ReportEntry {
            field: format!("{name}.attack.targets"),
            reason: format!(
                "targets wording \"{targets}\" is not \"one target\"; max_targets left unset"
            ),
        });
        0
    };
    Targeting {
        shape: TargetShape::Creatures as i32,
        range_feet,
        long_range_feet,
        max_targets,
        ..Default::default()
    }
}

/// `Targeting` for a save action's `area` (cone/line/cube/sphere; Cauldron's "radius" is a
/// sphere, confirmed with @Henry-theBinaryBuddies in MAPPING.md). No sample has a save ability
/// with no `area` at all; absent, this falls back to a single-creature target rather than
/// guessing a shape.
fn save_targeting(area: Option<&Value>, name: &str, report: &mut Vec<ReportEntry>) -> Targeting {
    let Some(area) = area else {
        return Targeting {
            shape: TargetShape::Creatures as i32,
            max_targets: 1,
            ..Default::default()
        };
    };
    let shape_str = optional_str(area, "shape").unwrap_or("");
    let shape = match shape_str {
        "cone" => Some(TargetShape::Cone),
        "line" => Some(TargetShape::Line),
        "cube" => Some(TargetShape::Cube),
        "radius" | "sphere" => Some(TargetShape::Sphere),
        _ => None,
    };
    let size_feet = optional_f32(area, "size").unwrap_or(0.0);
    let width_feet = optional_f32(area, "width").unwrap_or(0.0);
    match shape {
        Some(shape) => Targeting {
            shape: shape as i32,
            size_feet,
            width_feet,
            ..Default::default()
        },
        None => {
            report.push(ReportEntry {
                field: format!("{name}.area.shape"),
                reason: format!("\"{shape_str}\" is not a known area shape"),
            });
            Targeting::default()
        }
    }
}

type AbilityFields = (String, Vec<tt_contracts::v1::EffectStep>, Option<Targeting>);

fn attack_fields(
    entry: &Value,
    name: &str,
    report: &mut Vec<ReportEntry>,
) -> Option<AbilityFields> {
    let Some(attack) = entry.get("attack") else {
        report.push(ReportEntry {
            field: name.to_string(),
            reason: "attack action has no \"attack\" object; skipped".to_string(),
        });
        return None;
    };
    let Some(to_hit) = optional_i32(attack, "to_hit") else {
        report.push(ReportEntry {
            field: format!("{name}.attack.to_hit"),
            reason: "missing to_hit; ability skipped".to_string(),
        });
        return None;
    };
    let damage = damage_parts(entry.get("damage"), name, report);
    let targeting = attack_targeting(attack, name, report);
    let description = effects_text_description(entry, name, report);
    let step = tt_contracts::v1::EffectStep {
        step: Some(effect_step::Step::Attack(AttackStep {
            to_hit_bonus: to_hit,
            damage,
        })),
    };
    Some((description, vec![step], Some(targeting)))
}

fn save_fields(entry: &Value, name: &str, report: &mut Vec<ReportEntry>) -> Option<AbilityFields> {
    let Some(save) = entry.get("save") else {
        report.push(ReportEntry {
            field: name.to_string(),
            reason: "save action has no \"save\" object; skipped".to_string(),
        });
        return None;
    };
    let ability_str = optional_str(save, "ability").unwrap_or("");
    let Some((_, _, ability_score)) = SIX_ABILITIES.iter().find(|(k, _, _)| *k == ability_str)
    else {
        report.push(ReportEntry {
            field: format!("{name}.save.ability"),
            reason: format!("\"{ability_str}\" is not a known ability; ability skipped"),
        });
        return None;
    };
    let Some(dc) = optional_i32(save, "dc") else {
        report.push(ReportEntry {
            field: format!("{name}.save.dc"),
            reason: "missing dc; ability skipped".to_string(),
        });
        return None;
    };
    let damage = damage_parts(entry.get("on_fail"), name, report);
    let half_on_success = optional_str(entry, "on_success") == Some("half");
    let targeting = save_targeting(entry.get("area"), name, report);
    let description = effects_text_description(entry, name, report);
    let step = tt_contracts::v1::EffectStep {
        step: Some(effect_step::Step::Save(SaveStep {
            ability: *ability_score as i32,
            dc,
            damage,
            half_on_success,
        })),
    };
    Some((description, vec![step], Some(targeting)))
}

fn count_phrase(count: i32) -> String {
    match count {
        1 => "once".to_string(),
        2 => "twice".to_string(),
        n => format!("{n} times"),
    }
}

/// No `EffectStep` can express "use these other abilities, this many times each" (MAPPING.md gap
/// 5): a plain description built from `sequence`, always reported as resolved by hand.
fn multiattack_fields(entry: &Value, name: &str, report: &mut Vec<ReportEntry>) -> AbilityFields {
    let sequence = entry.get("sequence").and_then(Value::as_array);
    let parts: Vec<String> = sequence
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let action = optional_str(s, "action")?;
            let count = optional_i32(s, "count").unwrap_or(1);
            Some(format!("{action} {}", count_phrase(count)))
        })
        .collect();
    report.push(ReportEntry {
        field: name.to_string(),
        reason: "multiattack resolved by hand".to_string(),
    });
    (parts.join(", then "), Vec::new(), None)
}

fn text_fields(entry: &Value) -> AbilityFields {
    (optional_str(entry, "text").unwrap_or("").to_string(), Vec::new(), None)
}

/// One entry from `traits`, `actions`, `bonus_actions` or `reactions` -- traits are always
/// `kind: "text"`, the others are the hybrid action union (MAPPING.md). `None` means the entry
/// was reported and skipped; the rest of the file still imports.
fn action_ability(
    entry: &Value,
    cost: ActionCost,
    ids: &mut HashMap<String, u32>,
    report: &mut Vec<ReportEntry>,
    array_key: &str,
) -> Option<Ability> {
    let name = required_name(entry, report, array_key)?;
    let ability_id = unique_id(ids, name);
    let kind = optional_str(entry, "kind").unwrap_or("");
    let (description, steps, targeting) = match kind {
        "attack" => attack_fields(entry, name, report)?,
        "save" => save_fields(entry, name, report)?,
        "multiattack" => multiattack_fields(entry, name, report),
        "text" => text_fields(entry),
        other => {
            report.push(ReportEntry {
                field: name.to_string(),
                reason: format!("unknown action kind \"{other}\"; skipped"),
            });
            return None;
        }
    };
    Some(Ability {
        ability_id,
        name: name.to_string(),
        description,
        cost: cost as i32,
        resource_key: String::new(),
        resource_amount: 0,
        targeting,
        steps,
        provenance: Some(provenance()),
        recharge: recharge(entry),
        recharge_spent: false,
    })
}

fn legendary_actions(
    legendary: &Value,
    ids: &mut HashMap<String, u32>,
    report: &mut Vec<ReportEntry>,
    abilities: &mut Vec<Ability>,
    resources: &mut Vec<Resource>,
) {
    let per_round = optional_i32(legendary, "per_round").unwrap_or(0);
    resources.push(Resource {
        key: "legendary_actions".to_string(),
        label: "Legendary Actions".to_string(),
        current: per_round,
        max: per_round,
    });
    for option in array(legendary, "options") {
        let Some(name) = required_name(option, report, "legendary_actions.options") else {
            continue;
        };
        let ability_id = unique_id(ids, name);
        let cost = optional_i32(option, "cost").unwrap_or(1);
        abilities.push(Ability {
            ability_id,
            name: name.to_string(),
            description: optional_str(option, "text").unwrap_or("").to_string(),
            cost: ActionCost::Legendary as i32,
            resource_key: "legendary_actions".to_string(),
            resource_amount: cost,
            targeting: None,
            steps: Vec::new(),
            provenance: Some(provenance()),
            recharge: None,
            recharge_spent: false,
        });
    }
}

/// One spell, with no `EffectStep`: the export gives only its name, never its mechanical effect
/// (MAPPING.md's "Spellcasting" section). Always reported, since this is a permanent gap in
/// Creature Cauldron's own export, not something this importer could fill in.
fn spell_ability(
    spell: &Value,
    ids: &mut HashMap<String, u32>,
    report: &mut Vec<ReportEntry>,
    abilities: &mut Vec<Ability>,
    resource_key: &str,
    resource_amount: i32,
    array_key: &str,
) {
    let Some(name) = required_name(spell, report, array_key) else {
        return;
    };
    report.push(ReportEntry {
        field: name.to_string(),
        reason: "spell effect not in the export".to_string(),
    });
    abilities.push(Ability {
        ability_id: unique_id(ids, name),
        name: name.to_string(),
        description: String::new(),
        cost: ActionCost::Action as i32,
        resource_key: resource_key.to_string(),
        resource_amount,
        targeting: None,
        steps: Vec::new(),
        provenance: Some(provenance()),
        recharge: None,
        recharge_spent: false,
    });
}

fn spellcasting_abilities(
    spellcasting: &Value,
    ids: &mut HashMap<String, u32>,
    report: &mut Vec<ReportEntry>,
    abilities: &mut Vec<Ability>,
    resources: &mut Vec<Resource>,
) {
    let intro = optional_str(spellcasting, "intro").unwrap_or("");
    let footer = optional_str(spellcasting, "footer").unwrap_or("");
    let description = if footer.is_empty() {
        intro.to_string()
    } else {
        format!("{intro}\n\n{footer}")
    };
    if !description.is_empty() {
        abilities.push(Ability {
            ability_id: unique_id(ids, "Spellcasting"),
            name: "Spellcasting".to_string(),
            description,
            cost: ActionCost::Passive as i32,
            resource_key: String::new(),
            resource_amount: 0,
            targeting: None,
            steps: Vec::new(),
            provenance: Some(provenance()),
            recharge: None,
            recharge_spent: false,
        });
    }

    for spell in array(spellcasting, "cantrips") {
        spell_ability(spell, ids, report, abilities, "", 0, "spellcasting.cantrips");
    }
    for spell in array(spellcasting, "at_will") {
        spell_ability(spell, ids, report, abilities, "", 0, "spellcasting.at_will");
    }
    for level_entry in array(spellcasting, "slots") {
        let level = optional_i32(level_entry, "level").unwrap_or(0);
        let slots = optional_i32(level_entry, "slots").unwrap_or(0);
        let key = format!("spell_slot_{level}");
        resources.push(Resource {
            key: key.clone(),
            label: format!("Level {level} Slots"),
            current: slots,
            max: slots,
        });
        for spell in array(level_entry, "spells") {
            spell_ability(spell, ids, report, abilities, &key, 1, "spellcasting.slots.spells");
        }
    }
    for per_day in array(spellcasting, "per_day") {
        let uses = optional_i32(per_day, "uses").unwrap_or(0);
        let each = optional_bool(per_day, "each");
        let spells = array(per_day, "spells");
        if each {
            for spell in spells {
                let Some(spell_name) = optional_str(spell, "name") else { continue };
                let key = format!("{}_per_day", resource_slug(spell_name));
                resources.push(Resource {
                    key: key.clone(),
                    label: spell_name.to_string(),
                    current: uses,
                    max: uses,
                });
                spell_ability(
                    spell,
                    ids,
                    report,
                    abilities,
                    &key,
                    1,
                    "spellcasting.per_day.spells",
                );
            }
        } else {
            let names: Vec<&str> = spells.iter().filter_map(|s| optional_str(s, "name")).collect();
            if names.is_empty() {
                continue;
            }
            let key = format!(
                "{}_per_day",
                names
                    .iter()
                    .map(|n| resource_slug(n))
                    .collect::<Vec<_>>()
                    .join("_")
            );
            resources.push(Resource {
                key: key.clone(),
                label: names.join(" / "),
                current: uses,
                max: uses,
            });
            for spell in spells {
                spell_ability(
                    spell,
                    ids,
                    report,
                    abilities,
                    &key,
                    1,
                    "spellcasting.per_day.spells",
                );
            }
        }
    }
}
