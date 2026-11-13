use prost::Message;
use tt_contracts::v1::{
    effect_step, event, server_message, Ability, AbilityScore, AttackStep, Creature, DamagePart,
    DamageType, EffectStep, Event, SaveStep, ServerMessage, TokenMoved, Vec2,
};

fn sample_creature() -> Creature {
    let attack = EffectStep {
        step: Some(effect_step::Step::Attack(AttackStep {
            to_hit_bonus: 5,
            damage: vec![DamagePart {
                dice: "1d8+3".into(),
                r#type: DamageType::Slashing as i32,
            }],
        })),
    };
    let save = EffectStep {
        step: Some(effect_step::Step::Save(SaveStep {
            ability: AbilityScore::Dexterity as i32,
            dc: 13,
            damage: vec![DamagePart {
                dice: "2d6".into(),
                r#type: DamageType::Fire as i32,
            }],
            half_on_success: true,
        })),
    };
    Creature {
        creature_id: "c1".into(),
        name: "Goblin".into(),
        armor_class: 15,
        hp_current: 7,
        hp_max: 7,
        abilities: vec![Ability {
            ability_id: "a1".into(),
            name: "Scimitar".into(),
            steps: vec![attack, save],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn sample_server_message() -> ServerMessage {
    ServerMessage {
        kind: Some(server_message::Kind::Event(Event {
            seq: 42,
            scene_id: "s1".into(),
            kind: Some(event::Kind::TokenMoved(TokenMoved {
                token_id: "t1".into(),
                path: vec![Vec2 { x: 1.0, y: 2.5 }, Vec2 { x: 3.0, y: 4.0 }],
                level_id: "l1".into(),
                distance_feet: 10.0,
            })),
            ..Default::default()
        })),
    }
}

#[test]
fn creature_binary_roundtrip() {
    let creature = sample_creature();
    let bytes = creature.encode_to_vec();
    let decoded = Creature::decode(bytes.as_slice()).expect("decode");
    assert_eq!(creature, decoded);
}

#[test]
fn creature_json_roundtrip() {
    let creature = sample_creature();
    let json = serde_json::to_string(&creature).expect("serialize");
    let decoded: Creature = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(creature, decoded);
}

#[test]
fn json_uses_camel_case_and_enum_names() {
    let json = serde_json::to_value(sample_creature()).expect("serialize");
    assert_eq!(json["armorClass"], 15);
    assert!(json.get("armor_class").is_none());
    let save = &json["abilities"][0]["steps"][1]["save"];
    assert_eq!(save["ability"], "ABILITY_SCORE_DEXTERITY");
    assert_eq!(save["halfOnSuccess"], true);
    assert_eq!(save["damage"][0]["type"], "DAMAGE_TYPE_FIRE");
}

#[test]
fn server_message_roundtrips() {
    let msg = sample_server_message();
    let decoded = ServerMessage::decode(msg.encode_to_vec().as_slice()).expect("decode");
    assert_eq!(msg, decoded);
    let json = serde_json::to_string(&msg).expect("serialize");
    let from_json: ServerMessage = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(msg, from_json);
    assert!(json.contains("tokenMoved"));
}

#[test]
fn target_shape_self_json_roundtrip() {
    // `SELF` is a Rust keyword; build.rs patches pbjson output for it.
    use tt_contracts::v1::{TargetShape, Targeting};
    let t = Targeting {
        shape: TargetShape::Self_ as i32,
        ..Default::default()
    };
    let json = serde_json::to_value(t).expect("serialize");
    assert_eq!(json["shape"], "TARGET_SHAPE_SELF");
    let back: Targeting = serde_json::from_value(json).expect("deserialize");
    assert_eq!(t, back);
}

#[test]
fn monster_fields_roundtrip_in_both_forms() {
    use tt_contracts::v1::{ActionCost, Provenance, Recharge, SkillBonus};
    let creature = Creature {
        name: "Voidbloom Seer".into(),
        // No walking speed: flies and hovers.
        speed_feet: 0.0,
        fly_speed_feet: 40.0,
        can_hover: true,
        skills: vec![SkillBonus {
            skill: "perception".into(),
            bonus: 6,
        }],
        passive_perception: 16,
        condition_immunities: vec!["prone".into()],
        abilities: vec![
            Ability {
                name: "Keen Sight".into(),
                cost: ActionCost::Passive as i32,
                ..Default::default()
            },
            Ability {
                name: "Stormbreath".into(),
                recharge: Some(Recharge { min_roll: 5 }),
                ..Default::default()
            },
        ],
        provenance: Some(Provenance {
            source: "creature-cauldron".into(),
            external_id: "curated-voidbloom-seer".into(),
            origin_notes: "curated".into(),
            ..Default::default()
        }),
        ..Default::default()
    };

    let decoded = Creature::decode(creature.encode_to_vec().as_slice()).expect("decode");
    assert_eq!(creature, decoded);

    let json = serde_json::to_value(&creature).expect("serialize");
    assert_eq!(json["flySpeedFeet"], 40.0);
    assert_eq!(json["canHover"], true);
    assert_eq!(json["skills"][0]["skill"], "perception");
    assert_eq!(json["passivePerception"], 16);
    assert_eq!(json["conditionImmunities"][0], "prone");
    assert_eq!(json["abilities"][0]["cost"], "ACTION_COST_PASSIVE");
    assert_eq!(json["abilities"][1]["recharge"]["minRoll"], 5);
    assert_eq!(json["provenance"]["externalId"], "curated-voidbloom-seer");
    let back: Creature = serde_json::from_value(json).expect("deserialize");
    assert_eq!(creature, back);
}

#[test]
fn files_written_before_the_monster_fields_still_parse() {
    // A creature file from before these fields existed must load unchanged.
    let old = r#"{"name":"Goblin","armorClass":15,"speedFeet":30}"#;
    let creature: Creature = serde_json::from_str(old).expect("deserialize");
    assert_eq!(creature.speed_feet, 30.0);
    assert_eq!(creature.fly_speed_feet, 0.0);
    assert!(creature.skills.is_empty());
    assert!(creature.condition_immunities.is_empty());
}
