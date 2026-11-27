//! Tests for the pure core: intents, filtering, identity and the wire.
//! The socket loop is covered by tests/socket.rs.

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
use crate::ids::{IdSource as _, SequentialIds};
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
fn an_intent_with_no_kind_is_malformed() {
    let mut c = campaign();
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

// -------------------------------------------------------- set_level_background

fn asset(id: &str) -> tt_contracts::v1::AssetRef {
    tt_contracts::v1::AssetRef {
        asset_id: id.into(),
        media_type: "image/png".into(),
        url: format!("http://x/assets/{id}.png"),
        width: 64,
        height: 32,
    }
}

fn set_bg(scene: &str, level: &str, asset_id: &str) -> Intent {
    intent_of(intent::Kind::SetLevelBackground(
        tt_contracts::v1::SetLevelBackgroundIntent {
            scene_id: scene.into(),
            level_id: level.into(),
            asset_id: asset_id.into(),
        },
    ))
}

fn background_of(c: &Campaign, scene: &str, level: &str) -> Option<tt_contracts::v1::AssetRef> {
    c.scenes
        .iter()
        .find(|s| s.scene_id == scene)?
        .levels
        .iter()
        .find(|l| l.level_id == level)?
        .background
        .clone()
}

#[test]
fn gm_sets_and_clears_a_level_background() {
    let mut c = campaign();
    c.register_asset(asset("a1"));
    let events = run(&mut c, "gm", set_bg("s1", "l1", "a1")).expect("accepted");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].scene_id, "s1");
    assert_eq!(events[0].seq, 1);
    match events[0].kind.as_ref() {
        Some(event::Kind::SceneUpserted(u)) => {
            let scene = u.scene.as_ref().expect("scene");
            assert_eq!(scene.levels[0].background, Some(asset("a1")));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(background_of(&c, "s1", "l1"), Some(asset("a1")));
    // Other scenes untouched.
    assert_eq!(background_of(&c, "s2", "l-s2"), None);

    let events = run(&mut c, "gm", set_bg("s1", "l1", "")).expect("clear");
    assert_eq!(events.len(), 1);
    assert_eq!(background_of(&c, "s1", "l1"), None);
    assert_eq!(c.seq, 2);
}

#[test]
fn set_level_background_rejections_change_nothing() {
    let mut c = campaign();
    c.register_asset(asset("a1"));
    let scenes = c.scenes.clone();
    let tokens = c.tokens.clone();
    let cases = [
        ("alice", set_bg("s1", "l1", "a1"), "not_permitted"),
        ("spec", set_bg("s1", "l1", "a1"), "not_permitted"),
        ("gm", set_bg("nope", "l1", "a1"), "unknown_scene"),
        ("gm", set_bg("s1", "l-s2", "a1"), "unknown_level"),
        ("gm", set_bg("s1", "l1", "missing"), "unknown_asset"),
    ];
    for (actor, i, want) in cases {
        assert_eq!(code(run(&mut c, actor, i)), want);
        assert_untouched(&c, 0, 0, &tokens);
        assert_eq!(c.scenes, scenes);
    }
}

#[test]
fn snapshot_for_shows_the_new_background_to_everyone() {
    let mut c = campaign();
    c.register_asset(asset("a1"));
    run(&mut c, "gm", set_bg("s1", "l1", "a1")).expect("accepted");
    for who in ["gm", "alice", "spec"] {
        let snap = c.snapshot_for(&viewer(&c, who));
        let level = &snap.scenes[0].levels[0];
        assert_eq!(level.background, Some(asset("a1")), "{who}");
    }
}

#[test]
fn registering_an_asset_twice_keeps_the_first_record() {
    let mut c = campaign();
    c.register_asset(asset("a1"));
    let mut other = asset("a1");
    other.url = "http://elsewhere/a1.png".into();
    c.register_asset(other);
    assert_eq!(c.assets.get("a1"), Some(&asset("a1")));
    assert_eq!(c.assets.len(), 1);
}

// -------------------------------------------------------------- use_ability

mod use_ability {
    use super::*;
    use tt_contracts::v1 as pb;

    fn attack_step(bonus: i32, dice: &str) -> pb::EffectStep {
        pb::EffectStep {
            step: Some(pb::effect_step::Step::Attack(pb::AttackStep {
                to_hit_bonus: bonus,
                damage: vec![pb::DamagePart {
                    dice: dice.into(),
                    r#type: pb::DamageType::Slashing as i32,
                }],
            })),
        }
    }

    fn ability(id: &str, shape: pb::TargetShape, steps: Vec<pb::EffectStep>) -> pb::Ability {
        pb::Ability {
            ability_id: id.into(),
            name: id.into(),
            targeting: Some(pb::Targeting {
                shape: shape as i32,
                range_feet: 30.0,
                max_targets: 1,
                ..Default::default()
            }),
            steps,
            ..Default::default()
        }
    }

    /// The test campaign plus abilities. c-alice: `sword` (free), `smite`
    /// (spends 1 of 2 `slots`), `empty` (needs 1 of 0 `none`), `fireball`
    /// (cone, not implemented), `save` (a save step), `bad-dice` (unparsable
    /// damage dice). c-goblin: `claw`. Plus a token with no creature.
    fn armed() -> Campaign {
        let mut c = campaign();
        let attack = |id| {
            ability(
                id,
                pb::TargetShape::Creatures,
                vec![attack_step(5, "1d8+3")],
            )
        };
        let alice = c.creatures.iter_mut().find(|x| x.creature_id == "c-alice");
        let alice = alice.expect("c-alice");
        let mut smite = attack("smite");
        smite.resource_key = "slots".into();
        smite.resource_amount = 1;
        let mut empty = attack("empty");
        empty.resource_key = "none".into();
        empty.resource_amount = 1;
        alice.abilities = vec![
            attack("sword"),
            smite,
            empty,
            ability(
                "fireball",
                pb::TargetShape::Cone,
                vec![attack_step(5, "1d8")],
            ),
            ability(
                "save",
                pb::TargetShape::Creatures,
                vec![pb::EffectStep {
                    step: Some(pb::effect_step::Step::Save(pb::SaveStep::default())),
                }],
            ),
            ability(
                "bad-dice",
                pb::TargetShape::Creatures,
                vec![attack_step(5, "banana")],
            ),
        ];
        alice.resources = vec![pb::Resource {
            key: "slots".into(),
            label: "slots".into(),
            current: 2,
            max: 2,
        }];
        let goblin = c.creatures.iter_mut().find(|x| x.creature_id == "c-goblin");
        goblin.expect("c-goblin").abilities = vec![ability(
            "claw",
            pb::TargetShape::Creatures,
            vec![attack_step(4, "1d6")],
        )];
        c.tokens.push(token("t-bare", "", &["alice"], false, 175.0));
        c
    }

    fn use_ab(source: &str, ability: &str, targets: &[&str]) -> Intent {
        intent_of(intent::Kind::UseAbility(pb::UseAbilityIntent {
            source_token_id: source.into(),
            ability_id: ability.into(),
            target_token_ids: targets.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }))
    }

    fn resolution_of(e: &Event) -> &pb::Resolution {
        match e.kind.as_ref() {
            Some(event::Kind::ResolutionUpdated(u)) => u.resolution.as_ref().expect("resolution"),
            other => panic!("not a resolution event: {other:?}"),
        }
    }

    /// Attack roll 15 (+5 = 20, a hit on AC 15), damage 4 (+3).
    fn hit_script() -> Scripted {
        Scripted::new(&[14, 3])
    }

    fn slots(c: &Campaign) -> i32 {
        c.creatures
            .iter()
            .find(|x| x.creature_id == "c-alice")
            .and_then(|x| x.resources.first())
            .map(|r| r.current)
            .expect("slots")
    }

    #[test]
    fn controller_attack_appends_one_resolution_event() {
        let mut c = armed();
        let hp_before: Vec<(i32, i32)> = c
            .creatures
            .iter()
            .map(|x| (x.hp_current, x.hp_max))
            .collect();
        let tokens_before = c.tokens.clone();
        let events = run_with(
            &mut c,
            "alice",
            use_ab("t-alice", "sword", &["t-goblin"]),
            &mut hit_script(),
        )
        .unwrap();
        // No resource spent: the resolution, then the goblin's token.
        assert_eq!(events.len(), 2);
        let e = &events[0];
        assert_eq!((e.seq, e.scene_id.as_str()), (1, "s1"));
        assert_eq!(e.actor_user_id, "alice");
        assert_eq!(e.client_intent_id, "ci-1");
        assert!(e.warnings.is_empty());
        let r = resolution_of(e);
        assert_eq!(r.state, pb::ResolutionState::Done as i32);
        assert_eq!(r.source_token_id, "t-alice");
        assert_eq!(r.ability_id, "sword");
        assert_eq!(r.outcomes.len(), 1);
        let o = &r.outcomes[0];
        assert_eq!(o.token_id, "t-goblin");
        assert_eq!(o.outcome, pb::HitOutcome::Hit as i32);
        assert_eq!(o.roll.as_ref().expect("attack").total, 20);
        assert_eq!(o.damage_rolls.len(), 1);
        assert_eq!(o.damage_rolls[0].total, 7);
        // Damage lands on the token only: 7 hits 1 temporary then 6 hit
        // points (10 to 4), so Bloodied. Creature sheets never change.
        let hp_after: Vec<(i32, i32)> = c
            .creatures
            .iter()
            .map(|x| (x.hp_current, x.hp_max))
            .collect();
        assert_eq!(hp_before, hp_after);
        assert!(!o.dropped_to_zero);
        assert_eq!(o.damage.len(), 1);
        assert_eq!(o.damage[0].applied, 7);
        let goblin = c.tokens.iter().find(|t| t.token_id == "t-goblin").unwrap();
        assert_eq!(
            (goblin.hp_current, goblin.hp_temp, goblin.health_state),
            (Some(4), Some(0), pb::HealthState::Bloodied as i32)
        );
        match events[1].kind.as_ref() {
            Some(event::Kind::TokenUpserted(u)) => {
                assert_eq!(u.token.as_ref(), Some(goblin));
            }
            other => panic!("expected TokenUpserted, got {other:?}"),
        }
        assert_eq!(events[1].seq, 2);
        assert_eq!(events[1].client_intent_id, "ci-1");
        let others: Vec<&Token> = tokens_before
            .iter()
            .filter(|t| t.token_id != "t-goblin")
            .collect();
        for t in others {
            assert!(c.tokens.contains(t), "{} is unchanged", t.token_id);
        }
    }

    #[test]
    fn gm_can_use_any_creatures_ability() {
        let mut c = armed();
        let events = run_with(
            &mut c,
            "gm",
            use_ab("t-goblin", "claw", &["t-alice"]),
            &mut hit_script(),
        )
        .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(resolution_of(&events[0]).ability_id, "claw");
        assert_eq!(events[0].actor_user_id, "gm");
    }

    #[test]
    fn spent_resource_adds_a_creature_upserted_after_the_resolution() {
        let mut c = armed();
        let events = run_with(
            &mut c,
            "alice",
            use_ab("t-alice", "smite", &["t-goblin"]),
            &mut hit_script(),
        )
        .unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(
            events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [1, 2, 3],
            "consecutive seq"
        );
        assert!(events.iter().all(|e| e.client_intent_id == "ci-1"));
        assert!(matches!(
            events[0].kind,
            Some(event::Kind::ResolutionUpdated(_))
        ));
        assert!(matches!(
            events[1].kind,
            Some(event::Kind::TokenUpserted(_))
        ));
        match events[2].kind.as_ref() {
            Some(event::Kind::CreatureUpserted(u)) => {
                let cr = u.creature.as_ref().expect("creature");
                assert_eq!(cr.creature_id, "c-alice");
                assert_eq!(cr.resources[0].current, 1);
            }
            other => panic!("expected CreatureUpserted, got {other:?}"),
        }
        assert_eq!(slots(&c), 1, "campaign state updated");
    }

    #[test]
    fn warnings_ride_on_the_resolution_event_and_never_stop_the_action() {
        let mut c = armed();
        // `smite` now costs 3 of the 2 slots: it warns, still spends (the
        // slots floor at 0) and so emits a CreatureUpserted too. The goblin
        // is also moved beyond the 30 ft range (t-alice at x=105, 8 cells =
        // 40 ft).
        c.creatures
            .iter_mut()
            .find(|x| x.creature_id == "c-alice")
            .expect("c-alice")
            .abilities
            .iter_mut()
            .find(|a| a.ability_id == "smite")
            .expect("smite")
            .resource_amount = 3;
        c.scenes[0].grid.as_mut().expect("grid").kind = pb::GridKind::Square as i32;
        c.tokens
            .iter_mut()
            .find(|t| t.token_id == "t-goblin")
            .expect("goblin")
            .position = Some(p(385.0 + 280.0, 105.0));
        let events = run_with(
            &mut c,
            "alice",
            use_ab("t-alice", "smite", &["t-goblin"]),
            &mut hit_script(),
        )
        .unwrap();
        assert_eq!(events.len(), 3, "resolution, goblin token, creature");
        assert!(matches!(
            events[2].kind,
            Some(event::Kind::CreatureUpserted(_))
        ));
        assert_eq!(slots(&c), 0, "slots floor at zero");
        let r = resolution_of(&events[0]);
        let codes: Vec<&str> = r.warnings.iter().map(|w| w.code.as_str()).collect();
        assert_eq!(codes, ["not_enough_resource", "out_of_range"]);
        assert_eq!(events[0].warnings, r.warnings);
        // Only the first event carries them.
        assert!(events[1].warnings.is_empty() && events[2].warnings.is_empty());
        assert_eq!(r.outcomes.len(), 1, "still rolled");
    }

    #[test]
    fn plain_intents_carry_no_warnings() {
        let mut c = armed();
        let e = run(&mut c, "alice", chat("hi")).unwrap();
        assert!(e[0].warnings.is_empty());
    }

    #[test]
    fn ids_are_filled_non_empty_and_unique_and_rolls_are_public() {
        let mut c = armed();
        let events = run_with(
            &mut c,
            "alice",
            use_ab("t-alice", "sword", &["t-goblin"]),
            &mut hit_script(),
        )
        .unwrap();
        let r = resolution_of(&events[0]);
        let o = &r.outcomes[0];
        let rolls: Vec<&pb::RollResult> = o.roll.iter().chain(o.damage_rolls.iter()).collect();
        assert_eq!(rolls.len(), 2);
        let mut ids: Vec<&str> = rolls.iter().map(|x| x.roll_id.as_str()).collect();
        ids.push(&r.resolution_id);
        assert!(ids.iter().all(|i| !i.is_empty()));
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "ids unique: {ids:?}");
        for roll in rolls {
            assert_eq!(roll.roller_user_id, "alice");
            assert_eq!(roll.visibility, RollVisibility::Public as i32);
        }
    }

    #[test]
    fn gm_rolls_are_attributed_to_the_gm_and_public() {
        let mut c = armed();
        let events = run_with(
            &mut c,
            "gm",
            use_ab("t-goblin", "claw", &["t-alice"]),
            &mut hit_script(),
        )
        .unwrap();
        let o = &resolution_of(&events[0]).outcomes[0];
        let roll = o.roll.as_ref().expect("attack");
        assert_eq!(roll.roller_user_id, "gm");
        assert_eq!(roll.visibility, RollVisibility::Public as i32);
    }

    #[test]
    fn log_keeps_every_event_unredacted_in_order() {
        let mut c = armed();
        let mut all = Vec::new();
        all.extend(run(&mut c, "alice", chat("a")).unwrap());
        all.extend(
            run_with(
                &mut c,
                "alice",
                use_ab("t-alice", "smite", &["t-goblin"]),
                &mut hit_script(),
            )
            .unwrap(),
        );
        // A hidden target: only the GM may receive it, but the log keeps it.
        all.extend(
            run_with(
                &mut c,
                "gm",
                use_ab("t-alice", "sword", &["t-hidden"]),
                &mut hit_script(),
            )
            .unwrap(),
        );
        assert_eq!(
            all.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6]
        );
        assert_eq!(c.log, all);
        assert_eq!(c.seq, 6);
        let kept = resolution_of(&c.log[1]);
        assert_eq!(kept.outcomes[0].damage_rolls.len(), 1);
        assert!(matches!(c.log[2].kind, Some(event::Kind::TokenUpserted(_))));
        assert!(matches!(
            c.log[3].kind,
            Some(event::Kind::CreatureUpserted(_))
        ));
        assert_eq!(resolution_of(&c.log[4]).outcomes[0].token_id, "t-hidden");
        // The hidden token's numbers are in the log, unredacted.
        match c.log[5].kind.as_ref() {
            Some(event::Kind::TokenUpserted(u)) => {
                let t = u.token.as_ref().expect("token");
                assert_eq!((t.token_id.as_str(), t.hp_max), ("t-hidden", Some(12)));
                assert!(t.hp_current.is_some());
            }
            other => panic!("expected TokenUpserted, got {other:?}"),
        }
    }

    /// A rejected use leaves seq, log, tokens and creatures untouched.
    fn assert_rejected(c: &mut Campaign, actor: &str, i: Intent, expect: &str) {
        let (seq, log, tokens, creatures) =
            (c.seq, c.log.len(), c.tokens.clone(), c.creatures.clone());
        let r = run_with(c, actor, i, &mut hit_script());
        assert_eq!(code(r), expect);
        assert_untouched(c, seq, log, &tokens);
        assert_eq!(c.creatures, creatures);
    }

    #[test]
    fn permission_failures_are_not_permitted_and_leak_nothing() {
        let mut c = armed();
        // Not a controller; hidden; missing.
        for source in ["t-goblin", "t-hidden", "t-nobody"] {
            assert_rejected(
                &mut c,
                "alice",
                use_ab(source, "claw", &["t-alice"]),
                "not_permitted",
            );
        }
        // Another player's token; a spectator.
        assert_rejected(
            &mut c,
            "bob",
            use_ab("t-alice", "sword", &["t-goblin"]),
            "not_permitted",
        );
        assert_rejected(
            &mut c,
            "spec",
            use_ab("t-alice", "sword", &["t-goblin"]),
            "not_permitted",
        );
        // A hidden source token the player controls is still invisible.
        c.tokens
            .iter_mut()
            .find(|t| t.token_id == "t-alice")
            .expect("alice token")
            .hidden = true;
        assert_rejected(
            &mut c,
            "alice",
            use_ab("t-alice", "sword", &["t-goblin"]),
            "not_permitted",
        );
    }

    #[test]
    fn gm_naming_a_missing_source_gets_unknown_token() {
        let mut c = armed();
        assert_rejected(
            &mut c,
            "gm",
            use_ab("t-nobody", "sword", &["t-goblin"]),
            "unknown_token",
        );
    }

    #[test]
    fn source_without_a_creature_is_no_creature() {
        let mut c = armed();
        assert_rejected(
            &mut c,
            "alice",
            use_ab("t-bare", "sword", &["t-goblin"]),
            "no_creature",
        );
        // A token whose creature id points nowhere is the same.
        c.tokens
            .push(token("t-ghost", "c-gone", &["alice"], false, 175.0));
        assert_rejected(
            &mut c,
            "alice",
            use_ab("t-ghost", "sword", &["t-goblin"]),
            "no_creature",
        );
    }

    #[test]
    fn unknown_ability_is_rejected() {
        let mut c = armed();
        assert_rejected(
            &mut c,
            "alice",
            use_ab("t-alice", "no-such", &["t-goblin"]),
            "unknown_ability",
        );
        assert_rejected(&mut c, "gm", use_ab("t-alice", "", &[]), "unknown_ability");
    }

    #[test]
    fn creature_ability_without_targets_is_no_targets() {
        let mut c = armed();
        assert_rejected(
            &mut c,
            "alice",
            use_ab("t-alice", "smite", &[]),
            "no_targets",
        );
        assert_eq!(slots(&c), 2, "nothing spent");
    }

    #[test]
    fn bad_targets_are_unknown_target() {
        let mut c = armed();
        // Missing; hidden from a player; no creature.
        for target in ["t-nobody", "t-hidden", "t-bare"] {
            assert_rejected(
                &mut c,
                "alice",
                use_ab("t-alice", "sword", &["t-goblin", target]),
                "unknown_target",
            );
        }
        // The GM may target a hidden token, but not a missing one.
        assert_rejected(
            &mut c,
            "gm",
            use_ab("t-alice", "sword", &["t-nobody"]),
            "unknown_target",
        );
        assert_rejected(
            &mut c,
            "gm",
            use_ab("t-alice", "sword", &["t-bare"]),
            "unknown_target",
        );
        run(&mut c, "gm", use_ab("t-alice", "sword", &["t-hidden"])).unwrap();
    }

    #[test]
    fn unsupported_abilities_are_not_implemented() {
        let mut c = armed();
        for id in ["save", "bad-dice"] {
            assert_rejected(
                &mut c,
                "alice",
                use_ab("t-alice", id, &["t-goblin"]),
                "not_implemented",
            );
        }
        assert_eq!(slots(&c), 2);
    }

    #[test]
    fn entropy_failure_rejects_and_spends_nothing() {
        let mut c = armed();
        let mut rng = hit_script();
        rng.fail = true;
        let r = run_with(
            &mut c,
            "alice",
            use_ab("t-alice", "smite", &["t-goblin"]),
            &mut rng,
        );
        assert_eq!(code(r), "rng_unavailable");
        assert_eq!((c.seq, c.log.len(), slots(&c)), (0, 0, 2));
    }

    // ------------------------------------------------------------ visibility

    #[test]
    fn public_resolution_reaches_every_viewer() {
        let mut c = armed();
        let events = run_with(
            &mut c,
            "alice",
            use_ab("t-alice", "sword", &["t-goblin"]),
            &mut hit_script(),
        )
        .unwrap();
        for who in ["gm", "alice", "bob", "spec"] {
            let got = c.event_for(&events[0], &viewer(&c, who));
            assert_eq!(got.as_ref(), Some(&events[0]), "{who} sees it");
        }
    }

    #[test]
    fn resolution_with_a_hidden_token_is_gm_only() {
        let mut c = armed();
        // Hidden target, GM attacker.
        let events = run_with(
            &mut c,
            "gm",
            use_ab("t-alice", "sword", &["t-hidden"]),
            &mut hit_script(),
        )
        .unwrap();
        assert!(c.event_for(&events[0], &viewer(&c, "gm")).is_some());
        for who in ["alice", "bob", "spec"] {
            assert!(c.event_for(&events[0], &viewer(&c, who)).is_none());
        }
        // Hidden source (a monster in hiding attacks a player).
        c.creatures
            .iter_mut()
            .find(|x| x.creature_id == "c-hidden")
            .expect("hidden creature")
            .abilities = vec![ability(
            "stab",
            pb::TargetShape::Creatures,
            vec![attack_step(4, "1d6")],
        )];
        let events = run_with(
            &mut c,
            "gm",
            use_ab("t-hidden", "stab", &["t-alice"]),
            &mut hit_script(),
        )
        .unwrap();
        assert!(c.event_for(&events[0], &viewer(&c, "alice")).is_none());
        assert!(c.event_for(&events[0], &viewer(&c, "gm")).is_some());
    }

    #[test]
    fn a_monsters_resource_warning_reaches_only_those_who_see_its_sheet() {
        let mut c = armed();
        // The goblin's claw needs a use it does not have: the engine warns
        // not_enough_resource, naming the goblin's resource and uses left.
        let goblin = c
            .creatures
            .iter_mut()
            .find(|x| x.creature_id == "c-goblin")
            .expect("c-goblin");
        goblin.resources = vec![pb::Resource {
            key: "uses".into(),
            label: "uses".into(),
            current: 0,
            max: 1,
        }];
        goblin.abilities[0].resource_key = "uses".into();
        goblin.abilities[0].resource_amount = 1;
        let events = run_with(
            &mut c,
            "gm",
            use_ab("t-goblin", "claw", &["t-alice"]),
            &mut hit_script(),
        )
        .unwrap();
        let has_warning = |e: &Event| {
            e.warnings.iter().any(|w| w.code == "not_enough_resource")
                || resolution_of(e)
                    .warnings
                    .iter()
                    .any(|w| w.code == "not_enough_resource")
        };
        assert!(has_warning(&events[0]), "the log keeps the warning");
        let gm_copy = c.event_for(&events[0], &viewer(&c, "gm")).expect("gm");
        assert!(has_warning(&gm_copy), "the GM sees it");
        for who in ["alice", "bob", "spec"] {
            let copy = c
                .event_for(&events[0], &viewer(&c, who))
                .unwrap_or_else(|| panic!("{who} still gets the public resolution"));
            assert!(
                !has_warning(&copy),
                "{who} must not learn the goblin's uses"
            );
            assert_eq!(
                resolution_of(&copy).outcomes,
                resolution_of(&events[0]).outcomes,
                "{who} still sees the rolls"
            );
        }
    }

    #[test]
    fn a_player_never_receives_a_monsters_creature() {
        let mut c = armed();
        // Give the goblin a spendable resource so its claw emits a monster
        // CreatureUpserted for the event filter to hold back.
        let goblin = c
            .creatures
            .iter_mut()
            .find(|x| x.creature_id == "c-goblin")
            .expect("c-goblin");
        goblin.resources = vec![pb::Resource {
            key: "uses".into(),
            label: "uses".into(),
            current: 1,
            max: 1,
        }];
        goblin.abilities[0].resource_key = "uses".into();
        goblin.abilities[0].resource_amount = 1;
        let mut all = Vec::new();
        all.extend(
            run_with(
                &mut c,
                "alice",
                use_ab("t-alice", "smite", &["t-goblin"]),
                &mut hit_script(),
            )
            .unwrap(),
        );
        all.extend(
            run_with(
                &mut c,
                "gm",
                use_ab("t-goblin", "claw", &["t-alice"]),
                &mut hit_script(),
            )
            .unwrap(),
        );
        let monsters = ["c-goblin", "c-hidden"];
        let monster_upsert = all
            .iter()
            .find(|e| {
                matches!(&e.kind, Some(event::Kind::CreatureUpserted(u))
                    if u.creature.as_ref().is_some_and(|x| x.creature_id == "c-goblin"))
            })
            .expect("the claw emits a goblin CreatureUpserted");
        assert!(c.event_for(monster_upsert, &viewer(&c, "gm")).is_some());
        for who in ["alice", "bob", "spec"] {
            assert!(
                c.event_for(monster_upsert, &viewer(&c, who)).is_none(),
                "{who} must not get the goblin upsert"
            );
            let v = viewer(&c, who);
            let snap = c.snapshot_for(&v);
            assert!(
                snap.creatures
                    .iter()
                    .all(|x| !monsters.contains(&x.creature_id.as_str())),
                "{who} snapshot"
            );
            for e in &all {
                if let Some(out) = c.event_for(e, &v) {
                    if let Some(event::Kind::CreatureUpserted(u)) = out.kind {
                        let id = u.creature.expect("creature").creature_id;
                        assert!(!monsters.contains(&id.as_str()), "{who} got {id}");
                    }
                }
            }
        }
        // The goblin's own sheet is still the GM's.
        assert!(c
            .snapshot_for(&viewer(&c, "gm"))
            .creatures
            .iter()
            .any(|x| x.creature_id == "c-goblin"));
    }

    #[test]
    fn creature_upserted_keeps_its_own_visibility() {
        let mut c = armed();
        let events = run_with(
            &mut c,
            "alice",
            use_ab("t-alice", "smite", &["t-goblin"]),
            &mut hit_script(),
        )
        .unwrap();
        // alice's sheet: GM and alice (owner and controller) only.
        assert!(c.event_for(&events[2], &viewer(&c, "gm")).is_some());
        assert!(c.event_for(&events[2], &viewer(&c, "alice")).is_some());
        assert!(c.event_for(&events[2], &viewer(&c, "bob")).is_none());
        assert!(c.event_for(&events[2], &viewer(&c, "spec")).is_none());
    }
}

// ------------------------------------------------- applied results (#100)

/// Everything a rejected intent must leave alone.
type State = (
    u64,
    usize,
    Vec<Token>,
    Vec<Creature>,
    usize,
    std::collections::HashMap<String, bool>,
);

fn state_of(c: &Campaign) -> State {
    (
        c.seq,
        c.log.len(),
        c.tokens.clone(),
        c.creatures.clone(),
        c.groups.len(),
        c.removed_token_hidden.clone(),
    )
}

/// Abilities that change state, run against the plain test campaign.
mod applying {
    use super::*;
    use tt_contracts::v1 as pb;

    fn part(dice: &str, ty: pb::DamageType) -> pb::DamagePart {
        pb::DamagePart {
            dice: dice.into(),
            r#type: ty as i32,
        }
    }

    fn ability(id: &str, shape: pb::TargetShape, steps: Vec<pb::effect_step::Step>) -> pb::Ability {
        pb::Ability {
            ability_id: id.into(),
            name: id.into(),
            targeting: Some(pb::Targeting {
                shape: shape as i32,
                range_feet: 0.0,
                max_targets: 0,
                ..Default::default()
            }),
            steps: steps
                .into_iter()
                .map(|s| pb::EffectStep { step: Some(s) })
                .collect(),
            ..Default::default()
        }
    }

    fn attack() -> pb::effect_step::Step {
        pb::effect_step::Step::Attack(pb::AttackStep {
            to_hit_bonus: 5,
            damage: vec![part("1d8+3", pb::DamageType::Slashing)],
        })
    }

    fn save(half: bool) -> pb::effect_step::Step {
        pb::effect_step::Step::Save(pb::SaveStep {
            ability: pb::AbilityScore::Dexterity as i32,
            dc: 100,
            damage: vec![part("2d6", pb::DamageType::Fire)],
            half_on_success: half,
        })
    }

    fn effect(id: &str, unit: pb::DurationUnit, amount: u32) -> pb::effect_step::Step {
        pb::effect_step::Step::ApplyEffect(pb::ApplyEffectStep {
            effect_id: id.into(),
            duration: Some(pb::Duration {
                unit: unit as i32,
                amount,
                concentration: false,
            }),
            only_on_failure: false,
        })
    }

    /// Alice's creature gets: `sword` (attack), `burn` (a Dex save DC 100 for
    /// 2d6 fire, then `burning` for a minute), `blast` (the same save, cone),
    /// `ward` (a two round `warded` effect and nothing else).
    fn world() -> Campaign {
        let mut c = campaign();
        let alice = c.creatures.iter_mut().find(|x| x.creature_id == "c-alice");
        alice.expect("c-alice").abilities = vec![
            ability("sword", pb::TargetShape::Creatures, vec![attack()]),
            ability(
                "burn",
                pb::TargetShape::Creatures,
                vec![save(false), effect("burning", pb::DurationUnit::Minutes, 1)],
            ),
            ability("blast", pb::TargetShape::Cone, vec![save(true)]),
            ability(
                "ward",
                pb::TargetShape::Creatures,
                vec![effect("warded", pb::DurationUnit::Rounds, 2)],
            ),
        ];
        c
    }

    fn use_ab(source: &str, ability: &str, targets: &[&str]) -> Intent {
        intent_of(intent::Kind::UseAbility(pb::UseAbilityIntent {
            source_token_id: source.into(),
            ability_id: ability.into(),
            target_token_ids: targets.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }))
    }

    fn resolution_of(e: &Event) -> &pb::Resolution {
        match e.kind.as_ref() {
            Some(event::Kind::ResolutionUpdated(u)) => u.resolution.as_ref().expect("resolution"),
            other => panic!("not a resolution event: {other:?}"),
        }
    }

    fn token_of(e: &Event) -> &Token {
        match e.kind.as_ref() {
            Some(event::Kind::TokenUpserted(u)) => u.token.as_ref().expect("token"),
            other => panic!("not a token event: {other:?}"),
        }
    }

    fn stored<'a>(c: &'a Campaign, id: &str) -> &'a Token {
        c.tokens.iter().find(|t| t.token_id == id).expect("token")
    }

    #[test]
    fn a_shared_damage_roll_has_one_id_and_every_other_roll_its_own() {
        let mut c = world();
        // Every die shows 6: the save fails (6 + 0 < 100), 2d6 = 12.
        let events = run_with(
            &mut c,
            "gm",
            use_ab("t-alice", "burn", &["t-goblin", "t-hidden"]),
            &mut Scripted::new(&[5]),
        )
        .unwrap();
        let r = resolution_of(&events[0]);
        let burn: Vec<_> = r.outcomes.iter().filter(|o| o.step_index == 0).collect();
        assert_eq!(burn.len(), 2);
        let (a, b) = (&burn[0], &burn[1]);
        assert_eq!(a.damage_rolls.len(), 1);
        assert_eq!(a.damage_rolls[0], b.damage_rolls[0], "a copy, id included");
        assert!(!a.damage_rolls[0].roll_id.is_empty());
        let saves = [&a.roll, &b.roll].map(|r| r.as_ref().expect("save").roll_id.clone());
        assert_ne!(saves[0], saves[1], "each save roll has its own id");
        assert!(!saves.contains(&a.damage_rolls[0].roll_id));

        // Attack damage is rolled per target: equal-looking rolls stay apart.
        let mut c = world();
        c.creatures[0].abilities[0] = ability(
            "sword",
            pb::TargetShape::Creatures,
            vec![attack(), attack()],
        );
        let events = run_with(
            &mut c,
            "gm",
            use_ab("t-alice", "sword", &["t-goblin", "t-hidden"]),
            // Each hit: d20 15 (+5 beats AC 15), then 1d8 = 6.
            &mut Scripted::new(&[14, 5, 14, 5]),
        )
        .unwrap();
        let r = resolution_of(&events[0]);
        assert_eq!(r.outcomes.len(), 2);
        let (x, y) = (&r.outcomes[0], &r.outcomes[1]);
        assert_eq!(x.damage_rolls[0].total, y.damage_rolls[0].total);
        assert_ne!(x.damage_rolls[0].roll_id, y.damage_rolls[0].roll_id);
        let mut ids: Vec<&str> = r
            .outcomes
            .iter()
            .flat_map(|o| o.roll.iter().chain(o.damage_rolls.iter()))
            .map(|x| x.roll_id.as_str())
            .collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "four rolls, four ids");
    }

    #[test]
    fn damage_and_conditions_land_on_each_target_token_in_engine_order() {
        let mut c = world();
        let creatures_before = c.creatures.clone();
        let events = run_with(
            &mut c,
            "alice",
            use_ab("t-alice", "burn", &["t-goblin", "t-alice"]),
            &mut Scripted::new(&[5]),
        )
        .unwrap();
        // alice may target a visible goblin and herself.
        assert_eq!(events.len(), 3);
        assert_eq!(events.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3]);
        assert!(events.iter().all(|e| e.client_intent_id == "ci-1"));
        let r = resolution_of(&events[0]);
        assert_eq!(r.state, pb::ResolutionState::Done as i32);
        let (goblin, alice) = (token_of(&events[1]), token_of(&events[2]));
        assert_eq!(goblin.token_id, "t-goblin");
        assert_eq!(alice.token_id, "t-alice");
        // 12 fire: 1 temporary, 11 of 10 hit points, floor at 0.
        for t in [goblin, alice] {
            assert_eq!(
                (t.hp_current, t.hp_temp, t.health_state),
                (Some(0), Some(0), pb::HealthState::Down as i32)
            );
            assert_eq!(t.conditions, ["burning"]);
            assert_eq!(stored(&c, &t.token_id), t);
        }
        assert_eq!(c.creatures, creatures_before, "sheets are not changed");
        assert!(r.outcomes.iter().all(|o| o.dropped_to_zero));
        // The one group names exactly the two tokens.
        let g = c.groups.last().expect("group");
        assert_eq!((g.first_seq, g.last_seq), (1, 3));
        assert_eq!(g.changed_token_ids, ["t-alice", "t-goblin"]);
        assert!(g.changed_creature_ids.is_empty());
    }

    #[test]
    fn a_target_whose_state_did_not_change_gets_no_token_event() {
        let mut c = world();
        // A miss: d20 shows 1, no damage, nothing applied.
        let events = run_with(
            &mut c,
            "alice",
            use_ab("t-alice", "sword", &["t-goblin"]),
            &mut Scripted::new(&[0]),
        )
        .unwrap();
        assert_eq!(events.len(), 1);
        let g = c.groups.last().expect("group");
        assert!(g.changed_token_ids.is_empty() && g.changed_creature_ids.is_empty());
    }

    #[test]
    fn an_area_ability_resolves_with_area_not_checked() {
        let mut c = world();
        let events = run_with(
            &mut c,
            "gm",
            use_ab("t-alice", "blast", &["t-goblin"]),
            &mut Scripted::new(&[5]),
        )
        .unwrap();
        let r = resolution_of(&events[0]);
        let codes: Vec<&str> = r.warnings.iter().map(|w| w.code.as_str()).collect();
        assert_eq!(codes, ["area_not_checked"]);
        assert_eq!(events[0].warnings, r.warnings);
        assert_eq!(token_of(&events[1]).token_id, "t-goblin");
    }

    #[test]
    fn a_token_with_no_hit_points_warns_and_gets_no_token_event() {
        let mut c = world();
        let mut ghost = token("t-ghost", "c-goblin", &[], false, 455.0);
        (ghost.hp_current, ghost.hp_max, ghost.hp_temp) = (None, None, None);
        c.tokens.push(ghost);
        let events = run_with(
            &mut c,
            "gm",
            use_ab("t-alice", "burn", &["t-ghost"]),
            &mut Scripted::new(&[5]),
        )
        .unwrap();
        let r = resolution_of(&events[0]);
        assert!(r.warnings.iter().any(|w| w.code == "no_hit_points"));
        assert!(events
            .iter()
            .all(|e| !matches!(&e.kind, Some(event::Kind::TokenUpserted(u))
                if u.token.as_ref().is_some_and(|t| t.hp_current.is_some()))));
    }

    #[test]
    fn the_scenes_running_encounter_decides_the_timers() {
        let mut c = world();
        c.encounters.push(Encounter {
            encounter_id: "enc-1".into(),
            scene_id: "s1".into(),
            round: 3,
            combatants: vec![
                Combatant {
                    token_id: "t-alice".into(),
                    ..Default::default()
                },
                Combatant {
                    token_id: "t-goblin".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        let events = run(&mut c, "alice", use_ab("t-alice", "ward", &["t-goblin"])).unwrap();
        let g = token_of(&events[1]);
        assert_eq!(g.conditions, ["warded"]);
        assert_eq!(g.condition_timers.len(), 1);
        let t = &g.condition_timers[0];
        assert_eq!(
            (t.effect_id.as_str(), t.encounter_id.as_str(), t.ends_round),
            ("warded", "enc-1", 5)
        );
        assert_eq!(t.source_token_id, "t-alice");
        // An encounter on another scene does not count.
        let mut c = world();
        c.encounters.push(Encounter {
            encounter_id: "enc-2".into(),
            scene_id: "s2".into(),
            round: 1,
            ..Default::default()
        });
        let events = run(&mut c, "alice", use_ab("t-alice", "ward", &["t-goblin"])).unwrap();
        assert!(token_of(&events[1]).condition_timers.is_empty());
        assert!(resolution_of(&events[0])
            .warnings
            .iter()
            .any(|w| w.code == "duration_not_tracked"));
    }

    #[test]
    fn rejected_abilities_leave_everything_unchanged() {
        let mut c = world();
        let before = state_of(&c);
        let mut rng = Scripted::new(&[5]);
        rng.fail = true;
        let r = run_with(
            &mut c,
            "gm",
            use_ab("t-alice", "burn", &["t-goblin"]),
            &mut rng,
        );
        assert_eq!(code(r), "rng_unavailable");
        assert_eq!(state_of(&c), before);
    }
}

// ------------------------------------------- set hp, conditions, tokens

mod intents {
    use super::*;
    use tt_contracts::v1 as pb;

    fn refuse(c: &mut Campaign, actor: &str, i: Intent, expect: &str) {
        let before = state_of(c);
        assert_eq!(code(run(c, actor, i)), expect);
        assert_eq!(state_of(c), before, "{expect}: state changed");
    }

    fn set_hp(token_id: &str, cur: i32, temp: i32) -> Intent {
        intent_of(intent::Kind::SetHitPoints(pb::SetHitPointsIntent {
            token_id: token_id.into(),
            hp_current: cur,
            hp_temp: temp,
        }))
    }

    fn set_cond(token_id: &str, add: &[&str], remove: &[&str]) -> Intent {
        let own = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
        intent_of(intent::Kind::SetConditions(pb::SetConditionsIntent {
            token_id: token_id.into(),
            add: own(add),
            remove: own(remove),
        }))
    }

    fn create(f: impl FnOnce(&mut pb::CreateTokenIntent)) -> Intent {
        let mut t = pb::CreateTokenIntent {
            scene_id: "s1".into(),
            position: Some(p(35.0, 35.0)),
            ..Default::default()
        };
        f(&mut t);
        intent_of(intent::Kind::CreateToken(t))
    }

    fn remove(token_id: &str) -> Intent {
        intent_of(intent::Kind::RemoveToken(pb::RemoveTokenIntent {
            token_id: token_id.into(),
        }))
    }

    fn stored<'a>(c: &'a Campaign, id: &str) -> &'a Token {
        c.tokens.iter().find(|t| t.token_id == id).expect("token")
    }

    fn token_of(e: &Event) -> &Token {
        match e.kind.as_ref() {
            Some(event::Kind::TokenUpserted(u)) => u.token.as_ref().expect("token"),
            other => panic!("not a token event: {other:?}"),
        }
    }

    // -------------------------------------------------------- set_hit_points

    #[test]
    fn gm_and_controller_set_hit_points() {
        let mut c = campaign();
        let sheets = c.creatures.clone();
        let events = run(&mut c, "alice", set_hp("t-alice", 3, 0)).unwrap();
        assert_eq!(events.len(), 1);
        let t = token_of(&events[0]);
        // 3 of 12 is a quarter or less: near death. Temporary HP cleared.
        assert_eq!(
            (t.hp_current, t.hp_max, t.hp_temp, t.health_state),
            (
                Some(3),
                Some(12),
                Some(0),
                pb::HealthState::NearDeath as i32
            )
        );
        assert_eq!(stored(&c, "t-alice"), t);
        assert_eq!((events[0].seq, events[0].scene_id.as_str()), (1, "s1"));
        assert_eq!(events[0].client_intent_id, "ci-1");
        // The GM sets any token, even a hidden one, up to the maximum.
        let events = run(&mut c, "gm", set_hp("t-hidden", 12, 999)).unwrap();
        let t = token_of(&events[0]);
        assert_eq!(
            (t.hp_current, t.hp_temp, t.health_state),
            (Some(12), Some(999), pb::HealthState::Healthy as i32)
        );
        let events = run(&mut c, "gm", set_hp("t-goblin", 0, 0)).unwrap();
        assert_eq!(
            token_of(&events[0]).health_state,
            pb::HealthState::Down as i32
        );
        assert_eq!(c.creatures, sheets, "Creature.hp_* is not changed");
        assert_eq!(c.groups.len(), 3);
        assert_eq!(c.groups[0].changed_token_ids, ["t-alice"]);
        assert!(c.groups[0].changed_creature_ids.is_empty());
    }

    #[test]
    fn set_hit_points_rejections_change_nothing() {
        let mut c = campaign();
        c.tokens.push(token("t-prop", "", &[], false, 175.0));
        let prop = c.tokens.last_mut().expect("prop");
        (prop.hp_current, prop.hp_max, prop.hp_temp) = (None, None, None);
        // A player: someone else's, a hidden one, a missing one.
        for id in ["t-goblin", "t-hidden", "t-nobody"] {
            refuse(&mut c, "alice", set_hp(id, 1, 0), "not_permitted");
        }
        refuse(&mut c, "bob", set_hp("t-alice", 1, 0), "not_permitted");
        refuse(&mut c, "spec", set_hp("t-alice", 1, 0), "not_permitted");
        refuse(&mut c, "gm", set_hp("t-nobody", 1, 0), "unknown_token");
        refuse(&mut c, "gm", set_hp("t-prop", 1, 0), "no_hit_points");
        refuse(&mut c, "gm", set_hp("t-alice", -1, 0), "bad_hit_points");
        refuse(&mut c, "gm", set_hp("t-alice", 13, 0), "bad_hit_points");
        refuse(&mut c, "gm", set_hp("t-alice", 5, -1), "bad_hit_points");
        refuse(&mut c, "gm", set_hp("t-alice", 5, 1000), "bad_hit_points");
        // A controller of a token that is hidden gets not_permitted too.
        c.tokens[0].hidden = true;
        refuse(&mut c, "alice", set_hp("t-alice", 1, 0), "not_permitted");
    }

    // ---------------------------------------------------------- set_conditions

    #[test]
    fn conditions_are_added_removed_and_their_timers_go() {
        let mut c = campaign();
        c.tokens[1].conditions = vec!["prone".into(), "blessed".into()];
        c.tokens[1].condition_timers = vec![
            pb::ConditionTimer {
                effect_id: "blessed".into(),
                ends_round: 4,
                ..Default::default()
            },
            pb::ConditionTimer {
                effect_id: "prone".into(),
                ends_round: 2,
                ..Default::default()
            },
        ];
        let events = run(
            &mut c,
            "gm",
            set_cond(
                "t-goblin",
                &["poisoned", "prone", "poisoned"],
                &["blessed", "absent"],
            ),
        )
        .unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].warnings.is_empty());
        let t = token_of(&events[0]);
        assert_eq!(t.conditions, ["prone", "poisoned"]);
        assert_eq!(t.condition_timers.len(), 1);
        assert_eq!(t.condition_timers[0].effect_id, "prone");
        assert_eq!(stored(&c, "t-goblin"), t);
        // Nothing to change still answers with the token.
        let events = run(&mut c, "gm", set_cond("t-goblin", &["prone"], &["absent"])).unwrap();
        assert_eq!(token_of(&events[0]).conditions, ["prone", "poisoned"]);
        assert!(c.groups.last().unwrap().changed_token_ids.is_empty());
    }

    #[test]
    fn an_immune_condition_is_added_with_a_warning() {
        let mut c = campaign();
        c.creatures[1].condition_immunities = vec!["poisoned".into()];
        let events = run(
            &mut c,
            "gm",
            set_cond("t-goblin", &["poisoned", "prone"], &[]),
        )
        .unwrap();
        assert_eq!(events[0].warnings.len(), 1);
        assert_eq!(events[0].warnings[0].code, "condition_immune");
        assert_eq!(stored(&c, "t-goblin").conditions, ["poisoned", "prone"]);
    }

    #[test]
    fn set_conditions_rejections_change_nothing() {
        let mut c = campaign();
        refuse(
            &mut c,
            "alice",
            set_cond("t-alice", &["prone"], &[]),
            "not_permitted",
        );
        refuse(
            &mut c,
            "alice",
            set_cond("t-nobody", &["prone"], &[]),
            "not_permitted",
        );
        refuse(
            &mut c,
            "gm",
            set_cond("t-nobody", &["prone"], &[]),
            "unknown_token",
        );
        for bad in ["", "Prone", "has space", "dash-ed", &"a".repeat(41)] {
            refuse(
                &mut c,
                "gm",
                set_cond("t-goblin", &[bad], &[]),
                "bad_condition_id",
            );
            refuse(
                &mut c,
                "gm",
                set_cond("t-goblin", &[], &[bad]),
                "bad_condition_id",
            );
        }
        refuse(
            &mut c,
            "gm",
            set_cond("t-goblin", &["prone"], &["prone"]),
            "conflicting_conditions",
        );
        let many: Vec<String> = (0..33).map(|n| format!("c{n}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        refuse(
            &mut c,
            "gm",
            set_cond("t-goblin", &many, &[]),
            "too_many_conditions",
        );
        // Exactly 32 is fine.
        run(&mut c, "gm", set_cond("t-goblin", &many[..32], &[])).unwrap();
        assert_eq!(stored(&c, "t-goblin").conditions.len(), 32);
        refuse(
            &mut c,
            "gm",
            set_cond("t-goblin", &["one_more"], &[]),
            "too_many_conditions",
        );
    }

    // ------------------------------------------------------------ create_token

    #[test]
    fn create_token_copies_hit_points_and_mints_the_id() {
        let mut c = campaign();
        c.creatures[1].hp_current = 6;
        c.creatures[1].hp_temp = 2;
        c.creatures[1].size = pb::CreatureSize::Large as i32;
        let events = run(
            &mut c,
            "gm",
            create(|t| {
                t.creature_id = "c-goblin".into();
                t.hidden = true;
            }),
        )
        .unwrap();
        assert_eq!(events.len(), 1, "no controllers, no creature event");
        let t = token_of(&events[0]);
        assert_eq!(t.token_id, SequentialIds::new().next_id(1_000));
        assert_eq!(
            (t.scene_id.as_str(), t.level_id.as_str(), t.name.as_str()),
            ("s1", "l1", "c-goblin")
        );
        assert_eq!(t.position, Some(p(35.0, 35.0)));
        assert_eq!(t.size, pb::CreatureSize::Large as i32);
        assert_eq!(
            (t.hp_current, t.hp_max, t.hp_temp, t.health_state),
            (Some(6), Some(12), Some(2), pb::HealthState::Bloodied as i32)
        );
        assert!(t.hidden && t.conditions.is_empty() && t.condition_timers.is_empty());
        assert_eq!(c.tokens.last(), Some(t));
        assert_eq!(
            c.groups.last().unwrap().changed_token_ids,
            std::slice::from_ref(&t.token_id)
        );
        assert_eq!(events[0].scene_id, "s1");
    }

    #[test]
    fn create_token_with_controllers_also_sends_the_sheet() {
        let mut c = campaign();
        let events = run(
            &mut c,
            "gm",
            create(|t| {
                t.creature_id = "c-bob".into();
                t.name = "Bob the Bold".into();
                t.size = pb::CreatureSize::Small as i32;
                t.controller_user_ids = vec!["bob".into(), "bob".into()];
            }),
        )
        .unwrap();
        assert_eq!(events.len(), 2);
        let t = token_of(&events[0]);
        assert_eq!(t.name, "Bob the Bold");
        assert_eq!(t.size, pb::CreatureSize::Small as i32);
        assert_eq!(t.controller_user_ids, ["bob"]);
        match events[1].kind.as_ref() {
            Some(event::Kind::CreatureUpserted(u)) => {
                assert_eq!(u.creature.as_ref().unwrap().creature_id, "c-bob");
            }
            other => panic!("expected CreatureUpserted, got {other:?}"),
        }
        assert!(c.event_for(&events[1], &viewer(&c, "bob")).is_some());
        assert!(c.event_for(&events[1], &viewer(&c, "alice")).is_none());
    }

    #[test]
    fn create_token_prop_has_no_creature_and_no_hit_points() {
        let mut c = campaign();
        let events = run(&mut c, "gm", create(|t| t.level_id = String::new())).unwrap();
        let t = token_of(&events[0]);
        assert_eq!((t.name.as_str(), t.creature_id.as_str()), ("Token", ""));
        assert_eq!(t.size, pb::CreatureSize::Medium as i32);
        assert_eq!((t.hp_current, t.hp_max, t.hp_temp), (None, None, None));
        // Controllers on a prop: no creature to send.
        let events = run(
            &mut c,
            "gm",
            create(|t| t.controller_user_ids = vec!["alice".into()]),
        )
        .unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn create_token_rejections_change_nothing() {
        let mut c = campaign();
        refuse(&mut c, "alice", create(|_| {}), "not_permitted");
        refuse(
            &mut c,
            "gm",
            create(|t| t.scene_id = "nope".into()),
            "unknown_scene",
        );
        refuse(
            &mut c,
            "gm",
            create(|t| t.level_id = "nope".into()),
            "unknown_level",
        );
        refuse(
            &mut c,
            "gm",
            create(|t| t.creature_id = "nope".into()),
            "unknown_creature",
        );
        refuse(
            &mut c,
            "gm",
            create(|t| t.position = None),
            "bad_coordinate",
        );
        for bad in [f32::NAN, f32::INFINITY, 1_000_001.0, -1_000_001.0] {
            refuse(
                &mut c,
                "gm",
                create(|t| t.position = Some(p(bad, 0.0))),
                "bad_coordinate",
            );
            refuse(
                &mut c,
                "gm",
                create(|t| t.position = Some(p(0.0, bad))),
                "bad_coordinate",
            );
        }
        refuse(
            &mut c,
            "gm",
            create(|t| t.name = "x".repeat(121)),
            "name_too_long",
        );
        refuse(
            &mut c,
            "gm",
            create(|t| t.controller_user_ids = vec!["ghost".into()]),
            "unknown_user",
        );
        refuse(
            &mut c,
            "gm",
            create(|t| t.controller_user_ids = vec!["spec".into()]),
            "invalid_controller",
        );
        // Edges that are accepted.
        run(&mut c, "gm", create(|t| t.name = "x".repeat(120))).unwrap();
        run(
            &mut c,
            "gm",
            create(|t| t.position = Some(p(1_000_000.0, -1_000_000.0))),
        )
        .unwrap();
        // 500 tokens is the limit.
        while c.tokens.len() < 500 {
            run(&mut c, "gm", create(|_| {})).unwrap();
        }
        refuse(&mut c, "gm", create(|_| {}), "too_many_tokens");
        assert_eq!(c.tokens.len(), 500);
    }

    // ------------------------------------------------------------ remove_token

    #[test]
    fn remove_token_keeps_the_creature_and_who_could_see_it() {
        let mut c = campaign();
        let creatures = c.creatures.clone();
        let events = run(&mut c, "gm", remove("t-goblin")).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0].kind,
            Some(event::Kind::TokenRemoved(r)) if r.token_id == "t-goblin"
        ));
        assert_eq!(events[0].scene_id, "s1");
        assert!(c.tokens.iter().all(|t| t.token_id != "t-goblin"));
        assert_eq!(c.creatures, creatures);
        // It was visible: everyone gets the removal, the log keeps it.
        for who in ["gm", "alice", "bob", "spec"] {
            assert!(c.event_for(&events[0], &viewer(&c, who)).is_some(), "{who}");
        }
        assert_eq!(c.log[0], events[0]);
        assert_eq!(c.groups[0].changed_token_ids, ["t-goblin"]);
        // A hidden token: the GM only.
        let events = run(&mut c, "gm", remove("t-hidden")).unwrap();
        assert!(c.event_for(&events[0], &viewer(&c, "gm")).is_some());
        for who in ["alice", "bob", "spec"] {
            assert!(c.event_for(&events[0], &viewer(&c, who)).is_none(), "{who}");
        }
        assert!(c.removed_token_hidden["t-hidden"]);
        assert!(!c.removed_token_hidden["t-goblin"]);
    }

    #[test]
    fn remove_token_rejections_change_nothing() {
        let mut c = campaign();
        c.encounters.push(Encounter {
            encounter_id: "enc".into(),
            scene_id: "s1".into(),
            combatants: vec![Combatant {
                token_id: "t-alice".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        refuse(&mut c, "alice", remove("t-alice"), "not_permitted");
        refuse(&mut c, "alice", remove("t-nobody"), "not_permitted");
        refuse(&mut c, "gm", remove("t-nobody"), "unknown_token");
        refuse(&mut c, "gm", remove("t-alice"), "in_encounter");
        run(&mut c, "gm", remove("t-goblin")).unwrap();
    }

    // ------------------------------------------------- load and visibility

    #[test]
    fn loading_recomputes_health_state_from_the_numbers() {
        let mut snap = campaign().snapshot_for(&viewer(&campaign(), "gm"));
        for t in &mut snap.tokens {
            t.health_state = pb::HealthState::Down as i32;
        }
        snap.tokens[0].hp_current = Some(12);
        snap.tokens[1].hp_current = Some(6);
        snap.tokens[2].hp_current = Some(3);
        let mut prop = token("t-prop", "", &[], false, 1.0);
        (prop.hp_current, prop.hp_max, prop.hp_temp) = (None, None, None);
        prop.health_state = pb::HealthState::Down as i32;
        snap.tokens.push(prop);
        snap.tokens.push(Token {
            hp_current: None,
            hp_max: Some(8),
            ..token("t-fresh", "", &[], false, 1.0)
        });
        let c = Campaign::from_snapshot(snap);
        let states: Vec<i32> = c.tokens.iter().map(|t| t.health_state).collect();
        assert_eq!(
            states,
            [
                pb::HealthState::Healthy as i32,
                pb::HealthState::Bloodied as i32,
                pb::HealthState::NearDeath as i32,
                pb::HealthState::Down as i32, // no hp_max: left as stored
                pb::HealthState::Healthy as i32, // no hp_current: full
            ]
        );
    }

    #[test]
    fn a_player_never_receives_a_monsters_hit_points() {
        let mut c = campaign();
        c.creatures[0].abilities = vec![pb::Ability {
            ability_id: "sword".into(),
            name: "sword".into(),
            targeting: Some(pb::Targeting {
                shape: pb::TargetShape::Creatures as i32,
                ..Default::default()
            }),
            steps: vec![pb::EffectStep {
                step: Some(pb::effect_step::Step::Attack(pb::AttackStep {
                    to_hit_bonus: 5,
                    damage: vec![pb::DamagePart {
                        dice: "1d8+3".into(),
                        r#type: pb::DamageType::Slashing as i32,
                    }],
                })),
            }],
            ..Default::default()
        }];
        let mut events = Vec::new();
        events.extend(
            run_with(
                &mut c,
                "alice",
                {
                    intent_of(intent::Kind::UseAbility(pb::UseAbilityIntent {
                        source_token_id: "t-alice".into(),
                        ability_id: "sword".into(),
                        target_token_ids: vec!["t-goblin".into()],
                        ..Default::default()
                    }))
                },
                &mut Scripted::new(&[14, 3]),
            )
            .unwrap(),
        );
        events.extend(run(&mut c, "gm", set_hp("t-goblin", 5, 3)).unwrap());
        events.extend(run(&mut c, "gm", set_cond("t-goblin", &["prone"], &[])).unwrap());
        let made = run(&mut c, "gm", create(|t| t.creature_id = "c-goblin".into())).unwrap();
        let new_id = token_of(&made[0]).token_id.clone();
        events.extend(made);
        let mut seen = 0;
        for who in ["alice", "bob", "spec"] {
            let v = viewer(&c, who);
            let snap = c.snapshot_for(&v);
            for t in snap.tokens.iter().filter(|t| t.creature_id == "c-goblin") {
                assert_eq!(
                    (t.hp_current, t.hp_max, t.hp_temp),
                    (None, None, None),
                    "{who} snapshot"
                );
                assert_ne!(t.health_state, pb::HealthState::Unspecified as i32);
            }
            assert!(snap.tokens.iter().any(|t| t.token_id == new_id));
            for e in events.iter().filter_map(|e| c.event_for(e, &v)) {
                if let Some(event::Kind::TokenUpserted(u)) = &e.kind {
                    let t = u.token.as_ref().unwrap();
                    if t.creature_id == "c-goblin" {
                        seen += 1;
                        assert_eq!((t.hp_current, t.hp_max, t.hp_temp), (None, None, None));
                        assert_ne!(t.health_state, pb::HealthState::Unspecified as i32);
                    }
                }
            }
        }
        assert_eq!(seen, 3 * 4, "every viewer got the four goblin token events");
        // The GM got the numbers.
        let gm = viewer(&c, "gm");
        let e = c.event_for(&events[1], &gm).unwrap();
        assert!(token_of(&e).hp_current.is_some());
    }
}

// ------------------------------------------------------------- undo (#101)

mod undoing {
    use super::*;
    use tt_contracts::v1 as pb;

    fn attack_ability(id: &str, resource: &str) -> pb::Ability {
        pb::Ability {
            ability_id: id.into(),
            name: id.into(),
            resource_key: resource.into(),
            resource_amount: if resource.is_empty() { 0 } else { 1 },
            targeting: Some(pb::Targeting {
                shape: pb::TargetShape::Creatures as i32,
                ..Default::default()
            }),
            steps: vec![pb::EffectStep {
                step: Some(pb::effect_step::Step::Attack(pb::AttackStep {
                    to_hit_bonus: 5,
                    damage: vec![pb::DamagePart {
                        dice: "1d8+3".into(),
                        r#type: pb::DamageType::Slashing as i32,
                    }],
                })),
            }],
            ..Default::default()
        }
    }

    /// Choking Ash: a Constitution save DC 100 (always fails), poison damage,
    /// then `blinded` for one round.
    fn ash() -> pb::Ability {
        pb::Ability {
            ability_id: "ash".into(),
            name: "ash".into(),
            targeting: Some(pb::Targeting {
                shape: pb::TargetShape::Creatures as i32,
                ..Default::default()
            }),
            steps: vec![
                pb::EffectStep {
                    step: Some(pb::effect_step::Step::Save(pb::SaveStep {
                        ability: pb::AbilityScore::Constitution as i32,
                        dc: 100,
                        damage: vec![pb::DamagePart {
                            dice: "2d4".into(),
                            r#type: pb::DamageType::Poison as i32,
                        }],
                        half_on_success: true,
                    })),
                },
                pb::EffectStep {
                    step: Some(pb::effect_step::Step::ApplyEffect(pb::ApplyEffectStep {
                        effect_id: "blinded".into(),
                        duration: Some(pb::Duration {
                            unit: pb::DurationUnit::Rounds as i32,
                            amount: 1,
                            concentration: false,
                        }),
                        only_on_failure: true,
                    })),
                },
            ],
            ..Default::default()
        }
    }

    /// Alice's creature: `bolt` (spends a spell slot of 3) and `sword`.
    /// The goblin's creature: `ash`. An encounter runs on scene s1.
    fn world() -> Campaign {
        let mut c = campaign();
        let alice = c.creatures.iter_mut().find(|x| x.creature_id == "c-alice");
        let alice = alice.expect("c-alice");
        alice.abilities = vec![
            attack_ability("bolt", "spell_slot_1"),
            attack_ability("sword", ""),
        ];
        alice.resources = vec![pb::Resource {
            key: "spell_slot_1".into(),
            label: "1st".into(),
            current: 3,
            max: 3,
        }];
        let goblin = c.creatures.iter_mut().find(|x| x.creature_id == "c-goblin");
        goblin.expect("c-goblin").abilities = vec![ash()];
        c.encounters.push(Encounter {
            encounter_id: "enc-1".into(),
            scene_id: "s1".into(),
            round: 1,
            combatants: vec![Combatant {
                token_id: "t-alice".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        c
    }

    fn use_ab(source: &str, ability: &str, target: &str) -> Intent {
        intent_of(intent::Kind::UseAbility(pb::UseAbilityIntent {
            source_token_id: source.into(),
            ability_id: ability.into(),
            target_token_ids: vec![target.into()],
            ..Default::default()
        }))
    }

    fn undo(seq: u64) -> Intent {
        Intent {
            client_intent_id: "undo-1".into(),
            kind: Some(intent::Kind::Undo(pb::UndoIntent { event_seq: seq })),
        }
    }

    fn set_hp(token_id: &str, cur: i32, temp: i32) -> Intent {
        intent_of(intent::Kind::SetHitPoints(pb::SetHitPointsIntent {
            token_id: token_id.into(),
            hp_current: cur,
            hp_temp: temp,
        }))
    }

    fn set_cond(token_id: &str, add: &[&str], remove: &[&str]) -> Intent {
        let own = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
        intent_of(intent::Kind::SetConditions(pb::SetConditionsIntent {
            token_id: token_id.into(),
            add: own(add),
            remove: own(remove),
        }))
    }

    fn remove(token_id: &str) -> Intent {
        intent_of(intent::Kind::RemoveToken(pb::RemoveTokenIntent {
            token_id: token_id.into(),
        }))
    }

    fn create(creature_id: &str) -> Intent {
        intent_of(intent::Kind::CreateToken(pb::CreateTokenIntent {
            scene_id: "s1".into(),
            creature_id: creature_id.into(),
            position: Some(p(35.0, 35.0)),
            ..Default::default()
        }))
    }

    fn stored<'a>(c: &'a Campaign, id: &str) -> &'a Token {
        c.tokens.iter().find(|t| t.token_id == id).expect("token")
    }

    fn sheet<'a>(c: &'a Campaign, id: &str) -> &'a Creature {
        c.creatures
            .iter()
            .find(|t| t.creature_id == id)
            .expect("creature")
    }

    fn kinds(events: &[Event]) -> Vec<&event::Kind> {
        events
            .iter()
            .map(|e| e.kind.as_ref().expect("kind"))
            .collect()
    }

    fn refuse(c: &mut Campaign, actor: &str, i: Intent, expect: &str) -> Rejection {
        let before = state_of(c);
        let groups = c.groups.clone();
        let r = run(c, actor, i).expect_err("expected a rejection");
        assert_eq!(r.code, expect, "{}", r.message);
        assert_eq!(state_of(c), before, "{expect}: state changed");
        assert_eq!(c.groups, groups, "{expect}: a group changed");
        r
    }

    fn undone_seqs(events: &[Event]) -> Vec<u64> {
        events
            .iter()
            .filter_map(|e| match e.kind.as_ref() {
                Some(event::Kind::EventUndone(u)) => Some(u.undone_seq),
                _ => None,
            })
            .collect()
    }

    fn sword_hit(c: &mut Campaign, target: &str) -> Vec<Event> {
        run_with(
            c,
            "alice",
            use_ab("t-alice", "sword", target),
            &mut Scripted::new(&[14, 5]),
        )
        .unwrap()
    }

    #[test]
    fn undo_an_attack_restores_hit_points_health_state_and_the_spell_slot() {
        let mut c = world();
        let (goblin0, alice0) = (stored(&c, "t-goblin").clone(), sheet(&c, "c-alice").clone());
        // d20 15 + 5 beats AC 15; 1d8 shows 6, +3 = 9 damage.
        let used = run_with(
            &mut c,
            "alice",
            use_ab("t-alice", "bolt", "t-goblin"),
            &mut Scripted::new(&[14, 5]),
        )
        .unwrap();
        assert_eq!(used.len(), 3, "resolution, goblin token, alice sheet");
        assert_ne!(stored(&c, "t-goblin").hp_current, goblin0.hp_current);
        assert_ne!(stored(&c, "t-goblin").health_state, goblin0.health_state);
        assert_eq!(sheet(&c, "c-alice").resources[0].current, 2);

        let undone = run(&mut c, "gm", undo(used[0].seq)).unwrap();
        assert_eq!(stored(&c, "t-goblin"), &goblin0);
        assert_eq!(sheet(&c, "c-alice"), &alice0);
        assert_eq!(
            undone.iter().map(|e| e.seq).collect::<Vec<_>>(),
            [4, 5, 6, 7, 8]
        );
        assert!(undone.iter().all(|e| e.client_intent_id == "undo-1"));
        match kinds(&undone)[..] {
            [event::Kind::TokenUpserted(t), event::Kind::CreatureUpserted(s), event::Kind::EventUndone(a), event::Kind::EventUndone(b), event::Kind::EventUndone(d)] =>
            {
                assert_eq!(t.token.as_ref(), Some(&goblin0));
                assert_eq!(s.creature.as_ref(), Some(&alice0));
                assert_eq!((a.undone_seq, b.undone_seq, d.undone_seq), (1, 2, 3));
            }
            ref other => panic!("unexpected undo group: {other:?}"),
        }
        // The players see the restored health_state and no hit points.
        let e = c.event_for(&undone[0], &viewer(&c, "bob")).expect("token");
        match e.kind {
            Some(event::Kind::TokenUpserted(u)) => {
                let t = u.token.expect("token");
                assert_eq!(t.health_state, goblin0.health_state);
                assert_eq!((t.hp_current, t.hp_max, t.hp_temp), (None, None, None));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn undo_choking_ash_removes_the_condition_and_its_timer() {
        let mut c = world();
        let alice0 = stored(&c, "t-alice").clone();
        let used = run_with(
            &mut c,
            "gm",
            use_ab("t-goblin", "ash", "t-alice"),
            &mut Scripted::new(&[0, 1]),
        )
        .unwrap();
        let hit = stored(&c, "t-alice");
        assert_eq!(hit.conditions, ["blinded"]);
        assert_eq!(hit.condition_timers.len(), 1);
        run(&mut c, "gm", undo(used[0].seq)).unwrap();
        let back = stored(&c, "t-alice");
        assert!(back.conditions.is_empty() && back.condition_timers.is_empty());
        assert_eq!(back, &alice0);
    }

    #[test]
    fn undo_set_hit_points_set_conditions_and_move() {
        let mut c = world();
        let before = c.tokens.clone();
        let hp = run(&mut c, "gm", set_hp("t-goblin", 2, 0)).unwrap();
        run(&mut c, "gm", undo(hp[0].seq)).unwrap();
        assert_eq!(c.tokens, before);

        let cond = run(&mut c, "gm", set_cond("t-alice", &["prone"], &[])).unwrap();
        assert_eq!(stored(&c, "t-alice").conditions, ["prone"]);
        run(&mut c, "gm", undo(cond[0].seq)).unwrap();
        assert_eq!(c.tokens, before);

        let start = token_pos(&c, "t-goblin").unwrap();
        let path = vec![p(1.0, 1.0), p(3.0, 3.0)];
        let moved = run(&mut c, "gm", mv("t-goblin", path)).unwrap();
        assert_eq!(token_pos(&c, "t-goblin"), Some(p(3.0, 3.0)));
        let undone = run(&mut c, "gm", undo(moved[0].seq)).unwrap();
        assert_eq!(token_pos(&c, "t-goblin"), Some(start));
        match kinds(&undone)[..] {
            [event::Kind::TokenMoved(m), event::Kind::EventUndone(_)] => {
                assert_eq!(m.token_id, "t-goblin");
                assert_eq!(m.path, [start]);
                assert_eq!(m.level_id, "l1");
            }
            ref other => panic!("unexpected undo group: {other:?}"),
        }
        assert_eq!(c.tokens, before);
    }

    #[test]
    fn undo_a_create_and_a_remove() {
        let mut c = world();
        let made = run(&mut c, "gm", create("c-goblin")).unwrap();
        let new_id = match kinds(&made)[0] {
            event::Kind::TokenUpserted(u) => u.token.as_ref().unwrap().token_id.clone(),
            other => panic!("{other:?}"),
        };
        let undone = run(&mut c, "gm", undo(made[0].seq)).unwrap();
        assert!(c.tokens.iter().all(|t| t.token_id != new_id));
        match kinds(&undone)[..] {
            [event::Kind::TokenRemoved(r), event::Kind::EventUndone(_)] => {
                assert_eq!(r.token_id, new_id)
            }
            ref other => panic!("unexpected undo group: {other:?}"),
        }

        // A removed token comes back whole, with its sheet when controlled.
        let alice0 = stored(&c, "t-alice").clone();
        let goblin0 = stored(&c, "t-goblin").clone();
        let gone = run(&mut c, "gm", remove("t-goblin")).unwrap();
        assert!(c.removed_token_hidden.contains_key("t-goblin"));
        let back = run(&mut c, "gm", undo(gone[0].seq)).unwrap();
        assert_eq!(stored(&c, "t-goblin"), &goblin0);
        assert_eq!(stored(&c, "t-goblin").hp_current, Some(10));
        assert!(!c.removed_token_hidden.contains_key("t-goblin"));
        assert_eq!(back.len(), 2, "no sheet event: nobody controls the goblin");

        // t-alice is in the world's encounter: take her out first.
        c.encounters.clear();
        let gone = run(&mut c, "gm", remove("t-alice")).unwrap();
        let back = run(&mut c, "gm", undo(gone[0].seq)).unwrap();
        assert_eq!(stored(&c, "t-alice"), &alice0);
        match kinds(&back)[..] {
            [event::Kind::TokenUpserted(t), event::Kind::CreatureUpserted(s), event::Kind::EventUndone(_)] =>
            {
                assert_eq!(t.token.as_ref(), Some(&alice0));
                assert_eq!(s.creature.as_ref().unwrap().creature_id, "c-alice");
            }
            ref other => panic!("unexpected undo group: {other:?}"),
        }
    }

    #[test]
    fn every_refusal_code_in_order_and_nothing_changes() {
        let mut c = world();
        let hp = run(&mut c, "gm", set_hp("t-goblin", 5, 0)).unwrap();
        let said = run(&mut c, "alice", chat("hi")).unwrap();
        let d20 = roll("d20", RollMode::Normal, RollVisibility::Public, "");
        let rolled = run(&mut c, "gm", d20).unwrap();
        let ctl = run(&mut c, "gm", set_ctl("t-hidden", &["bob"])).unwrap();
        let bg = run(&mut c, "gm", set_bg("s1", "l1", "")).unwrap();
        let hp2 = run(&mut c, "gm", set_hp("t-goblin", 4, 0)).unwrap();

        for who in ["alice", "spec", "stranger"] {
            refuse(&mut c, who, undo(hp[0].seq), "not_permitted");
        }
        // The permission check comes first, even for a seq that is unknown.
        refuse(&mut c, "alice", undo(9999), "not_permitted");
        refuse(&mut c, "gm", undo(9999), "unknown_event");
        refuse(&mut c, "gm", undo(0), "unknown_event");
        for e in [&said, &rolled, &ctl, &bg] {
            refuse(&mut c, "gm", undo(e[0].seq), "not_undoable");
        }
        // Set controllers sent two events: the second is not a first event.
        assert_eq!(ctl.len(), 2);
        refuse(&mut c, "gm", undo(ctl[1].seq), "not_first_event");
        let r = refuse(&mut c, "gm", undo(hp[0].seq), "undo_blocked");
        assert!(r.message.contains(&hp2[0].seq.to_string()), "{}", r.message);
        // Undo the later change, then the earlier one, then neither again.
        let u2 = run(&mut c, "gm", undo(hp2[0].seq)).unwrap();
        refuse(&mut c, "gm", undo(hp2[0].seq), "already_undone");
        // An undo group cannot be undone, and an event inside it is not first.
        refuse(&mut c, "gm", undo(u2[0].seq), "not_undoable");
        refuse(&mut c, "gm", undo(u2[1].seq), "not_first_event");
        run(&mut c, "gm", undo(hp[0].seq)).unwrap();
        refuse(&mut c, "gm", undo(hp[0].seq), "already_undone");
    }

    #[test]
    fn events_before_the_seed_seq_are_unknown() {
        let mut c = world();
        c.seq = 40;
        let hp = run(&mut c, "gm", set_hp("t-goblin", 5, 0)).unwrap();
        assert_eq!(hp[0].seq, 41);
        refuse(&mut c, "gm", undo(40), "unknown_event");
        refuse(&mut c, "gm", undo(1), "unknown_event");
        run(&mut c, "gm", undo(41)).unwrap();
    }

    #[test]
    fn a_later_attack_blocks_the_earlier_undo_until_it_is_undone() {
        let mut c = world();
        let goblin0 = stored(&c, "t-goblin").clone();
        let first = sword_hit(&mut c, "t-goblin");
        let second = sword_hit(&mut c, "t-goblin");
        let r = refuse(&mut c, "gm", undo(first[0].seq), "undo_blocked");
        assert!(r.message.contains(&second[0].seq.to_string()));
        run(&mut c, "gm", undo(second[0].seq)).unwrap();
        run(&mut c, "gm", undo(first[0].seq)).unwrap();
        assert_eq!(stored(&c, "t-goblin"), &goblin0);
    }

    #[test]
    fn naming_the_attacks_token_event_is_not_first_event() {
        let mut c = world();
        let used = sword_hit(&mut c, "t-goblin");
        assert!(matches!(used[1].kind, Some(event::Kind::TokenUpserted(_))));
        let r = refuse(&mut c, "gm", undo(used[1].seq), "not_first_event");
        assert!(r.message.contains(&used[0].seq.to_string()));
    }

    #[test]
    fn a_creature_shared_by_two_groups_blocks_too() {
        let mut c = world();
        // Both attacks spend alice's slot but hit different tokens.
        let mut bolt = |target: &str| {
            run_with(
                &mut c,
                "alice",
                use_ab("t-alice", "bolt", target),
                &mut Scripted::new(&[14, 5]),
            )
            .unwrap()
        };
        let first = bolt("t-goblin");
        bolt("t-alice");
        refuse(&mut c, "gm", undo(first[0].seq), "undo_blocked");
    }

    #[test]
    fn the_log_keeps_the_originals_and_the_undo_group() {
        let mut c = world();
        let hp = run(&mut c, "gm", set_hp("t-goblin", 5, 0)).unwrap();
        let undone = run(&mut c, "gm", undo(hp[0].seq)).unwrap();
        let all: Vec<Event> = hp.iter().chain(undone.iter()).cloned().collect();
        assert_eq!(c.log, all);
        let seqs: Vec<u64> = c.log.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, [1, 2, 3]);
        assert_eq!(undone_seqs(&undone), [1]);
        // The undo group changed the token, so it is a group of its own.
        let g = c.groups.last().unwrap();
        assert_eq!((g.first_seq, g.last_seq), (2, 3));
        assert_eq!(g.changed_token_ids, ["t-goblin"]);
        assert!(c.groups[0].undone);
        assert!(!g.undone);
    }

    #[test]
    fn who_gets_the_event_undone() {
        let mut c = world();
        let (alice, bob, gm) = (viewer(&c, "alice"), viewer(&c, "bob"), viewer(&c, "gm"));
        // A public attack on the visible goblin: players see the undo.
        let used = sword_hit(&mut c, "t-goblin");
        let undone = run(&mut c, "gm", undo(used[0].seq)).unwrap();
        let marks: Vec<&Event> = undone
            .iter()
            .filter(|e| matches!(e.kind, Some(event::Kind::EventUndone(_))))
            .collect();
        assert_eq!(marks.len(), 2);
        for v in [&alice, &bob, &gm, &viewer(&c, "spec")] {
            assert!(marks.iter().all(|e| c.event_for(e, v).is_some()));
        }

        // A change to the hidden token: only the GM.
        let hp = run(&mut c, "gm", set_hp("t-hidden", 3, 0)).unwrap();
        let undone = run(&mut c, "gm", undo(hp[0].seq)).unwrap();
        for e in &undone {
            assert!(c.event_for(e, &gm).is_some());
            assert!(c.event_for(e, &alice).is_none(), "{:?}", e.kind);
        }

        // Removing a visible token and undoing it: players got the removal,
        // so they get the undo; for the hidden token they get neither.
        let gone = run(&mut c, "gm", remove("t-goblin")).unwrap();
        let back = run(&mut c, "gm", undo(gone[0].seq)).unwrap();
        assert!(back.iter().all(|e| c.event_for(e, &bob).is_some()));
        let gone = run(&mut c, "gm", remove("t-hidden")).unwrap();
        let back = run(&mut c, "gm", undo(gone[0].seq)).unwrap();
        assert!(back.iter().all(|e| c.event_for(e, &bob).is_none()));
        assert!(back.iter().all(|e| c.event_for(e, &gm).is_some()));

        // A create undone: the new token's removal and the undo reach players.
        let made = run(&mut c, "gm", create("")).unwrap();
        let back = run(&mut c, "gm", undo(made[0].seq)).unwrap();
        assert!(back.iter().all(|e| c.event_for(e, &bob).is_some()));
    }

    #[test]
    fn the_before_state_is_kept_whole_for_undoable_groups_only() {
        let mut c = world();
        let goblin0 = stored(&c, "t-goblin").clone();
        run(&mut c, "gm", remove("t-goblin")).unwrap();
        let g = c.groups.last().unwrap();
        assert_eq!(g.before_tokens, [goblin0]);
        assert_eq!(g.kind, crate::campaign::GroupKind::Edit);
        run(&mut c, "alice", chat("x")).unwrap();
        let g = c.groups.last().unwrap();
        assert!(g.before_tokens.is_empty() && g.before_creatures.is_empty());
    }
}

// ------------------------------------------------------- encounters (#102)

mod encountering {
    use super::*;
    use tt_contracts::v1 as pb;

    fn start(scene: &str, tokens: &[&str]) -> Intent {
        intent_of(intent::Kind::StartEncounter(pb::StartEncounterIntent {
            scene_id: scene.into(),
            token_ids: tokens.iter().map(|s| s.to_string()).collect(),
        }))
    }

    fn end_turn(encounter: &str) -> Intent {
        intent_of(intent::Kind::EndTurn(pb::EndTurnIntent {
            encounter_id: encounter.into(),
        }))
    }

    fn set_init(encounter: &str, token: &str, initiative: i32, order: u32) -> Intent {
        intent_of(intent::Kind::SetInitiative(pb::SetInitiativeIntent {
            encounter_id: encounter.into(),
            token_id: token.into(),
            initiative,
            order,
        }))
    }

    fn end_enc(encounter: &str) -> Intent {
        intent_of(intent::Kind::EndEncounter(pb::EndEncounterIntent {
            encounter_id: encounter.into(),
        }))
    }

    fn undo(seq: u64) -> Intent {
        intent_of(intent::Kind::Undo(pb::UndoIntent { event_seq: seq }))
    }

    /// The test campaign plus t-bob (bob, creature c-bob). Initiative
    /// bonuses: c-alice +2, c-goblin 0, c-bob 0, c-hidden 0.
    fn world() -> Campaign {
        let mut c = campaign();
        c.tokens
            .push(token("t-bob", "c-bob", &["bob"], false, 245.0));
        for cr in &mut c.creatures {
            cr.initiative_bonus = if cr.creature_id == "c-alice" { 2 } else { 0 };
        }
        c
    }

    fn combatant(token_id: &str, order: u32) -> pb::Combatant {
        pb::Combatant {
            token_id: token_id.into(),
            initiative: 20 - order as i32,
            order,
            ..Default::default()
        }
    }

    /// `enc-1` on s1 with the given order, in round `round`, turn 0.
    fn running(c: &mut Campaign, order: &[&str], round: u32) {
        c.encounters.push(Encounter {
            encounter_id: "enc-1".into(),
            scene_id: "s1".into(),
            round,
            turn_index: 0,
            combatants: order
                .iter()
                .enumerate()
                .map(|(i, t)| combatant(t, i as u32))
                .collect(),
        });
    }

    fn timer(effect: &str, source: &str, ends_round: u32, at_end: bool) -> pb::ConditionTimer {
        pb::ConditionTimer {
            effect_id: effect.into(),
            encounter_id: "enc-1".into(),
            source_token_id: source.into(),
            ends_round,
            at_end_of_turn: at_end,
        }
    }

    fn timed(c: &mut Campaign, token_id: &str, t: pb::ConditionTimer) {
        let tok = c.tokens.iter_mut().find(|x| x.token_id == token_id);
        let tok = tok.expect("token");
        tok.conditions.push(t.effect_id.clone());
        tok.condition_timers.push(t);
    }

    fn stored<'a>(c: &'a Campaign, id: &str) -> &'a Token {
        c.tokens.iter().find(|t| t.token_id == id).expect("token")
    }

    fn encounter_of(e: &Event) -> &Encounter {
        match e.kind.as_ref() {
            Some(event::Kind::EncounterUpdated(u)) => u.encounter.as_ref().expect("encounter"),
            other => panic!("not EncounterUpdated: {other:?}"),
        }
    }

    fn order_of(e: &Encounter) -> Vec<&str> {
        e.combatants.iter().map(|c| c.token_id.as_str()).collect()
    }

    fn upserted(e: &Event) -> &Token {
        match e.kind.as_ref() {
            Some(event::Kind::TokenUpserted(u)) => u.token.as_ref().expect("token"),
            other => panic!("not TokenUpserted: {other:?}"),
        }
    }

    fn refuse(c: &mut Campaign, actor: &str, i: Intent, rng: &mut Scripted, expect: &str) {
        let before = state_of(c);
        let encounters = c.encounters.clone();
        let r = run_with(c, actor, i, rng).expect_err("expected a rejection");
        assert_eq!(r.code, expect, "{}", r.message);
        assert_eq!(state_of(c), before, "{expect}: state changed");
        assert_eq!(c.encounters, encounters, "{expect}: encounters changed");
    }

    fn breath(spent: bool, min_roll: u32) -> pb::Ability {
        pb::Ability {
            ability_id: "breath".into(),
            name: "Fire Breath".into(),
            recharge: Some(pb::Recharge { min_roll }),
            recharge_spent: spent,
            ..Default::default()
        }
    }

    #[test]
    fn start_rolls_one_initiative_per_token_and_sorts_with_both_tie_breaks() {
        let mut c = world();
        // List order bob, goblin, alice, bob again. d20 raw shows 12, 12, 10:
        // totals 12, 12 and 10 + 2 = 12. All tie; alice wins on her bonus,
        // then bob before the goblin by list order.
        let events = run_with(
            &mut c,
            "gm",
            start("s1", &["t-bob", "t-goblin", "t-alice", "t-bob"]),
            &mut Scripted::new(&[11, 11, 9]),
        )
        .unwrap();
        assert_eq!(events.len(), 4, "three rolls and the encounter");
        let labels: Vec<_> = events[..3]
            .iter()
            .map(|e| roll_of(e).label.clone())
            .collect();
        assert_eq!(
            labels,
            [
                "t-bob initiative",
                "t-goblin initiative",
                "t-alice initiative"
            ]
        );
        for e in &events[..3] {
            let r = roll_of(e);
            assert_eq!(r.purpose, pb::RollPurpose::Initiative as i32);
            assert_eq!(r.visibility, RollVisibility::Public as i32);
            assert_eq!(r.roller_user_id, "gm");
            assert!(!r.roll_id.is_empty());
            assert_eq!(r.total, 12);
        }
        assert_eq!(roll_of(&events[0]).roller_creature_id, "c-bob");
        assert_eq!(roll_of(&events[2]).notation, "1d20+2");
        assert_eq!(roll_of(&events[2]).modifiers[0].value, 2);
        assert_eq!(roll_of(&events[1]).notation, "1d20");
        let enc = encounter_of(&events[3]);
        assert_eq!(order_of(enc), ["t-alice", "t-bob", "t-goblin"]);
        assert_eq!((enc.round, enc.turn_index), (1, 0));
        assert_eq!(enc.scene_id, "s1");
        assert!(!enc.encounter_id.is_empty());
        let orders: Vec<_> = enc.combatants.iter().map(|x| x.order).collect();
        assert_eq!(orders, [0, 1, 2]);
        assert_eq!(c.encounters.len(), 1);
        assert_eq!(&c.encounters[0], enc);
        assert!(events.iter().all(|e| e.scene_id == "s1"));
        // The group never changes tokens or creatures and cannot be undone.
        let g = c.groups.last().unwrap();
        assert_eq!(g.kind, crate::campaign::GroupKind::NotUndoable);
        assert!(g.changed_token_ids.is_empty());
    }

    #[test]
    fn a_higher_roll_goes_first_and_a_token_without_a_creature_has_no_bonus() {
        let mut c = world();
        c.tokens.push(token("t-prop", "", &[], false, 35.0));
        // alice 1 + 2, prop 20 + 0.
        let events = run_with(
            &mut c,
            "gm",
            start("s1", &["t-alice", "t-prop"]),
            &mut Scripted::new(&[0, 19]),
        )
        .unwrap();
        assert_eq!(roll_of(&events[1]).total, 20);
        assert_eq!(roll_of(&events[1]).roller_creature_id, "");
        let enc = encounter_of(&events[2]);
        assert_eq!(order_of(enc), ["t-prop", "t-alice"]);
        assert_eq!(enc.combatants[0].initiative, 20);
        assert_eq!(enc.combatants[1].initiative, 3);
    }

    #[test]
    fn a_hidden_token_rolls_gm_only_and_the_encounter_goes_to_the_gm_alone() {
        let mut c = world();
        let events = run(&mut c, "gm", start("s1", &["t-alice", "t-hidden"])).unwrap();
        assert_eq!(
            roll_of(&events[0]).visibility,
            RollVisibility::Public as i32
        );
        assert_eq!(
            roll_of(&events[1]).visibility,
            RollVisibility::GmOnly as i32
        );
        let (gm, alice) = (viewer(&c, "gm"), viewer(&c, "alice"));
        assert!(c.event_for(&events[0], &alice).is_some());
        assert!(c.event_for(&events[1], &alice).is_none());
        assert!(c.event_for(&events[2], &alice).is_none());
        assert!(c.event_for(&events[2], &gm).is_some());
        assert!(c.snapshot_for(&alice).encounters.is_empty());
        assert_eq!(c.snapshot_for(&gm).encounters.len(), 1);
        // A player cannot end this turn: the encounter is invisible to them.
        let id = c.encounters[0].encounter_id.clone();
        refuse(
            &mut c,
            "alice",
            end_turn(&id),
            &mut Scripted::new(&[0]),
            "not_permitted",
        );
    }

    #[test]
    fn ending_turns_wraps_the_round_and_expires_timers_in_order() {
        let mut c = world();
        running(&mut c, &["t-alice", "t-bob", "t-goblin"], 1);
        // Ends when the goblin's own turn ends in round 1.
        timed(&mut c, "t-goblin", timer("dazed", "t-goblin", 1, true));
        // Ends when alice's turn starts in round 2.
        timed(&mut c, "t-bob", timer("blinded", "t-alice", 2, false));
        // Another encounter's timer is left alone.
        let mut other = timer("held", "t-alice", 1, false);
        other.encounter_id = "enc-other".into();
        timed(&mut c, "t-goblin", other);

        // A player ends the turn of the token they control.
        let events = run(&mut c, "alice", end_turn("enc-1")).unwrap();
        assert_eq!(events.len(), 1);
        let enc = encounter_of(&events[0]);
        assert_eq!((enc.round, enc.turn_index), (1, 1));
        // Not bob's turn... alice (not bob's controller) is refused; bob may.
        refuse(
            &mut c,
            "alice",
            end_turn("enc-1"),
            &mut Scripted::new(&[0]),
            "not_permitted",
        );
        run(&mut c, "bob", end_turn("enc-1")).unwrap();
        assert_eq!(c.encounters[0].turn_index, 2);

        // The goblin's turn ends (GM): its end timer, then alice starts round 2
        // and bob's start timer goes. Token events follow the token order.
        let events = run(&mut c, "gm", end_turn("enc-1")).unwrap();
        let enc = encounter_of(&events[0]);
        assert_eq!((enc.round, enc.turn_index), (2, 0));
        assert_eq!(events.len(), 3);
        assert_eq!(upserted(&events[1]).token_id, "t-goblin");
        assert_eq!(upserted(&events[1]).conditions, ["held"]);
        assert_eq!(upserted(&events[2]).token_id, "t-bob");
        assert!(upserted(&events[2]).conditions.is_empty());
        assert!(stored(&c, "t-bob").condition_timers.is_empty());
        assert_eq!(stored(&c, "t-goblin").condition_timers.len(), 1);
        assert_eq!(c.encounters[0].round, 2);
        // The turn change touched two tokens and is not undoable.
        let g = c.groups.last().unwrap();
        assert_eq!(g.kind, crate::campaign::GroupKind::NotUndoable);
        assert_eq!(g.changed_token_ids, ["t-goblin", "t-bob"]);
    }

    #[test]
    fn a_one_combatant_encounter_wraps_onto_itself() {
        let mut c = world();
        running(&mut c, &["t-goblin"], 1);
        timed(&mut c, "t-goblin", timer("dazed", "t-goblin", 2, false));
        let events = run(&mut c, "gm", end_turn("enc-1")).unwrap();
        let enc = encounter_of(&events[0]);
        assert_eq!((enc.round, enc.turn_index), (2, 0));
        assert_eq!(upserted(&events[1]).token_id, "t-goblin");
        assert!(stored(&c, "t-goblin").condition_timers.is_empty());
    }

    #[test]
    fn setting_initiative_mid_round_re_sorts_and_keeps_the_turn() {
        let mut c = world();
        // alice, bob, goblin at 12 each (orders 0, 1, 2), as in the start test.
        let events = run_with(
            &mut c,
            "gm",
            start("s1", &["t-bob", "t-goblin", "t-alice"]),
            &mut Scripted::new(&[11, 11, 9]),
        )
        .unwrap();
        let id = encounter_of(&events[3]).encounter_id.clone();
        run(&mut c, "alice", end_turn(&id)).unwrap();
        assert_eq!(c.encounters[0].turn_index, 1, "bob's turn");

        // Same initiative, a later order: the goblin stays last.
        let events = run(&mut c, "gm", set_init(&id, "t-goblin", 12, 5)).unwrap();
        assert_eq!(events.len(), 1);
        let enc = encounter_of(&events[0]);
        assert_eq!(order_of(enc), ["t-alice", "t-bob", "t-goblin"]);
        assert_eq!(enc.combatants[2].order, 5);
        assert_eq!(enc.turn_index, 1);
        // Higher initiative: the goblin jumps in front, bob's turn follows him.
        let events = run(&mut c, "gm", set_init(&id, "t-goblin", 20, 9)).unwrap();
        let enc = encounter_of(&events[0]);
        assert_eq!(order_of(enc), ["t-goblin", "t-alice", "t-bob"]);
        assert_eq!(enc.turn_index, 2);
        assert_eq!(enc.round, 1);
        assert_eq!(c.encounters[0], *enc);
        // Equal initiative: the lower order acts first.
        let events = run(&mut c, "gm", set_init(&id, "t-bob", 12, 0)).unwrap();
        let enc = encounter_of(&events[0]);
        assert_eq!(order_of(enc), ["t-goblin", "t-alice", "t-bob"]);
        let events = run(&mut c, "gm", set_init(&id, "t-bob", 12, 0)).unwrap();
        assert_eq!(encounter_of(&events[0]).turn_index, 2);
        let events = run(&mut c, "gm", set_init(&id, "t-alice", 12, 3)).unwrap();
        let enc = encounter_of(&events[0]);
        assert_eq!(order_of(enc), ["t-goblin", "t-bob", "t-alice"]);
        assert_eq!(enc.turn_index, 1, "still bob's turn");
        let g = c.groups.last().unwrap();
        assert_eq!(g.kind, crate::campaign::GroupKind::NotUndoable);
    }

    #[test]
    fn recharge_rolls_a_d6_when_the_turn_starts_and_succeeds_or_fails() {
        let mut c = world();
        running(&mut c, &["t-alice", "t-bob", "t-goblin"], 1);
        c.encounters[0].turn_index = 1;
        let goblin = c.creatures.iter_mut().find(|x| x.creature_id == "c-goblin");
        goblin.expect("c-goblin").abilities = vec![
            breath(true, 5),
            // Spent but no recharge rule, and a ready one: no rolls for these.
            pb::Ability {
                ability_id: "plain".into(),
                recharge_spent: true,
                ..Default::default()
            },
            pb::Ability {
                recharge: Some(pb::Recharge { min_roll: 2 }),
                ..breath(false, 2)
            },
        ];
        // Fails: the die shows 4 (value 3) against 5.
        let events = run_with(&mut c, "bob", end_turn("enc-1"), &mut Scripted::new(&[3])).unwrap();
        assert_eq!(events.len(), 2, "no creature event on a failure");
        let r = roll_of(&events[1]);
        assert_eq!(r.label, "Fire Breath recharge");
        assert_eq!(r.total, 4);
        assert_eq!(r.visibility, RollVisibility::GmOnly as i32);
        assert_eq!(r.roller_creature_id, "c-goblin");
        assert!(!r.roll_id.is_empty());
        assert!(c.creatures[1].abilities[0].recharge_spent);
        assert!(c.event_for(&events[1], &viewer(&c, "alice")).is_none());

        // Succeeds on the next goblin turn: 5 or more. Alice and bob go first.
        run(&mut c, "gm", end_turn("enc-1")).unwrap();
        run(&mut c, "gm", end_turn("enc-1")).unwrap();
        assert_eq!(c.encounters[0].turn_index, 1);
        let events = run_with(&mut c, "gm", end_turn("enc-1"), &mut Scripted::new(&[4])).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(roll_of(&events[1]).total, 5);
        let Some(event::Kind::CreatureUpserted(u)) = events[2].kind.as_ref() else {
            panic!("expected CreatureUpserted, got {:?}", events[2].kind);
        };
        let sheet = u.creature.as_ref().unwrap();
        assert!(!sheet.abilities[0].recharge_spent);
        assert!(sheet.abilities[1].recharge_spent, "no rule, left alone");
        assert_eq!(&c.creatures[1], sheet);
        assert_eq!(c.groups.last().unwrap().changed_creature_ids, ["c-goblin"]);
    }

    #[test]
    fn an_owned_creatures_recharge_roll_is_public() {
        let mut c = world();
        running(&mut c, &["t-goblin", "t-alice"], 1);
        let alice = c.creatures.iter_mut().find(|x| x.creature_id == "c-alice");
        alice.expect("c-alice").abilities = vec![breath(true, 6)];
        let events = run(&mut c, "gm", end_turn("enc-1")).unwrap();
        let r = roll_of(&events[1]);
        assert_eq!(r.visibility, RollVisibility::Public as i32);
        assert!(c.event_for(&events[1], &viewer(&c, "bob")).is_some());
        // A hidden token's creature is GM only even with an owner.
        let mut c = world();
        running(&mut c, &["t-goblin", "t-hidden"], 1);
        let hidden = c.creatures.iter_mut().find(|x| x.creature_id == "c-hidden");
        let hidden = hidden.expect("c-hidden");
        hidden.owner_user_id = "bob".into();
        hidden.abilities = vec![breath(true, 6)];
        let events = run(&mut c, "gm", end_turn("enc-1")).unwrap();
        assert_eq!(
            roll_of(&events[1]).visibility,
            RollVisibility::GmOnly as i32
        );
    }

    #[test]
    fn ending_the_encounter_keeps_conditions_and_drops_their_timers() {
        let mut c = world();
        running(&mut c, &["t-alice", "t-goblin"], 3);
        timed(&mut c, "t-goblin", timer("blinded", "t-alice", 5, false));
        c.tokens
            .iter_mut()
            .find(|t| t.token_id == "t-goblin")
            .unwrap()
            .conditions
            .push("poisoned".into());
        let events = run(&mut c, "gm", end_enc("enc-1")).unwrap();
        assert_eq!(events.len(), 2);
        let Some(event::Kind::EncounterEnded(e)) = events[0].kind.as_ref() else {
            panic!("expected EncounterEnded, got {:?}", events[0].kind);
        };
        assert_eq!(e.encounter_id, "enc-1");
        assert_eq!(events[0].scene_id, "s1");
        let t = upserted(&events[1]);
        assert_eq!(t.token_id, "t-goblin");
        assert_eq!(t.conditions, ["blinded", "poisoned"]);
        assert!(t.condition_timers.is_empty());
        assert_eq!(stored(&c, "t-goblin"), t);
        assert!(c.encounters.is_empty());
        for who in ["gm", "alice", "spec"] {
            assert!(c.snapshot_for(&viewer(&c, who)).encounters.is_empty());
            assert!(c.event_for(&events[0], &viewer(&c, who)).is_some());
        }
        // The scene may start another one now.
        run(&mut c, "gm", start("s1", &["t-alice"])).unwrap();
    }

    #[test]
    fn ended_encounter_reaches_everyone_even_when_it_was_gm_only() {
        let mut c = world();
        running(&mut c, &["t-hidden", "t-alice"], 1);
        let events = run(&mut c, "gm", end_enc("enc-1")).unwrap();
        assert_eq!(events.len(), 1);
        for who in ["gm", "alice", "bob", "spec"] {
            assert!(c.event_for(&events[0], &viewer(&c, who)).is_some());
        }
    }

    #[test]
    fn encounter_changes_block_undo_and_cannot_be_undone() {
        let mut c = world();
        running(&mut c, &["t-alice", "t-goblin"], 1);
        timed(&mut c, "t-goblin", timer("blinded", "t-alice", 2, false));
        // Edit group on the goblin (seq 1), then the end of the encounter
        // changes the goblin's timers (seq 2, 3).
        let hp = intent_of(intent::Kind::SetHitPoints(pb::SetHitPointsIntent {
            token_id: "t-goblin".into(),
            hp_current: 5,
            hp_temp: 0,
        }));
        run(&mut c, "gm", hp).unwrap();
        run(&mut c, "gm", end_enc("enc-1")).unwrap();
        assert_eq!(c.groups[1].kind, crate::campaign::GroupKind::NotUndoable);
        assert_eq!(c.groups[1].changed_token_ids, ["t-goblin"]);
        assert!(c.groups[1].before_tokens.is_empty());
        assert_eq!(code(run(&mut c, "gm", undo(1))), "undo_blocked");
        assert_eq!(code(run(&mut c, "gm", undo(2))), "not_undoable");
        // Starting is not undoable either.
        let events = run(&mut c, "gm", start("s1", &["t-bob"])).unwrap();
        assert_eq!(code(run(&mut c, "gm", undo(events[0].seq))), "not_undoable");
    }

    #[test]
    fn every_refusal_code_leaves_everything_untouched() {
        let mut c = world();
        c.tokens.push(token("t-far", "", &[], false, 35.0));
        c.tokens.last_mut().unwrap().scene_id = "s2".into();
        let mut ok = Scripted::new(&[0]);
        let mut bad = Scripted::new(&[0]);
        bad.fail = true;

        refuse(
            &mut c,
            "alice",
            start("s1", &["t-alice"]),
            &mut ok,
            "not_permitted",
        );
        refuse(
            &mut c,
            "spec",
            start("s1", &["t-alice"]),
            &mut ok,
            "not_permitted",
        );
        refuse(
            &mut c,
            "gm",
            start("nope", &["t-alice"]),
            &mut ok,
            "unknown_scene",
        );
        refuse(&mut c, "gm", start("s1", &[]), &mut ok, "no_combatants");
        refuse(
            &mut c,
            "gm",
            start("s1", &["t-alice", "t-x"]),
            &mut ok,
            "unknown_token",
        );
        refuse(
            &mut c,
            "gm",
            start("s1", &["t-far"]),
            &mut ok,
            "unknown_token",
        );
        // The entropy check comes before any state change.
        refuse(
            &mut c,
            "gm",
            start("s1", &["t-alice"]),
            &mut bad,
            "rng_unavailable",
        );
        run(&mut c, "gm", start("s1", &["t-alice", "t-goblin"])).unwrap();
        let id = c.encounters[0].encounter_id.clone();
        refuse(
            &mut c,
            "gm",
            start("s1", &["t-bob"]),
            &mut ok,
            "encounter_running",
        );

        refuse(&mut c, "spec", end_turn(&id), &mut ok, "not_permitted");
        refuse(&mut c, "gm", end_turn("nope"), &mut ok, "unknown_encounter");
        refuse(&mut c, "bob", end_turn("nope"), &mut ok, "not_permitted");
        let current = c.encounters[0].combatants[0].token_id.clone();
        let outsider = if current == "t-alice" { "bob" } else { "alice" };
        refuse(&mut c, outsider, end_turn(&id), &mut ok, "not_permitted");

        refuse(
            &mut c,
            "alice",
            set_init(&id, "t-alice", 1, 0),
            &mut ok,
            "not_permitted",
        );
        refuse(
            &mut c,
            "gm",
            set_init("nope", "t-alice", 1, 0),
            &mut ok,
            "unknown_encounter",
        );
        refuse(
            &mut c,
            "gm",
            set_init(&id, "t-bob", 1, 0),
            &mut ok,
            "unknown_combatant",
        );

        refuse(&mut c, "alice", end_enc(&id), &mut ok, "not_permitted");
        refuse(&mut c, "gm", end_enc("nope"), &mut ok, "unknown_encounter");

        // A failed recharge roll rejects the whole turn change.
        let goblin = c.creatures.iter_mut().find(|x| x.creature_id == "c-goblin");
        goblin.expect("c-goblin").abilities = vec![breath(true, 5)];
        let gi = c.encounters[0]
            .combatants
            .iter()
            .position(|x| x.token_id == "t-goblin")
            .unwrap();
        c.encounters[0].turn_index = ((gi + 1) % 2) as u32;
        refuse(&mut c, "gm", end_turn(&id), &mut bad, "rng_unavailable");
    }

    #[test]
    fn choking_ash_blinds_for_one_round_and_the_wisps_next_turn_lifts_it() {
        let seed: pb::Snapshot = serde_json::from_str(include_str!(
            "../../../contracts/fixtures/seeds/pregen-table.json"
        ))
        .expect("pregen seed parses");
        let mut c = Campaign::from_snapshot(seed);
        // Wisp 20 + 2, Brannoch 1 + 2: the wisp acts first.
        let events = run_with(
            &mut c,
            "mock-gm",
            start("scene-mock", &["token-ember-wisp", "token-brannoch"]),
            &mut Scripted::new(&[19, 0]),
        )
        .unwrap();
        let enc = encounter_of(&events[2]);
        assert_eq!(order_of(enc), ["token-ember-wisp", "token-brannoch"]);
        let id = enc.encounter_id.clone();

        let ash = intent_of(intent::Kind::UseAbility(pb::UseAbilityIntent {
            source_token_id: "token-ember-wisp".into(),
            ability_id: "choking-ash".into(),
            target_token_ids: vec!["token-brannoch".into()],
            ..Default::default()
        }));
        // Every die shows 1: the Constitution save fails against DC 11.
        let events = run_with(&mut c, "mock-gm", ash, &mut Scripted::new(&[0])).unwrap();
        let brannoch = stored(&c, "token-brannoch");
        assert_eq!(brannoch.conditions, ["blinded"]);
        assert_eq!(brannoch.condition_timers.len(), 1);
        let t = &brannoch.condition_timers[0];
        assert_eq!(t.ends_round, 2);
        assert_eq!(t.encounter_id, id);
        assert_eq!(t.source_token_id, "token-ember-wisp");
        assert!(events.iter().any(|e| matches!(
            e.kind.as_ref(),
            Some(event::Kind::TokenUpserted(u))
                if u.token.as_ref().is_some_and(|t| t.token_id == "token-brannoch")
        )));

        // Round 1, Brannoch's turn: still blind. The wisp's round 2 turn lifts it.
        let events = run(&mut c, "mock-gm", end_turn(&id)).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(stored(&c, "token-brannoch").conditions, ["blinded"]);
        let events = run(&mut c, "mock-player", end_turn(&id)).unwrap();
        assert_eq!(encounter_of(&events[0]).round, 2);
        assert_eq!(upserted(&events[1]).token_id, "token-brannoch");
        assert!(stored(&c, "token-brannoch").conditions.is_empty());
        assert!(stored(&c, "token-brannoch").condition_timers.is_empty());
    }
}

// ------------------------------------------------------------ creature import

mod import {
    use super::*;
    use crate::campaign::{
        GroupKind, ImportOutcome, MAX_ABILITIES, MAX_CREATURES, MAX_CREATURE_NAME_CHARS,
        MAX_RESOURCES,
    };
    use tt_contracts::v1::{
        effect_step, Ability, AttackStep, DamagePart, EffectStep, FlatStep, Provenance, Resource,
        Warning,
    };

    fn imported(name: &str, external_id: &str) -> Creature {
        Creature {
            creature_id: "client-chosen".into(),
            name: name.into(),
            owner_user_id: "alice".into(),
            hp_current: 7,
            hp_max: 9,
            provenance: Some(Provenance {
                source: "adventurers-anvil".into(),
                external_id: external_id.into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn with_dice(mut c: Creature, damage: &str, healing: &str) -> Creature {
        let step = |s| EffectStep { step: Some(s) };
        c.abilities.push(Ability {
            ability_id: "a".into(),
            name: "a".into(),
            steps: vec![
                step(effect_step::Step::Attack(AttackStep {
                    to_hit_bonus: 3,
                    damage: vec![DamagePart {
                        dice: damage.into(),
                        ..Default::default()
                    }],
                })),
                step(effect_step::Step::Flat(FlatStep {
                    damage: vec![],
                    healing_dice: healing.into(),
                })),
            ],
            ..Default::default()
        });
        c
    }

    fn run_import(
        c: &mut Campaign,
        who: &str,
        creature: Creature,
        ids: &mut SequentialIds,
    ) -> Result<ImportOutcome, Rejection> {
        c.import_creature(who, creature, Vec::new(), 5_000, ids)
    }

    #[test]
    fn a_new_import_mints_an_id_clears_the_owner_and_logs_one_event() {
        let mut c = campaign();
        let mut ids = SequentialIds::new();
        let seq0 = c.seq;
        let n = c.creatures.len();
        let o = run_import(&mut c, "gm", imported("Wolf", "w1"), &mut ids).unwrap();
        assert!(!o.replaced);
        assert_ne!(o.creature.creature_id, "client-chosen");
        assert_eq!(o.creature.creature_id, SequentialIds::new().next_id(5_000));
        assert_eq!(o.creature.owner_user_id, "");
        assert_eq!(c.creatures.len(), n + 1);
        assert_eq!(c.creatures.last(), Some(&o.creature));
        let e = &o.event;
        assert_eq!((e.seq, e.at_ms), (seq0 + 1, 5_000));
        assert_eq!(c.seq, seq0 + 1);
        assert_eq!(c.log.last(), Some(e));
        assert_eq!(e.actor_user_id, "gm");
        assert_eq!(e.client_intent_id, "");
        assert_eq!(e.scene_id, "s1");
        match e.kind.as_ref() {
            Some(event::Kind::CreatureUpserted(u)) => {
                assert_eq!(u.creature.as_ref(), Some(&o.creature))
            }
            other => panic!("expected CreatureUpserted, got {other:?}"),
        }
        let g = c.groups.last().expect("a group");
        assert_eq!(g.kind, GroupKind::NotUndoable);
        assert_eq!((g.first_seq, g.last_seq), (e.seq, e.seq));
        assert_eq!(
            g.changed_creature_ids,
            std::slice::from_ref(&o.creature.creature_id)
        );
    }

    #[test]
    fn warnings_ride_on_the_event() {
        let mut c = campaign();
        let w = Warning {
            code: "kept_as_text".into(),
            message: "multiattack".into(),
        };
        let o = c
            .import_creature(
                "gm",
                imported("Wolf", "w1"),
                vec![w.clone()],
                1,
                &mut SequentialIds::new(),
            )
            .unwrap();
        assert_eq!(o.event.warnings, [w]);
    }

    #[test]
    fn a_re_import_replaces_in_place_and_keeps_the_id_and_token_hp() {
        let mut c = campaign();
        let mut ids = SequentialIds::new();
        let first = run_import(&mut c, "gm", imported("Wolf", "w1"), &mut ids).unwrap();
        let id = first.creature.creature_id.clone();
        c.tokens.push(token("t-wolf", &id, &[], false, 1.0));
        let n = c.creatures.len();
        let mut again = imported("Dire Wolf", "w1");
        again.hp_max = 30;
        again.hp_current = 30;
        let second = run_import(&mut c, "gm", again, &mut ids).unwrap();
        assert!(second.replaced);
        assert_eq!(second.creature.creature_id, id);
        assert_eq!(c.creatures.len(), n);
        let stored: Vec<_> = c.creatures.iter().filter(|x| x.creature_id == id).collect();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].name, "Dire Wolf");
        let t = c.tokens.iter().find(|t| t.token_id == "t-wolf").unwrap();
        assert_eq!((t.hp_current, t.hp_max), (Some(10), Some(12)));
        assert_eq!(c.seq, 2);
    }

    #[test]
    fn a_re_import_keeps_the_existing_owner() {
        let mut c = campaign();
        let mut ids = SequentialIds::new();
        let first = run_import(&mut c, "gm", imported("Wolf", "w1"), &mut ids).unwrap();
        assert_eq!(first.creature.owner_user_id, "");
        let i = c.creatures.len() - 1;
        c.creatures[i].owner_user_id = "bob".into();
        let second = run_import(&mut c, "gm", imported("Wolf", "w1"), &mut ids).unwrap();
        assert!(second.replaced);
        assert_eq!(second.creature.owner_user_id, "bob");
        assert_eq!(c.creatures[i].owner_user_id, "bob");
    }

    #[test]
    fn matching_needs_the_same_source_and_a_non_empty_external_id() {
        let mut c = campaign();
        let mut ids = SequentialIds::new();
        let a = run_import(&mut c, "gm", imported("A", "x"), &mut ids).unwrap();
        let mut other_source = imported("A", "x");
        other_source.provenance.as_mut().unwrap().source = "creature-cauldron".into();
        let b = run_import(&mut c, "gm", other_source, &mut ids).unwrap();
        assert!(!b.replaced);
        assert_ne!(a.creature.creature_id, b.creature.creature_id);
        let e1 = run_import(&mut c, "gm", imported("E", ""), &mut ids).unwrap();
        let e2 = run_import(&mut c, "gm", imported("E", ""), &mut ids).unwrap();
        assert!(!e1.replaced && !e2.replaced);
        assert_ne!(e1.creature.creature_id, e2.creature.creature_id);
    }

    #[test]
    fn invalid_creatures_are_refused_and_change_nothing() {
        let good = imported("Wolf", "w1");
        let mut cases: Vec<(&str, Creature)> = vec![
            ("empty name", imported("  ", "w1")),
            (
                "long name",
                imported(&"n".repeat(MAX_CREATURE_NAME_CHARS + 1), "w1"),
            ),
            ("bad attack dice", with_dice(good.clone(), "2d7", "")),
            ("empty attack dice", with_dice(good.clone(), "", "")),
            ("bad healing dice", with_dice(good.clone(), "1d6", "nope")),
        ];
        let mut c = good.clone();
        c.abilities = vec![Ability::default(); MAX_ABILITIES + 1];
        cases.push(("abilities", c));
        let mut c = good.clone();
        c.resources = vec![Resource::default(); MAX_RESOURCES + 1];
        cases.push(("resources", c));
        let mut c = good.clone();
        c.hp_max = -1;
        c.hp_current = 0;
        cases.push(("negative max", c));
        let mut c = good.clone();
        c.hp_current = 10;
        cases.push(("hp over max", c));
        let mut c = good.clone();
        c.hp_current = -1;
        cases.push(("negative hp", c));
        let mut c = good.clone();
        c.provenance = None;
        cases.push(("no provenance", c));
        let mut c = good;
        c.provenance.as_mut().unwrap().source.clear();
        cases.push(("empty source", c));
        for (what, creature) in cases {
            let mut c = campaign();
            let before = (c.seq, c.creatures.clone(), c.log.len(), c.groups.len());
            let err =
                run_import(&mut c, "gm", creature, &mut SequentialIds::new()).expect_err(what);
            assert_eq!(err.code, "bad_creature", "{what}: {}", err.message);
            assert_eq!(
                (c.seq, c.creatures.clone(), c.log.len(), c.groups.len()),
                before,
                "{what}"
            );
        }
    }

    #[test]
    fn valid_dice_pass_validation() {
        let mut c = campaign();
        let ok = with_dice(imported("Wolf", "w1"), "1d6+2", "2d4+1");
        assert!(run_import(&mut c, "gm", ok, &mut SequentialIds::new()).is_ok());
    }

    #[test]
    fn only_the_gm_may_import() {
        for who in ["alice", "spec", "stranger"] {
            let mut c = campaign();
            let err =
                run_import(&mut c, who, imported("W", "w"), &mut SequentialIds::new()).unwrap_err();
            assert_eq!(err.code, "not_permitted", "{who}");
            assert_eq!(c.seq, 0);
        }
    }

    #[test]
    fn the_cap_refuses_a_new_creature_but_allows_a_re_import() {
        let mut c = campaign();
        let mut ids = SequentialIds::new();
        let first = run_import(&mut c, "gm", imported("Wolf", "w1"), &mut ids).unwrap();
        while c.creatures.len() < MAX_CREATURES {
            let n = c.creatures.len();
            c.creatures.push(creature(&format!("filler-{n}"), ""));
        }
        let seq = c.seq;
        let err = run_import(&mut c, "gm", imported("Bear", "b1"), &mut ids).unwrap_err();
        assert_eq!(err.code, "too_many_creatures");
        assert_eq!((c.seq, c.creatures.len()), (seq, MAX_CREATURES));
        let again = run_import(&mut c, "gm", imported("Wolf 2", "w1"), &mut ids).unwrap();
        assert!(again.replaced);
        assert_eq!(again.creature.creature_id, first.creature.creature_id);
        assert_eq!(c.creatures.len(), MAX_CREATURES);
    }
}

/// Audit fixes (issue #132): target limits, undo edge cases, import checks,
/// the conditions cap.
mod audit_132 {
    use super::*;
    use crate::campaign::{
        validate_import, MAX_ARMOR_CLASS, MAX_CONDITIONS, MAX_DAMAGE_PARTS_PER_STEP,
        MAX_PASSIVE_PERCEPTION, MAX_PROFICIENCY_BONUS, MAX_STEPS_PER_ABILITY, MAX_TARGETS,
        MAX_TEMP_HP,
    };
    use tt_contracts::v1 as pb;

    fn dmg(dice: &str) -> pb::DamagePart {
        pb::DamagePart {
            dice: dice.into(),
            r#type: pb::DamageType::Slashing as i32,
        }
    }

    fn attack_step(dice: &str) -> pb::EffectStep {
        pb::EffectStep {
            step: Some(pb::effect_step::Step::Attack(pb::AttackStep {
                to_hit_bonus: 5,
                damage: vec![dmg(dice)],
            })),
        }
    }

    fn effect_step(id: &str) -> pb::EffectStep {
        pb::EffectStep {
            step: Some(pb::effect_step::Step::ApplyEffect(pb::ApplyEffectStep {
                effect_id: id.into(),
                ..Default::default()
            })),
        }
    }

    fn ability(id: &str, steps: Vec<pb::EffectStep>) -> pb::Ability {
        pb::Ability {
            ability_id: id.into(),
            name: id.into(),
            targeting: Some(pb::Targeting {
                shape: pb::TargetShape::Creatures as i32,
                range_feet: 60.0,
                ..Default::default()
            }),
            steps,
            ..Default::default()
        }
    }

    /// Alice's creature: `sword` (one attack), `flurry` (two attack steps),
    /// `curse` (40 apply-effect steps `fx0` .. `fx39`).
    fn world() -> Campaign {
        let mut c = campaign();
        let alice = c.creatures.iter_mut().find(|x| x.creature_id == "c-alice");
        alice.expect("c-alice").abilities = vec![
            ability("sword", vec![attack_step("1d8+3")]),
            ability("flurry", vec![attack_step("1d8"), attack_step("1d8")]),
            ability(
                "curse",
                (0..40).map(|n| effect_step(&format!("fx{n}"))).collect(),
            ),
        ];
        c
    }

    fn use_ab(ability: &str, targets: &[&str]) -> Intent {
        intent_of(intent::Kind::UseAbility(pb::UseAbilityIntent {
            source_token_id: "t-alice".into(),
            ability_id: ability.into(),
            target_token_ids: targets.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }))
    }

    fn refuse(c: &mut Campaign, actor: &str, i: Intent, expect: &str) {
        let before = state_of(c);
        let groups = c.groups.clone();
        assert_eq!(code(run(c, actor, i)), expect);
        assert_eq!(state_of(c), before, "{expect}: state changed");
        assert_eq!(c.groups, groups, "{expect}: a group changed");
    }

    fn stored<'a>(c: &'a Campaign, id: &str) -> &'a Token {
        c.tokens.iter().find(|t| t.token_id == id).expect("token")
    }

    fn warnings_of(events: &[Event]) -> &[pb::Warning] {
        let Some(event::Kind::ResolutionUpdated(u)) = events[0].kind.as_ref() else {
            panic!("{:?}", events[0].kind);
        };
        &u.resolution.as_ref().expect("resolution").warnings
    }

    // ----------------------------------------------------------- 1: targets

    #[test]
    fn a_repeated_target_is_refused_for_a_single_attack() {
        let mut c = world();
        refuse(
            &mut c,
            "alice",
            use_ab("sword", &["t-goblin", "t-goblin"]),
            "duplicate_target",
        );
        run(&mut c, "alice", use_ab("sword", &["t-goblin"])).unwrap();
        refuse(
            &mut c,
            "alice",
            use_ab("sword", &["t-goblin", "t-alice", "t-goblin"]),
            "duplicate_target",
        );
    }

    #[test]
    fn more_than_max_targets_ids_is_refused_before_any_per_id_work() {
        let mut c = world();
        let many = vec!["t-goblin"; MAX_TARGETS + 1];
        refuse(&mut c, "alice", use_ab("sword", &many), "too_many_targets");
        // The count wins over the unknown-id check.
        let junk: Vec<String> = (0..=MAX_TARGETS).map(|n| format!("nobody-{n}")).collect();
        let junk: Vec<&str> = junk.iter().map(String::as_str).collect();
        refuse(&mut c, "alice", use_ab("flurry", &junk), "too_many_targets");
        // A multiattack with 64 listed ids passes the count (then fails on
        // the repeat, since it has only two attack steps).
        let at_limit = vec!["t-goblin"; MAX_TARGETS];
        refuse(
            &mut c,
            "alice",
            use_ab("flurry", &at_limit),
            "duplicate_target",
        );
    }

    #[test]
    fn a_multiattack_may_repeat_a_target_once_per_attack_step() {
        let mut c = world();
        let events = run(&mut c, "alice", use_ab("flurry", &["t-goblin", "t-goblin"])).unwrap();
        assert!(!events.is_empty());
        refuse(
            &mut c,
            "alice",
            use_ab("flurry", &["t-goblin", "t-goblin", "t-goblin"]),
            "duplicate_target",
        );
    }

    #[test]
    fn the_max_targets_warning_stays_a_warning() {
        let mut c = world();
        let alice = c.creatures.iter_mut().find(|x| x.creature_id == "c-alice");
        let alice = alice.expect("c-alice");
        alice.abilities[0].targeting.as_mut().unwrap().max_targets = 1;
        let events = run(&mut c, "gm", use_ab("sword", &["t-goblin", "t-hidden"])).unwrap();
        let warnings = warnings_of(&events);
        assert!(warnings.iter().any(|w| w.code == "too_many_targets"));
    }

    // ------------------------------------------------- 2 and 3: undo edges

    fn undo(seq: u64) -> Intent {
        intent_of(intent::Kind::Undo(pb::UndoIntent { event_seq: seq }))
    }

    fn start(tokens: &[&str]) -> Intent {
        intent_of(intent::Kind::StartEncounter(pb::StartEncounterIntent {
            scene_id: "s1".into(),
            token_ids: tokens.iter().map(|s| s.to_string()).collect(),
        }))
    }

    fn end(encounter: &str) -> Intent {
        intent_of(intent::Kind::EndEncounter(pb::EndEncounterIntent {
            encounter_id: encounter.into(),
        }))
    }

    fn running(c: &mut Campaign) {
        c.encounters.push(Encounter {
            encounter_id: "enc-1".into(),
            scene_id: "s1".into(),
            round: 1,
            combatants: vec![Combatant {
                token_id: "t-alice".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
    }

    fn timed(c: &mut Campaign, token_id: &str, effect: &str) {
        let t = c.tokens.iter_mut().find(|x| x.token_id == token_id);
        let t = t.expect("token");
        t.conditions.push(effect.into());
        t.condition_timers.push(pb::ConditionTimer {
            effect_id: effect.into(),
            encounter_id: "enc-1".into(),
            source_token_id: "t-alice".into(),
            ends_round: 5,
            at_end_of_turn: false,
        });
    }

    fn clear_blinded() -> Intent {
        intent_of(intent::Kind::SetConditions(pb::SetConditionsIntent {
            token_id: "t-goblin".into(),
            add: vec![],
            remove: vec!["blinded".into()],
        }))
    }

    #[test]
    fn undoing_a_create_that_is_now_a_combatant_is_refused() {
        let mut c = world();
        let made = run(
            &mut c,
            "gm",
            intent_of(intent::Kind::CreateToken(pb::CreateTokenIntent {
                scene_id: "s1".into(),
                creature_id: "c-goblin".into(),
                position: Some(p(35.0, 35.0)),
                ..Default::default()
            })),
        )
        .unwrap();
        let Some(event::Kind::TokenUpserted(u)) = made[0].kind.as_ref() else {
            panic!("{:?}", made[0].kind);
        };
        let new_id = u.token.as_ref().unwrap().token_id.clone();
        run(&mut c, "gm", start(&[&new_id, "t-alice"])).unwrap();
        let encounters = c.encounters.clone();
        refuse(&mut c, "gm", undo(made[0].seq), "in_encounter");
        assert_eq!(c.encounters, encounters);
        assert!(c.tokens.iter().any(|t| t.token_id == new_id));
        // Once the encounter is over the undo works.
        let encounter_id = c.encounters[0].encounter_id.clone();
        run(&mut c, "gm", end(&encounter_id)).unwrap();
        run(&mut c, "gm", undo(made[0].seq)).unwrap();
        assert!(c.tokens.iter().all(|t| t.token_id != new_id));
    }

    #[test]
    fn undoing_a_remove_after_the_encounter_ended_drops_the_orphan_timer() {
        let mut c = world();
        running(&mut c);
        timed(&mut c, "t-goblin", "blinded");
        let gone = run(
            &mut c,
            "gm",
            intent_of(intent::Kind::RemoveToken(pb::RemoveTokenIntent {
                token_id: "t-goblin".into(),
            })),
        )
        .unwrap();
        run(&mut c, "gm", end("enc-1")).unwrap();
        let back = run(&mut c, "gm", undo(gone[0].seq)).unwrap();
        let goblin = stored(&c, "t-goblin");
        assert_eq!(goblin.conditions, ["blinded"]);
        assert!(goblin.condition_timers.is_empty());
        // The event the clients get shows the same token.
        let Some(event::Kind::TokenUpserted(u)) = back[0].kind.as_ref() else {
            panic!("{:?}", back[0].kind);
        };
        assert_eq!(u.token.as_ref(), Some(goblin));
    }

    #[test]
    fn undoing_set_conditions_after_the_encounter_ended_drops_the_orphan_timer() {
        let mut c = world();
        running(&mut c);
        timed(&mut c, "t-goblin", "blinded");
        let cleared = run(&mut c, "gm", clear_blinded()).unwrap();
        assert!(stored(&c, "t-goblin").conditions.is_empty());
        run(&mut c, "gm", end("enc-1")).unwrap();
        let back = run(&mut c, "gm", undo(cleared[0].seq)).unwrap();
        let goblin = stored(&c, "t-goblin");
        assert_eq!(goblin.conditions, ["blinded"]);
        assert!(goblin.condition_timers.is_empty());
        let Some(event::Kind::TokenUpserted(u)) = back[0].kind.as_ref() else {
            panic!("{:?}", back[0].kind);
        };
        assert_eq!(u.token.as_ref(), Some(goblin));
    }

    #[test]
    fn undo_keeps_a_timer_whose_encounter_is_still_running() {
        let mut c = world();
        running(&mut c);
        timed(&mut c, "t-goblin", "blinded");
        let cleared = run(&mut c, "gm", clear_blinded()).unwrap();
        run(&mut c, "gm", undo(cleared[0].seq)).unwrap();
        assert_eq!(stored(&c, "t-goblin").condition_timers.len(), 1);
    }

    // -------------------------------------------------- 4: validate_import

    fn good() -> Creature {
        Creature {
            name: "Wolf".into(),
            hp_current: 7,
            hp_max: 9,
            armor_class: 13,
            proficiency_bonus: 2,
            passive_perception: 13,
            speed_feet: 40.0,
            scores: Some(pb::AbilityScores {
                strength: 12,
                dexterity: 15,
                constitution: 12,
                intelligence: 3,
                wisdom: 12,
                charisma: 6,
            }),
            resources: vec![pb::Resource {
                key: "uses".into(),
                label: "uses".into(),
                current: 1,
                max: 2,
            }],
            abilities: vec![ability("bite", vec![attack_step("2d6+3")])],
            conditions: vec!["prone".into()],
            condition_immunities: vec!["poisoned".into()],
            provenance: Some(pb::Provenance {
                source: "adventurers-anvil".into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn hex() -> String {
        "ab".repeat(32)
    }

    fn asset_ref(url: &str, asset_id: &str) -> pb::AssetRef {
        pb::AssetRef {
            asset_id: asset_id.into(),
            url: url.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_complete_creature_passes_validation() {
        assert!(validate_import(&good()).is_ok());
        let mut c = good();
        c.portrait = Some(asset_ref(
            &format!("https://api.example.test/assets/{}.png", hex()),
            &hex(),
        ));
        c.passive_perception = 0;
        assert!(validate_import(&c).is_ok());
        c.portrait = Some(asset_ref(&format!("/assets/{}.webp", hex()), ""));
        assert!(validate_import(&c).is_ok());
        c.portrait = Some(pb::AssetRef::default());
        assert!(validate_import(&c).is_ok());
    }

    #[test]
    fn every_import_check_refuses_with_bad_creature_naming_the_field() {
        let edit = |f: &dyn Fn(&mut Creature)| {
            let mut c = good();
            f(&mut c);
            c
        };
        fn tg(c: &mut Creature) -> &mut pb::Targeting {
            c.abilities[0].targeting.as_mut().unwrap()
        }
        let many_ids = || {
            (0..=MAX_CONDITIONS)
                .map(|n| format!("c{n}"))
                .collect::<Vec<_>>()
        };
        let parts = |n: usize| {
            vec![pb::EffectStep {
                step: Some(pb::effect_step::Step::Flat(pb::FlatStep {
                    damage: (0..n).map(|_| dmg("1d4")).collect(),
                    healing_dice: String::new(),
                })),
            }]
        };
        let portrait = |url: &str, id: &str| {
            let r = asset_ref(url, id);
            edit(&move |c| c.portrait = Some(r.clone()))
        };
        // (text the message must contain, what is wrong, the creature)
        let cases: Vec<(&str, &str, Creature)> = vec![
            ("speed_feet", "NaN", edit(&|c| c.speed_feet = f32::NAN)),
            ("speed_feet", "inf", edit(&|c| c.speed_feet = f32::INFINITY)),
            ("speed_feet", "negative", edit(&|c| c.speed_feet = -5.0)),
            (
                "fly_speed_feet",
                "NaN",
                edit(&|c| c.fly_speed_feet = f32::NAN),
            ),
            (
                "fly_speed_feet",
                "negative",
                edit(&|c| c.fly_speed_feet = -1.0),
            ),
            (
                "swim_speed_feet",
                "-inf",
                edit(&|c| c.swim_speed_feet = f32::NEG_INFINITY),
            ),
            (
                "climb_speed_feet",
                "huge",
                edit(&|c| c.climb_speed_feet = 1e9),
            ),
            (
                "burrow_speed_feet",
                "NaN",
                edit(&|c| c.burrow_speed_feet = f32::NAN),
            ),
            ("range_feet", "NaN", edit(&|c| tg(c).range_feet = f32::NAN)),
            (
                "long_range_feet",
                "negative",
                edit(&|c| tg(c).long_range_feet = -1.0),
            ),
            (
                "size_feet",
                "inf",
                edit(&|c| tg(c).size_feet = f32::INFINITY),
            ),
            ("width_feet", "negative", edit(&|c| tg(c).width_feet = -1.0)),
            ("armor_class", "negative", edit(&|c| c.armor_class = -50)),
            (
                "armor_class",
                "too high",
                edit(&|c| c.armor_class = MAX_ARMOR_CLASS + 1),
            ),
            (
                "strength",
                "zero",
                edit(&|c| c.scores.as_mut().unwrap().strength = 0),
            ),
            (
                "dexterity",
                "31",
                edit(&|c| c.scores.as_mut().unwrap().dexterity = 31),
            ),
            (
                "constitution",
                "min",
                edit(&|c| c.scores.as_mut().unwrap().constitution = i32::MIN),
            ),
            (
                "intelligence",
                "max",
                edit(&|c| c.scores.as_mut().unwrap().intelligence = i32::MAX),
            ),
            (
                "wisdom",
                "negative",
                edit(&|c| c.scores.as_mut().unwrap().wisdom = -3),
            ),
            (
                "charisma",
                "zero",
                edit(&|c| c.scores.as_mut().unwrap().charisma = 0),
            ),
            (
                "proficiency_bonus",
                "negative",
                edit(&|c| c.proficiency_bonus = -1),
            ),
            (
                "proficiency_bonus",
                "max",
                edit(&|c| c.proficiency_bonus = i32::MAX),
            ),
            (
                "proficiency_bonus",
                "too high",
                edit(&|c| c.proficiency_bonus = MAX_PROFICIENCY_BONUS + 1),
            ),
            (
                "passive_perception",
                "negative",
                edit(&|c| c.passive_perception = -1),
            ),
            (
                "passive_perception",
                "too high",
                edit(&|c| c.passive_perception = MAX_PASSIVE_PERCEPTION + 1),
            ),
            ("hp_temp", "max", edit(&|c| c.hp_temp = i32::MAX)),
            (
                "hp_temp",
                "over cap",
                edit(&|c| c.hp_temp = MAX_TEMP_HP + 1),
            ),
            ("hp_temp", "negative", edit(&|c| c.hp_temp = -1)),
            (
                "min_roll",
                "1",
                edit(&|c| c.abilities[0].recharge = Some(pb::Recharge { min_roll: 1 })),
            ),
            (
                "min_roll",
                "7",
                edit(&|c| c.abilities[0].recharge = Some(pb::Recharge { min_roll: 7 })),
            ),
            (
                "min_roll",
                "0",
                edit(&|c| c.abilities[0].recharge = Some(pb::Recharge { min_roll: 0 })),
            ),
            (
                "ability_id",
                "empty",
                edit(&|c| c.abilities[0].ability_id.clear()),
            ),
            (
                "ability_id",
                "duplicate",
                edit(&|c| {
                    let again = c.abilities[0].clone();
                    c.abilities.push(again);
                }),
            ),
            (
                "resource",
                "empty key",
                edit(&|c| c.resources[0].key.clear()),
            ),
            (
                "resource",
                "duplicate key",
                edit(&|c| {
                    let again = c.resources[0].clone();
                    c.resources.push(again);
                }),
            ),
            (
                "resource",
                "current over max",
                edit(&|c| c.resources[0].current = 3),
            ),
            (
                "resource",
                "negative current",
                edit(&|c| c.resources[0].current = -1),
            ),
            (
                "resource",
                "negative max",
                edit(&|c| {
                    c.resources[0].current = 0;
                    c.resources[0].max = -1
                }),
            ),
            (
                "conditions",
                "not an id",
                edit(&|c| c.conditions = vec!["Blinded".into()]),
            ),
            (
                "conditions",
                "empty id",
                edit(&|c| c.conditions = vec![String::new()]),
            ),
            (
                "conditions",
                "too many",
                edit(&|c| c.conditions = many_ids()),
            ),
            (
                "condition_immunities",
                "not an id",
                edit(&|c| c.condition_immunities = vec!["has space".into()]),
            ),
            (
                "condition_immunities",
                "too many",
                edit(&|c| c.condition_immunities = many_ids()),
            ),
            (
                "effect_id",
                "capital",
                edit(&|c| c.abilities[0].steps.push(effect_step("Blinded"))),
            ),
            (
                "effect_id",
                "too long",
                edit(&|c| c.abilities[0].steps.push(effect_step(&"a".repeat(200)))),
            ),
            (
                "effect_id",
                "empty",
                edit(&|c| c.abilities[0].steps.push(effect_step(""))),
            ),
            (
                "steps",
                "too many",
                edit(&|c| {
                    c.abilities[0].steps = (0..=MAX_STEPS_PER_ABILITY)
                        .map(|_| attack_step("1d4"))
                        .collect()
                }),
            ),
            (
                "damage parts",
                "too many",
                edit(&|c| c.abilities[0].steps = parts(MAX_DAMAGE_PARTS_PER_STEP + 1)),
            ),
            (
                "doubled",
                "100d100",
                edit(&|c| c.abilities[0].steps = vec![attack_step("100d100")]),
            ),
            (
                "doubled",
                "51d6",
                edit(&|c| c.abilities[0].steps = vec![attack_step("51d6")]),
            ),
            (
                "portrait",
                "javascript",
                portrait("javascript:alert(1)", ""),
            ),
            (
                "portrait",
                "data url",
                portrait("data:image/png;base64,AAAA", ""),
            ),
            (
                "portrait",
                "foreign path",
                portrait("https://evil.example/x.png", ""),
            ),
            (
                "portrait",
                "short hex",
                portrait("https://h.test/assets/abc.png", ""),
            ),
            (
                "portrait",
                "bad extension",
                portrait(&format!("https://h.test/assets/{}.svg", hex()), ""),
            ),
            (
                "portrait",
                "id mismatch",
                portrait(
                    &format!("https://h.test/assets/{}.png", hex()),
                    &"cd".repeat(32),
                ),
            ),
            ("portrait", "id without url", portrait("", &hex())),
        ];
        for (field, what, creature) in cases {
            let err = validate_import(&creature).expect_err(what);
            assert_eq!(err.code, "bad_creature", "{field} / {what}");
            assert!(
                err.message.contains(field),
                "{field} / {what}: message {:?} does not name the field",
                err.message
            );
        }
    }

    #[test]
    fn a_critical_safe_attack_and_big_non_attack_dice_pass() {
        // 50d6 doubles to exactly 100 dice; a flat step is never doubled.
        let mut c = good();
        c.abilities[0].steps = vec![attack_step("50d6")];
        assert!(validate_import(&c).is_ok());
        c.abilities[0].steps = vec![pb::EffectStep {
            step: Some(pb::effect_step::Step::Flat(pb::FlatStep {
                damage: vec![dmg("100d6")],
                healing_dice: String::new(),
            })),
        }];
        assert!(validate_import(&c).is_ok());
    }

    #[test]
    fn a_bad_import_leaves_the_campaign_alone() {
        let mut c = world();
        let before = (c.seq, c.creatures.clone(), c.log.len(), c.groups.len());
        let mut creature = good();
        creature.abilities[0].steps.push(effect_step("Blinded"));
        let err = c
            .import_creature("gm", creature, Vec::new(), 5_000, &mut SequentialIds::new())
            .expect_err("bad effect id");
        assert_eq!(err.code, "bad_creature");
        assert_eq!(
            (c.seq, c.creatures.clone(), c.log.len(), c.groups.len()),
            before
        );
    }

    // ------------------------------------------------- 5: conditions cap

    #[test]
    fn forty_conditions_from_one_ability_keep_32_and_warn() {
        let mut c = world();
        let events = run(&mut c, "alice", use_ab("curse", &["t-goblin"])).unwrap();
        let capped = warnings_of(&events)
            .iter()
            .filter(|w| w.code == "too_many_conditions")
            .count();
        assert_eq!(capped, 1);
        assert_eq!(stored(&c, "t-goblin").conditions.len(), MAX_CONDITIONS);
    }

    // ----------------------------------------- 6: set_conditions lengths

    #[test]
    fn set_conditions_checks_the_list_lengths_first() {
        let mut c = world();
        let ids = |n: usize| (0..n).map(|i| format!("c{i}")).collect::<Vec<_>>();
        let set = |token: &str, add: Vec<String>, remove: Vec<String>| {
            intent_of(intent::Kind::SetConditions(pb::SetConditionsIntent {
                token_id: token.into(),
                add,
                remove,
            }))
        };
        refuse(
            &mut c,
            "gm",
            set("t-goblin", ids(100), vec![]),
            "too_many_conditions",
        );
        // Before the per-entry checks: junk ids and a missing token are not
        // looked at once the lists are too long.
        refuse(
            &mut c,
            "gm",
            set(
                "t-nobody",
                vec!["Bad Id".into(); 2 * MAX_CONDITIONS],
                vec!["x".into()],
            ),
            "too_many_conditions",
        );
        refuse(
            &mut c,
            "gm",
            set("t-goblin", ids(40), ids(2 * MAX_CONDITIONS)),
            "too_many_conditions",
        );
        // A player still gets not_permitted first.
        refuse(
            &mut c,
            "alice",
            set("t-goblin", ids(100), vec![]),
            "not_permitted",
        );
        // At the limit the old checks decide: 32 adds fit.
        run(&mut c, "gm", set("t-goblin", ids(MAX_CONDITIONS), vec![])).unwrap();
    }
}
