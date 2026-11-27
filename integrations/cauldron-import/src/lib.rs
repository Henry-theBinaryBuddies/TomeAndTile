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
    RulesEdition, SkillBonus,
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
/// Most condition ids a single creature may have; any beyond this are reported and dropped,
/// not imported.
pub const MAX_CONDITION_IMMUNITIES: usize = 32;

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
    /// A required number is present and the right type, but outside the range the field can
    /// actually hold (an ability score, a proficiency bonus, hit points below zero). Checked
    /// before any arithmetic is done with it, so a hostile or malformed value can never panic.
    OutOfRange {
        field: String,
        min: i32,
        max: i32,
        found: i32,
    },
    /// `monster.name`, after trimming, is empty, or is longer than the server accepts (120
    /// characters) -- both would otherwise be accepted here and bounced by the server as a
    /// generic `bad_creature` later.
    InvalidName {
        reason: &'static str,
    },
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
            ImportError::OutOfRange {
                field,
                min,
                max,
                found,
            } => write!(f, "{field} is {found}, outside the allowed range {min} to {max}"),
            ImportError::InvalidName { reason } => write!(f, "name is {reason}"),
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
    let format_version = root
        .get("format_version")
        .ok_or_else(|| ImportError::MissingField("format_version".into()))?;
    if format_version.as_i64() != Some(1) {
        return Err(ImportError::WrongFormatVersion(describe(format_version)));
    }

    let monster = root
        .get("monster")
        .ok_or_else(|| ImportError::MissingField("monster".into()))?;
    if !monster.is_object() {
        return Err(ImportError::WrongType {
            field: "monster".into(),
            expected: "object",
        });
    }

    import_stat_block(monster)
}

/// Walks the whole parsed document once, before any field is read, rejecting nesting, list and
/// string sizes a hostile file could use to waste time or memory.
fn check_limits(value: &Value, path: &str, depth: usize) -> Result<(), ImportError> {
    if depth > MAX_NESTING_DEPTH {
        return Err(ImportError::NestingTooDeep {
            max_depth: MAX_NESTING_DEPTH,
        });
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
                return Err(ImportError::ListTooLong {
                    field: path.to_string(),
                    max_len: MAX_LIST_LEN,
                });
            }
            for (i, item) in items.iter().enumerate() {
                check_limits(item, &format!("{path}[{i}]"), depth + 1)?;
            }
        }
        Value::Object(map) => {
            if map.len() > MAX_LIST_LEN {
                return Err(ImportError::ListTooLong {
                    field: path.to_string(),
                    max_len: MAX_LIST_LEN,
                });
            }
            for (k, v) in map {
                // A key's own length is capped the same as a string value's: an object key is
                // just as much attacker-controlled text as a value is (a skill name, for one).
                if k.chars().count() > MAX_STRING_LEN {
                    return Err(ImportError::StringTooLong {
                        field: format!("{path}.{k}"),
                        max_len: MAX_STRING_LEN,
                    });
                }
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
        Some(_) => Err(ImportError::WrongType {
            field: key.to_string(),
            expected: "string",
        }),
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
        Some(Value::Number(n)) => {
            n.as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .ok_or(ImportError::WrongType {
                    field,
                    expected: "integer",
                })
        }
        Some(_) => Err(ImportError::WrongType {
            field,
            expected: "integer",
        }),
        None => Err(ImportError::MissingField(field)),
    }
}

/// Like [`require_i32`], but also rejects a value outside `min..=max` before the caller ever
/// does arithmetic with it. Used for ability scores and the proficiency bonus, so a crafted
/// file with an extreme integer (`i32::MIN`, `i32::MAX`) is refused here rather than overflowing
/// a later subtraction or addition.
fn require_i32_range(
    value: &Value,
    path: &str,
    key: &str,
    min: i32,
    max: i32,
) -> Result<i32, ImportError> {
    let found = require_i32(value, path, key)?;
    if found < min || found > max {
        return Err(ImportError::OutOfRange {
            field: format!("{path}.{key}"),
            min,
            max,
            found,
        });
    }
    Ok(found)
}

/// `monster.{field}` as an object. Distinguishes "missing" from "present but the wrong type" --
/// `"armor_class": 15` (a bare number, not `{"value": 15}`) is the latter, not a missing
/// `armor_class.value`.
fn require_field_object<'a>(monster: &'a Value, field: &str) -> Result<&'a Value, ImportError> {
    let value = monster
        .get(field)
        .ok_or_else(|| ImportError::MissingField(field.to_string()))?;
    if !value.is_object() {
        return Err(ImportError::WrongType {
            field: field.to_string(),
            expected: "object",
        });
    }
    Ok(value)
}

/// Clamps an accepted-but-out-of-range number (armor class, a movement speed) into bounds the
/// rules engine can use, reporting what was done rather than silently guessing or hard-failing
/// the whole import over it.
fn bounded_i32(n: i32, field: &str, min: i32, max: i32, report: &mut Vec<ReportEntry>) -> i32 {
    let clamped = n.clamp(min, max);
    if clamped != n {
        report.push(ReportEntry {
            field: field.to_string(),
            reason: format!("{n} is outside {min} to {max}; clamped to {clamped}"),
        });
    }
    clamped
}

/// Same as [`bounded_i32`] for a movement speed: also rejects non-finite values (`1e300`, NaN),
/// which `clamp` cannot.
fn bounded_speed(raw: f32, field: &str, report: &mut Vec<ReportEntry>) -> f32 {
    const MAX_SPEED_FEET: f32 = 10_000.0;
    if !raw.is_finite() || !(0.0..=MAX_SPEED_FEET).contains(&raw) {
        report.push(ReportEntry {
            field: field.to_string(),
            reason: format!(
                "{raw} is not a valid speed (must be finite, 0 to {MAX_SPEED_FEET}); using 0"
            ),
        });
        return 0.0;
    }
    raw
}

fn optional_i32(value: &Value, key: &str) -> Option<i32> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
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

#[allow(clippy::type_complexity)]
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
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
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
    if name.trim().is_empty() {
        return Err(ImportError::InvalidName {
            reason: "blank after trimming",
        });
    }
    if name.chars().count() > 120 {
        return Err(ImportError::InvalidName {
            reason: "longer than 120 characters",
        });
    }

    let attribution = optional_str(monster, "attribution").unwrap_or("");
    let origin_notes = origin_notes(monster);

    let size_str = require_str(monster, "size")?;
    let size =
        creature_size(size_str).ok_or_else(|| ImportError::UnknownSize(size_str.to_string()))?;

    let armor_class_obj = require_field_object(monster, "armor_class")?;
    let armor_class = require_i32(armor_class_obj, "armor_class", "value")?;
    let armor_class = bounded_i32(armor_class, "armor_class.value", 0, 50, &mut report);

    let hit_points_obj = require_field_object(monster, "hit_points")?;
    let hp_max = require_i32(hit_points_obj, "hit_points", "average")?;
    if hp_max < 0 {
        return Err(ImportError::OutOfRange {
            field: "hit_points.average".to_string(),
            min: 0,
            max: i32::MAX,
            found: hp_max,
        });
    }

    let speeds = monster.get("speeds");
    let raw_speed = |key: &str| speeds.and_then(|s| optional_f32(s, key)).unwrap_or(0.0);
    let speed_feet = bounded_speed(raw_speed("walk"), "speeds.walk", &mut report);
    let fly_speed_feet = bounded_speed(raw_speed("fly"), "speeds.fly", &mut report);
    let swim_speed_feet = bounded_speed(raw_speed("swim"), "speeds.swim", &mut report);
    let climb_speed_feet = bounded_speed(raw_speed("climb"), "speeds.climb", &mut report);
    let burrow_speed_feet = bounded_speed(raw_speed("burrow"), "speeds.burrow", &mut report);
    let can_hover = speeds.map(|s| optional_bool(s, "hover")).unwrap_or(false);

    let abilities_obj = require_field_object(monster, "abilities")?;
    let scores = AbilityScores {
        strength: require_i32_range(abilities_obj, "abilities", "str", 1, 30)?,
        dexterity: require_i32_range(abilities_obj, "abilities", "dex", 1, 30)?,
        constitution: require_i32_range(abilities_obj, "abilities", "con", 1, 30)?,
        intelligence: require_i32_range(abilities_obj, "abilities", "int", 1, 30)?,
        wisdom: require_i32_range(abilities_obj, "abilities", "wis", 1, 30)?,
        charisma: require_i32_range(abilities_obj, "abilities", "cha", 1, 30)?,
    };

    let challenge_obj = require_field_object(monster, "challenge")?;
    let proficiency_bonus =
        require_i32_range(challenge_obj, "challenge", "proficiency_bonus", 0, 10)?;

    let mut save_proficiencies = Vec::new();
    match monster.get("saves") {
        None => {}
        Some(Value::Object(saves_obj)) => {
            for (key, modifier_fn, ability_score) in SIX_ABILITIES {
                let Some(raw) = saves_obj.get(key) else {
                    continue;
                };
                let Some(printed) = raw.as_i64().and_then(|n| i32::try_from(n).ok()) else {
                    report.push(ReportEntry {
                        field: format!("saves.{key}"),
                        reason: "save bonus is not an integer".to_string(),
                    });
                    continue;
                };
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
        Some(_) => {
            report.push(ReportEntry {
                field: "saves".to_string(),
                reason: "saves is not an object".to_string(),
            });
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
            match SKILL_IDS
                .iter()
                .find(|(cauldron_name, _)| *cauldron_name == name.as_str())
            {
                Some((_, skill_id)) => skills.push(SkillBonus {
                    skill: (*skill_id).to_string(),
                    bonus,
                }),
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

    let mut condition_immunities = Vec::new();
    if let Some(list) = monster
        .get("condition_immunities")
        .and_then(Value::as_array)
    {
        for item in list {
            let Some(id) = item.as_str() else {
                report.push(ReportEntry {
                    field: "condition_immunities[]".to_string(),
                    reason: "not a string".to_string(),
                });
                continue;
            };
            if !is_condition_id(id) {
                report.push(ReportEntry {
                    field: "condition_immunities[]".to_string(),
                    reason: format!(
                        "\"{id}\" is not a valid condition id (lower case, digits, underscore, 1 to 40 characters)"
                    ),
                });
                continue;
            }
            if condition_immunities.len() >= MAX_CONDITION_IMMUNITIES {
                report.push(ReportEntry {
                    field: "condition_immunities[]".to_string(),
                    reason: format!(
                        "more than {MAX_CONDITION_IMMUNITIES} condition immunities; dropped"
                    ),
                });
                continue;
            }
            condition_immunities.push(id.to_string());
        }
    }

    let senses = monster.get("senses");
    let passive_perception = senses
        .and_then(|s| optional_i32(s, "passive_perception"))
        .unwrap_or(0);
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
        edition: RulesEdition::RulesEdition2014 as i32, // all Cauldron monsters use the 2014 rules.
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
    let Some(Value::Array(list)) = monster.get(key) else {
        return Vec::new();
    };
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

/// A condition or label id, per the SRD-condition-id rule agreed in #95: `[a-z0-9_]{1,40}`.
fn is_condition_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars().count() <= 40
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
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
/// plus XP, folded into one display-only line. `monster.subtypes`, `.alignment` and
/// `armor_class.note` are deliberately left out, not an oversight: the issue's own example
/// ("Large dragon, CR 4 (1,100 XP)") has neither a subtype, an alignment, nor an AC note in it,
/// so this matches the acceptance test literally. MAPPING.md lists all three as "safely dropped"
/// for the same reason (flavor text the rules engine never reads).
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
            Err(ImportError::TooLarge {
                max_bytes: MAX_FILE_BYTES,
                actual_bytes: MAX_FILE_BYTES + 1
            })
        );
    }

    #[test]
    fn invalid_utf8_is_rejected_not_panicked_on() {
        assert_eq!(
            import_monster(&[0xff, 0xfe, 0xfd]),
            Err(ImportError::InvalidUtf8)
        );
    }

    #[test]
    fn truncated_json_is_rejected() {
        let result = import_monster(b"{\"app\": \"Creature Cau");
        assert!(matches!(result, Err(ImportError::InvalidJson(_))));
    }

    #[test]
    fn the_wrong_app_is_rejected_before_anything_else_is_read() {
        let bytes =
            serde_json::json!({ "app": "Something Else", "kind": "monster", "format_version": 1 })
                .to_string()
                .into_bytes();
        assert_eq!(
            import_monster(&bytes),
            Err(ImportError::WrongApp("\"Something Else\"".to_string()))
        );
    }

    #[test]
    fn an_encounter_export_is_rejected_by_name() {
        let bytes = serde_json::json!({ "app": "Creature Cauldron", "kind": "encounter", "format_version": 1 })
            .to_string()
            .into_bytes();
        assert_eq!(
            import_monster(&bytes),
            Err(ImportError::UnsupportedKind("\"encounter\"".to_string()))
        );
    }

    #[test]
    fn a_future_format_version_is_rejected() {
        let bytes = serde_json::json!({ "app": "Creature Cauldron", "kind": "monster", "format_version": 2 })
            .to_string()
            .into_bytes();
        assert_eq!(
            import_monster(&bytes),
            Err(ImportError::WrongFormatVersion("2".to_string()))
        );
    }

    #[test]
    fn a_missing_required_field_names_itself() {
        let mut monster = minimal_monster();
        monster.as_object_mut().unwrap().remove("name");
        let bytes = envelope(monster);
        assert_eq!(
            import_monster(&bytes),
            Err(ImportError::MissingField("name".to_string()))
        );
    }

    #[test]
    fn an_unknown_size_string_is_an_error_never_a_default() {
        let mut monster = minimal_monster();
        monster["size"] = Value::String("colossal".to_string());
        let bytes = envelope(monster);
        assert_eq!(
            import_monster(&bytes),
            Err(ImportError::UnknownSize("colossal".to_string()))
        );
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
        assert_eq!(
            import_monster(&bytes),
            Err(ImportError::NestingTooDeep {
                max_depth: MAX_NESTING_DEPTH
            })
        );
    }

    #[test]
    fn a_list_over_its_cap_is_rejected() {
        let mut monster = minimal_monster();
        monster["condition_immunities"] = Value::Array(
            (0..MAX_LIST_LEN + 1)
                .map(|_| Value::String("x".to_string()))
                .collect(),
        );
        let bytes = envelope(monster);
        match import_monster(&bytes) {
            Err(ImportError::ListTooLong { field, .. }) => {
                assert_eq!(field, "$.monster.condition_immunities");
            }
            other => panic!("expected ListTooLong, got {other:?}"),
        }
    }

    #[test]
    fn a_string_over_its_cap_is_rejected() {
        let mut monster = minimal_monster();
        monster["name"] = Value::String("x".repeat(MAX_STRING_LEN + 1));
        let bytes = envelope(monster);
        match import_monster(&bytes) {
            Err(ImportError::StringTooLong { field, .. }) => {
                assert_eq!(field, "$.monster.name");
            }
            other => panic!("expected StringTooLong, got {other:?}"),
        }
    }

    #[test]
    fn an_object_key_over_its_cap_is_rejected() {
        let mut monster = minimal_monster();
        let mut skills = serde_json::Map::new();
        skills.insert("x".repeat(MAX_STRING_LEN + 1), serde_json::json!(3));
        monster["skills"] = Value::Object(skills);
        let bytes = envelope(monster);
        assert!(matches!(
            import_monster(&bytes),
            Err(ImportError::StringTooLong { .. })
        ));
    }

    #[test]
    fn negative_hp_is_rejected() {
        let mut monster = minimal_monster();
        monster["hit_points"] = serde_json::json!({ "average": -5 });
        let bytes = envelope(monster);
        assert_eq!(
            import_monster(&bytes),
            Err(ImportError::OutOfRange {
                field: "hit_points.average".to_string(),
                min: 0,
                max: i32::MAX,
                found: -5
            })
        );
    }

    #[test]
    fn an_extreme_ability_score_is_rejected_never_panics() {
        let mut monster = minimal_monster();
        monster["abilities"]["con"] = serde_json::json!(i32::MIN);
        monster["saves"] = serde_json::json!({ "con": 5 });
        let bytes = envelope(monster);
        assert_eq!(
            import_monster(&bytes),
            Err(ImportError::OutOfRange {
                field: "abilities.con".to_string(),
                min: 1,
                max: 30,
                found: i32::MIN
            })
        );
    }

    #[test]
    fn an_extreme_proficiency_bonus_is_rejected_never_panics() {
        let mut monster = minimal_monster();
        monster["abilities"]["con"] = serde_json::json!(i32::MAX);
        monster["challenge"]["proficiency_bonus"] = serde_json::json!(i32::MAX);
        let bytes = envelope(monster);
        // con (out of 1..=30) is checked first, so that error surfaces before the
        // proficiency_bonus one would -- either way, nothing panics.
        assert!(matches!(
            import_monster(&bytes),
            Err(ImportError::OutOfRange { .. })
        ));
    }

    #[test]
    fn a_blank_name_is_rejected() {
        let mut monster = minimal_monster();
        monster["name"] = Value::String("   ".to_string());
        let bytes = envelope(monster);
        assert_eq!(
            import_monster(&bytes),
            Err(ImportError::InvalidName {
                reason: "blank after trimming"
            })
        );
    }

    #[test]
    fn a_name_over_120_characters_is_rejected() {
        let mut monster = minimal_monster();
        monster["name"] = Value::String("x".repeat(121));
        let bytes = envelope(monster);
        assert_eq!(
            import_monster(&bytes),
            Err(ImportError::InvalidName {
                reason: "longer than 120 characters"
            })
        );
    }

    #[test]
    fn a_bare_number_armor_class_is_a_wrong_type_not_a_missing_field() {
        let mut monster = minimal_monster();
        monster["armor_class"] = serde_json::json!(15);
        let bytes = envelope(monster);
        assert_eq!(
            import_monster(&bytes),
            Err(ImportError::WrongType {
                field: "armor_class".to_string(),
                expected: "object"
            })
        );
    }

    #[test]
    fn an_out_of_range_armor_class_is_clamped_and_reported() {
        let mut monster = minimal_monster();
        monster["armor_class"] = serde_json::json!({ "value": -99999 });
        let bytes = envelope(monster);
        let import = import_monster(&bytes).unwrap();
        assert_eq!(import.creature.armor_class, 0);
        assert_eq!(import.report[0].field, "armor_class.value");
    }

    #[test]
    fn a_non_finite_speed_is_reported_and_zeroed() {
        let mut monster = minimal_monster();
        monster["speeds"] = serde_json::json!({ "walk": 30, "fly": 1e300 });
        let bytes = envelope(monster);
        let import = import_monster(&bytes).unwrap();
        assert_eq!(import.creature.fly_speed_feet, 0.0);
        assert!(import.report.iter().any(|r| r.field == "speeds.fly"));
    }

    #[test]
    fn a_negative_speed_is_reported_and_zeroed() {
        let mut monster = minimal_monster();
        monster["speeds"] = serde_json::json!({ "walk": 30, "fly": -40 });
        let bytes = envelope(monster);
        let import = import_monster(&bytes).unwrap();
        assert_eq!(import.creature.fly_speed_feet, 0.0);
        assert!(import.report.iter().any(|r| r.field == "speeds.fly"));
    }

    #[test]
    fn a_non_integer_save_bonus_is_reported_not_silently_dropped() {
        let mut monster = minimal_monster();
        monster["saves"] = serde_json::json!({ "con": "+2", "dex": 2.0 });
        let bytes = envelope(monster);
        let import = import_monster(&bytes).unwrap();
        assert!(import.creature.save_proficiencies.is_empty());
        assert_eq!(import.report.len(), 2);
        assert!(import.report.iter().any(|r| r.field == "saves.con"));
        assert!(import.report.iter().any(|r| r.field == "saves.dex"));
    }

    #[test]
    fn saves_that_is_not_an_object_is_reported() {
        let mut monster = minimal_monster();
        monster["saves"] = serde_json::json!(["con"]);
        let bytes = envelope(monster);
        let import = import_monster(&bytes).unwrap();
        assert!(import.creature.save_proficiencies.is_empty());
        assert_eq!(import.report.len(), 1);
        assert_eq!(import.report[0].field, "saves");
    }

    #[test]
    fn a_non_string_condition_immunity_is_reported_and_dropped() {
        let mut monster = minimal_monster();
        monster["condition_immunities"] = serde_json::json!(["prone", 5]);
        let bytes = envelope(monster);
        let import = import_monster(&bytes).unwrap();
        assert_eq!(
            import.creature.condition_immunities,
            vec!["prone".to_string()]
        );
        assert_eq!(import.report.len(), 1);
    }

    #[test]
    fn a_junk_condition_immunity_id_is_reported_and_dropped() {
        let mut monster = minimal_monster();
        monster["condition_immunities"] = serde_json::json!(["prone", "Not An Id!!"]);
        let bytes = envelope(monster);
        let import = import_monster(&bytes).unwrap();
        assert_eq!(
            import.creature.condition_immunities,
            vec!["prone".to_string()]
        );
        assert_eq!(import.report.len(), 1);
    }

    #[test]
    fn more_than_32_condition_immunities_are_capped() {
        let mut monster = minimal_monster();
        monster["condition_immunities"] = Value::Array(
            (0..40)
                .map(|i| Value::String(format!("cond_{i}")))
                .collect(),
        );
        let bytes = envelope(monster);
        let import = import_monster(&bytes).unwrap();
        assert_eq!(
            import.creature.condition_immunities.len(),
            MAX_CONDITION_IMMUNITIES
        );
        assert_eq!(import.report.len(), 40 - MAX_CONDITION_IMMUNITIES);
    }

    #[test]
    fn origin_notes_folds_parent_id_generator_and_changes() {
        let mut monster = minimal_monster();
        monster["parent_id"] = Value::String("goblin-base".to_string());
        monster["generator"] = Value::String("reskin-tool".to_string());
        monster["changes"] = serde_json::json!(["bigger", "meaner"]);
        let bytes = envelope(monster);
        let import = import_monster(&bytes).unwrap();
        assert_eq!(
            import.creature.provenance.unwrap().origin_notes,
            "reskin of goblin-base via reskin-tool; changes: bigger, meaner"
        );
    }

    #[test]
    fn a_save_bonus_mismatch_is_reported_not_silently_changed() {
        let mut monster = minimal_monster();
        // con modifier 0 + proficiency 2 = 2, but the file claims 5.
        monster["saves"] = serde_json::json!({ "con": 5 });
        let bytes = envelope(monster);
        let import = import_monster(&bytes).unwrap();
        assert!(import
            .creature
            .save_proficiencies
            .contains(&(AbilityScore::Constitution as i32)));
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
