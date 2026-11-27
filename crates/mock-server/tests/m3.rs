//! Integration tests for the M3 intents (#96): damage, saves, healing,
//! conditions, hit points, tokens, encounters and undo. A real server on
//! 127.0.0.1 port 0, real WebSocket clients and a fixed dice roller.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tt_contracts::v1::{
    client_message, event, intent, server_message, ChatIntent, ClientMessage, CreateTokenIntent,
    EndEncounterIntent, EndTurnIntent, Event, HealthState, Hello, HitOutcome, Intent,
    IntentRejected, RemoveTokenIntent, Resolution, RollPurpose, ServerMessage, SetConditionsIntent,
    SetHitPointsIntent, SetInitiativeIntent, Snapshot, StartEncounterIntent, Token, UndoIntent,
    UseAbilityIntent, Vec2,
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

async fn event(ws: &mut Ws) -> Event {
    match recv(ws).await {
        server_message::Kind::Event(e) => e,
        other => panic!("expected an event, got {other:?}"),
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

async fn send_intent(ws: &mut Ws, id: &str, kind: intent::Kind) {
    send(
        ws,
        client_message::Kind::Intent(Intent {
            client_intent_id: id.into(),
            kind: Some(kind),
        }),
    )
    .await;
}

async fn use_ability(ws: &mut Ws, id: &str, source: &str, ability: &str, targets: &[&str]) {
    send_intent(
        ws,
        id,
        intent::Kind::UseAbility(UseAbilityIntent {
            source_token_id: source.into(),
            ability_id: ability.into(),
            target_token_ids: targets.iter().map(|t| t.to_string()).collect(),
            ..Default::default()
        }),
    )
    .await;
}

async fn set_hp(ws: &mut Ws, id: &str, token: &str, current: i32, temp: i32) {
    send_intent(
        ws,
        id,
        intent::Kind::SetHitPoints(SetHitPointsIntent {
            token_id: token.into(),
            hp_current: current,
            hp_temp: temp,
        }),
    )
    .await;
}

async fn set_conditions(ws: &mut Ws, id: &str, token: &str, add: &[&str], remove: &[&str]) {
    send_intent(
        ws,
        id,
        intent::Kind::SetConditions(SetConditionsIntent {
            token_id: token.into(),
            add: add.iter().map(|c| c.to_string()).collect(),
            remove: remove.iter().map(|c| c.to_string()).collect(),
        }),
    )
    .await;
}

async fn undo(ws: &mut Ws, id: &str, seq: u64) {
    send_intent(ws, id, intent::Kind::Undo(UndoIntent { event_seq: seq })).await;
}

async fn start_encounter(ws: &mut Ws, id: &str, scene: &str, tokens: &[&str]) {
    send_intent(
        ws,
        id,
        intent::Kind::StartEncounter(StartEncounterIntent {
            scene_id: scene.into(),
            token_ids: tokens.iter().map(|t| t.to_string()).collect(),
        }),
    )
    .await;
}

async fn end_turn(ws: &mut Ws, id: &str, encounter: &str) {
    send_intent(
        ws,
        id,
        intent::Kind::EndTurn(EndTurnIntent {
            encounter_id: encounter.into(),
        }),
    )
    .await;
}

/// Assert that this socket received a rejection with `code`.
async fn assert_rejected(ws: &mut Ws, code: &str) {
    match recv(ws).await {
        server_message::Kind::IntentRejected(IntentRejected {
            code: got, message, ..
        }) => {
            assert_eq!(got, code, "message was {message:?}");
        }
        other => panic!("expected a rejection, got {other:?}"),
    }
}

/// Assert that nothing is queued and that `seq` was the last event used: the
/// next event after a chat is that chat, numbered `seq + 1`.
async fn assert_settled(ws: &mut Ws, seq: u64) {
    send(
        ws,
        client_message::Kind::Intent(Intent {
            client_intent_id: "settle".into(),
            kind: Some(intent::Kind::Chat(ChatIntent {
                text: "sync".into(),
            })),
        }),
    )
    .await;
    let e = event(ws).await;
    assert!(matches!(e.kind, Some(event::Kind::ChatPosted(_))), "{e:?}");
    assert_eq!(e.seq, seq + 1);
}

fn resolution(e: &Event) -> &Resolution {
    match &e.kind {
        Some(event::Kind::ResolutionUpdated(r)) => r.resolution.as_ref().expect("resolution"),
        other => panic!("expected resolution_updated, got {other:?}"),
    }
}

fn upserted(e: &Event) -> &Token {
    match &e.kind {
        Some(event::Kind::TokenUpserted(u)) => u.token.as_ref().expect("token"),
        other => panic!("expected token_upserted, got {other:?}"),
    }
}

fn token<'a>(s: &'a Snapshot, id: &str) -> &'a Token {
    s.tokens.iter().find(|t| t.token_id == id).unwrap()
}

// ------------------------------------------------------------------ damage

#[tokio::test]
async fn player_hit_drops_the_goblin_for_the_gm_and_shows_the_player_a_state() {
    // d20 10 (+5 = 15 vs AC 13), then d8 4 (+3 = 7): the goblin goes 9 to 2.
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

    for (ws, hp) in [(&mut player, None), (&mut gm, Some(2))] {
        let e = event(ws).await;
        let o = &resolution(&e).outcomes[0];
        assert_eq!(o.outcome, HitOutcome::Hit as i32);
        assert_eq!(o.damage.len(), 1);
        assert_eq!((o.damage[0].rolled, o.damage[0].applied), (7, 7));
        assert!(!o.dropped_to_zero);
        let e = event(ws).await;
        assert_eq!((e.seq, e.client_intent_id.as_str()), (2, "u1"));
        let t = upserted(&e);
        assert_eq!(t.token_id, "token-goblin");
        assert_eq!(t.hp_current, hp);
        assert_eq!(t.hp_max.is_some(), hp.is_some());
        assert_eq!(t.health_state, HealthState::NearDeath as i32);
    }

    let (_p, s) = join(addr, "player").await;
    let goblin = token(&s, "token-goblin");
    assert_eq!(goblin.hp_current, None);
    assert_eq!(goblin.health_state, HealthState::NearDeath as i32);
    let (_g, s) = join(addr, "gm").await;
    assert_eq!(token(&s, "token-goblin").hp_current, Some(2));
}

#[tokio::test]
async fn dropping_the_goblin_to_zero_sets_dropped_to_zero_and_down() {
    // Hit, then d8 8 (+3 = 11) against 9 HP; a second hit finds it at 0.
    let addr = start(faces(&[10, 8, 10, 4])).await;
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
    let o = resolution(&event(&mut player).await).outcomes[0].clone();
    assert!(o.dropped_to_zero);
    // The roll was 11; the damage rolled and applied say so, HP just floors.
    assert_eq!((o.damage[0].rolled, o.damage[0].applied), (11, 11));
    let t = event(&mut player).await;
    assert_eq!(upserted(&t).health_state, HealthState::Down as i32);
    assert_eq!(upserted(&t).hp_current, None);
    event(&mut gm).await;
    assert_eq!(upserted(&event(&mut gm).await).hp_current, Some(0));

    // Already at 0: not dropped again, nothing changed, so no upsert.
    use_ability(
        &mut player,
        "u2",
        "token-brannoch",
        "longsword",
        &["token-goblin"],
    )
    .await;
    let e = event(&mut player).await;
    assert!(!resolution(&e).outcomes[0].dropped_to_zero);
    assert_settled(&mut player, 3).await;
}

#[tokio::test]
async fn temporary_hit_points_are_spent_first() {
    let addr = start(faces(&[10, 4])).await;
    let (mut player, _) = join(addr, "player").await;
    let (mut gm, _) = join(addr, "gm").await;
    set_hp(&mut gm, "s1", "token-goblin", 9, 3).await;
    assert_eq!(upserted(&event(&mut gm).await).hp_temp, Some(3));
    use_ability(
        &mut player,
        "u1",
        "token-brannoch",
        "longsword",
        &["token-goblin"],
    )
    .await;
    event(&mut gm).await;
    // 7 damage: 3 temporary, 4 from 9 leaves 5.
    let t = upserted(&event(&mut gm).await).clone();
    assert_eq!((t.hp_current, t.hp_temp), (Some(5), Some(0)));
    assert_eq!(t.health_state, HealthState::Healthy as i32);
    // The player saw the set-HP upsert, the resolution and the damage upsert.
    for _ in 0..3 {
        event(&mut player).await;
    }
}

// ------------------------------------------------------------------- saves

#[tokio::test]
async fn choking_ash_failed_save_halves_nothing_and_blinds() {
    // Brannoch saves CON: d20 3 + 2 (CON 14) + 2 (proficient) = 7 against DC 11.
    // Damage 2d4 = 3 and 4.
    let addr = start(faces(&[3, 3, 4])).await;
    let (mut gm, _) = join(addr, "gm").await;
    let (mut player, _) = join(addr, "player").await;
    use_ability(
        &mut gm,
        "u1",
        "token-ember-wisp",
        "choking-ash",
        &["token-brannoch"],
    )
    .await;
    let e = event(&mut gm).await;
    let r = resolution(&e);
    assert_eq!(r.outcomes.len(), 1);
    let o = &r.outcomes[0];
    assert_eq!(o.token_id, "token-brannoch");
    assert_eq!(o.outcome, HitOutcome::SaveFailed as i32);
    assert_eq!(o.step_index, 0);
    let save = o.roll.as_ref().unwrap();
    assert_eq!(save.purpose, RollPurpose::Save as i32);
    assert_eq!(save.roller_creature_id, "creature-brannoch");
    assert_eq!(save.notation, "1d20+4");
    assert_eq!(save.total, 7);
    assert_eq!(save.modifiers.len(), 2);
    assert_eq!(o.damage_rolls.len(), 1);
    assert_eq!(o.damage_rolls[0].notation, "2d4");
    assert_eq!(o.damage_rolls[0].purpose, RollPurpose::Damage as i32);
    assert_eq!(o.damage_rolls[0].roller_creature_id, "creature-ember-wisp");
    assert_eq!((o.damage[0].rolled, o.damage[0].applied), (7, 7));
    assert_eq!(o.effects_applied, ["blinded"]);
    let t = upserted(&event(&mut gm).await).clone();
    assert_eq!(t.hp_current, Some(21));
    assert_eq!(t.conditions, ["blinded"]);

    // The player controls Brannoch, so sees the numbers; conditions go to all.
    event(&mut player).await;
    let t = upserted(&event(&mut player).await).clone();
    assert_eq!((t.hp_current, t.conditions.len()), (Some(21), 1));
}

#[tokio::test]
async fn choking_ash_successful_save_halves_damage_and_does_not_blind() {
    // d20 10 + 4 = 14 against DC 11 succeeds; 2d4 = 3 and 4 is 7, half is 3.
    let addr = start(faces(&[10, 3, 4])).await;
    let (mut gm, _) = join(addr, "gm").await;
    use_ability(
        &mut gm,
        "u1",
        "token-ember-wisp",
        "choking-ash",
        &["token-brannoch"],
    )
    .await;
    let e = event(&mut gm).await;
    let o = &resolution(&e).outcomes[0];
    assert_eq!(o.outcome, HitOutcome::SaveSucceeded as i32);
    assert_eq!((o.damage[0].rolled, o.damage[0].applied), (7, 3));
    assert!(o.effects_applied.is_empty());
    let t = upserted(&event(&mut gm).await).clone();
    assert_eq!(t.hp_current, Some(25));
    assert!(t.conditions.is_empty());
}

#[tokio::test]
async fn a_save_with_no_half_damage_deals_none_on_success_and_rolls_no_damage() {
    // Sacred Flame (DC 13 DEX, no half): the goblin has no DEX proficiency,
    // so d20 11 + 2 = 13 meets the DC. Nothing is rolled for damage.
    let addr = start(faces(&[11])).await;
    let (mut player, _) = join(addr, "player").await;
    use_ability(
        &mut player,
        "u1",
        "token-maribel",
        "cantrip-sacred-flame",
        &["token-goblin"],
    )
    .await;
    let e = event(&mut player).await;
    let o = &resolution(&e).outcomes[0];
    assert_eq!(o.outcome, HitOutcome::SaveSucceeded as i32);
    assert!(o.damage.is_empty() && o.damage_rolls.is_empty());
    assert_eq!(o.roll.as_ref().unwrap().modifiers.len(), 1);
    assert_settled(&mut player, 1).await;
}

#[tokio::test]
async fn an_area_ability_rolls_damage_once_and_warns_once() {
    // Burning Hands (DC 13 DEX, half). Goblin: d20 5 + 2 = 7 fails. Wisp: d20
    // 10 + 2 + 2 = 14 succeeds. 3d6 once: 3, 3, 3 = 9 (goblin all 9, wisp 4).
    let addr = start(faces(&[5, 10, 3, 3, 3])).await;
    let (mut gm, _) = join(addr, "gm").await;
    let (mut player, _) = join(addr, "player").await;
    use_ability(
        &mut player,
        "u1",
        "token-ilvaren",
        "burning-hands",
        &["token-goblin", "token-ember-wisp"],
    )
    .await;
    let e = event(&mut gm).await;
    let codes: Vec<&str> = e.warnings.iter().map(|w| w.code.as_str()).collect();
    assert_eq!(codes, ["area_not_checked"]);
    let r = resolution(&e);
    assert_eq!(r.warnings, e.warnings);
    assert_eq!(r.outcomes.len(), 2);
    let (goblin, wisp) = (&r.outcomes[0], &r.outcomes[1]);
    assert_eq!(goblin.outcome, HitOutcome::SaveFailed as i32);
    assert_eq!(wisp.outcome, HitOutcome::SaveSucceeded as i32);
    assert_eq!(goblin.damage_rolls, wisp.damage_rolls);
    assert_eq!((goblin.damage[0].rolled, goblin.damage[0].applied), (9, 9));
    assert_eq!((wisp.damage[0].rolled, wisp.damage[0].applied), (9, 4));
    assert!(goblin.dropped_to_zero && !wisp.dropped_to_zero);
    // One upsert per changed target, in target order.
    let first = upserted(&event(&mut gm).await).clone();
    let second = upserted(&event(&mut gm).await).clone();
    assert_eq!(first.token_id, "token-goblin");
    assert_eq!(first.health_state, HealthState::Down as i32);
    assert_eq!(
        (second.token_id.as_str(), second.hp_current),
        ("token-ember-wisp", Some(7))
    );
    assert_settled(&mut gm, 3).await;
}

// ----------------------------------------------------- flat steps and effects

#[tokio::test]
async fn cure_wounds_is_capped_at_max_hit_points() {
    // Brannoch is at 20 of 28; 2d8 is 8 and 8, +6 = 22, but only 8 fit.
    let addr = start(faces(&[8, 8])).await;
    let (mut player, _) = join(addr, "player").await;
    set_hp(&mut player, "s1", "token-brannoch", 20, 0).await;
    event(&mut player).await;
    use_ability(
        &mut player,
        "u1",
        "token-maribel",
        "cure-wounds",
        &["token-brannoch"],
    )
    .await;
    let e = event(&mut player).await;
    let o = &resolution(&e).outcomes[0];
    assert_eq!(o.outcome, HitOutcome::Automatic as i32);
    assert_eq!(o.healing, 8);
    assert!(o.roll.is_none() && o.damage.is_empty());
    assert_eq!(o.damage_rolls.len(), 1);
    assert_eq!(o.damage_rolls[0].purpose, RollPurpose::Healing as i32);
    assert_eq!(o.damage_rolls[0].total, 22);
    assert_eq!(upserted(&event(&mut player).await).hp_current, Some(28));
}

#[tokio::test]
async fn second_wind_heals_the_source_token() {
    let addr = start(faces(&[5])).await;
    let (mut player, _) = join(addr, "player").await;
    set_hp(&mut player, "s1", "token-brannoch", 10, 0).await;
    event(&mut player).await;
    use_ability(&mut player, "u1", "token-brannoch", "second-wind", &[]).await;
    let e = event(&mut player).await;
    let r = resolution(&e);
    assert_eq!(r.outcomes.len(), 1);
    assert_eq!(r.outcomes[0].token_id, "token-brannoch");
    assert_eq!(r.outcomes[0].healing, 8);
    assert_eq!(upserted(&event(&mut player).await).hp_current, Some(18));
}

#[tokio::test]
async fn an_effect_only_ability_adds_the_condition_and_reports_it() {
    let addr = start(faces(&[])).await;
    let (mut player, _) = join(addr, "player").await;
    use_ability(
        &mut player,
        "u1",
        "token-korvath",
        "hunters-mark",
        &["token-goblin"],
    )
    .await;
    let e = event(&mut player).await;
    let o = &resolution(&e).outcomes[0];
    assert_eq!(o.outcome, HitOutcome::Automatic as i32);
    assert_eq!(o.effects_applied, ["hunters_mark"]);
    // The condition is public even though the player cannot see HP.
    let t = upserted(&event(&mut player).await).clone();
    assert_eq!(t.conditions, ["hunters_mark"]);
    assert_eq!(t.hp_current, None);
}

// ------------------------------------------------------ hit points, conditions

#[tokio::test]
async fn set_hit_points_updates_the_token_and_health_state() {
    let addr = start(faces(&[])).await;
    let (mut player, _) = join(addr, "player").await;
    set_hp(&mut player, "s1", "token-brannoch", 5, 2).await;
    let e = event(&mut player).await;
    assert_eq!((e.seq, e.client_intent_id.as_str()), (1, "s1"));
    let t = upserted(&e);
    assert_eq!(
        (t.hp_current, t.hp_max, t.hp_temp),
        (Some(5), Some(28), Some(2))
    );
    assert_eq!(t.health_state, HealthState::NearDeath as i32);
    // hp_temp 0 clears temporary hit points.
    set_hp(&mut player, "s2", "token-brannoch", 14, 0).await;
    let t = upserted(&event(&mut player).await).clone();
    assert_eq!(
        (t.hp_temp, t.health_state),
        (Some(0), HealthState::Bloodied as i32)
    );
}

#[tokio::test]
async fn set_hit_points_rejections() {
    let addr = start(faces(&[])).await;
    let (mut player, _) = join(addr, "player").await;
    let (mut gm, _) = join(addr, "gm").await;
    // A player who does not control the token, or names a missing one.
    set_hp(&mut player, "r1", "token-goblin", 1, 0).await;
    assert_rejected(&mut player, "not_permitted").await;
    set_hp(&mut player, "r2", "token-nobody", 1, 0).await;
    assert_rejected(&mut player, "not_permitted").await;
    set_hp(&mut gm, "r3", "token-nobody", 1, 0).await;
    assert_rejected(&mut gm, "unknown_token").await;
    for (current, temp) in [(-1, 0), (29, 0), (1, -1), (1, 1000)] {
        set_hp(&mut player, "r4", "token-brannoch", current, temp).await;
        assert_rejected(&mut player, "bad_hit_points").await;
    }
    set_hp(&mut player, "r5", "token-brannoch", 28, 999).await;
    assert_eq!(upserted(&event(&mut player).await).hp_temp, Some(999));
    event(&mut gm).await;

    // A prop has no hit points.
    send_intent(
        &mut gm,
        "p",
        intent::Kind::CreateToken(CreateTokenIntent {
            scene_id: "scene-mock".into(),
            position: Some(Vec2 { x: 70.0, y: 70.0 }),
            ..Default::default()
        }),
    )
    .await;
    let prop = upserted(&event(&mut gm).await).token_id.clone();
    set_hp(&mut gm, "r6", &prop, 0, 0).await;
    assert_rejected(&mut gm, "no_hit_points").await;
}

#[tokio::test]
async fn set_conditions_adds_removes_and_always_upserts() {
    let addr = start(faces(&[])).await;
    let (mut gm, _) = join(addr, "gm").await;
    let (mut player, _) = join(addr, "player").await;
    set_conditions(
        &mut gm,
        "c1",
        "token-goblin",
        &["prone", "blinded", "prone"],
        &[],
    )
    .await;
    let e = event(&mut gm).await;
    assert_eq!(upserted(&e).conditions, ["prone", "blinded"]);
    // The player sees the conditions but not the hit points.
    let t = upserted(&event(&mut player).await).clone();
    assert_eq!(t.conditions, ["prone", "blinded"]);
    assert_eq!(t.hp_current, None);

    // Removing one absent and one present; adding one already there.
    set_conditions(
        &mut gm,
        "c2",
        "token-goblin",
        &["prone"],
        &["blinded", "charmed"],
    )
    .await;
    assert_eq!(upserted(&event(&mut gm).await).conditions, ["prone"]);
    // No change is still answered.
    set_conditions(&mut gm, "c3", "token-goblin", &[], &["charmed"]).await;
    assert_eq!(upserted(&event(&mut gm).await).conditions, ["prone"]);
}

#[tokio::test]
async fn set_conditions_rejections() {
    let addr = start(faces(&[])).await;
    let (mut gm, _) = join(addr, "gm").await;
    let (mut player, _) = join(addr, "player").await;
    set_conditions(&mut player, "r1", "token-brannoch", &["prone"], &[]).await;
    assert_rejected(&mut player, "not_permitted").await;
    set_conditions(&mut gm, "r2", "token-nobody", &["prone"], &[]).await;
    assert_rejected(&mut gm, "unknown_token").await;
    let too_long = "a".repeat(41);
    for bad in ["", "Prone", "hunters-mark", "two words", too_long.as_str()] {
        set_conditions(&mut gm, "r3", "token-goblin", &[bad], &[]).await;
        assert_rejected(&mut gm, "bad_condition_id").await;
    }
    set_conditions(&mut gm, "r4", "token-goblin", &["prone"], &["prone"]).await;
    assert_rejected(&mut gm, "conflicting_conditions").await;
    let many: Vec<String> = (0..33).map(|i| format!("c{i}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    set_conditions(&mut gm, "r5", "token-goblin", &many, &[]).await;
    assert_rejected(&mut gm, "too_many_conditions").await;
    // Exactly 32 is fine, and the rejected attempt changed nothing.
    set_conditions(&mut gm, "r6", "token-goblin", &many[..32], &[]).await;
    assert_eq!(upserted(&event(&mut gm).await).conditions.len(), 32);
}

// ------------------------------------------------------------ create, remove

#[tokio::test]
async fn create_and_remove_a_token() {
    let addr = start(faces(&[])).await;
    let (mut gm, _) = join(addr, "gm").await;
    let (mut player, _) = join(addr, "player").await;
    send_intent(
        &mut gm,
        "k1",
        intent::Kind::CreateToken(CreateTokenIntent {
            scene_id: "scene-mock".into(),
            creature_id: "creature-goblin".into(),
            position: Some(Vec2 { x: 500.0, y: 300.0 }),
            ..Default::default()
        }),
    )
    .await;
    let e = event(&mut gm).await;
    let t = upserted(&e).clone();
    assert_eq!(t.token_id, "token-new-1");
    assert_eq!(
        (t.name.as_str(), t.level_id.as_str()),
        ("Goblin Skulker", "level-ground")
    );
    assert_eq!((t.hp_current, t.hp_max), (Some(9), Some(9)));
    assert_eq!(t.health_state, HealthState::Healthy as i32);
    assert!(t.conditions.is_empty());
    assert_eq!(t.position, Some(Vec2 { x: 500.0, y: 300.0 }));
    // No controllers: no creature sheet event, and the player sees no HP.
    let p = upserted(&event(&mut player).await).clone();
    assert_eq!((p.token_id.as_str(), p.hp_current), ("token-new-1", None));
    assert_settled(&mut gm, 1).await;
    event(&mut player).await;

    // With a controller the sheet follows, but only to those who may see it.
    send_intent(
        &mut gm,
        "k2",
        intent::Kind::CreateToken(CreateTokenIntent {
            scene_id: "scene-mock".into(),
            creature_id: "creature-goblin".into(),
            name: "Pet goblin".into(),
            position: Some(Vec2 { x: 1.0, y: 2.0 }),
            controller_user_ids: vec!["mock-player".into()],
            ..Default::default()
        }),
    )
    .await;
    let a = event(&mut gm).await;
    let b = event(&mut gm).await;
    assert_eq!(
        (upserted(&a).token_id.as_str(), upserted(&a).name.as_str()),
        ("token-new-2", "Pet goblin")
    );
    assert!(matches!(b.kind, Some(event::Kind::CreatureUpserted(_))));
    assert_eq!(
        (a.seq + 1, a.client_intent_id.as_str()),
        (b.seq, b.client_intent_id.as_str())
    );
    let a = event(&mut player).await;
    assert_eq!(upserted(&a).hp_current, Some(9));
    let b = event(&mut player).await;
    assert!(matches!(b.kind, Some(event::Kind::CreatureUpserted(_))));

    send_intent(
        &mut gm,
        "k3",
        intent::Kind::RemoveToken(RemoveTokenIntent {
            token_id: "token-new-1".into(),
        }),
    )
    .await;
    let e = event(&mut gm).await;
    assert!(matches!(&e.kind, Some(event::Kind::TokenRemoved(r)) if r.token_id == "token-new-1"));
    let (_g, s) = join(addr, "gm").await;
    assert!(s.tokens.iter().all(|t| t.token_id != "token-new-1"));
    assert!(s.tokens.iter().any(|t| t.token_id == "token-new-2"));
}

#[tokio::test]
async fn create_and_remove_token_rejections() {
    let addr = start(faces(&[])).await;
    let (mut gm, _) = join(addr, "gm").await;
    let (mut player, _) = join(addr, "player").await;
    let base = || CreateTokenIntent {
        scene_id: "scene-mock".into(),
        position: Some(Vec2 { x: 1.0, y: 1.0 }),
        ..Default::default()
    };
    let cases: Vec<(&str, CreateTokenIntent)> = vec![
        (
            "unknown_scene",
            CreateTokenIntent {
                scene_id: "nope".into(),
                ..base()
            },
        ),
        (
            "unknown_level",
            CreateTokenIntent {
                level_id: "nope".into(),
                ..base()
            },
        ),
        (
            "unknown_creature",
            CreateTokenIntent {
                creature_id: "nope".into(),
                ..base()
            },
        ),
        (
            "bad_coordinate",
            CreateTokenIntent {
                position: Some(Vec2 {
                    x: f32::NAN,
                    y: 0.0,
                }),
                ..base()
            },
        ),
        (
            "bad_coordinate",
            CreateTokenIntent {
                position: None,
                ..base()
            },
        ),
        (
            "name_too_long",
            CreateTokenIntent {
                name: "x".repeat(121),
                ..base()
            },
        ),
        (
            "unknown_user",
            CreateTokenIntent {
                controller_user_ids: vec!["nobody".into()],
                ..base()
            },
        ),
        (
            "invalid_controller",
            CreateTokenIntent {
                controller_user_ids: vec!["mock-gm".into()],
                ..base()
            },
        ),
    ];
    for (code, c) in cases {
        send_intent(&mut gm, "k", intent::Kind::CreateToken(c)).await;
        assert_rejected(&mut gm, code).await;
    }
    send_intent(&mut player, "k", intent::Kind::CreateToken(base())).await;
    assert_rejected(&mut player, "not_permitted").await;
    // A name of exactly 120 characters is fine, and ids are minted only on success.
    send_intent(
        &mut gm,
        "k",
        intent::Kind::CreateToken(CreateTokenIntent {
            name: "x".repeat(120),
            ..base()
        }),
    )
    .await;
    assert_eq!(upserted(&event(&mut gm).await).token_id, "token-new-1");
    event(&mut player).await;

    let remove = |id: &str| {
        intent::Kind::RemoveToken(RemoveTokenIntent {
            token_id: id.into(),
        })
    };
    send_intent(&mut player, "d1", remove("token-aria")).await;
    assert_rejected(&mut player, "not_permitted").await;
    send_intent(&mut gm, "d2", remove("token-nobody")).await;
    assert_rejected(&mut gm, "unknown_token").await;
}

// -------------------------------------------------------------- encounters

#[tokio::test]
async fn encounter_start_end_turn_and_round_two() {
    // Goblin d20 5 + 2 = 7; Brannoch d20 12 + 2 = 14, so Brannoch goes first.
    let addr = start(faces(&[5, 12])).await;
    let (mut gm, _) = join(addr, "gm").await;
    let (mut player, _) = join(addr, "player").await;
    start_encounter(
        &mut gm,
        "e1",
        "scene-mock",
        &["token-goblin", "token-brannoch", "token-goblin"],
    )
    .await;
    let mut seen = Vec::new();
    for _ in 0..3 {
        seen.push(event(&mut gm).await);
    }
    assert_eq!(seen.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(seen.iter().all(|e| e.client_intent_id == "e1"));
    let totals: Vec<(i32, i32, String)> = seen[..2]
        .iter()
        .map(|e| match &e.kind {
            Some(event::Kind::DiceRolled(d)) => {
                let r = d.roll.as_ref().unwrap();
                assert_eq!(r.purpose, RollPurpose::Initiative as i32);
                (
                    r.total,
                    r.dice[0].value as i32,
                    r.roller_creature_id.clone(),
                )
            }
            other => panic!("expected dice_rolled, got {other:?}"),
        })
        .collect();
    assert_eq!(
        totals,
        [
            (7, 5, "creature-goblin".to_string()),
            (14, 12, "creature-brannoch".to_string())
        ]
    );
    let encounter = match &seen[2].kind {
        Some(event::Kind::EncounterUpdated(u)) => u.encounter.clone().unwrap(),
        other => panic!("expected encounter_updated, got {other:?}"),
    };
    assert_eq!((encounter.round, encounter.turn_index), (1, 0));
    let order: Vec<(&str, i32)> = encounter
        .combatants
        .iter()
        .map(|c| (c.token_id.as_str(), c.initiative))
        .collect();
    assert_eq!(order, [("token-brannoch", 14), ("token-goblin", 7)]);
    for _ in 0..3 {
        event(&mut player).await;
    }
    let id = encounter.encounter_id.clone();

    // A later joiner sees it in the Snapshot.
    let (_ws, s) = join(addr, "player").await;
    assert_eq!(s.encounters.len(), 1);
    assert_eq!(s.encounters[0], encounter);

    // The player controls Brannoch, whose turn it is; not the goblin's.
    end_turn(&mut player, "t1", &id).await;
    let e = event(&mut player).await;
    let enc = match e.kind {
        Some(event::Kind::EncounterUpdated(u)) => u.encounter.unwrap(),
        other => panic!("expected encounter_updated, got {other:?}"),
    };
    assert_eq!((enc.round, enc.turn_index), (1, 1));
    end_turn(&mut player, "t2", &id).await;
    assert_rejected(&mut player, "not_permitted").await;
    // The GM may end any turn; the order wraps into round 2.
    end_turn(&mut gm, "t3", &id).await;
    event(&mut gm).await;
    let e = event(&mut gm).await;
    match e.kind {
        Some(event::Kind::EncounterUpdated(u)) => {
            let enc = u.encounter.unwrap();
            assert_eq!((enc.round, enc.turn_index), (2, 0));
        }
        other => panic!("expected encounter_updated, got {other:?}"),
    }
    end_turn(&mut gm, "t4", "encounter-nope").await;
    assert_rejected(&mut gm, "unknown_encounter").await;
}

#[tokio::test]
async fn set_initiative_resorts_and_keeps_the_turn_on_the_same_combatant() {
    let addr = start(faces(&[5, 12])).await;
    let (mut gm, _) = join(addr, "gm").await;
    start_encounter(
        &mut gm,
        "e1",
        "scene-mock",
        &["token-goblin", "token-brannoch"],
    )
    .await;
    for _ in 0..3 {
        event(&mut gm).await;
    }
    end_turn(&mut gm, "t1", "encounter-1").await;
    event(&mut gm).await; // turn is now the goblin's
    send_intent(
        &mut gm,
        "i1",
        intent::Kind::SetInitiative(SetInitiativeIntent {
            encounter_id: "encounter-1".into(),
            token_id: "token-goblin".into(),
            initiative: 20,
            order: 0,
        }),
    )
    .await;
    let e = event(&mut gm).await;
    let enc = match e.kind {
        Some(event::Kind::EncounterUpdated(u)) => u.encounter.unwrap(),
        other => panic!("expected encounter_updated, got {other:?}"),
    };
    let order: Vec<(&str, i32)> = enc
        .combatants
        .iter()
        .map(|c| (c.token_id.as_str(), c.initiative))
        .collect();
    assert_eq!(order, [("token-goblin", 20), ("token-brannoch", 14)]);
    // It is still the goblin's turn, now at index 0.
    assert_eq!((enc.turn_index, enc.round), (0, 1));

    // Equal initiative: the lower `order` goes first. Brannoch was renumbered
    // to order 1, so the goblin at order 5 falls behind it, and the turn stays
    // with the goblin at its new index.
    send_intent(
        &mut gm,
        "i2",
        intent::Kind::SetInitiative(SetInitiativeIntent {
            encounter_id: "encounter-1".into(),
            token_id: "token-goblin".into(),
            initiative: 14,
            order: 5,
        }),
    )
    .await;
    let e = event(&mut gm).await;
    match e.kind {
        Some(event::Kind::EncounterUpdated(u)) => {
            let enc = u.encounter.unwrap();
            let order: Vec<&str> = enc.combatants.iter().map(|c| c.token_id.as_str()).collect();
            assert_eq!(order, ["token-brannoch", "token-goblin"]);
            assert_eq!(enc.turn_index, 1);
        }
        other => panic!("expected encounter_updated, got {other:?}"),
    }
    send_intent(
        &mut gm,
        "i3",
        intent::Kind::SetInitiative(SetInitiativeIntent {
            encounter_id: "encounter-1".into(),
            token_id: "token-aria".into(),
            initiative: 1,
            order: 0,
        }),
    )
    .await;
    assert_rejected(&mut gm, "unknown_combatant").await;
}

#[tokio::test]
async fn end_encounter_clears_it_from_later_snapshots() {
    let addr = start(faces(&[5, 12])).await;
    let (mut gm, _) = join(addr, "gm").await;
    let (mut player, _) = join(addr, "player").await;
    start_encounter(
        &mut gm,
        "e1",
        "scene-mock",
        &["token-goblin", "token-brannoch"],
    )
    .await;
    for _ in 0..3 {
        event(&mut gm).await;
        event(&mut player).await;
    }
    // A token in the encounter cannot be removed.
    send_intent(
        &mut gm,
        "d",
        intent::Kind::RemoveToken(RemoveTokenIntent {
            token_id: "token-goblin".into(),
        }),
    )
    .await;
    assert_rejected(&mut gm, "in_encounter").await;
    send_intent(
        &mut player,
        "x",
        intent::Kind::EndEncounter(EndEncounterIntent {
            encounter_id: "encounter-1".into(),
        }),
    )
    .await;
    assert_rejected(&mut player, "not_permitted").await;
    send_intent(
        &mut gm,
        "x2",
        intent::Kind::EndEncounter(EndEncounterIntent {
            encounter_id: "encounter-nope".into(),
        }),
    )
    .await;
    assert_rejected(&mut gm, "unknown_encounter").await;
    send_intent(
        &mut gm,
        "x3",
        intent::Kind::EndEncounter(EndEncounterIntent {
            encounter_id: "encounter-1".into(),
        }),
    )
    .await;
    for ws in [&mut gm, &mut player] {
        let e = event(ws).await;
        assert!(
            matches!(&e.kind, Some(event::Kind::EncounterEnded(x)) if x.encounter_id == "encounter-1")
        );
    }
    let (_ws, s) = join(addr, "gm").await;
    assert!(s.encounters.is_empty());
    // A new one may start on the scene, with the next id.
    start_encounter(&mut gm, "e2", "scene-mock", &["token-goblin"]).await;
    for _ in 0..2 {
        event(&mut gm).await;
    }
}

#[tokio::test]
async fn start_encounter_rejections() {
    let addr = start(faces(&[])).await;
    let (mut gm, _) = join(addr, "gm").await;
    let (mut player, _) = join(addr, "player").await;
    start_encounter(&mut player, "r1", "scene-mock", &["token-goblin"]).await;
    assert_rejected(&mut player, "not_permitted").await;
    start_encounter(&mut gm, "r2", "scene-nope", &["token-goblin"]).await;
    assert_rejected(&mut gm, "unknown_scene").await;
    start_encounter(&mut gm, "r3", "scene-mock", &[]).await;
    assert_rejected(&mut gm, "no_combatants").await;
    start_encounter(
        &mut gm,
        "r4",
        "scene-mock",
        &["token-goblin", "token-nobody"],
    )
    .await;
    assert_rejected(&mut gm, "unknown_token").await;
    start_encounter(&mut gm, "ok", "scene-mock", &["token-aria"]).await;
    // Aria has no creature: initiative bonus 0, so the roll is 1d20+0.
    let e = event(&mut gm).await;
    match e.kind {
        Some(event::Kind::DiceRolled(d)) => {
            let r = d.roll.unwrap();
            assert_eq!(
                (r.notation.as_str(), r.total, r.roller_creature_id.as_str()),
                ("1d20+0", 1, "")
            );
        }
        other => panic!("expected dice_rolled, got {other:?}"),
    }
    event(&mut gm).await;
    start_encounter(&mut gm, "r5", "scene-mock", &["token-goblin"]).await;
    assert_rejected(&mut gm, "encounter_running").await;
    // Nothing was rolled by the rejected starts: seqs 1 and 2 are the first.
    assert_settled(&mut gm, 2).await;
}

// -------------------------------------------------------------------- undo

#[tokio::test]
async fn undo_restores_the_goblin_after_a_damaging_attack() {
    let addr = start(faces(&[10, 4])).await;
    let (mut gm, _) = join(addr, "gm").await;
    let (mut player, _) = join(addr, "player").await;
    use_ability(
        &mut player,
        "u1",
        "token-brannoch",
        "longsword",
        &["token-goblin"],
    )
    .await;
    for _ in 0..2 {
        event(&mut gm).await;
        event(&mut player).await;
    }
    undo(&mut gm, "z1", 1).await;
    // Compensating upsert, then one EventUndone per event of the group.
    let comp = event(&mut gm).await;
    assert_eq!((comp.seq, comp.client_intent_id.as_str()), (3, "z1"));
    let t = upserted(&comp).clone();
    assert_eq!(
        (t.token_id.as_str(), t.hp_current),
        ("token-goblin", Some(9))
    );
    assert_eq!(t.health_state, HealthState::Healthy as i32);
    for (seq, undone) in [(4, 1), (5, 2)] {
        let e = event(&mut gm).await;
        assert_eq!((e.seq, e.client_intent_id.as_str()), (seq, "z1"));
        assert!(matches!(&e.kind, Some(event::Kind::EventUndone(u)) if u.undone_seq == undone));
    }
    // The player gets the same events, with the goblin's HP hidden.
    let t = upserted(&event(&mut player).await).clone();
    assert_eq!(
        (t.hp_current, t.health_state),
        (None, HealthState::Healthy as i32)
    );
    let (_ws, s) = join(addr, "gm").await;
    assert_eq!(token(&s, "token-goblin").hp_current, Some(9));
    assert_settled(&mut gm, 5).await;
}

#[tokio::test]
async fn undo_rejections() {
    let addr = start(faces(&[10, 4, 10, 4])).await;
    let (mut gm, _) = join(addr, "gm").await;
    let (mut player, _) = join(addr, "player").await;
    // seq 1-2: attack. seq 3: chat. seq 4-5: a second attack.
    use_ability(
        &mut player,
        "u1",
        "token-brannoch",
        "longsword",
        &["token-goblin"],
    )
    .await;
    send_intent(
        &mut player,
        "c",
        intent::Kind::Chat(ChatIntent { text: "hi".into() }),
    )
    .await;
    use_ability(
        &mut player,
        "u2",
        "token-brannoch",
        "longsword",
        &["token-goblin"],
    )
    .await;
    for _ in 0..5 {
        event(&mut gm).await;
        event(&mut player).await;
    }

    undo(&mut player, "z", 1).await;
    assert_rejected(&mut player, "not_permitted").await;
    undo(&mut gm, "z", 0).await;
    assert_rejected(&mut gm, "unknown_event").await;
    undo(&mut gm, "z", 99).await;
    assert_rejected(&mut gm, "unknown_event").await;
    undo(&mut gm, "z", 2).await;
    assert_rejected(&mut gm, "not_first_event").await;
    undo(&mut gm, "z", 3).await;
    assert_rejected(&mut gm, "not_undoable").await;
    // The first attack is no longer the latest group.
    undo(&mut gm, "z", 1).await;
    assert_rejected(&mut gm, "not_supported_by_mock").await;

    // Undo the latest (seq 4 plus 5), then the one before it is the latest.
    undo(&mut gm, "z4", 4).await;
    for _ in 0..3 {
        event(&mut gm).await; // compensating upsert and two EventUndone
    }
    undo(&mut gm, "z4", 4).await;
    assert_rejected(&mut gm, "already_undone").await;
    // The undo's own events are not undoable.
    undo(&mut gm, "z", 6).await;
    assert_rejected(&mut gm, "not_undoable").await;
    undo(&mut gm, "z1", 1).await;
    for _ in 0..3 {
        event(&mut gm).await;
    }
    let (_ws, s) = join(addr, "gm").await;
    assert_eq!(token(&s, "token-goblin").hp_current, Some(9));
}

#[tokio::test]
async fn undo_of_set_conditions_create_remove_and_move() {
    let addr = start(faces(&[])).await;
    let (mut gm, _) = join(addr, "gm").await;
    // seq 1: conditions.
    set_conditions(&mut gm, "c", "token-goblin", &["prone"], &[]).await;
    event(&mut gm).await;
    undo(&mut gm, "z1", 1).await;
    assert!(upserted(&event(&mut gm).await).conditions.is_empty());
    event(&mut gm).await;

    // seq 4: remove the goblin; undoing brings it back with its HP.
    send_intent(
        &mut gm,
        "d",
        intent::Kind::RemoveToken(RemoveTokenIntent {
            token_id: "token-goblin".into(),
        }),
    )
    .await;
    event(&mut gm).await;
    undo(&mut gm, "z2", 4).await;
    let t = upserted(&event(&mut gm).await).clone();
    assert_eq!(
        (t.token_id.as_str(), t.hp_current),
        ("token-goblin", Some(9))
    );
    event(&mut gm).await;
    let (_ws, s) = join(addr, "gm").await;
    assert_eq!(
        s.tokens[1].token_id, "token-goblin",
        "back in its old place"
    );

    // seq 7: create a token; undoing removes it.
    send_intent(
        &mut gm,
        "k",
        intent::Kind::CreateToken(CreateTokenIntent {
            scene_id: "scene-mock".into(),
            position: Some(Vec2 { x: 1.0, y: 1.0 }),
            ..Default::default()
        }),
    )
    .await;
    event(&mut gm).await;
    undo(&mut gm, "z3", 7).await;
    let e = event(&mut gm).await;
    assert!(matches!(&e.kind, Some(event::Kind::TokenRemoved(r)) if r.token_id == "token-new-1"));
    event(&mut gm).await;

    // seq 10: move Aria; undoing moves her back.
    send_intent(
        &mut gm,
        "m",
        intent::Kind::MoveToken(tt_contracts::v1::MoveTokenIntent {
            token_id: "token-aria".into(),
            path: vec![Vec2 { x: 300.0, y: 300.0 }],
            level_id: String::new(),
        }),
    )
    .await;
    event(&mut gm).await;
    undo(&mut gm, "z4", 10).await;
    match event(&mut gm).await.kind {
        Some(event::Kind::TokenMoved(m)) => {
            assert_eq!(m.token_id, "token-aria");
            assert_eq!(m.path, [Vec2 { x: 105.0, y: 105.0 }]);
        }
        other => panic!("expected token_moved, got {other:?}"),
    }
}

// ----------------------------------------------------- audit fixes (#132)

#[tokio::test]
async fn use_ability_refuses_too_many_and_repeated_targets() {
    let addr = start(faces(&[10, 4])).await;
    let (mut player, _) = join(addr, "player").await;
    use_ability(
        &mut player,
        "t1",
        "token-brannoch",
        "longsword",
        &["token-goblin", "token-goblin"],
    )
    .await;
    assert_rejected(&mut player, "duplicate_target").await;
    let many = vec!["token-goblin"; 65];
    use_ability(&mut player, "t2", "token-brannoch", "longsword", &many).await;
    assert_rejected(&mut player, "too_many_targets").await;
    // Scorching Ray has three attack steps: an id may appear three times.
    use_ability(
        &mut player,
        "t3",
        "token-ilvaren",
        "scorching-ray",
        &["token-goblin"; 4],
    )
    .await;
    assert_rejected(&mut player, "duplicate_target").await;
    // Nothing was used: the first event is still seq 1.
    assert_settled(&mut player, 0).await;
}

#[tokio::test]
async fn set_conditions_checks_list_lengths_first() {
    let addr = start(faces(&[])).await;
    let (mut gm, _) = join(addr, "gm").await;
    let many: Vec<String> = (0..100).map(|i| format!("c{i}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    set_conditions(&mut gm, "l1", "token-nobody", &many, &[]).await;
    assert_rejected(&mut gm, "too_many_conditions").await;
}

#[tokio::test]
async fn undoing_a_create_of_a_combatant_is_in_encounter() {
    let addr = start(faces(&[])).await;
    let (mut gm, _) = join(addr, "gm").await;
    send_intent(
        &mut gm,
        "k1",
        intent::Kind::CreateToken(CreateTokenIntent {
            scene_id: "scene-mock".into(),
            creature_id: "creature-goblin".into(),
            position: Some(Vec2 { x: 500.0, y: 300.0 }),
            ..Default::default()
        }),
    )
    .await;
    let made = event(&mut gm).await;
    let id = upserted(&made).token_id.clone();
    start_encounter(&mut gm, "s1", "scene-mock", &[id.as_str()]).await;
    let _initiative_roll = event(&mut gm).await;
    let _encounter = event(&mut gm).await;
    undo(&mut gm, "u1", made.seq).await;
    assert_rejected(&mut gm, "in_encounter").await;
    let (_ws, s) = join(addr, "gm").await;
    assert!(s.tokens.iter().any(|t| t.token_id == id));
}
