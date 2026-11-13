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
