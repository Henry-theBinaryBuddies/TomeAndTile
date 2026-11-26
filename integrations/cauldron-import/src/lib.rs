//! Creature Cauldron importer (issue #105: envelope, identity, core stat block).
//!
//! [`import_monster`] is a pure function from the bytes of a Creature Cauldron monster export
//! to a [`Creature`] plus a report of what did not come across. No network, no file system, no
//! clock, no global state, no randomness: `exported_at` is never read. Traits, actions and
//! spellcasting ([`Creature::abilities`], [`Creature::resources`]) are issue #106's job and are
//! left empty here. See `MAPPING.md` for the full field-by-field design this follows, and
//! docs/adr/0010-integrations.md.

use std::fmt;

use serde_json::Value;
use tt_contracts::v1::{
    AbilityScore, AbilityScores, Creature, CreatureKind, CreatureSize, DamageType, Provenance,
    SkillBonus,
};

/// A file above this size is refused before anything is parsed. The largest sample in
/// `tests/samples/` is about 5 KiB.
pub const MAX_FILE_BYTES: usize = 256 * 1024;
/// Deepest a JSON value (nested objects/arrays) may go before the file is refused.
pub const MAX_NESTING_DEPTH: usize = 32;
/// Longest a JSON array or object (by entry count) may be before the file is refused.
pub const MAX_LIST_LEN: usize = 256;
/// Longest a single JSON string value may be before the file is refused.
pub const MAX_STRING_LEN: usize = 4096;

/// One field the importer could not bring across, for the caller to show or log. Never changes
/// what was imported; see `monster.saves` in `MAPPING.md` for the one case (a save-bonus
/// mismatch) that reports without dropping anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportEntry {
    /// Dotted path into the source file, e.g. `"senses.darkvision"`.
    pub field: String,
    pub reason: String,
}

/// What [`import_monster`] produces on success.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub creature: Creature,
    pub report: Vec<ReportEntry>,
}

/// Why a file was refused outright. Every variant names the value or field involved, so the
/// message never has to guess at what went wrong.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportError {
    TooLarge {
        max_bytes: usize,
        actual_bytes: usize,
    },
    InvalidUtf8,
    InvalidJson(String),
    WrongType {
        field: String,
        expected: &'static str,
    },
    NestingTooDeep {
        max_depth: usize,
    },
    ListTooLong {
        field: String,
        max_len: usize,
    },
    StringTooLong {
        field: String,
        max_len: usize,
    },
    WrongApp(String),
    UnsupportedKind(String),
    WrongFormatVersion(String),
    MissingField(String),
    UnknownSize(String),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::TooLarge { max_bytes, actual_bytes } => write!(
                f,
                "the file is {actual_bytes} bytes, over the {max_bytes}-byte limit"
            ),
            ImportError::InvalidUtf8 => write!(f, "the file is not valid UTF-8"),
            ImportError::InvalidJson(message) => write!(f, "the file is not valid JSON: {message}"),
            ImportError::WrongType { field, expected } => {
                write!(f, "{field} should be a {expected}")
            }
            ImportError::NestingTooDeep { max_depth } => {
                write!(f, "the file nests more than {max_depth} levels deep")
            }
            ImportError::ListTooLong { field, max_len } => {
                write!(f, "{field} has more than {max_len} entries")
            }
            ImportError::StringTooLong { field, max_len } => {
                write!(f, "{field} is longer than {max_len} characters")
            }
            ImportError::WrongApp(found) => write!(
                f,
                "this file is not a Creature Cauldron export (app: {found})"
            ),
            ImportError::UnsupportedKind(found) => {
                write!(f, "kind {found} is not supported yet (only \"monster\" is)")
            }
            ImportError::WrongFormatVersion(found) => write!(
                f,
                "format_version {found} is not supported (only 1 is); the importer may need updating"
            ),
            ImportError::MissingField(field) => write!(f, "{field} is required but missing"),
            ImportError::UnknownSize(found) => write!(f, "{found} is not a known creature size"),
        }
    }
}

impl std::error::Error for ImportError {}

/// Imports one Creature Cauldron monster export. Pure: no network, no file system, no clock, no
/// global state, no randomness, and nothing in the file is executed or evaluated.
pub fn import_monster(bytes: &[u8]) -> Result<Import, ImportError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge {
            max_bytes: MAX_FILE_BYTES,
            actual_bytes: bytes.len(),
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ImportError::InvalidUtf8)?;
    let root: Value =
        serde_json::from_str(text).map_err(|e| ImportError::InvalidJson(e.to_string()))?;
    check_limits(&root, "$", 0)?;

    // Envelope: app, kind, format_version, checked in that order before anything else is read.
    let app = require_str(&root, "app")?;
    if app != "Creature Cauldron" {
        return Err(ImportError::WrongApp(describe(&root["app"])));
    }
    let kind = require_str(&root, "kind")?;
    if kind != "monster" {
        return Err(ImportError::UnsupportedKind(describe(&root["kind"])));
    }
    let format_version = root.get("format_version").ok_or_else(|| ImportError::MissingField("format_version".into()))?;
    if format_version.as_i64() != Some(1) {
        return Err(ImportError::WrongFormatVersion(describe(format_version)));
    }

    let monster = root.get("monster").ok_or_else(|| ImportError::MissingField("monster".into()))?;
    if !monster.is_object() {
        return Err(ImportError::WrongType { field: "monster".into(), expected: "object" });
    }

    import_stat_block(monster)
}

/// Walks the whole parsed document once, before any field is read, rejecting nesting, list and
/// string sizes a hostile file could use to waste time or memory.
fn check_limits(value: &Value, path: &str, depth: usize) -> Result<(), ImportError> {
    if depth > MAX_NESTING_DEPTH {
        return Err(ImportError::NestingTooDeep { max_depth: MAX_NESTING_DEPTH });
    }
    match value {
        Value::String(s) => {
            if s.chars().count() > MAX_STRING_LEN {
                return Err(ImportError::StringTooLong {
                    field: path.to_string(),
                    max_len: MAX_STRING_LEN,
                });
            }
        }
        Value::Array(items) => {
            if items.len() > MAX_LIST_LEN {
                return Err(ImportError::ListTooLong { field: path.to_string(), max_len: MAX_LIST_LEN });
            }
            for (i, item) in items.iter().enumerate() {
                check_limits(item, &format!("{path}[{i}]"), depth + 1)?;
            }
        }
        Value::Object(map) => {
            if map.len() > MAX_LIST_LEN {
                return Err(ImportError::ListTooLong { field: path.to_string(), max_len: MAX_LIST_LEN });
            }
            for (k, v) in map {
                check_limits(v, &format!("{path}.{k}"), depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn require_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, ImportError> {
    match value.get(key) {
        Some(Value::String(s)) => Ok(s.as_str()),
        Some(_) => Err(ImportError::WrongType { field: key.to_string(), expected: "string" }),
        None => Err(ImportError::MissingField(key.to_string())),
    }
}

fn optional_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    match value.get(key) {
        Some(Value::String(s)) => Some(s.as_str()),
        _ => None,
    }
}

fn require_i32(value: &Value, path: &str, key: &str) -> Result<i32, ImportError> {
    let field = format!("{path}.{key}");
    match value.get(key) {
        Some(Value::Number(n)) => n
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .ok_or(ImportError::WrongType { field, expected: "integer" }),
        Some(_) => Err(ImportError::WrongType { field, expected: "integer" }),
        None => Err(ImportError::MissingField(field)),
    }
}

fn optional_i32(value: &Value, key: &str) -> Option<i32> {
    value.get(key).and_then(Value::as_i64).and_then(|n| i32::try_from(n).ok())
}

fn optional_f32(value: &Value, key: &str) -> Option<f32> {
    value.get(key).and_then(Value::as_f64).map(|n| n as f32)
}

fn optional_bool(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// A human-readable stand-in for a found JSON value, for an error message. `null`/missing reads
/// as the literal word `null`.
fn describe(value: &Value) -> String {
    if value.is_null() {
        "null".to_string()
    } else {
        value.to_string()
    }
}

const SIX_ABILITIES: [(&str, fn(&AbilityScores) -> i32, AbilityScore); 6] = [
    ("str", |s| s.strength, AbilityScore::Strength),
    ("dex", |s| s.dexterity, AbilityScore::Dexterity),
    ("con", |s| s.constitution, AbilityScore::Constitution),
    ("int", |s| s.intelligence, AbilityScore::Intelligence),
    ("wis", |s| s.wisdom, AbilityScore::Wisdom),
    ("cha", |s| s.charisma, AbilityScore::Charisma),
];

/// The eighteen SRD skills, by Creature Cauldron's title-case name, to the contract's
/// lower-case-with-underscores skill id.
const SKILL_IDS: [(&str, &str); 18] = [
    ("Acrobatics", "acrobatics"),
    ("Animal Handling", "animal_handling"),
    ("Arcana", "arcana"),
    ("Athletics", "athletics"),
    ("Deception", "deception"),
    ("History", "history"),
    ("Insight", "insight"),
    ("Intimidation", "intimidation"),
    ("Investigation", "investigation"),
    ("Medicine", "medicine"),
    ("Nature", "nature"),
    ("Perception", "perception"),
    ("Performance", "performance"),
    ("Persuasion", "persuasion"),
    ("Religion", "religion"),
    ("Sleight of Hand", "sleight_of_hand"),
    ("Stealth", "stealth"),
    ("Survival", "survival"),
];

fn damage_type(name: &str) -> Option<DamageType> {
    Some(match name.to_ascii_lowercase().as_str() {
        "acid" => DamageType::Acid,
        "bludgeoning" => DamageType::Bludgeoning,
        "cold" => DamageType::Cold,
        "fire" => DamageType::Fire,
        "force" => DamageType::Force,
        "lightning" => DamageType::Lightning,
        "necrotic" => DamageType::Necrotic,
        "piercing" => DamageType::Piercing,
        "poison" => DamageType::Poison,
        "psychic" => DamageType::Psychic,
        "radiant" => DamageType::Radiant,
        "slashing" => DamageType::Slashing,
        "thunder" => DamageType::Thunder,
        _ => return None,
    })
}

/// A D&D ability modifier: floor((score - 10) / 2), correct for scores below 10 too.
fn ability_modifier(score: i32) -> i32 {
    (score - 10).div_euclid(2)
}

fn format_thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let bytes = digits.as_bytes();
    let mut out = String::new();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

fn import_stat_block(monster: &Value) -> Result<Import, ImportError> {
    let mut report = Vec::new();

    let id = require_str(monster, "id")?;
    let name = require_str(monster, "name")?;

    let attribution = optional_str(monster, "attribution").unwrap_or("");
    let origin_notes = origin_notes(monster);

    let size_str = require_str(monster, "size")?;
    let size = creature_size(size_str).ok_or_else(|| ImportError::UnknownSize(size_str.to_string()))?;

    let armor_class_obj = monster
        .get("armor_class")
        .ok_or_else(|| ImportError::MissingField("armor_class".into()))?;
    let armor_class = require_i32(armor_class_obj, "armor_class", "value")?;

    let hit_points_obj = monster
        .get("hit_points")
        .ok_or_else(|| ImportError::MissingField("hit_points".into()))?;
    let hp_max = require_i32(hit_points_obj, "hit_points", "average")?;

    let speeds = monster.get("speeds");
    let speed_feet = speeds.and_then(|s| optional_f32(s, "walk")).unwrap_or(0.0);
    let fly_speed_feet = speeds.and_then(|s| optional_f32(s, "fly")).unwrap_or(0.0);
    let swim_speed_feet = speeds.and_then(|s| optional_f32(s, "swim")).unwrap_or(0.0);
    let climb_speed_feet = speeds.and_then(|s| optional_f32(s, "climb")).unwrap_or(0.0);
    let burrow_speed_feet = speeds.and_then(|s| optional_f32(s, "burrow")).unwrap_or(0.0);
    let can_hover = speeds.map(|s| optional_bool(s, "hover")).unwrap_or(false);

    let abilities_obj = monster
        .get("abilities")
        .ok_or_else(|| ImportError::MissingField("abilities".into()))?;
    let scores = AbilityScores {
        strength: require_i32(abilities_obj, "abilities", "str")?,
        dexterity: require_i32(abilities_obj, "abilities", "dex")?,
        constitution: require_i32(abilities_obj, "abilities", "con")?,
        intelligence: require_i32(abilities_obj, "abilities", "int")?,
        wisdom: require_i32(abilities_obj, "abilities", "wis")?,
        charisma: require_i32(abilities_obj, "abilities", "cha")?,
    };

    let challenge_obj = monster
        .get("challenge")
        .ok_or_else(|| ImportError::MissingField("challenge".into()))?;
    let proficiency_bonus = require_i32(challenge_obj, "challenge", "proficiency_bonus")?;

    let saves_obj = monster.get("saves");
    let mut save_proficiencies = Vec::new();
    if let Some(saves_obj) = saves_obj {
        for (key, modifier_fn, ability_score) in SIX_ABILITIES {
            let Some(printed) = optional_i32(saves_obj, key) else { continue };
            save_proficiencies.push(ability_score);
            let expected = ability_modifier(modifier_fn(&scores)) + proficiency_bonus;
            if expected != printed {
                report.push(ReportEntry {
                    field: format!("saves.{key}"),
                    reason: format!(
                        "printed save bonus {printed} does not match ability modifier + proficiency bonus ({expected})"
                    ),
                });
            }
        }
    }

    let mut skills = Vec::new();
    if let Some(Value::Object(skills_obj)) = monster.get("skills") {
        for (name, bonus) in skills_obj {
            let Some(bonus) = bonus.as_i64().and_then(|n| i32::try_from(n).ok()) else {
                report.push(ReportEntry {
                    field: format!("skills.{name}"),
                    reason: "skill bonus is not an integer".to_string(),
                });
                continue;
            };
            match SKILL_IDS.iter().find(|(cauldron_name, _)| *cauldron_name == name.as_str()) {
                Some((_, skill_id)) => skills.push(SkillBonus { skill: (*skill_id).to_string(), bonus }),
                None => report.push(ReportEntry {
                    field: format!("skills.{name}"),
                    reason: "not one of the 18 SRD skills".to_string(),
                }),
            }
        }
    }

    let (vulnerabilities, resistances, immunities) = (
        damage_type_list(monster, "vulnerabilities", &mut report),
        damage_type_list(monster, "resistances", &mut report),
        damage_type_list(monster, "immunities", &mut report),
    );

    let condition_immunities: Vec<String> = monster
        .get("condition_immunities")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();

    let senses = monster.get("senses");
    let passive_perception = senses.and_then(|s| optional_i32(s, "passive_perception")).unwrap_or(0);
    if let Some(senses) = senses {
        for key in ["darkvision", "blindsight", "tremorsense", "truesight"] {
            if let Some(range) = optional_i32(senses, key) {
                report.push(ReportEntry {
                    field: format!("senses.{key}"),
                    reason: format!("{range} ft, but Creature has no field for this sense"),
                });
            }
        }
    }

    let summary = summary(monster);

    let creature = Creature {
        creature_id: String::new(),
        kind: CreatureKind::Monster as i32,
        name: name.to_string(),
        edition: 1, // RulesEdition::RULES_EDITION_2014 -- all Cauldron monsters use the 2014 rules.
        size: size as i32,
        scores: Some(scores),
        proficiency_bonus,
        armor_class,
        hp_current: hp_max,
        hp_max,
        hp_temp: 0,
        speed_feet,
        initiative_bonus: 0,
        save_proficiencies: save_proficiencies.into_iter().map(|a| a as i32).collect(),
        resistances: resistances.into_iter().map(|d| d as i32).collect(),
        immunities: immunities.into_iter().map(|d| d as i32).collect(),
        vulnerabilities: vulnerabilities.into_iter().map(|d| d as i32).collect(),
        resources: Vec::new(),
        abilities: Vec::new(),
        conditions: Vec::new(),
        summary,
        portrait: None,
        provenance: Some(Provenance {
            source: "creature-cauldron".to_string(),
            attribution: attribution.to_string(),
            external_id: id.to_string(),
            origin_notes,
        }),
        owner_user_id: String::new(),
        fly_speed_feet,
        swim_speed_feet,
        climb_speed_feet,
        burrow_speed_feet,
        can_hover,
        skills,
        passive_perception,
        condition_immunities,
    };

    Ok(Import { creature, report })
}

fn origin_notes(monster: &Value) -> String {
    let parent_id = optional_str(monster, "parent_id");
    let generator = optional_str(monster, "generator");
    let changes: Vec<&str> = monster
        .get("changes")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut note = match (parent_id, generator) {
        (Some(parent_id), Some(generator)) => format!("reskin of {parent_id} via {generator}"),
        (Some(parent_id), None) => format!("reskin of {parent_id}"),
        (None, Some(generator)) => format!("generated via {generator}"),
        (None, None) => String::new(),
    };
    if !changes.is_empty() {
        if !note.is_empty() {
            note.push_str("; ");
        }
        note.push_str("changes: ");
        note.push_str(&changes.join(", "));
    }
    note
}

fn damage_type_list(monster: &Value, key: &str, report: &mut Vec<ReportEntry>) -> Vec<DamageType> {
    let Some(Value::Array(list)) = monster.get(key) else { return Vec::new() };
    list.iter()
        .filter_map(|v| {
            let name = v.as_str()?;
            match damage_type(name) {
                Some(d) => Some(d),
                None => {
                    report.push(ReportEntry {
                        field: format!("{key}[]"),
                        reason: format!("{name} is not a known damage type"),
                    });
                    None
                }
            }
        })
        .collect()
}

fn creature_size(name: &str) -> Option<CreatureSize> {
    Some(match name {
        "tiny" => CreatureSize::Tiny,
        "small" => CreatureSize::Small,
        "medium" => CreatureSize::Medium,
        "large" => CreatureSize::Large,
        "huge" => CreatureSize::Huge,
        "gargantuan" => CreatureSize::Gargantuan,
        _ => return None,
    })
}

/// "Large dragon, CR 4 (1,100 XP)": `monster.type`, the creature's size, and challenge rating
/// plus XP, folded into one display-only line (`monster.subtypes`, `.alignment` and
/// `armor_class.note` are safely dropped, per MAPPING.md).
fn summary(monster: &Value) -> String {
    let size = optional_str(monster, "size").unwrap_or("");
    let size_titlecase = size
        .get(0..1)
        .map(|first| first.to_ascii_uppercase() + &size[1..])
        .unwrap_or_default();
    let creature_type = optional_str(monster, "type");
    let challenge = monster.get("challenge");
    let cr = challenge.and_then(|c| optional_str(c, "cr"));
    let xp = challenge.and_then(|c| optional_i32(c, "xp"));

    let mut parts = Vec::new();
    match creature_type {
        Some(t) => parts.push(format!("{size_titlecase} {t}")),
        None => parts.push(size_titlecase),
    }
    if let Some(cr) = cr {
        match xp {
            Some(xp) => parts.push(format!("CR {cr} ({} XP)", format_thousands(xp as i64))),
            None => parts.push(format!("CR {cr}")),
        }
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(monster: Value) -> Vec<u8> {
        serde_json::json!({
            "app": "Creature Cauldron",
            "kind": "monster",
            "format_version": 1,
            "exported_at": "2026-11-12T22:00:00.000Z",
            "monster": monster
        })
        .to_string()
        .into_bytes()
    }

    fn minimal_monster() -> Value {
        serde_json::json!({
            "id": "test-1",
            "name": "Test Creature",
            "size": "medium",
            "armor_class": { "value": 10 },
            "hit_points": { "average": 5 },
            "speeds": { "walk": 30 },
            "abilities": { "str": 10, "dex": 10, "con": 10, "int": 10, "wis": 10, "cha": 10 },
            "challenge": { "cr": "0", "proficiency_bonus": 2 },
        })
    }

    #[test]
    fn oversized_input_is_rejected_before_parsing() {
        let bytes = vec![b' '; MAX_FILE_BYTES + 1];
        assert_eq!(
            import_monster(&bytes),
            Err(ImportError::TooLarge { max_bytes: MAX_FILE_BYTES, actual_bytes: MAX_FILE_BYTES + 1 })
        );
    }

    #[test]
    fn invalid_utf8_is_rejected_not_panicked_on() {
        assert_eq!(import_monster(&[0xff, 0xfe, 0xfd]), Err(ImportError::InvalidUtf8));
    }

    #[test]
    fn truncated_json_is_rejected() {
        let result = import_monster(b"{\"app\": \"Creature Cau");
        assert!(matches!(result, Err(ImportError::InvalidJson(_))));
    }

    #[test]
    fn the_wrong_app_is_rejected_before_anything_else_is_read() {
        let bytes = serde_json::json!({ "app": "Something Else", "kind": "monster", "format_version": 1 })
            .to_string()
            .into_bytes();
        assert_eq!(import_monster(&bytes), Err(ImportError::WrongApp("\"Something Else\"".to_string())));
    }

    #[test]
    fn an_encounter_export_is_rejected_by_name() {
        let bytes = serde_json::json!({ "app": "Creature Cauldron", "kind": "encounter", "format_version": 1 })
            .to_string()
            .into_bytes();
        assert_eq!(import_monster(&bytes), Err(ImportError::UnsupportedKind("\"encounter\"".to_string())));
    }

    #[test]
    fn a_future_format_version_is_rejected() {
        let bytes = serde_json::json!({ "app": "Creature Cauldron", "kind": "monster", "format_version": 2 })
            .to_string()
            .into_bytes();
        assert_eq!(import_monster(&bytes), Err(ImportError::WrongFormatVersion("2".to_string())));
    }

    #[test]
    fn a_missing_required_field_names_itself() {
        let mut monster = minimal_monster();
        monster.as_object_mut().unwrap().remove("name");
        let bytes = envelope(monster);
        assert_eq!(import_monster(&bytes), Err(ImportError::MissingField("name".to_string())));
    }

    #[test]
    fn an_unknown_size_string_is_an_error_never_a_default() {
        let mut monster = minimal_monster();
        monster["size"] = Value::String("colossal".to_string());
        let bytes = envelope(monster);
        assert_eq!(import_monster(&bytes), Err(ImportError::UnknownSize("colossal".to_string())));
    }

    #[test]
    fn deep_nesting_is_rejected() {
        let mut value = Value::String("x".to_string());
        for _ in 0..(MAX_NESTING_DEPTH + 5) {
            value = serde_json::json!({ "n": value });
        }
        let bytes = serde_json::json!({ "app": "Creature Cauldron", "kind": "monster", "format_version": 1, "monster": value })
            .to_string()
            .into_bytes();
        assert_eq!(import_monster(&bytes), Err(ImportError::NestingTooDeep { max_depth: MAX_NESTING_DEPTH }));
    }

    #[test]
    fn a_list_over_its_cap_is_rejected() {
        let mut monster = minimal_monster();
        monster["condition_immunities"] = Value::Array((0..MAX_LIST_LEN + 1).map(|_| Value::String("x".to_string())).collect());
        let bytes = envelope(monster);
        assert!(matches!(import_monster(&bytes), Err(ImportError::ListTooLong { .. })));
    }

    #[test]
    fn a_string_over_its_cap_is_rejected() {
        let mut monster = minimal_monster();
        monster["name"] = Value::String("x".repeat(MAX_STRING_LEN + 1));
        let bytes = envelope(monster);
        assert!(matches!(import_monster(&bytes), Err(ImportError::StringTooLong { .. })));
    }

    #[test]
    fn a_save_bonus_mismatch_is_reported_not_silently_changed() {
        let mut monster = minimal_monster();
        // con modifier 0 + proficiency 2 = 2, but the file claims 5.
        monster["saves"] = serde_json::json!({ "con": 5 });
        let bytes = envelope(monster);
        let import = import_monster(&bytes).unwrap();
        assert!(import.creature.save_proficiencies.contains(&(AbilityScore::Constitution as i32)));
        assert_eq!(import.report.len(), 1);
        assert_eq!(import.report[0].field, "saves.con");
    }

    #[test]
    fn a_skill_name_outside_the_srd_eighteen_is_reported_and_left_out() {
        let mut monster = minimal_monster();
        monster["skills"] = serde_json::json!({ "Made Up Skill": 3 });
        let bytes = envelope(monster);
        let import = import_monster(&bytes).unwrap();
        assert!(import.creature.skills.is_empty());
        assert_eq!(import.report[0].field, "skills.Made Up Skill");
    }

    #[test]
    fn missing_walk_speed_gives_zero_never_another_modes_speed() {
        let mut monster = minimal_monster();
        monster["speeds"] = serde_json::json!({ "fly": 50 });
        let bytes = envelope(monster);
        let import = import_monster(&bytes).unwrap();
        assert_eq!(import.creature.speed_feet, 0.0);
        assert_eq!(import.creature.fly_speed_feet, 50.0);
    }
}
