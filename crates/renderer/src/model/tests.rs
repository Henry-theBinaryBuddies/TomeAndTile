use tt_contracts::v1::{
    client_message, event, intent, server_message, AssetRef, ClientMessage, CreatureSize, Event,
    Grid, GridKind, IntentRejected, Level, Role, Scene, SceneUpserted, ServerMessage, Snapshot,
    Token, TokenMoved, TokenRemoved, TokenUpserted, Vec2,
};

use super::*;
use crate::camera::{Camera, MAX_ZOOM, MIN_ZOOM};
use crate::draw::{color_for_id, DrawItem};
use crate::wire::encode_hello;

// ------------------------------------------------------------------ builders

fn v2(x: f32, y: f32) -> Option<Vec2> {
    Some(Vec2 { x, y })
}

fn scene(id: &str, cell: f32) -> Scene {
    Scene {
        scene_id: id.into(),
        campaign_id: "c".into(),
        name: "s".into(),
        grid: Some(Grid {
            kind: GridKind::Square as i32,
            cell_size: cell,
            ..Default::default()
        }),
        levels: vec![Level {
            level_id: "L1".into(),
            ..Default::default()
        }],
        walls: vec![],
    }
}

fn token(id: &str, x: f32, y: f32, size: CreatureSize, controllers: &[&str]) -> Token {
    Token {
        token_id: id.into(),
        scene_id: "S".into(),
        level_id: "L1".into(),
        position: v2(x, y),
        size: size as i32,
        controller_user_ids: controllers.iter().map(|s| (*s).to_owned()).collect(),
        ..Default::default()
    }
}

fn snapshot(user: &str, role: Role, tokens: Vec<Token>) -> Snapshot {
    Snapshot {
        seq: 5,
        user_id: user.into(),
        role: role as i32,
        scenes: vec![scene("S", 70.0)],
        tokens,
        active_scene_id: "S".into(),
        ..Default::default()
    }
}

fn model_as(user: &str, role: Role, tokens: Vec<Token>) -> Model {
    let mut m = Model::new(0xabc);
    m.apply_snapshot(snapshot(user, role, tokens));
    m.resize(800.0, 600.0);
    // Make screen == world for easy arithmetic in the tests.
    m.camera = Camera {
        origin: Pt::new(0.0, 0.0),
        zoom: 1.0,
        width: 800.0,
        height: 600.0,
    };
    m.needs_fit = false;
    m
}

fn event_of(kind: event::Kind) -> Event {
    Event {
        seq: 6,
        kind: Some(kind),
        ..Default::default()
    }
}

fn moved(token_id: &str, x: f32, y: f32) -> Event {
    event_of(event::Kind::TokenMoved(TokenMoved {
        token_id: token_id.into(),
        path: vec![Vec2 { x, y }],
        ..Default::default()
    }))
}

fn server_bytes(kind: server_message::Kind) -> Vec<u8> {
    ServerMessage { kind: Some(kind) }.encode_to_vec()
}

fn decode_move(bytes: &[u8]) -> (String, tt_contracts::v1::MoveTokenIntent) {
    let msg = ClientMessage::decode(bytes).expect("decodes");
    let Some(client_message::Kind::Intent(i)) = msg.kind else {
        panic!("not an intent");
    };
    let Some(intent::Kind::MoveToken(m)) = i.kind else {
        panic!("not a move");
    };
    (i.client_intent_id, m)
}

fn pos(m: &Model, id: &str) -> (f32, f32) {
    let p = m.token(id).unwrap().position.unwrap();
    (p.x, p.y)
}

/// Drag a token from screen `from` to `to` and release at `now`.
fn drag(m: &mut Model, from: (f64, f64), to: (f64, f64), now: f64) -> Option<Vec<u8>> {
    m.pointer_down(from.0, from.1, 0);
    m.pointer_move((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
    m.pointer_move(to.0, to.1);
    m.pointer_up(to.0, to.1, now)
}

// ------------------------------------------------------- snapshot and events

#[test]
fn snapshot_builds_state() {
    let m = model_as(
        "u1",
        Role::Player,
        vec![token("a", 105.0, 105.0, CreatureSize::Medium, &["u1"])],
    );
    assert_eq!(m.user_id(), "u1");
    assert_eq!(m.token_count(), 1);
    assert_eq!(m.active_scene().unwrap().scene_id, "S");
    assert_eq!(m.current_level().unwrap().level_id, "L1");
    assert_eq!(m.last_seq(), 5);
    assert_eq!(m.grid().cell, 70.0);
}

#[test]
fn snapshot_replaces_previous_state() {
    let mut m = model_as(
        "u1",
        Role::Gm,
        vec![token("a", 0.0, 0.0, CreatureSize::Medium, &[])],
    );
    m.apply_snapshot(snapshot(
        "u1",
        Role::Gm,
        vec![token("b", 0.0, 0.0, CreatureSize::Medium, &[])],
    ));
    assert!(m.token("a").is_none());
    assert!(m.token("b").is_some());
}

#[test]
fn token_upsert_inserts_then_replaces() {
    let mut m = model_as("u", Role::Gm, vec![]);
    let up = |x| {
        event_of(event::Kind::TokenUpserted(TokenUpserted {
            token: Some(token("t", x, 0.0, CreatureSize::Medium, &[])),
        }))
    };
    m.apply_event(up(10.0));
    m.apply_event(up(20.0));
    assert_eq!(m.token_count(), 1);
    assert_eq!(pos(&m, "t"), (20.0, 0.0));
}

#[test]
fn token_removed_and_moved_apply() {
    let mut m = model_as(
        "u",
        Role::Gm,
        vec![
            token("a", 0.0, 0.0, CreatureSize::Medium, &[]),
            token("b", 0.0, 0.0, CreatureSize::Medium, &[]),
        ],
    );
    m.apply_event(moved("a", 300.0, 200.0));
    assert_eq!(pos(&m, "a"), (300.0, 200.0));
    m.apply_event(event_of(event::Kind::TokenRemoved(TokenRemoved {
        token_id: "b".into(),
    })));
    assert!(m.token("b").is_none());
    assert_eq!(m.token_count(), 1);
}

#[test]
fn token_moved_uses_last_waypoint_and_level() {
    let mut m = model_as(
        "u",
        Role::Gm,
        vec![token("a", 0.0, 0.0, CreatureSize::Medium, &[])],
    );
    m.apply_event(event_of(event::Kind::TokenMoved(TokenMoved {
        token_id: "a".into(),
        path: vec![Vec2 { x: 1.0, y: 1.0 }, Vec2 { x: 5.0, y: 6.0 }],
        level_id: "L2".into(),
        distance_feet: 5.0,
    })));
    assert_eq!(pos(&m, "a"), (5.0, 6.0));
    assert_eq!(m.token("a").unwrap().level_id, "L2");
}

#[test]
fn scene_upserted_adds_and_updates() {
    let mut m = model_as("u", Role::Gm, vec![]);
    let up = |cell| {
        event_of(event::Kind::SceneUpserted(SceneUpserted {
            scene: Some(scene("S", cell)),
        }))
    };
    m.apply_event(up(100.0));
    assert_eq!(m.grid().cell, 100.0);
    m.apply_event(event_of(event::Kind::SceneUpserted(SceneUpserted {
        scene: Some(scene("S2", 50.0)),
    })));
    assert_eq!(m.active_scene().unwrap().scene_id, "S");
}

#[test]
fn unrelated_events_are_ignored() {
    let mut m = model_as(
        "u",
        Role::Gm,
        vec![token("a", 1.0, 2.0, CreatureSize::Medium, &[])],
    );
    m.apply_event(event_of(event::Kind::ChatPosted(
        tt_contracts::v1::ChatPosted::default(),
    )));
    m.apply_event(Event::default());
    assert_eq!(pos(&m, "a"), (1.0, 2.0));
    assert_eq!(m.last_seq(), 6);
}

// ----------------------------------------------------------- hostile input

#[test]
fn events_for_unknown_tokens_are_harmless() {
    let mut m = model_as("u", Role::Gm, vec![]);
    m.apply_event(moved("ghost", 1.0, 1.0));
    m.apply_event(event_of(event::Kind::TokenRemoved(TokenRemoved {
        token_id: "ghost".into(),
    })));
    m.apply_event(event_of(event::Kind::TokenMoved(TokenMoved {
        token_id: "ghost".into(),
        ..Default::default()
    })));
    assert_eq!(m.token_count(), 0);
}

#[test]
fn non_finite_numbers_are_rejected() {
    let mut m = model_as(
        "u",
        Role::Gm,
        vec![token("a", 7.0, 8.0, CreatureSize::Medium, &[])],
    );
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        m.apply_event(moved("a", bad, 1.0));
        m.apply_event(moved("a", 1.0, bad));
        m.apply_event(event_of(event::Kind::TokenUpserted(TokenUpserted {
            token: Some(token("a", bad, 0.0, CreatureSize::Medium, &[])),
        })));
        m.apply_event(event_of(event::Kind::TokenUpserted(TokenUpserted {
            token: Some(token("new", 0.0, bad, CreatureSize::Medium, &[])),
        })));
    }
    assert_eq!(pos(&m, "a"), (7.0, 8.0));
    assert!(m.token("new").is_none());
    let mut s = snapshot(
        "u",
        Role::Gm,
        vec![token("n", f32::NAN, 0.0, CreatureSize::Medium, &[])],
    );
    s.scenes[0].grid.as_mut().unwrap().cell_size = f32::NAN;
    m.apply_snapshot(s);
    assert_eq!(m.token_count(), 0);
    assert_eq!(m.grid().cell, DEFAULT_CELL);
    let _ = m.draw_list();
}

#[test]
fn huge_coordinates_are_clamped() {
    let mut m = model_as(
        "u",
        Role::Gm,
        vec![token("a", 0.0, 0.0, CreatureSize::Medium, &[])],
    );
    m.apply_event(moved("a", 3.0e38, -3.0e38));
    let (x, y) = pos(&m, "a");
    assert!(
        f64::from(x) <= crate::camera::COORD_LIMIT && f64::from(y) >= -crate::camera::COORD_LIMIT
    );
    let _ = m.draw_list();
}

#[test]
fn bad_grid_sizes_fall_back() {
    for bad in [0.0_f32, -5.0, 0.5, f32::INFINITY, 1.0e9] {
        let mut m = model_as("u", Role::Gm, vec![]);
        m.apply_event(event_of(event::Kind::SceneUpserted(SceneUpserted {
            scene: Some(scene("S", bad)),
        })));
        assert_eq!(m.grid().cell, DEFAULT_CELL, "cell {bad}");
        assert!(m.draw_list().items.len() <= MAX_GRID_LINES + 8);
    }
}

#[test]
fn tokens_without_ids_and_scenes_without_ids_are_dropped() {
    let mut s = snapshot(
        "u",
        Role::Gm,
        vec![token("", 0.0, 0.0, CreatureSize::Medium, &[])],
    );
    s.scenes.push(Scene::default());
    let mut m = Model::new(1);
    m.apply_snapshot(s);
    assert_eq!(m.token_count(), 0);
    m.apply_event(event_of(event::Kind::SceneUpserted(SceneUpserted {
        scene: Some(Scene::default()),
    })));
    assert_eq!(m.active_scene().unwrap().scene_id, "S");
}

#[test]
fn no_scene_draws_nothing_and_never_panics() {
    let mut m = Model::new(1);
    m.resize(100.0, 100.0);
    assert!(m.draw_list().items.is_empty());
    m.pointer_down(1.0, 1.0, 0);
    m.pointer_move(2.0, 2.0);
    assert!(m.pointer_up(2.0, 2.0, 0.0).is_none());
    m.wheel(1.0, 1.0, 10.0);
    assert!(m.take_image_requests().is_empty());
}

#[test]
fn garbage_bytes_are_errors_or_ignored_never_panics() {
    let mut m = model_as(
        "u",
        Role::Gm,
        vec![token("a", 0.0, 0.0, CreatureSize::Medium, &[])],
    );
    // Deterministic pseudo-random garbage (xorshift), no rand crate.
    let mut s: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for len in 0..300 {
        let bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        let _ = m.apply_server_message(&bytes);
    }
    // Truncations of a valid message.
    let valid = server_bytes(server_message::Kind::Snapshot(snapshot(
        "u",
        Role::Gm,
        vec![token("z", 1.0, 1.0, CreatureSize::Large, &[])],
    )));
    for cut in 0..valid.len() {
        let _ = m.apply_server_message(&valid[..cut]);
    }
    assert!(m.apply_server_message(&[0xff, 0xff, 0xff]).is_err());
    let _ = m.draw_list();
}

#[test]
fn bad_bytes_leave_model_untouched() {
    let mut m = model_as(
        "u",
        Role::Gm,
        vec![token("a", 9.0, 9.0, CreatureSize::Medium, &[])],
    );
    assert!(m.apply_server_message(&[0x0a, 0xff]).is_err());
    assert_eq!(m.token_count(), 1);
    assert_eq!(pos(&m, "a"), (9.0, 9.0));
}

#[test]
fn server_message_dispatch() {
    let mut m = Model::new(1);
    let snap = server_bytes(server_message::Kind::Snapshot(snapshot(
        "u",
        Role::Gm,
        vec![token("a", 0.0, 0.0, CreatureSize::Medium, &[])],
    )));
    assert_eq!(m.apply_server_message(&snap), Ok(Applied::Snapshot));
    let ev = server_bytes(server_message::Kind::Event(moved("a", 70.0, 70.0)));
    assert_eq!(m.apply_server_message(&ev), Ok(Applied::Event));
    assert_eq!(pos(&m, "a"), (70.0, 70.0));
    let rej = server_bytes(server_message::Kind::IntentRejected(IntentRejected {
        client_intent_id: "x".into(),
        code: "not_permitted".into(),
        message: "no".into(),
    }));
    // "x" is not one of this renderer's moves, so it is remembered but not reported.
    assert_eq!(m.apply_server_message(&rej), Ok(Applied::Ignored));
    assert_eq!(m.last_rejection().unwrap().client_intent_id, "x");
    let media = server_bytes(server_message::Kind::MediaToken(Default::default()));
    assert_eq!(m.apply_server_message(&media), Ok(Applied::Ignored));
    assert_eq!(m.apply_server_message(&[]), Ok(Applied::Ignored));
}

// ------------------------------------------------------------------- camera

#[test]
fn camera_round_trip() {
    let mut cam = Camera::default();
    cam.set_viewport(800.0, 600.0);
    cam.origin = Pt::new(-123.5, 456.25);
    cam.zoom = 0.37;
    for (x, y) in [(0.0, 0.0), (10.0, 20.0), (-5000.0, 9000.0), (799.0, 599.0)] {
        let s = cam.world_to_screen(Pt::new(x, y));
        let w = cam.screen_to_world(s);
        assert!((w.x - x).abs() < 1e-9 && (w.y - y).abs() < 1e-9);
        let back = cam.world_to_screen(cam.screen_to_world(Pt::new(x, y)));
        assert!((back.x - x).abs() < 1e-9 && (back.y - y).abs() < 1e-9);
    }
}

#[test]
fn zoom_about_keeps_the_point_fixed() {
    let mut cam = Camera::default();
    cam.set_viewport(800.0, 600.0);
    cam.origin = Pt::new(100.0, 50.0);
    let anchor = Pt::new(300.0, 200.0);
    let before = cam.screen_to_world(anchor);
    for f in [1.5, 0.5, 2.0, 0.9, 1.1] {
        cam.zoom_about(anchor, f);
        let after = cam.screen_to_world(anchor);
        assert!((after.x - before.x).abs() < 1e-6 && (after.y - before.y).abs() < 1e-6);
    }
}

#[test]
fn zoom_is_clamped() {
    let mut cam = Camera::default();
    cam.set_viewport(800.0, 600.0);
    for _ in 0..200 {
        cam.zoom_about(Pt::new(10.0, 10.0), 2.0);
    }
    assert_eq!(cam.zoom, MAX_ZOOM);
    for _ in 0..400 {
        cam.zoom_about(Pt::new(10.0, 10.0), 0.5);
    }
    assert_eq!(cam.zoom, MIN_ZOOM);
}

#[test]
fn camera_ignores_non_finite_input() {
    let mut cam = Camera::default();
    cam.set_viewport(800.0, 600.0);
    let before = cam;
    cam.zoom_about(Pt::new(f64::NAN, 0.0), 2.0);
    cam.zoom_about(Pt::new(0.0, 0.0), f64::INFINITY);
    cam.zoom_about(Pt::new(0.0, 0.0), -1.0);
    cam.pan_by_screen(f64::NAN, 1.0);
    cam.set_viewport(f64::NAN, 1.0);
    cam.fit(Pt::new(f64::NAN, 0.0), Pt::new(1.0, 1.0), 0.0);
    assert_eq!(cam, before);
}

#[test]
fn pan_moves_the_world_with_the_pointer() {
    let mut m = model_as("u", Role::Player, vec![]);
    m.pointer_down(100.0, 100.0, 0);
    m.pointer_move(150.0, 130.0);
    m.pointer_up(150.0, 130.0, 0.0);
    // Dragging right/down shows more of the top-left: origin moves up-left.
    assert_eq!(m.camera().origin, Pt::new(-50.0, -30.0));
}

#[test]
fn wheel_zooms_about_the_pointer() {
    let mut m = model_as("u", Role::Player, vec![]);
    let anchor = Pt::new(400.0, 300.0);
    let w = m.camera().screen_to_world(anchor);
    m.wheel(anchor.x, anchor.y, -200.0);
    assert!(m.camera().zoom > 1.0);
    let w2 = m.camera().screen_to_world(anchor);
    assert!((w.x - w2.x).abs() < 1e-6 && (w.y - w2.y).abs() < 1e-6);
    m.wheel(f64::NAN, 0.0, 1.0);
    m.wheel(0.0, 0.0, f64::NAN);
}

#[test]
fn initial_fit_frames_the_content_once_sized() {
    let mut m = Model::new(1);
    m.apply_snapshot(snapshot(
        "u",
        Role::Gm,
        vec![token("a", 105.0, 105.0, CreatureSize::Medium, &[])],
    ));
    // No viewport yet: nothing happens, and nothing panics.
    assert_eq!(m.camera().zoom, 1.0);
    m.resize(1000.0, 600.0);
    let (min, max) = m.camera().visible_world();
    assert!(min.x <= 0.0 && min.y <= 0.0 && max.x >= 840.0 && max.y >= 560.0);
    // After the user zooms, a later resize does not re-fit.
    m.wheel(0.0, 0.0, -300.0);
    let z = m.camera().zoom;
    m.resize(900.0, 500.0);
    assert_eq!(m.camera().zoom, z);
}

#[test]
fn fit_happens_when_the_snapshot_arrives_after_the_resize() {
    // The order in the web page: the host sizes the canvas first, the server's
    // Snapshot arrives later. The scene must be framed on arrival, not at the next resize.
    let mut m = Model::new(1);
    m.resize(1000.0, 600.0);
    assert_eq!(m.camera().zoom, 1.0);
    m.apply_snapshot(snapshot(
        "u",
        Role::Gm,
        vec![token("a", 105.0, 105.0, CreatureSize::Medium, &[])],
    ));
    let (min, max) = m.camera().visible_world();
    assert!(min.x <= 0.0 && min.y <= 0.0 && max.x >= 840.0 && max.y >= 560.0);
    assert_ne!(m.camera().zoom, 1.0);
    // A reconnect's Snapshot for the same scene does not move a camera the user placed.
    m.wheel(0.0, 0.0, -300.0);
    let (z, o) = (m.camera().zoom, m.camera().origin);
    m.apply_snapshot(snapshot("u", Role::Gm, vec![]));
    assert_eq!((m.camera().zoom, m.camera().origin), (z, o));
}

fn snapshot_with_unsized_map() -> Snapshot {
    let mut s = snapshot("u", Role::Gm, vec![]);
    s.scenes[0].levels[0].background = Some(AssetRef {
        url: "https://x/map.png".into(),
        ..Default::default()
    });
    s
}

#[test]
fn fit_waits_for_an_unsized_map_image_then_frames_the_map() {
    let mut m = Model::new(1);
    m.resize(1000.0, 500.0);
    m.apply_snapshot(snapshot_with_unsized_map());
    // The map's size is unknown until its image loads: do not frame the tokens instead.
    assert_eq!(m.camera().zoom, 1.0);
    m.mark_image_loaded("https://x/map.png", 2000, 1000);
    let (min, max) = m.camera().visible_world();
    assert!(min.x <= 0.0 && max.x >= 2000.0 && min.y <= 0.0 && max.y >= 1000.0);
}

#[test]
fn fit_falls_back_to_the_tokens_when_the_map_image_fails() {
    let mut m = Model::new(1);
    m.resize(1000.0, 500.0);
    m.apply_snapshot(snapshot_with_unsized_map());
    assert_eq!(m.camera().zoom, 1.0);
    m.mark_image_failed("https://x/map.png");
    assert_ne!(m.camera().zoom, 1.0);
}

#[test]
fn map_size_from_asset_frames_the_map() {
    let mut s = snapshot("u", Role::Gm, vec![]);
    s.scenes[0].levels[0].background = Some(AssetRef {
        url: "https://x/map.png".into(),
        width: 2000,
        height: 1000,
        ..Default::default()
    });
    let mut m = Model::new(1);
    m.apply_snapshot(s);
    m.resize(1000.0, 500.0);
    let (min, max) = m.camera().visible_world();
    assert!(min.x <= 0.0 && max.x >= 2000.0 && min.y <= 0.0 && max.y >= 1000.0);
}

// --------------------------------------------------------------- hit-testing

#[test]
fn hit_test_finds_topmost() {
    let m = model_as(
        "u",
        Role::Gm,
        vec![
            token("under", 200.0, 200.0, CreatureSize::Medium, &[]),
            token("over", 220.0, 200.0, CreatureSize::Medium, &[]),
        ],
    );
    assert_eq!(m.hit_test(Pt::new(210.0, 200.0)), Some("over"));
    assert_eq!(m.hit_test(Pt::new(170.0, 200.0)), Some("under"));
    assert_eq!(m.hit_test(Pt::new(500.0, 500.0)), None);
    assert_eq!(m.hit_test(Pt::new(f64::NAN, 0.0)), None);
}

#[test]
fn hit_test_uses_token_size() {
    let m = model_as(
        "u",
        Role::Gm,
        vec![
            token("big", 300.0, 300.0, CreatureSize::Large, &[]),
            token("small", 600.0, 300.0, CreatureSize::Medium, &[]),
        ],
    );
    // Large = 2 cells = 140 units across: radius 70.
    assert_eq!(m.hit_test(Pt::new(300.0 + 69.0, 300.0)), Some("big"));
    assert_eq!(m.hit_test(Pt::new(300.0 + 72.0, 300.0)), None);
    // Medium radius is 35.
    assert_eq!(m.hit_test(Pt::new(600.0 + 34.0, 300.0)), Some("small"));
    assert_eq!(m.hit_test(Pt::new(600.0 + 40.0, 300.0)), None);
    // The corner of the bounding square is outside the disc.
    assert_eq!(m.hit_test(Pt::new(600.0 + 30.0, 300.0 + 30.0)), None);
}

#[test]
fn hit_test_ignores_other_scenes_and_levels() {
    let mut other_scene = token("elsewhere", 100.0, 100.0, CreatureSize::Medium, &[]);
    other_scene.scene_id = "S-other".into();
    let mut other_level = token("upstairs", 100.0, 100.0, CreatureSize::Medium, &[]);
    other_level.level_id = "L2".into();
    let m = model_as("u", Role::Gm, vec![other_scene, other_level]);
    assert_eq!(m.hit_test(Pt::new(100.0, 100.0)), None);
    assert!(!m
        .draw_list()
        .items
        .iter()
        .any(|i| matches!(i, DrawItem::Disc { .. })));
}

#[test]
fn tiny_tokens_have_a_minimum_hit_radius_on_screen() {
    let mut m = model_as(
        "u",
        Role::Gm,
        vec![token("t", 100.0, 100.0, CreatureSize::Tiny, &[])],
    );
    // Tiny: radius 17.5 world. At zoom 0.1 that is 1.75 px; min is 8 px = 80 world.
    m.camera.zoom = 0.1;
    assert_eq!(m.hit_test(Pt::new(100.0 + 70.0, 100.0)), Some("t"));
    assert_eq!(m.hit_test(Pt::new(100.0 + 90.0, 100.0)), None);
}

// ------------------------------------------------------------- permissions

#[test]
fn drag_permission() {
    let t = token("t", 0.0, 0.0, CreatureSize::Medium, &["owner"]);
    let cases = [
        ("gm", Role::Gm, true),
        ("owner", Role::Player, true),
        ("other", Role::Player, false),
        ("owner", Role::Spectator, false),
        ("owner", Role::Unspecified, false),
    ];
    for (user, role, expect) in cases {
        let m = model_as(user, role, vec![t.clone()]);
        assert_eq!(m.can_move(&t), expect, "{user} {role:?}");
    }
    // An empty user id never matches an empty controller entry.
    let mut sneaky = t.clone();
    sneaky.controller_user_ids = vec![String::new()];
    let m = model_as("", Role::Player, vec![sneaky.clone()]);
    assert!(!m.can_move(&sneaky));
}

#[test]
fn player_cannot_start_a_drag_on_someone_elses_token() {
    let mut m = model_as(
        "p",
        Role::Player,
        vec![token("t", 105.0, 105.0, CreatureSize::Medium, &["other"])],
    );
    assert!(drag(&mut m, (105.0, 105.0), (400.0, 300.0), 0.0).is_none());
    assert!(!m.has_pending("t"));
    assert_eq!(m.selected(), Some("t"));
    // The press panned instead.
    assert_ne!(m.camera().origin, Pt::new(0.0, 0.0));
    assert_eq!(pos(&m, "t"), (105.0, 105.0));
}

// ------------------------------------------------------------------ snapping

#[test]
fn snap_odd_sizes_to_cell_centres() {
    let m = model_as("u", Role::Gm, vec![]);
    for (x, y, ex, ey) in [
        (0.0, 0.0, 35.0, 35.0),
        (69.9, 69.9, 35.0, 35.0),
        (70.0, 70.0, 105.0, 105.0),
        (-0.1, -0.1, -35.0, -35.0),
        (-70.1, 140.0, -105.0, 175.0),
    ] {
        let s = m.snap(Pt::new(x, y), 1.0);
        assert_eq!((s.x, s.y), (ex, ey), "({x},{y})");
    }
    let tiny = m.snap(Pt::new(100.0, 100.0), 0.5);
    assert_eq!((tiny.x, tiny.y), (105.0, 105.0));
    let huge = m.snap(Pt::new(100.0, 100.0), 3.0);
    assert_eq!((huge.x, huge.y), (105.0, 105.0));
}

#[test]
fn snap_even_sizes_to_corners() {
    let m = model_as("u", Role::Gm, vec![]);
    let s = m.snap(Pt::new(100.0, 30.0), 2.0);
    assert_eq!((s.x, s.y), (70.0, 0.0));
    let s = m.snap(Pt::new(106.0, 106.0), 4.0);
    assert_eq!((s.x, s.y), (140.0, 140.0));
}

#[test]
fn snap_respects_grid_offset_and_cell_size() {
    let mut s = snapshot("u", Role::Gm, vec![]);
    let g = s.scenes[0].grid.as_mut().unwrap();
    g.cell_size = 50.0;
    g.offset = v2(10.0, 20.0);
    let mut m = Model::new(1);
    m.apply_snapshot(s);
    let p = m.snap(Pt::new(12.0, 22.0), 1.0);
    assert_eq!((p.x, p.y), (35.0, 45.0));
}

#[test]
fn no_snap_without_a_square_grid() {
    let mut s = snapshot("u", Role::Gm, vec![]);
    s.scenes[0].grid.as_mut().unwrap().kind = GridKind::HexRows as i32;
    let mut m = Model::new(1);
    m.apply_snapshot(s);
    assert_eq!(m.snap(Pt::new(12.3, 45.6), 1.0), Pt::new(12.3, 45.6));
}

// ---------------------------------------------------------------- drag flow

fn player_model() -> Model {
    model_as(
        "p",
        Role::Player,
        vec![
            token("mine", 105.0, 105.0, CreatureSize::Medium, &["p"]),
            token("theirs", 385.0, 245.0, CreatureSize::Small, &[]),
        ],
    )
}

#[test]
fn drag_yields_a_snapped_move_intent() {
    let mut m = player_model();
    // Grab slightly off-centre; release over the cell at column 5, row 3.
    let bytes = drag(&mut m, (110.0, 100.0), (360.0, 230.0), 1000.0).expect("an intent");
    let (id, mv) = decode_move(&bytes);
    assert!(id.starts_with("ci-abc-"), "{id}");
    assert_eq!(mv.token_id, "mine");
    assert_eq!(mv.level_id, "L1");
    assert_eq!(mv.path.len(), 1);
    // Pointer ended at (360,230); grab offset (-5, +5) puts the centre at
    // (355, 235): cell column 5, row 3, centre (385, 245).
    assert_eq!((mv.path[0].x, mv.path[0].y), (385.0, 245.0));
}

#[test]
fn drag_does_not_move_the_token_until_the_event() {
    let mut m = player_model();
    m.pointer_down(105.0, 105.0, 0);
    m.pointer_move(300.0, 105.0);
    assert!(m.is_dragging());
    // While dragging, the highlight and preview are in the draw list.
    let preview = m.draw_list();
    assert!(preview.items.iter().any(
        |i| matches!(i, DrawItem::Disc { cx, cy, .. } if (*cx - 300.0).abs() < 0.01 && (*cy - 105.0).abs() < 0.01)
    ));
    let bytes = m.pointer_up(300.0, 105.0, 0.0).expect("intent");
    let (_, mv) = decode_move(&bytes);
    // The model's token is still at its old position.
    assert_eq!(pos(&m, "mine"), (105.0, 105.0));
    assert!(m.has_pending("mine"));
    // ...but it is drawn at the snapped destination.
    let dest = (mv.path[0].x, mv.path[0].y);
    assert_eq!(dest, (315.0, 105.0));
    let drawn = m.draw_list();
    assert!(drawn
        .items
        .iter()
        .any(|i| matches!(i, DrawItem::Disc { cx, cy, .. } if (*cx, *cy) == (dest.0, dest.1))));
    // The server's event makes it real and clears the preview.
    m.apply_event(moved("mine", dest.0, dest.1));
    assert_eq!(pos(&m, "mine"), dest);
    assert!(!m.has_pending("mine"));
}

#[test]
fn rejection_clears_the_preview() {
    let mut m = player_model();
    let bytes = drag(&mut m, (105.0, 105.0), (300.0, 105.0), 0.0).unwrap();
    let (id, _) = decode_move(&bytes);
    // A rejection for some other intent changes nothing.
    m.apply_rejection(&Rejection {
        client_intent_id: "other".into(),
        code: "c".into(),
        message: "m".into(),
    });
    assert!(m.has_pending("mine"));
    m.apply_rejection(&Rejection {
        client_intent_id: id,
        code: "not_permitted".into(),
        message: "nope".into(),
    });
    assert!(!m.has_pending("mine"));
    assert_eq!(pos(&m, "mine"), (105.0, 105.0));
    assert_eq!(m.last_rejection().unwrap().code, "not_permitted");
}

#[test]
fn only_a_refusal_of_the_renderers_own_move_is_reported() {
    let mut m = player_model();
    let bytes = drag(&mut m, (105.0, 105.0), (300.0, 105.0), 0.0).unwrap();
    let (id, _) = decode_move(&bytes);
    let refuse = |id: &str| {
        server_bytes(server_message::Kind::IntentRejected(IntentRejected {
            client_intent_id: id.into(),
            code: "not_permitted".into(),
            message: "no".into(),
        }))
    };
    // A refused chat or dice intent (some other id) is not the map's business.
    assert_eq!(
        m.apply_server_message(&refuse("a-chat-id")),
        Ok(Applied::Ignored)
    );
    assert!(m.has_pending("mine"));
    // The refusal of this move is reported, and the preview is cleared.
    let Ok(Applied::Rejected(r)) = m.apply_server_message(&refuse(&id)) else {
        panic!("expected the move's refusal to be reported");
    };
    assert_eq!(
        (r.client_intent_id.as_str(), r.code.as_str()),
        (id.as_str(), "not_permitted")
    );
    assert!(!m.has_pending("mine"));
}

#[test]
fn preview_times_out() {
    let mut m = player_model();
    drag(&mut m, (105.0, 105.0), (300.0, 105.0), 1000.0).unwrap();
    m.expire_pending(1000.0 + PENDING_TIMEOUT_MS);
    assert!(m.has_pending("mine"));
    m.expire_pending(1000.0 + PENDING_TIMEOUT_MS + 1.0);
    assert!(!m.has_pending("mine"));
}

#[test]
fn pending_token_cannot_be_grabbed_again() {
    let mut m = player_model();
    drag(&mut m, (105.0, 105.0), (300.0, 105.0), 0.0).unwrap();
    // The disc is drawn (and hit) at the pending destination.
    assert!(drag(&mut m, (315.0, 105.0), (500.0, 105.0), 1.0).is_none());
    assert_eq!(m.selected(), Some("mine"));
}

#[test]
fn click_without_movement_selects_only() {
    let mut m = player_model();
    m.pointer_down(105.0, 105.0, 0);
    m.pointer_move(106.0, 106.0);
    assert!(m.pointer_up(106.0, 106.0, 0.0).is_none());
    assert_eq!(m.selected(), Some("mine"));
    assert!(!m.has_pending("mine"));
    // Clicking empty space clears the selection.
    m.pointer_down(700.0, 500.0, 0);
    m.pointer_up(700.0, 500.0, 0.0);
    assert_eq!(m.selected(), None);
}

#[test]
fn dropping_back_in_the_same_cell_sends_nothing() {
    let mut m = player_model();
    assert!(drag(&mut m, (105.0, 105.0), (120.0, 90.0), 0.0).is_none());
    assert!(!m.has_pending("mine"));
}

#[test]
fn large_tokens_snap_to_corners_in_the_intent() {
    let mut m = model_as(
        "gm",
        Role::Gm,
        vec![token("ogre", 140.0, 140.0, CreatureSize::Large, &[])],
    );
    let bytes = drag(&mut m, (140.0, 140.0), (300.0, 230.0), 0.0).unwrap();
    let (_, mv) = decode_move(&bytes);
    assert_eq!((mv.path[0].x, mv.path[0].y), (280.0, 210.0));
}

#[test]
fn intent_ids_are_fresh() {
    let mut m = player_model();
    let a = decode_move(&drag(&mut m, (105.0, 105.0), (300.0, 105.0), 0.0).unwrap()).0;
    m.apply_event(moved("mine", 315.0, 105.0));
    let b = decode_move(&drag(&mut m, (315.0, 105.0), (105.0, 105.0), 0.0).unwrap()).0;
    assert_ne!(a, b);
}

#[test]
fn removed_token_ends_the_drag() {
    let mut m = player_model();
    m.pointer_down(105.0, 105.0, 0);
    m.pointer_move(200.0, 200.0);
    m.apply_event(event_of(event::Kind::TokenRemoved(TokenRemoved {
        token_id: "mine".into(),
    })));
    assert!(m.pointer_up(200.0, 200.0, 0.0).is_none());
    assert_eq!(m.selected(), None);
}

#[test]
fn pointer_cancel_abandons_the_drag() {
    let mut m = player_model();
    m.pointer_down(105.0, 105.0, 0);
    m.pointer_move(200.0, 200.0);
    m.pointer_cancel();
    assert!(!m.is_dragging());
    assert!(m.pointer_up(200.0, 200.0, 0.0).is_none());
    assert_eq!(pos(&m, "mine"), (105.0, 105.0));
}

#[test]
fn non_primary_button_pans_even_over_a_token() {
    let mut m = player_model();
    m.pointer_down(105.0, 105.0, 2);
    m.pointer_move(125.0, 105.0);
    assert!(m.pointer_up(125.0, 105.0, 0.0).is_none());
    assert_eq!(m.camera().origin, Pt::new(-20.0, 0.0));
    assert_eq!(m.selected(), None);
}

#[test]
fn non_finite_pointer_input_is_ignored() {
    let mut m = player_model();
    m.pointer_down(f64::NAN, 1.0, 0);
    m.pointer_move(f64::INFINITY, 1.0);
    assert!(m.pointer_up(f64::NAN, f64::NAN, 0.0).is_none());
    assert_eq!(m.camera().origin, Pt::new(0.0, 0.0));
}

// ---------------------------------------------------------------- draw list

fn grid_rects(list: &crate::draw::DrawList) -> usize {
    list.items
        .iter()
        .filter(|i| matches!(i, DrawItem::Rect { .. }))
        .count()
}

#[test]
fn grid_lines_are_bounded_whatever_the_zoom_and_viewport() {
    let mut m = model_as("u", Role::Gm, vec![]);
    for (w, h) in [(800.0, 600.0), (4000.0, 3000.0), (16000.0, 16000.0)] {
        for zoom in [0.02, 0.05, 0.1, 0.1, 0.2, 0.5, 1.0, 2.0, 8.0, 16.0] {
            m.camera = Camera {
                origin: Pt::new(-1234.5, 99.0),
                zoom,
                width: w,
                height: h,
            };
            let n = grid_rects(&m.draw_list());
            assert!(n <= MAX_GRID_LINES, "{n} lines at zoom {zoom} in {w}x{h}");
        }
    }
}

#[test]
fn grid_vanishes_when_zoomed_far_out_and_fades_in_between() {
    let mut m = model_as("u", Role::Gm, vec![]);
    m.camera.zoom = 0.05; // 3.5 px cells
    assert_eq!(grid_rects(&m.draw_list()), 0);
    m.camera.zoom = 1.0;
    let full = m.draw_list();
    assert!(grid_rects(&full) > 0);
    let alpha = |l: &crate::draw::DrawList| match l.items.iter().find_map(|i| match i {
        DrawItem::Rect { color, .. } => Some(*color),
        _ => None,
    }) {
        Some(c) => c[3],
        None => 0.0,
    };
    m.camera.zoom = 10.0 / 70.0; // 10 px cells: between min and full
    let faded = m.draw_list();
    assert!(alpha(&faded) > 0.0 && alpha(&faded) < alpha(&full));
}

#[test]
fn grid_covers_only_the_visible_area() {
    let m = model_as("u", Role::Gm, vec![]);
    let list = m.draw_list();
    // 800x600 at 70px: 12 vertical lines (0..=770) and 9 horizontal (0..=560).
    assert_eq!(grid_rects(&list), 12 + 9);
}

#[test]
fn grid_is_clipped_to_the_map_when_known() {
    let mut s = snapshot("u", Role::Gm, vec![]);
    s.scenes[0].levels[0].background = Some(AssetRef {
        url: "https://x/m.png".into(),
        width: 140,
        height: 140,
        ..Default::default()
    });
    let mut m = Model::new(1);
    m.apply_snapshot(s);
    m.camera = Camera {
        origin: Pt::new(-100.0, -100.0),
        zoom: 1.0,
        width: 800.0,
        height: 600.0,
    };
    m.needs_fit = false;
    let list = m.draw_list();
    // Placeholder rect + lines at 0, 70, 140 in each direction = 6.
    assert_eq!(grid_rects(&list), 1 + 6);
}

#[test]
fn hex_and_none_grids_draw_no_lines() {
    for kind in [GridKind::HexRows, GridKind::None, GridKind::Unspecified] {
        let mut s = snapshot("u", Role::Gm, vec![]);
        s.scenes[0].grid.as_mut().unwrap().kind = kind as i32;
        let mut m = Model::new(1);
        m.apply_snapshot(s);
        m.camera.set_viewport(800.0, 600.0);
        assert_eq!(grid_rects(&m.draw_list()), 0);
    }
}

#[test]
fn map_is_a_placeholder_until_loaded_then_an_image() {
    let url = "https://x/map.png";
    let mut s = snapshot("u", Role::Gm, vec![]);
    s.scenes[0].levels[0].background = Some(AssetRef {
        url: url.into(),
        width: 700,
        height: 490,
        ..Default::default()
    });
    let mut m = Model::new(1);
    m.apply_snapshot(s);
    m.resize(800.0, 600.0);
    let has_image = |m: &Model| {
        m.draw_list()
            .items
            .iter()
            .any(|i| matches!(i, DrawItem::Image { url: u, .. } if u == url))
    };
    assert!(!has_image(&m));
    assert!(matches!(m.draw_list().items[0], DrawItem::Rect { .. }));
    assert_eq!(m.take_image_requests(), vec![url.to_owned()]);
    assert!(m.take_image_requests().is_empty(), "requested once");
    m.mark_image_failed(url);
    assert!(!has_image(&m));
    let DrawItem::Rect { color, .. } = m.draw_list().items[0].clone() else {
        panic!()
    };
    assert!(color[0] > color[2], "failed placeholder is reddish");
    m.mark_image_loaded(url, 700, 490);
    assert!(has_image(&m));
}

#[test]
fn token_images_are_used_only_when_ready() {
    let url = "https://x/t.png";
    let mut t = token("t", 105.0, 105.0, CreatureSize::Medium, &[]);
    t.image = Some(AssetRef {
        url: url.into(),
        ..Default::default()
    });
    let mut m = model_as("u", Role::Gm, vec![t]);
    let disc_image = |m: &Model| {
        m.draw_list().items.iter().find_map(|i| match i {
            DrawItem::Disc { image, .. } => Some(image.clone()),
            _ => None,
        })
    };
    assert_eq!(disc_image(&m), Some(None));
    assert_eq!(m.take_image_requests().len(), 1);
    assert_eq!(disc_image(&m), Some(None));
    m.mark_image_loaded(url, 64, 64);
    assert_eq!(disc_image(&m), Some(Some(url.to_owned())));
    // A later failure (nothing to retry) is drawn as no image.
    let mut m2 = model_as("u", Role::Gm, vec![]);
    m2.mark_image_failed(url);
    assert!(m2.image_state(url).is_some());
}

#[test]
fn unsafe_image_urls_are_never_requested() {
    for bad in [
        "javascript:alert(1)",
        "data:image/png;base64,AAAA",
        "file:///etc/passwd",
        "blob:abc",
        "",
        "http://ok\n.com/x",
    ] {
        assert!(!loadable_url(bad), "{bad:?}");
    }
    for good in [
        "https://cdn.example.com/a.png",
        "HTTP://example.com/a.png",
        "/assets/a.png",
        "a/b.png",
        "a.png?x=1:2",
    ] {
        assert!(loadable_url(good), "{good:?}");
    }
    assert!(!loadable_url(&format!("https://x/{}", "a".repeat(5000))));
}

#[test]
fn token_draw_items_match_the_camera() {
    let mut m = model_as(
        "u",
        Role::Gm,
        vec![token("t", 105.0, 105.0, CreatureSize::Medium, &[])],
    );
    m.camera.zoom = 2.0;
    m.camera.origin = Pt::new(5.0, 5.0);
    let list = m.draw_list();
    let disc = list
        .items
        .iter()
        .find_map(|i| match i {
            DrawItem::Disc { cx, cy, r, .. } => Some((*cx, *cy, *r)),
            _ => None,
        })
        .unwrap();
    assert_eq!(disc, (200.0, 200.0, 70.0));
}

#[test]
fn selection_adds_a_highlight_ring() {
    let mut m = model_as(
        "u",
        Role::Gm,
        vec![token("t", 105.0, 105.0, CreatureSize::Medium, &[])],
    );
    let rings = |m: &Model| {
        m.draw_list()
            .items
            .iter()
            .filter(|i| matches!(i, DrawItem::Ring { .. }))
            .count()
    };
    let before = rings(&m);
    m.pointer_down(105.0, 105.0, 0);
    m.pointer_up(105.0, 105.0, 0.0);
    assert_eq!(rings(&m), before + 1);
}

#[test]
fn hidden_tokens_are_drawn_translucent() {
    let mut t = token("t", 105.0, 105.0, CreatureSize::Medium, &[]);
    t.hidden = true;
    let m = model_as("gm", Role::Gm, vec![t]);
    let alpha = m
        .draw_list()
        .items
        .iter()
        .find_map(|i| match i {
            DrawItem::Disc { color, .. } => Some(color[3]),
            _ => None,
        })
        .unwrap();
    assert!(alpha < 1.0);
}

#[test]
fn token_colour_is_stable_and_varied() {
    assert_eq!(color_for_id("token-aria"), color_for_id("token-aria"));
    assert_ne!(color_for_id("token-aria"), color_for_id("token-goblin"));
    for id in ["", "a", "b", "token-1", "ünï"] {
        let c = color_for_id(id);
        assert!(c.iter().all(|v| (0.0..=1.0).contains(v)), "{id}: {c:?}");
    }
}

// ------------------------------------------------------------------- picking

fn picking_model() -> Model {
    let mut m = player_model();
    m.set_picking(true);
    m
}

/// Press, move to `to` in two steps and release; returns the intent, if any.
fn click_through(m: &mut Model, from: (f64, f64), to: (f64, f64)) -> Option<Vec<u8>> {
    m.pointer_down(from.0, from.1, 0);
    m.pointer_move((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
    m.pointer_move(to.0, to.1);
    m.pointer_up(to.0, to.1, 0.0)
}

fn rings_of(m: &Model, colour: [f32; 4]) -> Vec<f32> {
    m.draw_list()
        .items
        .iter()
        .filter_map(|i| match i {
            DrawItem::Ring { r, color, .. } if *color == colour => Some(*r),
            _ => None,
        })
        .collect()
}

const SELECT_COLOUR: [f32; 4] = [1.0, 0.85, 0.2, 1.0];

#[test]
fn picking_is_off_by_default_and_settable() {
    let mut m = player_model();
    assert!(!m.picking());
    m.set_picking(true);
    assert!(m.picking());
    m.set_picking(false);
    assert!(!m.picking());
}

#[test]
fn click_on_a_token_while_picking_queues_its_id() {
    let mut m = picking_model();
    m.pointer_down(385.0, 245.0, 0);
    assert!(!m.is_dragging());
    assert!(m.pointer_up(385.0, 245.0, 0.0).is_none());
    assert_eq!(m.take_picks(), vec!["theirs".to_owned()]);
    assert!(m.take_picks().is_empty(), "take_picks drains");
    assert!(!m.has_pending("theirs"));
    assert_eq!(pos(&m, "theirs"), (385.0, 245.0));
}

#[test]
fn own_token_is_picked_not_dragged() {
    for (user, role) in [("p", Role::Player), ("gm", Role::Gm)] {
        let mut m = model_as(
            user,
            role,
            vec![token("mine", 105.0, 105.0, CreatureSize::Medium, &["p"])],
        );
        m.set_picking(true);
        m.pointer_down(105.0, 105.0, 0);
        // 5 px: over the drag threshold, under the pick slop.
        m.pointer_move(110.0, 105.0);
        assert!(!m.is_dragging(), "{user}: must not drag");
        assert!(
            m.pointer_up(110.0, 105.0, 0.0).is_none(),
            "{user}: no intent"
        );
        assert_eq!(m.take_picks(), vec!["mine".to_owned()], "{user}");
        assert!(!m.has_pending("mine"));
        assert_eq!(pos(&m, "mine"), (105.0, 105.0));
        assert_eq!(m.camera().origin, Pt::new(0.0, 0.0), "a tap does not pan");
    }
}

#[test]
fn a_full_drag_gesture_on_an_own_token_sends_no_move_while_picking() {
    let mut m = picking_model();
    m.pointer_down(105.0, 105.0, 0);
    m.pointer_move(250.0, 200.0);
    m.pointer_move(400.0, 300.0);
    assert!(!m.is_dragging());
    assert_eq!(m.camera().origin, Pt::new(-295.0, -195.0), "it panned");
    let preview = [1.0, 1.0, 1.0, 0.7];
    assert!(
        rings_of(&m, preview).is_empty(),
        "no drag-preview ring is drawn"
    );
    assert!(m.pointer_up(400.0, 300.0, 0.0).is_none());
    assert!(m.take_picks().is_empty());
    assert!(!m.has_pending("mine"));
    assert_eq!(pos(&m, "mine"), (105.0, 105.0));
}

#[test]
fn moving_past_the_slop_from_a_token_pans_by_the_full_travel_and_picks_nothing() {
    let mut m = picking_model();
    assert!(click_through(&mut m, (385.0, 245.0), (425.0, 245.0)).is_none());
    assert!(m.take_picks().is_empty());
    assert_eq!(m.camera().origin, Pt::new(-40.0, 0.0));
    // Coming back onto the token afterwards does not revive a pick.
    let on = m.camera().world_to_screen(Pt::new(385.0, 245.0));
    m.pointer_down(on.x, on.y, 0);
    m.pointer_move(on.x + 20.0, on.y);
    m.pointer_move(on.x, on.y);
    m.pointer_up(on.x, on.y, 0.0);
    assert!(m.take_picks().is_empty());
}

#[test]
fn nine_pixels_pans_by_exactly_nine_and_picks_nothing() {
    let mut m = picking_model();
    m.pointer_down(385.0, 245.0, 0);
    m.pointer_move(394.0, 245.0);
    assert_eq!(m.camera().origin, Pt::new(-9.0, 0.0));
    assert!(m.pointer_up(394.0, 245.0, 0.0).is_none());
    assert!(m.take_picks().is_empty());
}

#[test]
fn a_wobble_under_the_slop_still_picks_and_the_map_stays_put() {
    let mut m = picking_model();
    m.pointer_down(385.0, 245.0, 0);
    m.pointer_move(391.0, 245.0);
    assert_eq!(m.camera().origin, Pt::new(0.0, 0.0));
    m.pointer_up(391.0, 245.0, 0.0);
    assert_eq!(m.take_picks(), vec!["theirs".to_owned()]);
    assert_eq!(m.camera().origin, Pt::new(0.0, 0.0));
}

#[test]
fn empty_space_while_picking_pans_and_picks_nothing() {
    let mut m = picking_model();
    assert!(click_through(&mut m, (700.0, 500.0), (720.0, 500.0)).is_none());
    assert!(m.take_picks().is_empty());
    assert_eq!(m.camera().origin, Pt::new(-20.0, 0.0));
    // A plain click on empty ground does nothing either.
    m.pointer_down(700.0, 500.0, 0);
    assert!(m.pointer_up(700.0, 500.0, 0.0).is_none());
    assert!(m.take_picks().is_empty());
}

#[test]
fn right_button_on_a_token_while_picking_pans_and_picks_nothing() {
    let mut m = picking_model();
    m.pointer_down(385.0, 245.0, 2);
    m.pointer_move(405.0, 245.0);
    assert!(m.pointer_up(405.0, 245.0, 0.0).is_none());
    assert!(m.take_picks().is_empty());
    assert_eq!(m.camera().origin, Pt::new(-20.0, 0.0));
}

#[test]
fn picking_leaves_the_selection_unchanged() {
    let mut m = player_model();
    m.pointer_down(105.0, 105.0, 0);
    m.pointer_up(105.0, 105.0, 0.0);
    assert_eq!(m.selected(), Some("mine"));
    m.set_picking(true);
    // Pick another token, click empty ground, right-click: selection stays.
    m.pointer_down(385.0, 245.0, 0);
    m.pointer_up(385.0, 245.0, 0.0);
    assert_eq!(m.selected(), Some("mine"));
    m.pointer_down(700.0, 500.0, 0);
    m.pointer_up(700.0, 500.0, 0.0);
    assert_eq!(m.selected(), Some("mine"));
    m.pointer_down(385.0, 245.0, 2);
    m.pointer_up(385.0, 245.0, 0.0);
    assert_eq!(m.selected(), Some("mine"));
}

#[test]
fn tokens_off_the_view_cannot_be_picked() {
    let mut elsewhere = token("other-scene", 105.0, 105.0, CreatureSize::Medium, &[]);
    elsewhere.scene_id = "S2".into();
    let mut upstairs = token("other-level", 245.0, 105.0, CreatureSize::Medium, &[]);
    upstairs.level_id = "L2".into();
    let mut m = model_as(
        "gm",
        Role::Gm,
        vec![
            elsewhere,
            upstairs,
            token("here", 385.0, 105.0, CreatureSize::Medium, &[]),
        ],
    );
    m.set_picking(true);
    for x in [105.0, 245.0] {
        m.pointer_down(x, 105.0, 0);
        m.pointer_up(x, 105.0, 0.0);
    }
    assert!(m.take_picks().is_empty());
    m.pointer_down(385.0, 105.0, 0);
    m.pointer_up(385.0, 105.0, 0.0);
    assert_eq!(m.take_picks(), vec!["here".to_owned()]);
}

#[test]
fn a_token_the_renderer_never_received_cannot_be_picked() {
    let mut m = picking_model();
    m.set_highlights(["ghost".to_owned()]);
    m.pointer_down(600.0, 400.0, 0);
    m.pointer_up(600.0, 400.0, 0.0);
    assert!(m.take_picks().is_empty());
}

#[test]
fn a_token_with_a_pending_move_can_still_be_picked() {
    let mut m = player_model();
    drag(&mut m, (105.0, 105.0), (315.0, 105.0), 0.0).expect("intent");
    assert!(m.has_pending("mine"));
    m.set_picking(true);
    // Hit-testing uses the pending destination.
    m.pointer_down(315.0, 105.0, 0);
    assert!(m.pointer_up(315.0, 105.0, 0.0).is_none());
    assert_eq!(m.take_picks(), vec!["mine".to_owned()]);
}

#[test]
fn with_picking_off_click_selects_and_drag_moves() {
    let mut m = player_model();
    m.pointer_down(385.0, 245.0, 0);
    m.pointer_up(385.0, 245.0, 0.0);
    assert_eq!(m.selected(), Some("theirs"));
    assert!(m.take_picks().is_empty());
    let bytes = drag(&mut m, (105.0, 105.0), (315.0, 105.0), 0.0).expect("intent");
    assert_eq!(decode_move(&bytes).1.token_id, "mine");
    assert!(m.take_picks().is_empty());
    // Switching picking on and off again restores that behaviour.
    m.set_picking(true);
    m.set_picking(false);
    m.apply_event(moved("mine", 315.0, 105.0));
    assert!(drag(&mut m, (315.0, 105.0), (105.0, 105.0), 1.0).is_some());
}

#[test]
fn turning_picking_on_mid_drag_abandons_the_drag() {
    let mut m = player_model();
    m.pointer_down(105.0, 105.0, 0);
    m.pointer_move(200.0, 200.0);
    assert!(m.is_dragging());
    m.set_picking(true);
    assert!(!m.is_dragging());
    assert!(m.pointer_up(200.0, 200.0, 0.0).is_none());
    assert!(!m.has_pending("mine"));
    assert_eq!(pos(&m, "mine"), (105.0, 105.0));
    assert!(m.take_picks().is_empty());
}

#[test]
fn turning_picking_off_mid_press_drops_the_pick() {
    let mut m = picking_model();
    m.pointer_down(385.0, 245.0, 0);
    m.set_picking(false);
    // Released at the same point: only the mode change can stop the pick.
    assert!(m.pointer_up(385.0, 245.0, 0.0).is_none());
    assert!(m.take_picks().is_empty());
    assert_eq!(m.selected(), None, "and it did not select either");
}

#[test]
fn turning_picking_off_mid_press_continues_as_a_pan() {
    let mut m = picking_model();
    m.pointer_down(385.0, 245.0, 0);
    m.set_picking(false);
    m.pointer_move(395.0, 245.0);
    assert_eq!(m.camera().origin, Pt::new(-10.0, 0.0));
}

#[test]
fn pointer_cancel_drops_a_pick_press() {
    let mut m = picking_model();
    m.pointer_down(385.0, 245.0, 0);
    m.pointer_cancel();
    assert!(m.pointer_up(385.0, 245.0, 0.0).is_none());
    assert!(m.take_picks().is_empty());
}

#[test]
fn a_token_removed_mid_press_is_not_picked() {
    let mut m = picking_model();
    m.pointer_down(385.0, 245.0, 0);
    m.apply_event(event_of(event::Kind::TokenRemoved(TokenRemoved {
        token_id: "theirs".into(),
    })));
    assert!(m.pointer_up(385.0, 245.0, 0.0).is_none());
    assert!(m.take_picks().is_empty());
}

#[test]
fn a_token_moved_to_another_level_mid_press_is_not_picked() {
    let mut m = picking_model();
    m.pointer_down(385.0, 245.0, 0);
    m.apply_event(event_of(event::Kind::TokenMoved(TokenMoved {
        token_id: "theirs".into(),
        path: vec![Vec2 { x: 385.0, y: 245.0 }],
        level_id: "L2".into(),
        ..Default::default()
    })));
    assert!(m.pointer_up(385.0, 245.0, 0.0).is_none());
    assert!(m.take_picks().is_empty());
}

#[test]
fn snapshot_keeps_picking_and_highlights_and_clears_queued_picks() {
    let mut m = picking_model();
    m.set_highlights(["mine".to_owned()]);
    m.pointer_down(385.0, 245.0, 0);
    m.pointer_up(385.0, 245.0, 0.0);
    m.pointer_down(385.0, 245.0, 0);
    m.apply_snapshot(snapshot(
        "p",
        Role::Player,
        vec![token("mine", 105.0, 105.0, CreatureSize::Medium, &["p"])],
    ));
    assert!(m.picking());
    assert!(m.is_highlighted("mine"));
    assert!(m.pointer_up(385.0, 245.0, 0.0).is_none());
    assert!(m.take_picks().is_empty(), "queue and press were dropped");
}

#[test]
fn several_picks_come_out_in_order() {
    let mut m = picking_model();
    for (x, y) in [(385.0, 245.0), (105.0, 105.0), (385.0, 245.0)] {
        m.pointer_down(x, y, 0);
        m.pointer_up(x, y, 0.0);
    }
    assert_eq!(m.take_picks(), vec!["theirs", "mine", "theirs"]);
}

#[test]
fn highlights_replace_rather_than_add() {
    let mut m = player_model();
    m.set_highlights(["mine".to_owned()]);
    assert!(m.is_highlighted("mine") && !m.is_highlighted("theirs"));
    m.set_highlights(["theirs".to_owned()]);
    assert!(!m.is_highlighted("mine") && m.is_highlighted("theirs"));
    m.set_highlights(Vec::new());
    assert!(!m.is_highlighted("mine") && !m.is_highlighted("theirs"));
}

#[test]
fn empty_ids_are_ignored_in_highlights() {
    let mut m = player_model();
    m.set_highlights([String::new(), "mine".to_owned()]);
    assert!(!m.is_highlighted(""));
    assert!(m.is_highlighted("mine"));
}

#[test]
fn unknown_highlight_ids_are_harmless() {
    let mut m = player_model();
    let before = m.draw_list();
    m.set_highlights(["nobody".to_owned(), "nope".to_owned()]);
    assert_eq!(m.draw_list(), before, "nothing extra is drawn");
}

#[test]
fn a_highlighted_token_gets_a_distinct_outer_ring() {
    let mut m = model_as(
        "gm",
        Role::Gm,
        vec![token("t", 105.0, 105.0, CreatureSize::Medium, &[])],
    );
    m.set_highlights(["t".to_owned()]);
    let list = m.draw_list();
    let disc_r = list
        .items
        .iter()
        .find_map(|i| match i {
            DrawItem::Disc { r, .. } => Some(*r),
            _ => None,
        })
        .unwrap();
    assert!(list.items.iter().any(|i| matches!(
        i,
        DrawItem::Ring { r, thickness, color, .. }
            if *color == HIGHLIGHT_COLOR && *r == disc_r + 8.0 && *thickness == 3.0
    )));
    assert_ne!(HIGHLIGHT_COLOR, SELECT_COLOUR);
}

#[test]
fn selected_and_highlighted_shows_both_rings() {
    let mut m = model_as(
        "gm",
        Role::Gm,
        vec![token("t", 105.0, 105.0, CreatureSize::Medium, &[])],
    );
    m.pointer_down(105.0, 105.0, 0);
    m.pointer_up(105.0, 105.0, 0.0);
    m.set_highlights(["t".to_owned()]);
    let sel = rings_of(&m, SELECT_COLOUR);
    let hi = rings_of(&m, HIGHLIGHT_COLOR);
    assert_eq!((sel.len(), hi.len()), (1, 1));
    assert!(
        hi[0] > sel[0],
        "highlight ring sits outside the selection ring"
    );
}

#[test]
fn highlights_draw_with_picking_off_and_clearing_removes_them() {
    let mut m = player_model();
    assert!(!m.picking());
    m.set_highlights(["mine".to_owned(), "theirs".to_owned()]);
    assert_eq!(rings_of(&m, HIGHLIGHT_COLOR).len(), 2);
    m.set_highlights(["theirs".to_owned()]);
    assert_eq!(rings_of(&m, HIGHLIGHT_COLOR).len(), 1);
    m.set_highlights(Vec::new());
    assert!(rings_of(&m, HIGHLIGHT_COLOR).is_empty());
}

// ------------------------------------------------------- health and conditions

fn hurt(id: &str, state: HealthState) -> Token {
    let mut t = token(id, 385.0, 245.0, CreatureSize::Small, &[]);
    t.health_state = state as i32;
    t
}

fn one_token_model(t: Token) -> Model {
    model_as("p", Role::Player, vec![t])
}

/// Every ring drawn at the token's own edge (radius r, not the outer rings).
fn edge_rings(m: &Model) -> Vec<([f32; 4], f32)> {
    let r = (m.token_diameter(m.token("x").unwrap()) * 0.5 * m.camera().zoom) as f32;
    m.draw_list()
        .items
        .iter()
        .filter_map(|i| match i {
            DrawItem::Ring {
                r: rr,
                color,
                thickness,
                ..
            } if (*rr - r).abs() < 1e-3 => Some((*color, *thickness)),
            _ => None,
        })
        .collect()
}

#[test]
fn each_health_state_draws_its_ring_in_place_of_the_outline() {
    for (state, colour, thickness) in [
        (HealthState::Unspecified, OUTLINE_COLOR, 2.0),
        (HealthState::Healthy, OUTLINE_COLOR, 2.0),
        (HealthState::Bloodied, BLOODIED_COLOR, 3.0),
        (HealthState::NearDeath, NEAR_DEATH_COLOR, 3.0),
        (HealthState::Down, DOWN_COLOR, 3.0),
    ] {
        let m = one_token_model(hurt("x", state));
        assert_eq!(edge_rings(&m), vec![(colour, thickness)], "{state:?}");
    }
}

#[test]
fn health_colours_never_match_the_selection_or_highlight_rings() {
    for c in [BLOODIED_COLOR, NEAR_DEATH_COLOR, DOWN_COLOR] {
        assert_ne!(c, SELECT_COLOUR);
        assert_ne!(c, HIGHLIGHT_COLOR);
    }
}

#[test]
fn a_downed_token_is_drawn_faded() {
    let disc_alpha = |m: &Model| {
        m.draw_list()
            .items
            .iter()
            .find_map(|i| match i {
                DrawItem::Disc {
                    color,
                    image: None,
                    r,
                    ..
                } if *r > CONDITION_MARKER_PX => Some(color[3]),
                _ => None,
            })
            .unwrap()
    };
    let up = one_token_model(hurt("x", HealthState::Bloodied));
    let down = one_token_model(hurt("x", HealthState::Down));
    assert!((disc_alpha(&down) - disc_alpha(&up) * DOWN_FADE).abs() < 1e-6);
}

#[test]
fn hit_points_alone_never_draw_a_health_ring() {
    let mut t = hurt("x", HealthState::Unspecified);
    t.hp_current = Some(1);
    t.hp_max = Some(40);
    let m = one_token_model(t);
    assert_eq!(edge_rings(&m), vec![(OUTLINE_COLOR, 2.0)]);
}

fn marker_count(m: &Model) -> usize {
    m.draw_list()
        .items
        .iter()
        .filter(|i| matches!(i, DrawItem::Disc { color, .. } if *color == CONDITION_MARKER_COLOR))
        .count()
}

#[test]
fn a_token_with_conditions_gets_one_marker_at_its_upper_right() {
    let mut t = hurt("x", HealthState::Healthy);
    let none = one_token_model(t.clone());
    assert_eq!(marker_count(&none), 0);
    t.conditions = vec!["poisoned".into(), "prone".into()];
    let m = one_token_model(t);
    assert_eq!(marker_count(&m), 1, "one marker, however many conditions");
    let (cx, cy) = (385.0_f32, 245.0_f32);
    let marker = m
        .draw_list()
        .items
        .into_iter()
        .find_map(|i| match i {
            DrawItem::Disc {
                cx: x,
                cy: y,
                r,
                color,
                ..
            } if color == CONDITION_MARKER_COLOR => Some((x, y, r)),
            _ => None,
        })
        .unwrap();
    assert!(marker.0 > cx && marker.1 < cy, "upper right: {marker:?}");
    assert!(marker.2 >= 6.0);
}

#[test]
fn ring_and_marker_follow_token_upserted_and_go_with_token_removed() {
    let mut m = one_token_model(hurt("x", HealthState::Healthy));
    let mut changed = hurt("x", HealthState::NearDeath);
    changed.conditions = vec!["blinded".into()];
    m.apply_event(event_of(event::Kind::TokenUpserted(TokenUpserted {
        token: Some(changed),
    })));
    assert_eq!(edge_rings(&m), vec![(NEAR_DEATH_COLOR, 3.0)]);
    assert_eq!(marker_count(&m), 1);
    m.apply_event(event_of(event::Kind::TokenRemoved(TokenRemoved {
        token_id: "x".into(),
    })));
    assert_eq!(rings_of(&m, NEAR_DEATH_COLOR), Vec::<f32>::new());
    assert_eq!(marker_count(&m), 0);
}

#[test]
fn selected_highlighted_and_bloodied_draws_all_three_rings_inside_out() {
    let mut m = one_token_model(hurt("x", HealthState::Bloodied));
    m.pointer_down(385.0, 245.0, 0);
    m.pointer_up(385.0, 245.0, 0.0);
    m.set_highlights(["x".to_owned()]);
    let rings: Vec<([f32; 4], f32)> = m
        .draw_list()
        .items
        .iter()
        .filter_map(|i| match i {
            DrawItem::Ring { r, color, .. }
                if [BLOODIED_COLOR, SELECT_COLOUR, HIGHLIGHT_COLOR].contains(color) =>
            {
                Some((*color, *r))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        rings.iter().map(|x| x.0).collect::<Vec<_>>(),
        vec![BLOODIED_COLOR, SELECT_COLOUR, HIGHLIGHT_COLOR]
    );
    assert!(rings[0].1 < rings[1].1 && rings[1].1 < rings[2].1);
}

// --------------------------------------------------------------------- wire

#[test]
fn hello_encodes_and_decodes() {
    let bytes = encode_hello("campaign-mock", "dev:mock-gm");
    let msg = ClientMessage::decode(bytes.as_slice()).unwrap();
    let Some(client_message::Kind::Hello(h)) = msg.kind else {
        panic!("not a hello");
    };
    assert_eq!(h.campaign_id, "campaign-mock");
    assert_eq!(h.session_token, "dev:mock-gm");
    assert_eq!(h.last_seq, 0);
}

// ------------------------------------------------------ the shipped fixture

#[test]
fn mock_fixture_shaped_snapshot_works_end_to_end() {
    // Mirrors contracts/fixtures/mock/snapshot.json (no serde_json here: it is
    // not an approved dependency of this crate).
    let mut s = snapshot(
        "mock-player",
        Role::Player,
        vec![
            token(
                "token-aria",
                105.0,
                105.0,
                CreatureSize::Medium,
                &["mock-player"],
            ),
            token("token-goblin", 385.0, 245.0, CreatureSize::Small, &[]),
        ],
    );
    s.scenes[0].levels[0].level_id = "level-ground".into();
    for t in &mut s.tokens {
        t.level_id = "level-ground".into();
    }
    let mut m = Model::new(7);
    m.apply_snapshot(s);
    m.resize(1000.0, 700.0);
    assert_eq!(m.token_count(), 2);
    let list = m.draw_list();
    assert!(list
        .items
        .iter()
        .any(|i| matches!(i, DrawItem::Disc { .. })));
    let c = m.camera().world_to_screen(Pt::new(105.0, 105.0));
    let to = m.camera().world_to_screen(Pt::new(315.0, 105.0));
    let bytes = drag(&mut m, (c.x, c.y), (to.x, to.y), 0.0).expect("intent");
    let (_, mv) = decode_move(&bytes);
    assert_eq!(mv.token_id, "token-aria");
    assert_eq!(mv.level_id, "level-ground");
    assert_eq!((mv.path[0].x, mv.path[0].y), (315.0, 105.0));
}
