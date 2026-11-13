//! Tests for the pure core: intents, filtering, identity and the wire.
//! The socket loop itself is not covered (no WebSocket client is available).

use prost::Message as _;
use tt_contracts::v1::{
    event, intent, server_message, Combatant, Creature, Encounter, Event, Grid, Hello, Intent,
    Level, Member, Role, RollDiceIntent, RollMode, RollVisibility, Scene, ServerMessage,
    SetControllersIntent, Token, Vec2,
};
use tt_contracts::v1::{ChatIntent, MoveTokenIntent};
use tt_rules::dice::Rng;

use crate::auth::{authenticate, parse_dev_token, AuthError};
use crate::campaign::{Campaign, Rejection, MAX_CHAT_CHARS, MAX_PATH_POINTS};
use crate::ids::SequentialIds;
use crate::net::{encode, rejection_message};
use crate::rng::RollSource;
use crate::view::Viewer;

// ------------------------------------------------------------------ helpers

/// Scripted randomness: cycles through `values`; `fail` simulates an OS
/// entropy failure. A d-N die shows `value % N + 1`.
struct Scripted {
    values: Vec<u64>,
    i: usize,
    fail: bool,
}

impl Scripted {
    fn new(values: &[u64]) -> Self {
        Self {
            values: values.to_vec(),
            i: 0,
            fail: false,
        }
    }
}

impl Rng for Scripted {
    fn next_u64(&mut self) -> u64 {
        let v = self.values[self.i % self.values.len()];
        self.i += 1;
        v
    }
}

impl RollSource for Scripted {
    fn take_failure(&mut self) -> bool {
        self.fail
    }
}

fn member(id: &str, role: Role) -> Member {
    Member {
        user_id: id.into(),
        display_name: id.into(),
        role: role as i32,
        online: false,
    }
}

fn token(id: &str, creature: &str, controllers: &[&str], hidden: bool, x: f32) -> Token {
    Token {
        token_id: id.into(),
        scene_id: "s1".into(),
        level_id: "l1".into(),
        creature_id: creature.into(),
        name: id.into(),
        position: Some(Vec2 { x, y: 105.0 }),
        hp_current: Some(10),
        hp_max: Some(12),
        hp_temp: Some(1),
        controller_user_ids: controllers.iter().map(|s| s.to_string()).collect(),
        hidden,
        ..Default::default()
    }
}

fn creature(id: &str, owner: &str) -> Creature {
    Creature {
        creature_id: id.into(),
        name: id.into(),
        owner_user_id: owner.into(),
        armor_class: 15,
        hp_current: 10,
        hp_max: 12,
        ..Default::default()
    }
}

fn scene(id: &str, cell: f32, feet: f32) -> Scene {
    Scene {
        scene_id: id.into(),
        campaign_id: "camp".into(),
        name: id.into(),
        grid: Some(Grid {
            cell_size: cell,
            feet_per_cell: feet,
            ..Default::default()
        }),
        levels: vec![Level {
            level_id: format!("l-{id}"),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// gm; players alice and bob; spectator spec.
/// Tokens: t-alice (alice, creature c-alice), t-goblin (nobody, creature
/// c-goblin), t-hidden (nobody, hidden, creature c-hidden).
fn campaign() -> Campaign {
    let mut s1 = scene("s1", 70.0, 5.0);
    s1.levels[0].level_id = "l1".into();
    Campaign {
        campaign_id: "camp".into(),
        members: vec![
            member("gm", Role::Gm),
            member("alice", Role::Player),
            member("bob", Role::Player),
            member("spec", Role::Spectator),
        ],
        scenes: vec![s1, scene("s2", 100.0, 10.0)],
        tokens: vec![
            token("t-alice", "c-alice", &["alice"], false, 105.0),
            token("t-goblin", "c-goblin", &[], false, 385.0),
            token("t-hidden", "c-hidden", &[], true, 525.0),
        ],
        creatures: vec![
            creature("c-alice", "alice"),
            creature("c-goblin", ""),
            creature("c-hidden", ""),
            creature("c-bob", "bob"),
        ],
        active_scene_id: "s1".into(),
        ..Default::default()
    }
}

fn viewer(c: &Campaign, id: &str) -> Viewer {
    Viewer {
        user_id: id.into(),
        role: c.role_of(id).expect("test user is a member"),
    }
}

fn intent_of(kind: intent::Kind) -> Intent {
    Intent {
        client_intent_id: "ci-1".into(),
        kind: Some(kind),
    }
}

fn p(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}

fn mv(token_id: &str, path: Vec<Vec2>) -> Intent {
    intent_of(intent::Kind::MoveToken(MoveTokenIntent {
        token_id: token_id.into(),
        path,
        level_id: String::new(),
    }))
}

fn chat(text: &str) -> Intent {
    intent_of(intent::Kind::Chat(ChatIntent { text: text.into() }))
}

fn roll(notation: &str, mode: RollMode, vis: RollVisibility, label: &str) -> Intent {
    intent_of(intent::Kind::RollDice(RollDiceIntent {
        notation: notation.into(),
        mode: mode as i32,
        visibility: vis as i32,
        label: label.into(),
    }))
}

fn set_ctl(token_id: &str, users: &[&str]) -> Intent {
    intent_of(intent::Kind::SetControllers(SetControllersIntent {
        token_id: token_id.into(),
        controller_user_ids: users.iter().map(|s| s.to_string()).collect(),
    }))
}

/// Apply with fixed time, scripted dice and sequential ids.
fn run(c: &mut Campaign, actor: &str, i: Intent) -> Result<Vec<Event>, Rejection> {
    run_with(c, actor, i, &mut Scripted::new(&[0]))
}

fn run_with(
    c: &mut Campaign,
    actor: &str,
    i: Intent,
    rng: &mut Scripted,
) -> Result<Vec<Event>, Rejection> {
    c.apply(actor, i, 1_000, rng, &mut SequentialIds::new())
}

fn code(r: Result<Vec<Event>, Rejection>) -> &'static str {
    r.expect_err("expected a rejection").code
}

fn token_pos(c: &Campaign, id: &str) -> Option<Vec2> {
    c.tokens.iter().find(|t| t.token_id == id)?.position
}

fn roll_of(e: &Event) -> &tt_contracts::v1::RollResult {
    match e.kind.as_ref() {
        Some(event::Kind::DiceRolled(d)) => d.roll.as_ref().expect("roll present"),
        other => panic!("not a dice event: {other:?}"),
    }
}

/// Assert a rejected intent changed nothing.
fn assert_untouched(c: &Campaign, seq: u64, log: usize, tokens: &[Token]) {
    assert_eq!(c.seq, seq);
    assert_eq!(c.log.len(), log);
    assert_eq!(c.tokens, tokens);
}

// --------------------------------------------------------------- move_token

#[test]
fn gm_moves_any_token_and_state_changes() {
    let mut c = campaign();
    let events = run(
        &mut c,
        "gm",
        mv("t-goblin", vec![p(455.0, 105.0), p(455.0, 245.0)]),
    )
    .unwrap();
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!((e.seq, e.at_ms, e.scene_id.as_str()), (1, 1_000, "s1"));
    assert_eq!(e.actor_user_id, "gm");
    assert_eq!(e.client_intent_id, "ci-1");
    match e.kind.as_ref() {
        Some(event::Kind::TokenMoved(m)) => {
            // 385->455 = 70 units, then 140 units: 3 cells at 5 ft.
            assert_eq!(m.distance_feet, 15.0);
            assert_eq!(m.path.len(), 2);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(token_pos(&c, "t-goblin"), Some(p(455.0, 245.0)));
    assert_eq!(c.log, events);
}

#[test]
fn player_moves_controlled_token() {
    let mut c = campaign();
    run(&mut c, "alice", mv("t-alice", vec![p(175.0, 105.0)])).unwrap();
    assert_eq!(token_pos(&c, "t-alice"), Some(p(175.0, 105.0)));
}

#[test]
fn player_cannot_move_uncontrolled_missing_or_hidden_token() {
    let mut c = campaign();
    // Even a hidden token the player is (wrongly) listed on stays immovable.
    c.tokens[2].controller_user_ids = vec!["alice".into()];
    let before = c.tokens.clone();
    for id in ["t-goblin", "t-nope", "t-hidden"] {
        let r = run(&mut c, "alice", mv(id, vec![p(1.0, 1.0)]));
        assert_eq!(code(r), "not_permitted", "{id}");
    }
    // Bob does not control Alice's token.
    assert_eq!(
        code(run(&mut c, "bob", mv("t-alice", vec![p(1.0, 1.0)]))),
        "not_permitted"
    );
    assert_untouched(&c, 0, 0, &before);
}

#[test]
fn gm_gets_unknown_token() {
    let mut c = campaign();
    assert_eq!(
        code(run(&mut c, "gm", mv("t-nope", vec![p(1.0, 1.0)]))),
        "unknown_token"
    );
}

#[test]
fn set_controllers_then_player_can_move() {
    let mut c = campaign();
    assert_eq!(
        code(run(&mut c, "bob", mv("t-goblin", vec![p(1.0, 1.0)]))),
        "not_permitted"
    );
    run(&mut c, "gm", set_ctl("t-goblin", &["bob"])).unwrap();
    run(&mut c, "bob", mv("t-goblin", vec![p(1.0, 1.0)])).unwrap();
    assert_eq!(token_pos(&c, "t-goblin"), Some(p(1.0, 1.0)));
}

#[test]
fn bad_paths_are_rejected_and_change_nothing() {
    let mut c = campaign();
    let before = c.tokens.clone();
    let too_many = vec![p(1.0, 1.0); MAX_PATH_POINTS + 1];
    let cases: Vec<(Vec<Vec2>, &str)> = vec![
        (vec![], "empty_path"),
        (too_many, "path_too_long"),
        (vec![p(f32::NAN, 0.0)], "bad_coordinate"),
        (vec![p(0.0, f32::INFINITY)], "bad_coordinate"),
        (
            vec![p(1.0, 1.0), p(f32::NEG_INFINITY, 0.0)],
            "bad_coordinate",
        ),
        (vec![p(1.0e30, 0.0)], "bad_coordinate"),
    ];
    for (path, expected) in cases {
        assert_eq!(code(run(&mut c, "gm", mv("t-goblin", path))), expected);
    }
    assert_untouched(&c, 0, 0, &before);
    // The maximum is fine.
    run(
        &mut c,
        "gm",
        mv("t-goblin", vec![p(1.0, 1.0); MAX_PATH_POINTS]),
    )
    .unwrap();
}

#[test]
fn level_id_is_applied_or_rejected() {
    let mut c = campaign();
    let m = |level: &str| {
        intent_of(intent::Kind::MoveToken(MoveTokenIntent {
            token_id: "t-goblin".into(),
            path: vec![p(2.0, 2.0)],
            level_id: level.into(),
        }))
    };
    assert_eq!(code(run(&mut c, "gm", m("nowhere"))), "unknown_level");
    assert_eq!(c.tokens[1].level_id, "l1");
    run(&mut c, "gm", m("l1")).unwrap();
    assert_eq!(c.tokens[1].level_id, "l1");
}

#[test]
fn distance_uses_the_tokens_own_scene() {
    let mut c = campaign();
    c.tokens[1].scene_id = "s2".into();
    c.tokens[1].position = Some(p(0.0, 0.0));
    // s2: cell 100 units = 10 ft; 200 units = 20 ft (s1 would say 14 ft).
    let events = run(&mut c, "gm", mv("t-goblin", vec![p(200.0, 0.0)])).unwrap();
    match events[0].kind.as_ref() {
        Some(event::Kind::TokenMoved(m)) => assert_eq!(m.distance_feet, 20.0),
        other => panic!("{other:?}"),
    }
    assert_eq!(events[0].scene_id, "s2");
}

// --------------------------------------------------------------------- chat

#[test]
fn chat_happy_and_limits() {
    let mut c = campaign();
    let events = run(&mut c, "alice", chat("  hello  ")).unwrap();
    match events[0].kind.as_ref() {
        Some(event::Kind::ChatPosted(m)) => {
            assert_eq!((m.user_id.as_str(), m.text.as_str()), ("alice", "hello"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(code(run(&mut c, "alice", chat(""))), "empty_chat");
    assert_eq!(code(run(&mut c, "alice", chat(" \n\t "))), "empty_chat");
    run(&mut c, "alice", chat(&"x".repeat(MAX_CHAT_CHARS))).unwrap();
    assert_eq!(
        code(run(&mut c, "alice", chat(&"x".repeat(MAX_CHAT_CHARS + 1)))),
        "chat_too_long"
    );
    // Characters, not bytes.
    run(&mut c, "gm", chat(&"é".repeat(MAX_CHAT_CHARS))).unwrap();
    assert_eq!(c.seq, 3);
}

// -------------------------------------------------------------------- dice

#[test]
fn roll_happy_path_builds_full_result() {
    let mut c = campaign();
    // d20 shows 14 (13 % 20 + 1); 5 would be a d6 showing 6 but only one die.
    let mut rng = Scripted::new(&[13]);
    let i = roll(
        " D20 + 5 ",
        RollMode::Normal,
        RollVisibility::Public,
        "Stealth",
    );
    let events = run_with(&mut c, "alice", i, &mut rng).unwrap();
    let r = roll_of(&events[0]);
    assert_eq!(r.notation, "1d20+5");
    assert_eq!(r.total, 19);
    assert_eq!(r.dice.len(), 1);
    assert_eq!(r.dice[0].value, 14);
    assert_eq!(r.modifiers.len(), 1);
    assert_eq!(r.modifiers[0].value, 5);
    assert_eq!(r.roller_user_id, "alice");
    assert_eq!(r.label, "Stealth");
    assert_eq!(r.purpose, tt_contracts::v1::RollPurpose::Freeform as i32);
    assert_eq!(r.mode, RollMode::Normal as i32);
    assert_eq!(r.roll_id.len(), 36);
    assert!(!r.critical && !r.fumble);
}

#[test]
fn roll_critical_and_advantage_are_reported() {
    let mut c = campaign();
    let mut rng = Scripted::new(&[19, 4]);
    let i = roll("d20", RollMode::Advantage, RollVisibility::Public, "");
    let events = run_with(&mut c, "alice", i, &mut rng).unwrap();
    let r = roll_of(&events[0]);
    assert_eq!(r.mode, RollMode::Advantage as i32);
    assert_eq!(r.dice.len(), 2);
    assert_eq!(r.total, 20);
    assert!(r.critical);
}

#[test]
fn only_a_gm_can_hide_a_roll() {
    let mut c = campaign();
    let hidden = |who: &str, vis, c: &mut Campaign| {
        let events = run(c, who, roll("d20", RollMode::Normal, vis, "")).unwrap();
        roll_of(&events[0]).visibility
    };
    assert_eq!(
        hidden("alice", RollVisibility::GmOnly, &mut c),
        RollVisibility::Public as i32
    );
    assert_eq!(
        hidden("gm", RollVisibility::GmOnly, &mut c),
        RollVisibility::GmOnly as i32
    );
    assert_eq!(
        hidden("gm", RollVisibility::Public, &mut c),
        RollVisibility::Public as i32
    );
    assert_eq!(
        hidden("gm", RollVisibility::Unspecified, &mut c),
        RollVisibility::Public as i32
    );
}

#[test]
fn bad_notation_maps_to_dice_error_code() {
    let mut c = campaign();
    let go = |c: &mut Campaign, n: &str| {
        run(
            c,
            "alice",
            roll(n, RollMode::Normal, RollVisibility::Public, ""),
        )
    };
    let r = go(&mut c, "").unwrap_err();
    assert_eq!(r.code, "dice_empty");
    assert!(!r.message.is_empty());
    assert_eq!(code(go(&mut c, "2d")), "dice_syntax");
    assert_eq!(code(go(&mut c, "1d7")), "dice_bad_sides");
    assert_eq!(code(go(&mut c, "101d6")), "dice_too_many_dice");
    assert_eq!(c.seq, 0);
}

#[test]
fn label_limit() {
    let mut c = campaign();
    let go = |c: &mut Campaign, n: usize| {
        run(
            c,
            "alice",
            roll(
                "d20",
                RollMode::Normal,
                RollVisibility::Public,
                &"l".repeat(n),
            ),
        )
    };
    go(&mut c, 120).unwrap();
    assert_eq!(code(go(&mut c, 121)), "label_too_long");
}

#[test]
fn rng_failure_rejects_and_changes_nothing() {
    let mut c = campaign();
    let mut rng = Scripted::new(&[0]);
    rng.fail = true;
    let i = roll("d20", RollMode::Normal, RollVisibility::Public, "");
    assert_eq!(
        code(run_with(&mut c, "alice", i, &mut rng)),
        "rng_unavailable"
    );
    assert_eq!(c.seq, 0);
    assert!(c.log.is_empty());
}

// ---------------------------------------------------------- set_controllers

#[test]
fn set_controllers_rules() {
    let mut c = campaign();
    let before = c.tokens.clone();
    assert_eq!(
        code(run(&mut c, "alice", set_ctl("t-alice", &["alice", "bob"]))),
        "not_permitted"
    );
    assert_eq!(
        code(run(&mut c, "gm", set_ctl("t-nope", &["bob"]))),
        "unknown_token"
    );
    assert_eq!(
        code(run(&mut c, "gm", set_ctl("t-alice", &["bob", "ghost"]))),
        "unknown_user"
    );
    assert_eq!(
        code(run(&mut c, "gm", set_ctl("t-alice", &["spec"]))),
        "invalid_controller"
    );
    assert_untouched(&c, 0, 0, &before);

    let events = run(&mut c, "gm", set_ctl("t-alice", &["bob", "bob", "gm"])).unwrap();
    assert_eq!(c.tokens[0].controller_user_ids, ["bob", "gm"]);
    // Token update, then the creature sheet for the new controller.
    assert_eq!(events.len(), 2);
    assert_eq!((events[0].seq, events[1].seq), (1, 2));
    assert!(matches!(
        events[0].kind,
        Some(event::Kind::TokenUpserted(_))
    ));
    assert!(matches!(
        events[1].kind,
        Some(event::Kind::CreatureUpserted(_))
    ));
    // Clearing the list is allowed.
    run(&mut c, "gm", set_ctl("t-alice", &[])).unwrap();
    assert!(c.tokens[0].controller_user_ids.is_empty());
}

// --------------------------------------------------- spectators / the rest

#[test]
fn spectators_and_strangers_are_refused_everything() {
    let mut c = campaign();
    let before = c.tokens.clone();
    let intents = vec![
        mv("t-alice", vec![p(1.0, 1.0)]),
        chat("hi"),
        roll("d20", RollMode::Normal, RollVisibility::Public, ""),
        set_ctl("t-alice", &["spec"]),
        intent_of(intent::Kind::Undo(tt_contracts::v1::UndoIntent {
            event_seq: 1,
        })),
        Intent::default(),
    ];
    for who in ["spec", "nobody", ""] {
        for i in &intents {
            assert_eq!(code(run(&mut c, who, i.clone())), "not_permitted", "{who}");
        }
    }
    assert_untouched(&c, 0, 0, &before);
}

#[test]
fn unimplemented_intents_are_rejected() {
    use tt_contracts::v1 as pb;
    let mut c = campaign();
    let all = vec![
        intent::Kind::UseAbility(pb::UseAbilityIntent::default()),
        intent::Kind::StartEncounter(pb::StartEncounterIntent::default()),
        intent::Kind::EndTurn(pb::EndTurnIntent::default()),
        intent::Kind::SetInitiative(pb::SetInitiativeIntent::default()),
        intent::Kind::Undo(pb::UndoIntent::default()),
        intent::Kind::SetHitPoints(pb::SetHitPointsIntent::default()),
    ];
    for k in all {
        assert_eq!(
            code(run(&mut c, "gm", intent_of(k.clone()))),
            "not_implemented"
        );
        assert_eq!(code(run(&mut c, "alice", intent_of(k))), "not_implemented");
    }
    let r = run(&mut c, "gm", Intent::default());
    assert_eq!(code(r), "malformed_intent");
    assert_eq!(c.seq, 0);
}

#[test]
fn seq_strictly_increases_and_log_matches() {
    let mut c = campaign();
    let mut seen = Vec::new();
    seen.extend(run(&mut c, "alice", chat("a")).unwrap());
    let _ = run(&mut c, "alice", chat(""));
    seen.extend(run(&mut c, "gm", set_ctl("t-goblin", &["bob"])).unwrap());
    seen.extend(run(&mut c, "bob", mv("t-goblin", vec![p(3.0, 3.0)])).unwrap());
    let seqs: Vec<u64> = seen.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [1, 2, 3, 4]);
    assert_eq!(c.seq, 4);
    assert_eq!(c.log, seen);
}

// ---------------------------------------------------------------- filtering

#[test]
fn gm_only_roll_reaches_only_the_gm() {
    let mut c = campaign();
    let secret = run(
        &mut c,
        "gm",
        roll("d20", RollMode::Normal, RollVisibility::GmOnly, ""),
    )
    .unwrap();
    let public = run(
        &mut c,
        "gm",
        roll("d20", RollMode::Normal, RollVisibility::Public, ""),
    )
    .unwrap();
    let (gm, alice, spec) = (viewer(&c, "gm"), viewer(&c, "alice"), viewer(&c, "spec"));
    assert!(c.event_for(&secret[0], &gm).is_some());
    assert!(c.event_for(&secret[0], &alice).is_none());
    assert!(c.event_for(&secret[0], &spec).is_none());
    assert!(c.event_for(&public[0], &alice).is_some());
    assert!(c.event_for(&public[0], &spec).is_some());
    // An unspecified visibility is never shared.
    let mut odd = public[0].clone();
    if let Some(event::Kind::DiceRolled(d)) = odd.kind.as_mut() {
        d.roll.as_mut().unwrap().visibility = RollVisibility::Unspecified as i32;
    }
    assert!(c.event_for(&odd, &alice).is_none());
    assert!(c.event_for(&odd, &gm).is_some());
}

#[test]
fn snapshot_omits_what_a_player_may_not_see() {
    let c = campaign();
    let s = c.snapshot_for(&viewer(&c, "alice"));
    assert_eq!((s.user_id.as_str(), s.role), ("alice", Role::Player as i32));
    assert_eq!(s.seq, 0);
    assert_eq!(s.active_scene_id, "s1");
    assert_eq!(s.members.len(), 4);
    assert_eq!(s.scenes.len(), 2);
    let ids: Vec<&str> = s.tokens.iter().map(|t| t.token_id.as_str()).collect();
    assert_eq!(ids, ["t-alice", "t-goblin"]);
    // Controlled token keeps hp; the goblin loses it.
    assert_eq!(s.tokens[0].hp_current, Some(10));
    assert_eq!(s.tokens[0].hp_max, Some(12));
    assert_eq!(s.tokens[0].hp_temp, Some(1));
    assert_eq!(
        (
            s.tokens[1].hp_current,
            s.tokens[1].hp_max,
            s.tokens[1].hp_temp
        ),
        (None, None, None)
    );
    // Alice's sheet only: not the goblin's, not hidden, not Bob's.
    let sheets: Vec<&str> = s.creatures.iter().map(|c| c.creature_id.as_str()).collect();
    assert_eq!(sheets, ["c-alice"]);
}

#[test]
fn snapshot_for_gm_and_others() {
    let c = campaign();
    let gm = c.snapshot_for(&viewer(&c, "gm"));
    assert_eq!((gm.user_id.as_str(), gm.role), ("gm", Role::Gm as i32));
    assert_eq!(gm.tokens.len(), 3);
    assert!(gm.tokens.iter().all(|t| t.hp_current == Some(10)));
    assert_eq!(gm.creatures.len(), 4);
    let bob = c.snapshot_for(&viewer(&c, "bob"));
    assert_eq!(bob.tokens[0].hp_current, None);
    let sheets: Vec<&str> = bob
        .creatures
        .iter()
        .map(|c| c.creature_id.as_str())
        .collect();
    assert_eq!(sheets, ["c-bob"]);
    let spec = c.snapshot_for(&viewer(&c, "spec"));
    assert_eq!(
        (spec.user_id.as_str(), spec.role),
        ("spec", Role::Spectator as i32)
    );
    assert_eq!(spec.tokens.len(), 2);
    assert!(spec.tokens.iter().all(|t| t.hp_current.is_none()));
    assert!(spec.creatures.is_empty());
}

#[test]
fn spectator_listed_as_controller_still_sees_no_hp_or_sheet() {
    let mut c = campaign();
    c.tokens[0].controller_user_ids.push("spec".into());
    c.creatures[0].owner_user_id = "spec".into();
    let s = c.snapshot_for(&viewer(&c, "spec"));
    assert!(s.tokens.iter().all(|t| t.hp_current.is_none()));
    assert!(s.creatures.is_empty());
}

#[test]
fn hidden_token_events_do_not_reach_players() {
    let mut c = campaign();
    let moved = run(&mut c, "gm", mv("t-hidden", vec![p(9.0, 9.0)])).unwrap();
    let upserted = run(&mut c, "gm", set_ctl("t-hidden", &["bob"])).unwrap();
    for who in ["alice", "bob", "spec"] {
        let v = viewer(&c, who);
        for e in moved.iter().chain(&upserted) {
            assert!(c.event_for(e, &v).is_none(), "{who} got {:?}", e.kind);
        }
    }
    let gm = viewer(&c, "gm");
    assert!(moved
        .iter()
        .chain(&upserted)
        .all(|e| c.event_for(e, &gm).is_some()));
    // Control of a hidden token does not reveal it or its creature either.
    let s = c.snapshot_for(&viewer(&c, "bob"));
    assert!(s.tokens.iter().all(|t| t.token_id != "t-hidden"));
    assert!(s.creatures.iter().all(|c| c.creature_id != "c-hidden"));
}

#[test]
fn visible_token_move_reaches_everyone() {
    let mut c = campaign();
    let moved = run(&mut c, "gm", mv("t-goblin", vec![p(9.0, 9.0)])).unwrap();
    for who in ["gm", "alice", "bob", "spec"] {
        assert!(c.event_for(&moved[0], &viewer(&c, who)).is_some(), "{who}");
    }
}

#[test]
fn token_upserted_redacts_hp_for_non_controllers() {
    let mut c = campaign();
    let events = run(&mut c, "gm", set_ctl("t-goblin", &["bob"])).unwrap();
    let hp_of = |who: &str| match c.event_for(&events[0], &viewer(&c, who)).map(|e| e.kind) {
        Some(Some(event::Kind::TokenUpserted(u))) => u.token.map(|t| t.hp_current),
        other => panic!("{who}: {other:?}"),
    };
    assert_eq!(hp_of("gm"), Some(Some(10)));
    assert_eq!(hp_of("bob"), Some(Some(10)));
    assert_eq!(hp_of("alice"), Some(None));
    assert_eq!(hp_of("spec"), Some(None));
    // The source event in the log keeps the full token.
    assert!(matches!(
        &c.log[0].kind,
        Some(event::Kind::TokenUpserted(u)) if u.token.as_ref().unwrap().hp_current == Some(10)
    ));
    // A controller who is removed gets a redacted token.
    let after = run(&mut c, "gm", set_ctl("t-goblin", &[])).unwrap();
    match c
        .event_for(&after[0], &viewer(&c, "bob"))
        .and_then(|e| e.kind)
    {
        Some(event::Kind::TokenUpserted(u)) => assert_eq!(u.token.unwrap().hp_current, None),
        other => panic!("{other:?}"),
    }
}

#[test]
fn new_controller_gets_the_creature_sheet() {
    let mut c = campaign();
    let events = run(&mut c, "gm", set_ctl("t-goblin", &["bob"])).unwrap();
    let sheet = &events[1];
    assert!(c.event_for(sheet, &viewer(&c, "gm")).is_some());
    assert!(c.event_for(sheet, &viewer(&c, "bob")).is_some());
    assert!(c.event_for(sheet, &viewer(&c, "alice")).is_none());
    assert!(c.event_for(sheet, &viewer(&c, "spec")).is_none());
    let ids: Vec<String> = c
        .snapshot_for(&viewer(&c, "bob"))
        .creatures
        .into_iter()
        .map(|c| c.creature_id)
        .collect();
    assert_eq!(ids, ["c-goblin", "c-bob"]);
}

#[test]
fn encounters_with_hidden_combatants_are_gm_only() {
    let mut c = campaign();
    let enc = |token: &str| Encounter {
        encounter_id: format!("e-{token}"),
        scene_id: "s1".into(),
        combatants: vec![Combatant {
            token_id: token.into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    c.encounters = vec![enc("t-alice"), enc("t-hidden"), enc("t-missing")];
    let alice = c.snapshot_for(&viewer(&c, "alice"));
    assert_eq!(alice.encounters.len(), 1);
    assert_eq!(alice.encounters[0].encounter_id, "e-t-alice");
    assert_eq!(c.snapshot_for(&viewer(&c, "gm")).encounters.len(), 3);
}

#[test]
fn event_kinds_not_emitted_yet_default_to_gm_only() {
    let c = campaign();
    let mk = |kind| Event {
        seq: 1,
        kind: Some(kind),
        ..Default::default()
    };
    let removed = mk(event::Kind::TokenRemoved(tt_contracts::v1::TokenRemoved {
        token_id: "t-goblin".into(),
    }));
    let undone = mk(event::Kind::EventUndone(tt_contracts::v1::EventUndone {
        undone_seq: 1,
    }));
    let empty = Event::default();
    let (gm, alice) = (viewer(&c, "gm"), viewer(&c, "alice"));
    for e in [&removed, &undone, &empty] {
        assert!(c.event_for(e, &alice).is_none());
    }
    assert!(c.event_for(&removed, &gm).is_some());
    assert!(c.event_for(&undone, &gm).is_some());
}

// ----------------------------------------------------------------- identity

fn hello(token: &str, campaign_id: &str) -> Hello {
    Hello {
        campaign_id: campaign_id.into(),
        session_token: token.into(),
        last_seq: 0,
    }
}

#[test]
fn dev_token_parsing() {
    assert_eq!(parse_dev_token("dev:alice"), Some("alice"));
    assert_eq!(parse_dev_token("dev:a:b"), Some("a:b"));
    assert_eq!(parse_dev_token("dev:"), None);
    assert_eq!(parse_dev_token("alice"), None);
    assert_eq!(parse_dev_token("gm"), None);
    assert_eq!(parse_dev_token(""), None);
    assert_eq!(parse_dev_token("DEV:alice"), None);
}

#[test]
fn dev_auth_signs_in_members_with_their_role() {
    let c = campaign();
    let v = authenticate(&c, true, &hello("dev:gm", "")).unwrap();
    assert_eq!((v.user_id.as_str(), v.role), ("gm", Role::Gm));
    let v = authenticate(&c, true, &hello("dev:alice", "camp")).unwrap();
    assert_eq!(v.role, Role::Player);
    let v = authenticate(&c, true, &hello("dev:spec", "")).unwrap();
    assert_eq!(v.role, Role::Spectator);
}

#[test]
fn dev_auth_refuses_everyone_else() {
    let c = campaign();
    let err = |t: &str, id: &str| authenticate(&c, true, &hello(t, id)).unwrap_err();
    assert_eq!(err("dev:mallory", ""), AuthError::UnknownMember);
    assert_eq!(err("", ""), AuthError::BadToken);
    assert_eq!(err("dev:", ""), AuthError::BadToken);
    assert_eq!(err("alice", ""), AuthError::BadToken);
    assert_eq!(err("dev:alice", "other-campaign"), AuthError::WrongCampaign);
    // A member with an unset role is not a player by default.
    let mut odd = campaign();
    odd.members.push(Member {
        user_id: "odd".into(),
        ..Default::default()
    });
    assert_eq!(
        authenticate(&odd, true, &hello("dev:odd", "")).unwrap_err(),
        AuthError::UnknownMember
    );
}

#[test]
fn disabled_dev_auth_refuses_every_hello() {
    let c = campaign();
    for token in ["dev:gm", "dev:alice", "dev:mallory", "", "gm"] {
        assert_eq!(
            authenticate(&c, false, &hello(token, "")).unwrap_err(),
            AuthError::Disabled
        );
    }
}

// --------------------------------------------------------------------- wire

#[test]
fn server_messages_round_trip_through_prost() {
    let mut c = campaign();
    let events = run(
        &mut c,
        "gm",
        roll("2d6+1", RollMode::Normal, RollVisibility::Public, "x"),
    )
    .unwrap();
    let msgs = vec![
        server_message::Kind::Event(events[0].clone()),
        server_message::Kind::Snapshot(c.snapshot_for(&viewer(&c, "alice"))),
        rejection_message("ci-9", &Rejection::new("not_permitted", "no"))
            .kind
            .unwrap(),
    ];
    for kind in msgs {
        let bytes = encode(kind.clone());
        let decoded = ServerMessage::decode(bytes.as_ref()).unwrap();
        assert_eq!(decoded.kind, Some(kind));
    }
}
