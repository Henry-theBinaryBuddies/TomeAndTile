//! Integration tests: a real server on port 0 and real WebSocket clients.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tt_contracts::v1::{
    client_message, event, intent, server_message, ChatIntent, ClientMessage, Die, Event, Hello,
    Intent, MoveTokenIntent, RollDiceIntent, RollMode, RollVisibility, ServerMessage, Snapshot,
    UseAbilityIntent, Vec2,
};
use tt_mock_server::dice::XorShift64Star;

const WAIT: Duration = Duration::from_secs(5);

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn start_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ =
            tt_mock_server::serve_with_rng(listener, Box::new(XorShift64Star::new(12345))).await;
    });
    addr
}

async fn connect(addr: SocketAddr) -> Ws {
    let (ws, _) = timeout(WAIT, connect_async(format!("ws://{addr}/ws")))
        .await
        .expect("connect timed out")
        .expect("connect failed");
    ws
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
                .expect("server sent an undecodable frame")
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

async fn recv_rejected(ws: &mut Ws) -> tt_contracts::v1::IntentRejected {
    match recv(ws).await {
        server_message::Kind::IntentRejected(r) => r,
        other => panic!("expected a rejection, got {other:?}"),
    }
}

/// Connect, say Hello with `token`, return the socket and its Snapshot.
async fn join(addr: SocketAddr, token: &str) -> (Ws, Snapshot) {
    let mut ws = connect(addr).await;
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

fn move_intent(token_id: &str, path: &[(f32, f32)]) -> intent::Kind {
    intent::Kind::MoveToken(MoveTokenIntent {
        token_id: token_id.into(),
        path: path.iter().map(|&(x, y)| Vec2 { x, y }).collect(),
        level_id: String::new(),
    })
}

fn roll_intent(notation: &str, mode: RollMode, visibility: RollVisibility) -> intent::Kind {
    intent::Kind::RollDice(RollDiceIntent {
        notation: notation.into(),
        mode: mode as i32,
        visibility: visibility as i32,
        label: "test roll".into(),
    })
}

/// Wait until the server closes the socket (close frame, EOF or error).
async fn expect_closed(ws: &mut Ws) {
    loop {
        match timeout(WAIT, ws.next())
            .await
            .expect("timed out waiting for the connection to close")
        {
            None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
            Some(Ok(Message::Binary(b))) => panic!("expected close, got a frame: {b:?}"),
            Some(Ok(_)) => continue,
        }
    }
}

#[tokio::test]
async fn hello_gets_snapshot_with_scene_and_two_tokens() {
    let addr = start_server().await;
    let (_ws, snapshot) = join(addr, "anything").await;
    assert_eq!(snapshot.seq, 0);
    assert_eq!(snapshot.user_id, "mock-player");
    assert_eq!(snapshot.role, tt_contracts::v1::Role::Player as i32);
    assert_eq!(snapshot.scenes.len(), 1);
    assert_eq!(snapshot.tokens.len(), 2);
    let goblin = snapshot.tokens.iter().find(|t| t.name == "Goblin").unwrap();
    assert_eq!(goblin.hp_current, None);
    let aria = snapshot.tokens.iter().find(|t| t.name == "Aria").unwrap();
    assert_eq!(aria.hp_current, Some(24));

    let (_gm, snapshot) = join(addr, "gm").await;
    assert_eq!(snapshot.user_id, "mock-gm");
    assert_eq!(snapshot.role, tt_contracts::v1::Role::Gm as i32);
}

#[tokio::test]
async fn move_token_emits_event_with_distance_and_increasing_seq() {
    let addr = start_server().await;
    let (mut ws, snapshot) = join(addr, "p").await;
    let scene_id = snapshot.scenes[0].scene_id.clone();

    // Aria starts at (105,105): 70 up to (105,175), then 140 across to (245,175).
    send_intent(
        &mut ws,
        "m1",
        move_intent("token-aria", &[(105.0, 175.0), (245.0, 175.0)]),
    )
    .await;
    let first = recv_event(&mut ws).await;
    assert_eq!(first.seq, 1);
    assert_eq!(first.client_intent_id, "m1");
    assert_eq!(first.actor_user_id, "mock-player");
    assert_eq!(first.scene_id, scene_id);
    assert!(first.at_ms > 0);
    match first.kind {
        Some(event::Kind::TokenMoved(m)) => {
            assert_eq!(m.token_id, "token-aria");
            assert_eq!(m.path.len(), 2);
            assert_eq!(m.distance_feet, 15.0);
        }
        other => panic!("expected token_moved, got {other:?}"),
    }

    // Second move starts from the new position (245,175): 70 across = 5 ft.
    send_intent(&mut ws, "m2", move_intent("token-aria", &[(315.0, 175.0)])).await;
    let second = recv_event(&mut ws).await;
    assert_eq!(second.seq, 2);
    match second.kind {
        Some(event::Kind::TokenMoved(m)) => assert_eq!(m.distance_feet, 5.0),
        other => panic!("expected token_moved, got {other:?}"),
    }

    send_intent(&mut ws, "m3", move_intent("nobody", &[(0.0, 0.0)])).await;
    let r = recv_rejected(&mut ws).await;
    assert_eq!(
        (r.code.as_str(), r.client_intent_id.as_str()),
        ("unknown_token", "m3")
    );
    send_intent(&mut ws, "m4", move_intent("token-aria", &[])).await;
    let r = recv_rejected(&mut ws).await;
    assert_eq!(
        (r.code.as_str(), r.client_intent_id.as_str()),
        ("empty_path", "m4")
    );
}

#[tokio::test]
async fn events_reach_every_client_and_late_joiners_see_new_state() {
    let addr = start_server().await;
    let (mut a, _) = join(addr, "p").await;
    let (mut b, _) = join(addr, "gm").await;

    send_intent(&mut a, "a1", move_intent("token-goblin", &[(455.0, 245.0)])).await;
    let seen_by_a = recv_event(&mut a).await;
    let seen_by_b = recv_event(&mut b).await;
    assert_eq!(seen_by_a, seen_by_b);
    assert_eq!(seen_by_b.seq, 1);
    assert_eq!(seen_by_b.client_intent_id, "a1");
    assert_eq!(seen_by_b.actor_user_id, "mock-player");

    let (_c, snapshot) = join(addr, "p").await;
    assert_eq!(snapshot.seq, 1);
    let goblin = snapshot
        .tokens
        .iter()
        .find(|t| t.token_id == "token-goblin")
        .unwrap();
    assert_eq!(goblin.position, Some(Vec2 { x: 455.0, y: 245.0 }));
}

#[tokio::test]
async fn chat_is_broadcast_and_empty_or_long_chat_is_rejected() {
    let addr = start_server().await;
    let (mut ws, _) = join(addr, "gm").await;

    send_intent(
        &mut ws,
        "c1",
        intent::Kind::Chat(ChatIntent {
            text: "hello table".into(),
        }),
    )
    .await;
    let e = recv_event(&mut ws).await;
    assert_eq!(e.seq, 1);
    match e.kind {
        Some(event::Kind::ChatPosted(c)) => {
            assert_eq!(c.user_id, "mock-gm");
            assert_eq!(c.text, "hello table");
        }
        other => panic!("expected chat_posted, got {other:?}"),
    }

    send_intent(
        &mut ws,
        "c2",
        intent::Kind::Chat(ChatIntent {
            text: String::new(),
        }),
    )
    .await;
    let r = recv_rejected(&mut ws).await;
    assert_eq!(
        (r.code.as_str(), r.client_intent_id.as_str()),
        ("empty_chat", "c2")
    );

    send_intent(
        &mut ws,
        "c3",
        intent::Kind::Chat(ChatIntent {
            text: "x".repeat(2001),
        }),
    )
    .await;
    assert_eq!(recv_rejected(&mut ws).await.code, "chat_too_long");

    // Exactly 2000 characters is fine, and a rejected intent used no seq.
    send_intent(
        &mut ws,
        "c4",
        intent::Kind::Chat(ChatIntent {
            text: "x".repeat(2000),
        }),
    )
    .await;
    assert_eq!(recv_event(&mut ws).await.seq, 2);
}

fn rolled(e: Event) -> tt_contracts::v1::RollResult {
    match e.kind {
        Some(event::Kind::DiceRolled(d)) => d.roll.expect("roll present"),
        other => panic!("expected dice_rolled, got {other:?}"),
    }
}

#[tokio::test]
async fn dice_rolls_follow_notation_modes_and_visibility() {
    let addr = start_server().await;
    let (mut ws, _) = join(addr, "p").await;

    send_intent(
        &mut ws,
        "r1",
        roll_intent("2d6+3", RollMode::Normal, RollVisibility::GmOnly),
    )
    .await;
    let roll = rolled(recv_event(&mut ws).await);
    assert_eq!(roll.dice.len(), 2);
    assert!(roll
        .dice
        .iter()
        .all(|d: &Die| (1..=6).contains(&d.value) && d.sides == 6));
    let sum: u32 = roll.dice.iter().map(|d| d.value).sum();
    assert_eq!(roll.total, sum as i32 + 3);
    assert_eq!(roll.modifiers.len(), 1);
    assert_eq!(roll.modifiers[0].source, "modifier");
    assert_eq!(roll.modifiers[0].value, 3);
    assert_eq!(roll.visibility, RollVisibility::Public as i32);
    assert_eq!(roll.roller_user_id, "mock-player");
    assert_eq!(roll.label, "test roll");
    assert_eq!(roll.notation, "2d6+3");
    assert!(!roll.roll_id.is_empty());

    send_intent(
        &mut ws,
        "r2",
        roll_intent("1d20", RollMode::Advantage, RollVisibility::Public),
    )
    .await;
    let roll = rolled(recv_event(&mut ws).await);
    assert_eq!(roll.dice.len(), 2);
    assert_eq!(roll.dice.iter().filter(|d| d.dropped).count(), 1);
    let kept = roll.dice.iter().find(|d| !d.dropped).unwrap().value;
    assert_eq!(kept, roll.dice.iter().map(|d| d.value).max().unwrap());
    assert_eq!(roll.total, kept as i32);
    assert_eq!(roll.mode, RollMode::Advantage as i32);

    send_intent(
        &mut ws,
        "r3",
        roll_intent("1d20", RollMode::Disadvantage, RollVisibility::Public),
    )
    .await;
    let roll = rolled(recv_event(&mut ws).await);
    let kept = roll.dice.iter().find(|d| !d.dropped).unwrap().value;
    assert_eq!(kept, roll.dice.iter().map(|d| d.value).min().unwrap());

    send_intent(
        &mut ws,
        "r4",
        roll_intent("banana", RollMode::Normal, RollVisibility::Public),
    )
    .await;
    let r = recv_rejected(&mut ws).await;
    assert_eq!(
        (r.code.as_str(), r.client_intent_id.as_str()),
        ("bad_notation", "r4")
    );
}

#[tokio::test]
async fn unsupported_intent_is_rejected_with_its_id() {
    let addr = start_server().await;
    let (mut ws, _) = join(addr, "p").await;
    send_intent(
        &mut ws,
        "u1",
        intent::Kind::UseAbility(UseAbilityIntent {
            source_token_id: "token-aria".into(),
            ability_id: "whatever".into(),
            ..Default::default()
        }),
    )
    .await;
    let r = recv_rejected(&mut ws).await;
    assert_eq!(r.code, "not_supported_by_mock");
    assert_eq!(r.client_intent_id, "u1");
}

#[tokio::test]
async fn non_hello_first_message_closes_the_connection() {
    let addr = start_server().await;
    let mut ws = connect(addr).await;
    send_intent(
        &mut ws,
        "early",
        intent::Kind::Chat(ChatIntent { text: "hi".into() }),
    )
    .await;
    expect_closed(&mut ws).await;
}

#[tokio::test]
async fn garbage_frame_closes_only_that_connection() {
    let addr = start_server().await;
    let (mut good, _) = join(addr, "p").await;
    let (mut bad, _) = join(addr, "p").await;

    timeout(WAIT, bad.send(Message::Binary(vec![0xff; 16].into())))
        .await
        .unwrap()
        .unwrap();
    expect_closed(&mut bad).await;

    send_intent(
        &mut good,
        "g1",
        intent::Kind::Chat(ChatIntent {
            text: "still here".into(),
        }),
    )
    .await;
    assert_eq!(recv_event(&mut good).await.seq, 1);

    // Garbage before Hello closes too, without disturbing anyone.
    let mut early = connect(addr).await;
    timeout(WAIT, early.send(Message::Binary(vec![0xff; 16].into())))
        .await
        .unwrap()
        .unwrap();
    expect_closed(&mut early).await;
}

#[tokio::test]
async fn text_frames_are_ignored_and_oversized_messages_close() {
    let addr = start_server().await;
    let (mut ws, _) = join(addr, "p").await;
    timeout(WAIT, ws.send(Message::Text("ignored".into())))
        .await
        .unwrap()
        .unwrap();
    send_intent(
        &mut ws,
        "t1",
        intent::Kind::Chat(ChatIntent { text: "ok".into() }),
    )
    .await;
    assert_eq!(recv_event(&mut ws).await.client_intent_id, "t1");

    timeout(
        WAIT,
        ws.send(Message::Binary(vec![0u8; 64 * 1024 + 1].into())),
    )
    .await
    .unwrap()
    .unwrap();
    expect_closed(&mut ws).await;
}
