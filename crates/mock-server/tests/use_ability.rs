//! Integration tests for the pregen seed and `use_ability`: a real server on
//! 127.0.0.1 port 0, real WebSocket clients and a fixed dice roller.

use std::collections::{HashSet, VecDeque};
use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tt_contracts::v1::{
    client_message, event, intent, server_message, ChatIntent, ClientMessage, Event, Hello,
    HitOutcome, Intent, IntentRejected, Resolution, ResolutionState, Role, RollMode, RollPurpose,
    RollVisibility, ServerMessage, Snapshot, UseAbilityIntent,
};
use tt_mock_server::dice::Rng;

const WAIT: Duration = Duration::from_secs(5);

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Hands out queued values; once empty it returns 0 (every die shows 1).
/// A die of `s` sides shows `value % s + 1`, so queue `face - 1`.
struct Fixed(VecDeque<u64>);

impl Rng for Fixed {
    fn next_u64(&mut self) -> u64 {
        self.0.pop_front().unwrap_or(0)
    }
}

/// Queue die faces in the order the mock draws them.
fn faces(list: &[u64]) -> Fixed {
    Fixed(list.iter().map(|f| f - 1).collect())
}

async fn start(rng: Fixed) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = tt_mock_server::serve_with_rng(listener, Box::new(rng)).await;
    });
    addr
}

async fn send(ws: &mut Ws, kind: client_message::Kind) {
    let bytes = ClientMessage { kind: Some(kind) }.encode_to_vec();
    timeout(WAIT, ws.send(Message::Binary(bytes.into())))
        .await
        .expect("send timed out")
        .expect("send failed");
}

async fn recv(ws: &mut Ws) -> server_message::Kind {
    loop {
        let frame = timeout(WAIT, ws.next())
            .await
            .expect("timed out waiting for a server message")
            .expect("socket closed")
            .expect("socket error");
        if let Message::Binary(bytes) = frame {
            return ServerMessage::decode(&bytes[..])
                .expect("undecodable frame")
                .kind
                .expect("server message has a kind");
        }
    }
}

async fn recv_event(ws: &mut Ws) -> Event {
    match recv(ws).await {
        server_message::Kind::Event(e) => e,
        other => panic!("expected an event, got {other:?}"),
    }
}

async fn recv_rejected(ws: &mut Ws) -> IntentRejected {
    match recv(ws).await {
        server_message::Kind::IntentRejected(r) => r,
        other => panic!("expected a rejection, got {other:?}"),
    }
}

async fn join(addr: SocketAddr, token: &str) -> (Ws, Snapshot) {
    let (mut ws, _) = timeout(WAIT, connect_async(format!("ws://{addr}/ws")))
        .await
        .expect("connect timed out")
        .expect("connect failed");
    send(
        &mut ws,
        client_message::Kind::Hello(Hello {
            campaign_id: "campaign-mock".into(),
            session_token: token.into(),
            last_seq: 0,
        }),
    )
    .await;
    match recv(&mut ws).await {
        server_message::Kind::Snapshot(s) => (ws, s),
        other => panic!("expected a snapshot, got {other:?}"),
    }
}

async fn use_ability(ws: &mut Ws, id: &str, source: &str, ability: &str, targets: &[&str]) {
    use_ability_mode(ws, id, source, ability, targets, RollMode::Normal).await;
}

async fn use_ability_mode(
    ws: &mut Ws,
    id: &str,
    source: &str,
    ability: &str,
    targets: &[&str],
    mode: RollMode,
) {
    send(
        ws,
        client_message::Kind::Intent(Intent {
            client_intent_id: id.into(),
            kind: Some(intent::Kind::UseAbility(UseAbilityIntent {
                source_token_id: source.into(),
                ability_id: ability.into(),
                target_token_ids: targets.iter().map(|t| t.to_string()).collect(),
                mode: mode as i32,
                ..Default::default()
            })),
        }),
    )
    .await;
}

async fn send_chat(ws: &mut Ws, text: &str) {
    send(
        ws,
        client_message::Kind::Intent(Intent {
            client_intent_id: "chat".into(),
            kind: Some(intent::Kind::Chat(ChatIntent { text: text.into() })),
        }),
    )
    .await;
}

fn resolution(e: &Event) -> &Resolution {
    match &e.kind {
        Some(event::Kind::ResolutionUpdated(r)) => r.resolution.as_ref().expect("resolution"),
        other => panic!("expected resolution_updated, got {other:?}"),
    }
}

/// Assert that this socket received a rejection with `code`.
async fn assert_rejected(ws: &mut Ws, code: &str) {
    let r = recv_rejected(ws).await;
    assert_eq!(r.code, code, "message was {:?}", r.message);
}

// ---------------------------------------------------------------- snapshots

#[tokio::test]
async fn player_snapshot_has_the_pregens_but_not_the_goblin_creature() {
    let addr = start(faces(&[])).await;
    let (_ws, s) = join(addr, "player").await;
    assert_eq!(s.role, Role::Player as i32);
    let mut ids: Vec<&str> = s.creatures.iter().map(|c| c.creature_id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        [
            "creature-brannoch",
            "creature-ilvaren",
            "creature-korvath",
            "creature-maribel",
            "creature-tamsin"
        ]
    );
    assert_eq!(s.tokens.len(), 7);
}

#[tokio::test]
async fn gm_snapshot_has_every_creature() {
    let addr = start(faces(&[])).await;
    let (_ws, s) = join(addr, "gm").await;
    assert_eq!(s.role, Role::Gm as i32);
    assert_eq!(s.creatures.len(), 6);
    assert!(s
        .creatures
        .iter()
        .any(|c| c.creature_id == "creature-goblin"));
}

#[tokio::test]
async fn token_hp_is_not_redacted_for_anyone() {
    let addr = start(faces(&[])).await;
    for token in ["player", "gm"] {
        let (_ws, s) = join(addr, token).await;
        let hp = |id: &str| {
            s.tokens
                .iter()
                .find(|t| t.token_id == id)
                .unwrap()
                .hp_current
        };
        assert_eq!(hp("token-aria"), Some(24));
        assert_eq!(hp("token-brannoch"), Some(28));
        assert_eq!(hp("token-goblin"), None);
    }
}

// ----------------------------------------------------------------- attacks

#[tokio::test]
async fn hit_rolls_attack_and_damage_with_exact_values() {
    // d20 = 10 (+5 = 15 vs AC 13: hit), then d8 = 4 (+3 = 7).
    let addr = start(faces(&[10, 4])).await;
    let (mut ws, _) = join(addr, "player").await;
    use_ability(
        &mut ws,
        "u1",
        "token-brannoch",
        "longsword",
        &["token-goblin"],
    )
    .await;
    let e = recv_event(&mut ws).await;
    assert_eq!(e.seq, 1);
    assert_eq!(e.client_intent_id, "u1");
    assert_eq!(e.actor_user_id, "mock-player");
    assert!(e.warnings.is_empty());
    let r = resolution(&e);
    assert_eq!(r.resolution_id, "res-1");
    assert_eq!(r.source_token_id, "token-brannoch");
    assert_eq!(r.ability_id, "longsword");
    assert_eq!(r.ability_name, "Longsword");
    assert_eq!(r.state, ResolutionState::Done as i32);
    assert!(r.warnings.is_empty());
    assert_eq!(r.outcomes.len(), 1);
    let o = &r.outcomes[0];
    assert_eq!(o.token_id, "token-goblin");
    assert_eq!(o.outcome, HitOutcome::Hit as i32);
    assert_eq!(o.step_index, 0);
    assert!(o.damage.is_empty());
    assert!(!o.dropped_to_zero);

    let attack = o.roll.as_ref().unwrap();
    assert_eq!(attack.roll_id, "roll-1-1");
    assert_eq!(attack.notation, "1d20+5");
    assert_eq!(attack.purpose, RollPurpose::Attack as i32);
    assert_eq!(attack.total, 15);
    assert_eq!(attack.dice.len(), 1);
    assert_eq!(attack.dice[0].value, 10);
    assert_eq!(attack.modifiers.len(), 1);
    assert_eq!(attack.modifiers[0].value, 5);
    assert_eq!(attack.mode, RollMode::Normal as i32);
    assert!(!attack.critical && !attack.fumble);
    assert_eq!(attack.visibility, RollVisibility::Public as i32);
    assert_eq!(attack.roller_creature_id, "creature-brannoch");
    assert_eq!(attack.roller_user_id, "mock-player");
    assert_eq!(attack.label, "Longsword attack");

    assert_eq!(o.damage_rolls.len(), 1);
    let dmg = &o.damage_rolls[0];
    assert_eq!(dmg.roll_id, "roll-1-2");
    assert_eq!(dmg.notation, "1d8+3");
    assert_eq!(dmg.purpose, RollPurpose::Damage as i32);
    assert_eq!(dmg.total, 7);
    assert_eq!(dmg.label, "Longsword damage");
    assert_eq!(dmg.roller_creature_id, "creature-brannoch");
    assert_eq!(dmg.visibility, RollVisibility::Public as i32);
}

#[tokio::test]
async fn miss_rolls_no_damage() {
    // 5 + 5 = 10 against AC 13.
    let addr = start(faces(&[5, 8])).await;
    let (mut ws, _) = join(addr, "player").await;
    use_ability(
        &mut ws,
        "u1",
        "token-brannoch",
        "longsword",
        &["token-goblin"],
    )
    .await;
    let e = recv_event(&mut ws).await;
    let o = &resolution(&e).outcomes[0];
    assert_eq!(o.outcome, HitOutcome::Miss as i32);
    assert_eq!(o.roll.as_ref().unwrap().total, 10);
    assert!(o.damage_rolls.is_empty());
}

#[tokio::test]
async fn a_total_equal_to_armor_class_hits() {
    // 8 + 5 = 13 against AC 13: meeting AC is a hit. Then 1d8 = 4 (+3 = 7).
    let addr = start(faces(&[8, 4])).await;
    let (mut ws, _) = join(addr, "player").await;
    use_ability(
        &mut ws,
        "u1",
        "token-brannoch",
        "longsword",
        &["token-goblin"],
    )
    .await;
    let e = recv_event(&mut ws).await;
    let o = &resolution(&e).outcomes[0];
    assert_eq!(o.roll.as_ref().unwrap().total, 13);
    assert_eq!(o.outcome, HitOutcome::Hit as i32);
    assert_eq!(o.damage_rolls.len(), 1);
    assert_eq!(o.damage_rolls[0].total, 7);
}

#[tokio::test]
async fn natural_20_is_a_critical_hit_that_doubles_dice_not_modifiers() {
    // d20 = 20, then 2d8 = 3 and 6 (+3 = 12).
    let addr = start(faces(&[20, 3, 6])).await;
    let (mut ws, _) = join(addr, "player").await;
    use_ability(
        &mut ws,
        "u1",
        "token-brannoch",
        "longsword",
        &["token-goblin"],
    )
    .await;
    let e = recv_event(&mut ws).await;
    let o = &resolution(&e).outcomes[0];
    assert_eq!(o.outcome, HitOutcome::CriticalHit as i32);
    assert!(o.roll.as_ref().unwrap().critical);
    let dmg = &o.damage_rolls[0];
    assert_eq!(dmg.notation, "2d8+3");
    assert_eq!(dmg.dice.len(), 2);
    assert_eq!(dmg.modifiers.len(), 1);
    assert_eq!(dmg.modifiers[0].value, 3);
    assert_eq!(dmg.total, 12);
}

#[tokio::test]
async fn natural_1_is_a_fumble_and_a_miss_even_against_low_armor() {
    // Ilvaren's Fire Bolt (+5) on a natural 1 is 6; the point is the flag.
    let addr = start(faces(&[1, 5])).await;
    let (mut ws, _) = join(addr, "player").await;
    use_ability(
        &mut ws,
        "u1",
        "token-ilvaren",
        "cantrip-fire-bolt",
        &["token-goblin"],
    )
    .await;
    let e = recv_event(&mut ws).await;
    let o = &resolution(&e).outcomes[0];
    assert_eq!(o.outcome, HitOutcome::Miss as i32);
    assert!(o.roll.as_ref().unwrap().fumble);
    assert!(o.damage_rolls.is_empty());
}

#[tokio::test]
async fn advantage_keeps_the_higher_die() {
    // First die 3, second 15: keeps 15, +5 = 20, a hit; then d8 = 2.
    let addr = start(faces(&[3, 15, 2])).await;
    let (mut ws, _) = join(addr, "player").await;
    use_ability_mode(
        &mut ws,
        "u1",
        "token-brannoch",
        "longsword",
        &["token-goblin"],
        RollMode::Advantage,
    )
    .await;
    let e = recv_event(&mut ws).await;
    let o = &resolution(&e).outcomes[0];
    let attack = o.roll.as_ref().unwrap();
    assert_eq!(attack.mode, RollMode::Advantage as i32);
    assert_eq!(attack.dice.len(), 2);
    assert!(attack.dice[0].dropped && !attack.dice[1].dropped);
    assert_eq!(attack.total, 20);
    assert_eq!(o.outcome, HitOutcome::Hit as i32);
    assert_eq!(o.damage_rolls[0].total, 5);
}

#[tokio::test]
async fn gm_may_attack_with_a_monster() {
    // Goblin Skulker attacks Brannoch (AC 19): 14 + bonus.
    let addr = start(faces(&[1])).await;
    let (mut ws, _) = join(addr, "gm").await;
    use_ability(
        &mut ws,
        "u1",
        "token-goblin",
        "rusty-blade",
        &["token-brannoch"],
    )
    .await;
    let e = recv_event(&mut ws).await;
    assert_eq!(e.actor_user_id, "mock-gm");
    let r = resolution(&e);
    assert_eq!(r.outcomes[0].token_id, "token-brannoch");
    let attack = r.outcomes[0].roll.as_ref().unwrap();
    assert_eq!(attack.roller_creature_id, "creature-goblin");
    assert_eq!(attack.roller_user_id, "mock-gm");
}

#[tokio::test]
async fn scorching_ray_resolves_one_target_per_step_with_step_index() {
    // Ray 0: d20 10 (15, hit) then 2d6 = 3,4. Ray 1: d20 2 (7, miss).
    // Ray 2: d20 20 (crit) then 4d6 = 1,2,3,4. Targets: goblin, tamsin, goblin.
    let addr = start(faces(&[10, 3, 4, 2, 20, 1, 2, 3, 4])).await;
    let (mut ws, _) = join(addr, "player").await;
    use_ability(
        &mut ws,
        "u1",
        "token-ilvaren",
        "scorching-ray",
        &["token-goblin", "token-tamsin", "token-goblin"],
    )
    .await;
    let e = recv_event(&mut ws).await;
    assert!(e.warnings.is_empty());
    let r = resolution(&e);
    assert!(r.warnings.is_empty());
    assert_eq!(r.outcomes.len(), 3);
    let steps: Vec<u32> = r.outcomes.iter().map(|o| o.step_index).collect();
    assert_eq!(steps, [0, 1, 2]);
    let tokens: Vec<&str> = r.outcomes.iter().map(|o| o.token_id.as_str()).collect();
    assert_eq!(tokens, ["token-goblin", "token-tamsin", "token-goblin"]);
    assert_eq!(r.outcomes[0].outcome, HitOutcome::Hit as i32);
    assert_eq!(r.outcomes[0].damage_rolls[0].total, 7);
    assert_eq!(r.outcomes[1].outcome, HitOutcome::Miss as i32);
    assert!(r.outcomes[1].damage_rolls.is_empty());
    assert_eq!(r.outcomes[2].outcome, HitOutcome::CriticalHit as i32);
    assert_eq!(r.outcomes[2].damage_rolls[0].notation, "4d6");
    assert_eq!(r.outcomes[2].damage_rolls[0].total, 10);
}

#[tokio::test]
async fn scorching_ray_ignores_extra_targets_and_skips_unlisted_steps() {
    let addr = start(faces(&[])).await;
    let (mut ws, _) = join(addr, "player").await;
    use_ability(
        &mut ws,
        "u1",
        "token-ilvaren",
        "scorching-ray",
        &[
            "token-goblin",
            "token-goblin",
            "token-goblin",
            "token-tamsin",
        ],
    )
    .await;
    let e = recv_event(&mut ws).await;
    let r = resolution(&e);
    assert_eq!(r.outcomes.len(), 3);
    let codes: Vec<&str> = e.warnings.iter().map(|w| w.code.as_str()).collect();
    assert_eq!(codes, ["extra_targets_ignored"]);
    assert_eq!(r.warnings, e.warnings);

    // Two targets: only steps 0 and 1 are rolled, and no warning.
    use_ability(
        &mut ws,
        "u2",
        "token-ilvaren",
        "scorching-ray",
        &["token-goblin", "token-tamsin"],
    )
    .await;
    let e = recv_event(&mut ws).await;
    assert!(e.warnings.is_empty());
    let r = resolution(&e);
    assert_eq!(r.resolution_id, "res-2");
    let steps: Vec<u32> = r.outcomes.iter().map(|o| o.step_index).collect();
    assert_eq!(steps, [0, 1]);
}

#[tokio::test]
async fn too_many_targets_warns_and_still_resolves_every_target() {
    let addr = start(faces(&[])).await;
    let (mut ws, _) = join(addr, "player").await;
    use_ability(
        &mut ws,
        "u1",
        "token-brannoch",
        "longsword",
        &["token-goblin", "token-tamsin"],
    )
    .await;
    let e = recv_event(&mut ws).await;
    let r = resolution(&e);
    assert_eq!(r.outcomes.len(), 2);
    assert_eq!(e.warnings.len(), 1);
    assert_eq!(e.warnings[0].code, "too_many_targets");
    assert_eq!(r.warnings, e.warnings);
}

#[tokio::test]
async fn both_connections_get_the_same_event_and_no_upserts() {
    let addr = start(faces(&[10, 4])).await;
    let (mut player, _) = join(addr, "player").await;
    let (mut gm, _) = join(addr, "gm").await;
    use_ability(
        &mut player,
        "u1",
        "token-brannoch",
        "longsword",
        &["token-goblin"],
    )
    .await;
    let for_player = recv_event(&mut player).await;
    let for_gm = recv_event(&mut gm).await;
    assert_eq!(for_player, for_gm);
    assert!(matches!(
        for_gm.kind,
        Some(event::Kind::ResolutionUpdated(_))
    ));

    // The very next thing each side sees is the chat: no CreatureUpserted or
    // TokenUpserted slipped in after the attack.
    send_chat(&mut player, "after").await;
    for ws in [&mut player, &mut gm] {
        let next = recv_event(ws).await;
        assert_eq!(next.seq, 2);
        assert!(matches!(next.kind, Some(event::Kind::ChatPosted(_))));
    }
    // And the state did not change: HP in a fresh snapshot is as seeded.
    let (_ws, s) = join(addr, "gm").await;
    let hp = |id: &str| {
        s.tokens
            .iter()
            .find(|t| t.token_id == id)
            .unwrap()
            .hp_current
    };
    assert_eq!(hp("token-brannoch"), Some(28));
    assert_eq!(hp("token-goblin"), None);
}

#[tokio::test]
async fn resolution_and_roll_ids_are_unique_across_resolutions_and_freeform_rolls() {
    // Every ray crits (a 20, then 4d6), so each resolution makes 3 attack and
    // 3 damage rolls; one more die is left for each freeform 1d20 after it.
    let mut queue = Vec::new();
    for _ in 0..3 {
        for _ in 0..3 {
            queue.extend([20, 1, 1, 1, 1]);
        }
        queue.push(1);
    }
    let addr = start(faces(&queue)).await;
    let (mut ws, _) = join(addr, "player").await;
    let mut ids = HashSet::new();
    let mut resolutions = HashSet::new();
    for i in 0..3 {
        use_ability(
            &mut ws,
            "u",
            "token-ilvaren",
            "scorching-ray",
            &["token-goblin"; 3],
        )
        .await;
        let e = recv_event(&mut ws).await;
        let r = resolution(&e);
        assert!(resolutions.insert(r.resolution_id.clone()));
        assert_eq!(r.outcomes.len(), 3);
        for o in &r.outcomes {
            assert_eq!(o.outcome, HitOutcome::CriticalHit as i32);
            assert!(ids.insert(o.roll.as_ref().unwrap().roll_id.clone()));
            assert_eq!(o.damage_rolls.len(), 1);
            for d in &o.damage_rolls {
                assert!(ids.insert(d.roll_id.clone()));
            }
        }
        // A freeform roll between resolutions must not collide either.
        send(
            &mut ws,
            client_message::Kind::Intent(Intent {
                client_intent_id: format!("f{i}"),
                kind: Some(intent::Kind::RollDice(tt_contracts::v1::RollDiceIntent {
                    notation: "1d20".into(),
                    ..Default::default()
                })),
            }),
        )
        .await;
        let e = recv_event(&mut ws).await;
        match e.kind {
            Some(event::Kind::DiceRolled(d)) => {
                assert!(ids.insert(d.roll.unwrap().roll_id));
            }
            other => panic!("expected dice_rolled, got {other:?}"),
        }
    }
    // Per resolution 3 attack + 3 damage ids, plus one freeform id.
    assert_eq!(ids.len(), 3 * (6 + 1));
}

// -------------------------------------------------------------- rejections

/// Each case combines two faults and expects the code for the earlier check.
#[tokio::test]
async fn rejection_order_is_permission_then_ability_then_targets_then_support() {
    let addr = start(faces(&[])).await;
    let (mut player, _) = join(addr, "player").await;
    // Permission beats the ability and target lookups.
    use_ability(&mut player, "o1", "token-goblin", "fireball", &[]).await;
    assert_rejected(&mut player, "not_permitted").await;
    use_ability(
        &mut player,
        "o2",
        "token-goblin",
        "rusty-blade",
        &["token-nobody"],
    )
    .await;
    assert_rejected(&mut player, "not_permitted").await;
    // The ability lookup beats the target checks.
    use_ability(&mut player, "o3", "token-brannoch", "fireball", &[]).await;
    assert_rejected(&mut player, "unknown_ability").await;
    use_ability(
        &mut player,
        "o4",
        "token-brannoch",
        "fireball",
        &["token-nobody"],
    )
    .await;
    assert_rejected(&mut player, "unknown_ability").await;
    // Target checks beat the attack-only support check.
    use_ability(&mut player, "o5", "token-maribel", "cure-wounds", &[]).await;
    assert_rejected(&mut player, "no_targets").await;
    use_ability(
        &mut player,
        "o6",
        "token-maribel",
        "cure-wounds",
        &["token-nobody"],
    )
    .await;
    assert_rejected(&mut player, "unknown_target").await;
    // The GM's missing-token check also precedes everything after it.
    let (mut gm, _) = join(addr, "gm").await;
    use_ability(&mut gm, "o7", "token-nobody", "fireball", &[]).await;
    assert_rejected(&mut gm, "unknown_token").await;
}

#[tokio::test]
async fn player_not_controlling_the_source_token_is_not_permitted() {
    let addr = start(faces(&[])).await;
    let (mut player, _) = join(addr, "player").await;
    let (mut gm, _) = join(addr, "gm").await;
    use_ability(
        &mut player,
        "u1",
        "token-goblin",
        "rusty-blade",
        &["token-brannoch"],
    )
    .await;
    let r = recv_rejected(&mut player).await;
    assert_eq!(
        (r.code.as_str(), r.client_intent_id.as_str()),
        ("not_permitted", "u1")
    );

    // Rejections go to the sender only: the GM sees only the next chat, and
    // the rejection used no seq.
    send_chat(&mut player, "x").await;
    assert_eq!(recv_event(&mut gm).await.seq, 1);
}

#[tokio::test]
async fn player_naming_a_missing_source_token_is_not_permitted() {
    let addr = start(faces(&[])).await;
    let (mut ws, _) = join(addr, "player").await;
    use_ability(
        &mut ws,
        "u1",
        "token-nobody",
        "longsword",
        &["token-goblin"],
    )
    .await;
    assert_rejected(&mut ws, "not_permitted").await;
}

#[tokio::test]
async fn gm_naming_a_missing_source_token_is_unknown_token() {
    let addr = start(faces(&[])).await;
    let (mut ws, _) = join(addr, "gm").await;
    use_ability(
        &mut ws,
        "u1",
        "token-nobody",
        "longsword",
        &["token-goblin"],
    )
    .await;
    assert_rejected(&mut ws, "unknown_token").await;
}

#[tokio::test]
async fn source_token_without_a_creature_is_no_creature() {
    let addr = start(faces(&[])).await;
    // token-aria has no creature, and mock-player controls it.
    let (mut ws, _) = join(addr, "player").await;
    use_ability(&mut ws, "u1", "token-aria", "whatever", &["token-goblin"]).await;
    assert_rejected(&mut ws, "no_creature").await;
}

#[tokio::test]
async fn ability_not_on_the_creature_is_unknown_ability() {
    let addr = start(faces(&[])).await;
    let (mut ws, _) = join(addr, "player").await;
    use_ability(
        &mut ws,
        "u1",
        "token-brannoch",
        "fireball",
        &["token-goblin"],
    )
    .await;
    assert_rejected(&mut ws, "unknown_ability").await;
}

#[tokio::test]
async fn creature_targeting_ability_without_targets_is_no_targets() {
    let addr = start(faces(&[])).await;
    let (mut ws, _) = join(addr, "player").await;
    use_ability(&mut ws, "u1", "token-brannoch", "longsword", &[]).await;
    assert_rejected(&mut ws, "no_targets").await;
}

#[tokio::test]
async fn missing_or_creatureless_target_is_unknown_target() {
    let addr = start(faces(&[])).await;
    let (mut ws, _) = join(addr, "player").await;
    use_ability(
        &mut ws,
        "u1",
        "token-brannoch",
        "longsword",
        &["token-nobody"],
    )
    .await;
    assert_rejected(&mut ws, "unknown_target").await;
    // token-aria exists but has no creature.
    use_ability(
        &mut ws,
        "u2",
        "token-brannoch",
        "longsword",
        &["token-aria"],
    )
    .await;
    assert_rejected(&mut ws, "unknown_target").await;
    // One bad entry among good ones rejects the whole intent.
    use_ability(
        &mut ws,
        "u3",
        "token-brannoch",
        "longsword",
        &["token-goblin", "token-nobody"],
    )
    .await;
    assert_rejected(&mut ws, "unknown_target").await;
}

#[tokio::test]
async fn non_attack_abilities_are_not_supported_by_the_mock() {
    let addr = start(faces(&[])).await;
    let (mut ws, _) = join(addr, "player").await;
    // A flat (healing) step, a save step, and a passive with no steps.
    use_ability(
        &mut ws,
        "u1",
        "token-maribel",
        "cure-wounds",
        &["token-brannoch"],
    )
    .await;
    assert_rejected(&mut ws, "not_supported_by_mock").await;
    use_ability(
        &mut ws,
        "u2",
        "token-maribel",
        "cantrip-sacred-flame",
        &["token-goblin"],
    )
    .await;
    assert_rejected(&mut ws, "not_supported_by_mock").await;
    use_ability(
        &mut ws,
        "u3",
        "token-brannoch",
        "action-surge",
        &["token-goblin"],
    )
    .await;
    assert_rejected(&mut ws, "not_supported_by_mock").await;

    // Rejections used no seq.
    send_chat(&mut ws, "x").await;
    assert_eq!(recv_event(&mut ws).await.seq, 1);
}
