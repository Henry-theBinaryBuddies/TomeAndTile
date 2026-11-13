//! WebSocket integration tests: real clients against `tt_server::serve` on an
//! ephemeral port. Every socket read is wrapped in a timeout so a bug fails a
//! test instead of hanging it. "Nothing arrives" is checked with a sentinel:
//! a public chat sent afterwards must be the very next message.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tt_contracts::v1::{
    client_message, event, intent, server_message, ChatIntent, ClientMessage, Creature, Event,
    Hello, Intent, Level, Member, MoveTokenIntent, Role, RollDiceIntent, RollMode, RollVisibility,
    Scene, ServerMessage, SetControllersIntent, Snapshot, Token, Vec2,
};
use tt_server::net::MAX_MESSAGE_BYTES;
use tt_server::{serve, Config};

const WAIT: Duration = Duration::from_secs(5);

// ------------------------------------------------------------------ fixture

fn member(id: &str, role: Role) -> Member {
    Member {
        user_id: id.into(),
        display_name: id.into(),
        role: role as i32,
        online: false,
    }
}

fn token(id: &str, creature: &str, controllers: &[&str], hidden: bool) -> Token {
    Token {
        token_id: id.into(),
        scene_id: "s1".into(),
        level_id: "l1".into(),
        creature_id: creature.into(),
        name: id.into(),
        position: Some(Vec2 { x: 105.0, y: 105.0 }),
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
        hp_current: 10,
        hp_max: 12,
        ..Default::default()
    }
}

/// Campaign "camp": gm; players alice and bob; spectator spec. Tokens:
/// t-alice (alice), t-goblin (nobody), t-hidden (nobody, hidden).
fn seed() -> Snapshot {
    Snapshot {
        members: vec![
            member("gm", Role::Gm),
            member("alice", Role::Player),
            member("bob", Role::Player),
            member("spec", Role::Spectator),
        ],
        scenes: vec![Scene {
            scene_id: "s1".into(),
            campaign_id: "camp".into(),
            name: "s1".into(),
            levels: vec![Level {
                level_id: "l1".into(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        tokens: vec![
            token("t-alice", "c-alice", &["alice"], false),
            token("t-goblin", "c-goblin", &[], false),
            token("t-hidden", "c-hidden", &[], true),
        ],
        creatures: vec![
            creature("c-alice", "alice"),
            creature("c-goblin", ""),
            creature("c-hidden", ""),
        ],
        active_scene_id: "s1".into(),
        ..Default::default()
    }
}

// ------------------------------------------------------------------ helpers

/// A running server; aborted on drop so no task outlives its test.
struct Server {
    addr: SocketAddr,
    handle: JoinHandle<std::io::Result<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn start(dev_auth: bool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(serve(
        listener,
        Config {
            seed: seed(),
            dev_auth,
        },
    ));
    Server { addr, handle }
}

struct Client {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

fn client_message(kind: client_message::Kind) -> Vec<u8> {
    ClientMessage { kind: Some(kind) }.encode_to_vec()
}

fn hello(token: &str, campaign_id: &str) -> Vec<u8> {
    client_message(client_message::Kind::Hello(Hello {
        campaign_id: campaign_id.into(),
        session_token: token.into(),
        last_seq: 0,
    }))
}

fn intent_bytes(id: &str, kind: intent::Kind) -> Vec<u8> {
    client_message(client_message::Kind::Intent(Intent {
        client_intent_id: id.into(),
        kind: Some(kind),
    }))
}

fn chat_kind(text: &str) -> intent::Kind {
    intent::Kind::Chat(ChatIntent { text: text.into() })
}

fn move_kind(token_id: &str) -> intent::Kind {
    intent::Kind::MoveToken(MoveTokenIntent {
        token_id: token_id.into(),
        path: vec![Vec2 { x: 175.0, y: 105.0 }],
        level_id: String::new(),
    })
}

fn roll_kind(visibility: RollVisibility, label: &str) -> intent::Kind {
    intent::Kind::RollDice(RollDiceIntent {
        notation: "1d20".into(),
        mode: RollMode::Normal as i32,
        visibility: visibility as i32,
        label: label.into(),
    })
}

impl Client {
    async fn connect(addr: SocketAddr) -> Client {
        let url = format!("ws://{addr}/ws");
        let (ws, _) = tokio::time::timeout(WAIT, connect_async(url))
            .await
            .expect("connect in time")
            .expect("handshake");
        Client { ws }
    }

    async fn send_raw(&mut self, bytes: Vec<u8>) {
        self.ws
            .send(Message::Binary(bytes.into()))
            .await
            .expect("send");
    }

    async fn send_intent(&mut self, id: &str, kind: intent::Kind) {
        self.send_raw(intent_bytes(id, kind)).await;
    }

    /// Next server message; panics on close, error or timeout.
    async fn recv(&mut self) -> ServerMessage {
        loop {
            let next = tokio::time::timeout(WAIT, self.ws.next())
                .await
                .expect("a message in time");
            match next {
                Some(Ok(Message::Binary(bytes))) => {
                    return ServerMessage::decode(&bytes[..]).expect("decodes")
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                other => panic!("expected a binary message, got {other:?}"),
            }
        }
    }

    async fn recv_event(&mut self) -> Event {
        match self.recv().await.kind {
            Some(server_message::Kind::Event(e)) => e,
            other => panic!("expected an event, got {other:?}"),
        }
    }

    async fn recv_rejection(&mut self) -> tt_contracts::v1::IntentRejected {
        match self.recv().await.kind {
            Some(server_message::Kind::IntentRejected(r)) => r,
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    /// The next message must be this public chat (nothing came before it).
    async fn expect_chat(&mut self, text: &str) -> Event {
        let e = self.recv_event().await;
        match e.kind.as_ref() {
            Some(event::Kind::ChatPosted(c)) if c.text == text => e,
            other => panic!("expected chat {text:?} next, got {other:?}"),
        }
    }

    /// Wait for the server to end the connection; any data frame fails.
    async fn expect_closed(&mut self) {
        self.expect_closed_within(WAIT).await;
    }

    async fn expect_closed_within(&mut self, limit: Duration) {
        loop {
            let next = tokio::time::timeout(limit, self.ws.next())
                .await
                .expect("connection closed in time");
            match next {
                Some(Ok(Message::Binary(b))) => {
                    panic!("unexpected data on a closing socket: {b:?}")
                }
                Some(Ok(Message::Text(t))) => panic!("unexpected text: {t:?}"),
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue,
                // Close frame, clean end, or reset: all mean "closed".
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
            }
        }
    }
}

/// Connect and join as `user`; returns the client and its Snapshot.
async fn join(server: &Server, user: &str) -> (Client, Snapshot) {
    let mut c = Client::connect(server.addr).await;
    c.send_raw(hello(&format!("dev:{user}"), "")).await;
    match c.recv().await.kind {
        Some(server_message::Kind::Snapshot(s)) => (c, s),
        other => panic!("expected a snapshot for {user}, got {other:?}"),
    }
}

/// Send a public chat from `sender` and return its text.
async fn sentinel(sender: &mut Client, n: u32) -> String {
    let text = format!("sentinel-{n}");
    sender
        .send_intent(&format!("s-{n}"), chat_kind(&text))
        .await;
    text
}

fn token_in<'a>(s: &'a Snapshot, id: &str) -> Option<&'a Token> {
    s.tokens.iter().find(|t| t.token_id == id)
}

// -------------------------------------------------------------------- join

#[tokio::test]
async fn valid_hello_gets_snapshot_with_identity_and_role() {
    let server = start(true).await;
    for (user, role) in [
        ("gm", Role::Gm),
        ("alice", Role::Player),
        ("spec", Role::Spectator),
    ] {
        let (_c, snap) = join(&server, user).await;
        assert_eq!(snap.user_id, user);
        assert_eq!(snap.role, role as i32);
        assert_eq!(snap.members.len(), 4);
        assert_eq!(snap.active_scene_id, "s1");
    }
}

#[tokio::test]
async fn matching_campaign_id_is_accepted() {
    let server = start(true).await;
    let mut c = Client::connect(server.addr).await;
    c.send_raw(hello("dev:alice", "camp")).await;
    assert!(matches!(
        c.recv().await.kind,
        Some(server_message::Kind::Snapshot(_))
    ));
}

#[tokio::test]
async fn dev_auth_off_closes_without_snapshot() {
    let server = start(false).await;
    let mut c = Client::connect(server.addr).await;
    c.send_raw(hello("dev:gm", "")).await;
    c.expect_closed().await;
}

#[tokio::test]
async fn bad_hellos_are_closed_without_snapshot() {
    let server = start(true).await;
    let cases = [
        hello("dev:mallory", ""),      // unknown user
        hello("gm", ""),               // not a dev token
        hello("dev:", ""),             // empty dev id
        hello("", ""),                 // empty token
        hello("dev:gm", "other-camp"), // wrong campaign
    ];
    for bytes in cases {
        let mut c = Client::connect(server.addr).await;
        c.send_raw(bytes).await;
        c.expect_closed().await;
    }
}

#[tokio::test]
async fn first_frame_must_be_a_hello() {
    let server = start(true).await;
    let cases = [
        intent_bytes("x", chat_kind("hi")),
        vec![0xFF; 16], // garbage
        Vec::new(),     // decodes, but to a message with no kind
    ];
    for bytes in cases {
        let mut c = Client::connect(server.addr).await;
        c.send_raw(bytes).await;
        c.expect_closed().await;
    }
}

#[tokio::test]
async fn text_frame_before_hello_is_skipped() {
    let server = start(true).await;
    let mut c = Client::connect(server.addr).await;
    c.ws.send(Message::Text("hello?".into())).await.unwrap();
    c.send_raw(hello("dev:gm", "")).await;
    assert!(matches!(
        c.recv().await.kind,
        Some(server_message::Kind::Snapshot(_))
    ));
}

#[tokio::test]
async fn silent_socket_is_closed_by_the_hello_timeout() {
    let server = start(true).await;
    let mut c = Client::connect(server.addr).await;
    // Freeze the clock only now, so connecting ran on real time. With
    // nothing else to do, the runtime then jumps to the server's deadline.
    tokio::time::pause();
    let before = tokio::time::Instant::now();
    c.expect_closed_within(Duration::from_secs(60)).await;
    let waited = before.elapsed();
    assert!(
        waited >= Duration::from_secs(9) && waited <= Duration::from_secs(11),
        "closed after {waited:?} of virtual time"
    );
}

// -------------------------------------------------------------- broadcast

#[tokio::test]
async fn chat_reaches_everyone_with_the_same_seq() {
    let server = start(true).await;
    let (mut gm, _) = join(&server, "gm").await;
    let (mut alice, _) = join(&server, "alice").await;
    alice.send_intent("c1", chat_kind("  hello table ")).await;
    let a = alice.expect_chat("hello table").await;
    let g = gm.expect_chat("hello table").await;
    assert_eq!(a.seq, 1);
    assert_eq!(a.actor_user_id, "alice");
    assert_eq!(g.client_intent_id, "c1");
    assert_eq!(a, g);
}

#[tokio::test]
async fn rejected_intent_goes_only_to_the_sender() {
    let server = start(true).await;
    let (mut gm, _) = join(&server, "gm").await;
    let (mut alice, _) = join(&server, "alice").await;
    let (mut bob, _) = join(&server, "bob").await;
    // alice does not control the goblin.
    alice.send_intent("bad-1", move_kind("t-goblin")).await;
    let r = alice.recv_rejection().await;
    assert_eq!(r.client_intent_id, "bad-1");
    assert_eq!(r.code, "not_permitted");
    let s = sentinel(&mut alice, 1).await;
    // The sentinel is the first thing everyone else sees, and it is seq 1:
    // the rejection consumed no sequence number.
    assert_eq!(bob.expect_chat(&s).await.seq, 1);
    assert_eq!(gm.expect_chat(&s).await.seq, 1);
    alice.expect_chat(&s).await;
}

// -------------------------------------------------------------- filtering

#[tokio::test]
async fn snapshot_hides_hp_and_hidden_tokens_from_players() {
    let server = start(true).await;
    let (_g, gm_snap) = join(&server, "gm").await;
    let (_a, alice_snap) = join(&server, "alice").await;
    let (_b, bob_snap) = join(&server, "bob").await;

    assert_eq!(gm_snap.tokens.len(), 3);
    assert!(token_in(&gm_snap, "t-goblin").unwrap().hp_current.is_some());

    // alice: sees her own hp, not the goblin's, and no hidden token.
    assert!(token_in(&alice_snap, "t-hidden").is_none());
    let own = token_in(&alice_snap, "t-alice").unwrap();
    assert_eq!(
        (own.hp_current, own.hp_max, own.hp_temp),
        (Some(10), Some(12), Some(1))
    );
    let goblin = token_in(&alice_snap, "t-goblin").unwrap();
    assert_eq!(
        (goblin.hp_current, goblin.hp_max, goblin.hp_temp),
        (None, None, None)
    );

    // bob controls nothing: no hp at all, not even on alice's token.
    for t in &bob_snap.tokens {
        assert_eq!(
            (t.hp_current, t.hp_max, t.hp_temp),
            (None, None, None),
            "{}",
            t.token_id
        );
    }
}

#[tokio::test]
async fn gm_only_roll_reaches_the_gm_but_not_the_player() {
    let server = start(true).await;
    let (mut gm, _) = join(&server, "gm").await;
    let (mut alice, _) = join(&server, "alice").await;
    gm.send_intent("r1", roll_kind(RollVisibility::GmOnly, "secret"))
        .await;
    match gm.recv_event().await.kind {
        Some(event::Kind::DiceRolled(d)) => {
            let roll = d.roll.expect("roll");
            assert_eq!(roll.visibility, RollVisibility::GmOnly as i32);
            assert_eq!(roll.label, "secret");
        }
        other => panic!("expected the roll, got {other:?}"),
    }
    let s = sentinel(&mut gm, 1).await;
    alice.expect_chat(&s).await;
    gm.expect_chat(&s).await;
}

#[tokio::test]
async fn player_roll_is_forced_public() {
    let server = start(true).await;
    let (mut gm, _) = join(&server, "gm").await;
    let (mut alice, _) = join(&server, "alice").await;
    alice
        .send_intent("r1", roll_kind(RollVisibility::GmOnly, ""))
        .await;
    for c in [&mut alice, &mut gm] {
        match c.recv_event().await.kind {
            Some(event::Kind::DiceRolled(d)) => {
                assert_eq!(d.roll.unwrap().visibility, RollVisibility::Public as i32);
            }
            other => panic!("expected the roll, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn hidden_token_move_reaches_the_gm_only() {
    let server = start(true).await;
    let (mut gm, _) = join(&server, "gm").await;
    let (mut alice, _) = join(&server, "alice").await;
    gm.send_intent("m1", move_kind("t-hidden")).await;
    match gm.recv_event().await.kind {
        Some(event::Kind::TokenMoved(m)) => assert_eq!(m.token_id, "t-hidden"),
        other => panic!("expected the move, got {other:?}"),
    }
    let s = sentinel(&mut gm, 1).await;
    // alice's first message is the chat (seq 2): the move never reached her.
    assert_eq!(alice.expect_chat(&s).await.seq, 2);
    gm.expect_chat(&s).await;
}

#[tokio::test]
async fn set_controllers_delivers_token_and_creature_then_allows_moving() {
    let server = start(true).await;
    let (mut gm, _) = join(&server, "gm").await;
    let (mut alice, _) = join(&server, "alice").await;
    let (mut bob, bob_snap) = join(&server, "bob").await;
    assert!(token_in(&bob_snap, "t-goblin")
        .unwrap()
        .hp_current
        .is_none());
    assert!(!bob_snap
        .creatures
        .iter()
        .any(|c| c.creature_id == "c-goblin"));

    gm.send_intent(
        "sc1",
        intent::Kind::SetControllers(SetControllersIntent {
            token_id: "t-goblin".into(),
            controller_user_ids: vec!["bob".into()],
        }),
    )
    .await;

    // bob: the token with hp now visible, then the creature sheet.
    match bob.recv_event().await.kind {
        Some(event::Kind::TokenUpserted(u)) => {
            let t = u.token.expect("token");
            assert_eq!(t.token_id, "t-goblin");
            assert_eq!(t.controller_user_ids, vec!["bob".to_string()]);
            assert_eq!(t.hp_current, Some(10));
        }
        other => panic!("expected TokenUpserted, got {other:?}"),
    }
    match bob.recv_event().await.kind {
        Some(event::Kind::CreatureUpserted(u)) => {
            assert_eq!(u.creature.expect("creature").creature_id, "c-goblin");
        }
        other => panic!("expected CreatureUpserted, got {other:?}"),
    }
    // The GM gets both events too.
    gm.recv_event().await;
    gm.recv_event().await;
    // alice sees the token (hp still hidden) but not the creature sheet.
    match alice.recv_event().await.kind {
        Some(event::Kind::TokenUpserted(u)) => {
            let t = u.token.expect("token");
            assert_eq!((t.token_id.as_str(), t.hp_current), ("t-goblin", None));
        }
        other => panic!("expected TokenUpserted, got {other:?}"),
    }

    // bob can now move it, and everyone who may see it hears about it.
    bob.send_intent("mv1", move_kind("t-goblin")).await;
    for c in [&mut bob, &mut gm, &mut alice] {
        let e = c.recv_event().await;
        match e.kind {
            Some(event::Kind::TokenMoved(m)) => assert_eq!(m.token_id, "t-goblin"),
            other => panic!("expected TokenMoved, got {other:?}"),
        }
        assert_eq!(e.seq, 3);
    }
    // alice never got the creature sheet: her next message is the sentinel.
    let s = sentinel(&mut gm, 1).await;
    alice.expect_chat(&s).await;
}

#[tokio::test]
async fn seq_strictly_increases_across_intents() {
    let server = start(true).await;
    let (mut alice, snap) = join(&server, "alice").await;
    let (mut gm, _) = join(&server, "gm").await;
    for i in 0..6 {
        alice
            .send_intent(&format!("c{i}"), chat_kind(&format!("m{i}")))
            .await;
    }
    let mut last = snap.seq;
    for i in 0..6 {
        let e = alice.expect_chat(&format!("m{i}")).await;
        assert!(e.seq > last, "seq {} after {last}", e.seq);
        last = e.seq;
    }
    // A rejection in the middle does not disturb the numbering.
    alice.send_intent("bad", move_kind("t-goblin")).await;
    alice.send_intent("c-last", chat_kind("last")).await;
    alice.recv_rejection().await;
    let after = alice.expect_chat("last").await.seq;
    assert_eq!(after, last + 1);
    // The GM saw the same strictly increasing sequence.
    let mut gm_last = 0;
    for i in 0..6 {
        let e = gm.expect_chat(&format!("m{i}")).await;
        assert!(e.seq > gm_last);
        gm_last = e.seq;
    }
    assert_eq!(gm.expect_chat("last").await.seq, after);
}

// ------------------------------------------------------- hostile / robust

#[tokio::test]
async fn oversized_message_ends_the_connection() {
    let server = start(true).await;
    let big = intent_bytes("big", chat_kind(&"x".repeat(MAX_MESSAGE_BYTES + 1024)));
    assert!(big.len() > MAX_MESSAGE_BYTES);
    // After joining. The server may reset before the write finishes.
    let (mut c, _) = join(&server, "alice").await;
    let _ = c.ws.send(Message::Binary(big.clone().into())).await;
    c.expect_closed().await;
    // Before joining.
    let mut c = Client::connect(server.addr).await;
    let _ = c.ws.send(Message::Binary(big.into())).await;
    c.expect_closed().await;
}

#[tokio::test]
async fn garbage_after_join_closes_the_connection() {
    let server = start(true).await;
    let (mut c, _) = join(&server, "alice").await;
    c.send_raw(vec![0xFF; 16]).await;
    c.expect_closed().await;
}

#[tokio::test]
async fn text_frames_and_a_second_hello_are_ignored() {
    let server = start(true).await;
    let (mut gm, _) = join(&server, "gm").await;
    let (mut alice, _) = join(&server, "alice").await;
    alice
        .ws
        .send(Message::Text("not binary".into()))
        .await
        .unwrap();
    alice.send_raw(hello("dev:gm", "")).await; // would change identity if honoured
    alice.send_raw(hello("dev:mallory", "")).await; // would close if honoured
    alice.send_intent("c1", chat_kind("still here")).await;
    let e = alice.expect_chat("still here").await;
    assert_eq!(e.actor_user_id, "alice");
    gm.expect_chat("still here").await;
    // The second Hello did not make alice the GM.
    alice.send_intent("bad", move_kind("t-goblin")).await;
    assert_eq!(alice.recv_rejection().await.code, "not_permitted");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_that_stops_reading_is_dropped() {
    const MESSAGES: u32 = 12_000;
    let server = start(true).await;
    let (mut gm, _) = join(&server, "gm").await;
    let (mut slow, _) = join(&server, "alice").await;
    // `slow` never reads while the GM floods. Once its socket buffers are
    // full its connection task blocks in send and the 256-event broadcast
    // buffer overflows. The GM waits for each echo so it cannot lag itself.
    let text = "x".repeat(2000);
    for _ in 0..MESSAGES {
        gm.send_intent("flood", chat_kind(&text)).await;
        gm.expect_chat(&text).await;
    }
    // Drain what the slow client was sent; the stream must then end.
    let mut received = 0u32;
    loop {
        let next = tokio::time::timeout(WAIT, slow.ws.next())
            .await
            .expect("slow client sees the close in time");
        match next {
            Some(Ok(Message::Binary(_))) => received += 1,
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
            Some(Ok(_)) => continue,
        }
    }
    // The snapshot counts as one binary frame.
    assert!(
        received <= MESSAGES,
        "slow client got {received} of {MESSAGES} events: it was not dropped"
    );
    // The server and the GM are fine.
    gm.send_intent("after", chat_kind("alive")).await;
    gm.expect_chat("alive").await;
}
