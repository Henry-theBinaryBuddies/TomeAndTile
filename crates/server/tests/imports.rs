//! Creature import over real HTTP and the WebSocket (ADR 0015). Same raw
//! HTTP/1.1 client as tests/assets.rs. Every network wait has a timeout so a
//! bug fails a test instead of hanging.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tt_contracts::v1::{
    client_message, event, intent, server_message, ChatIntent, ClientMessage, CreateTokenIntent,
    Creature, Event, Hello, ImportResult, Intent, Level, Member, Role, Scene, ServerMessage,
    Snapshot, Vec2,
};
use tt_server::{serve, Config};

const WAIT: Duration = Duration::from_secs(10);
const MAX_IMPORT_BYTES: usize = 1024 * 1024;

const ANVIL: [(&str, &str); 5] = [
    (
        "aldric",
        include_str!("../../../contracts/fixtures/anvil/aldric-thorne-fighter-3.json"),
    ),
    (
        "borin",
        include_str!("../../../contracts/fixtures/anvil/borin-stonehelm-fighter-4-wizard-1.json"),
    ),
    (
        "bronna",
        include_str!("../../../contracts/fixtures/anvil/bronna-ironhearth-cleric-3.json"),
    ),
    (
        "kael",
        include_str!("../../../contracts/fixtures/anvil/kael-wren-ranger-3.json"),
    ),
    (
        "mordecai",
        include_str!("../../../contracts/fixtures/anvil/mordecai-ash-warlock-3.json"),
    ),
];
const CAULDRON: &str =
    include_str!("../../../integrations/cauldron-import/tests/samples/monster-cinderkite.json");

// ------------------------------------------------------------------ fixture

fn member(id: &str, role: Role) -> Member {
    Member {
        user_id: id.into(),
        display_name: id.into(),
        role: role as i32,
        online: false,
    }
}

fn seed() -> Snapshot {
    Snapshot {
        members: vec![
            member("gm", Role::Gm),
            member("alice", Role::Player),
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
        active_scene_id: "s1".into(),
        ..Default::default()
    }
}

struct Server {
    addr: SocketAddr,
    handle: JoinHandle<std::io::Result<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Imports must work with uploads off, so no asset directory is given.
async fn start() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(serve(
        listener,
        Config {
            seed: seed(),
            dev_auth: true,
            media: None,
            assets: None,
            rolls: None,
        },
    ));
    Server { addr, handle }
}

// -------------------------------------------------------------- HTTP client

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn json_code(&self) -> String {
        assert_eq!(
            self.header("content-type"),
            Some("application/json"),
            "error body must be JSON"
        );
        let v: serde_json::Value = serde_json::from_slice(&self.body).expect("json body");
        assert!(v["message"].is_string(), "message present: {v}");
        v["code"].as_str().expect("code").to_string()
    }

    fn result(&self) -> ImportResult {
        assert_eq!(self.status, 200, "{}", String::from_utf8_lossy(&self.body));
        assert_eq!(self.header("content-type"), Some("application/x-protobuf"));
        self.assert_cors();
        ImportResult::decode(&self.body[..]).expect("ImportResult")
    }

    fn assert_cors(&self) {
        assert_eq!(self.header("access-control-allow-origin"), Some("*"));
    }
}

async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body_chunks: &[&[u8]],
) -> Resp {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let mut head = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
    head.push_str("Host: test.local:1234\r\n");
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let (mut rd, mut wr) = stream.split();
    let reader = async {
        let mut out = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            match rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
        }
        out
    };
    // A write error (the server answering early and closing, as for 413) is
    // tolerated because the response is read concurrently.
    let writer = async {
        if wr.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        for chunk in body_chunks {
            if wr.write_all(chunk).await.is_err() {
                return;
            }
        }
    };
    let (raw, ()) = tokio::time::timeout(WAIT, async { tokio::join!(reader, writer) })
        .await
        .expect("http exchange in time");
    parse(&raw)
}

fn parse(raw: &[u8]) -> Resp {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no header end in {:?}", String::from_utf8_lossy(raw)));
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|s| s.parse().ok())
        .expect("status");
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Resp {
        status,
        headers,
        body: raw[split + 4..].to_vec(),
    }
}

async fn import(addr: SocketAddr, token: Option<&str>, campaign: &str, body: &[u8]) -> Resp {
    let mut headers = vec![("Content-Length", body.len().to_string())];
    if let Some(t) = token {
        headers.push(("Authorization", format!("Bearer {t}")));
    }
    http(
        addr,
        "POST",
        &format!("/campaigns/{campaign}/imports"),
        &headers,
        &[body],
    )
    .await
}

async fn gm_import(addr: SocketAddr, body: &str) -> Resp {
    import(addr, Some("dev:gm"), "camp", body.as_bytes()).await
}

/// A fixture as a mutable JSON value.
fn json_of(text: &str) -> serde_json::Value {
    serde_json::from_str(text).expect("fixture is JSON")
}

// ----------------------------------------------------------------- HTTP tests

#[tokio::test]
async fn each_anvil_fixture_imports_with_200() {
    let server = start().await;
    for (name, text) in ANVIL {
        let result = gm_import(server.addr, text).await.result();
        let creature = result.creature.expect("creature");
        assert!(!result.replaced, "{name}");
        assert!(result.notes.is_empty(), "{name}");
        assert!(!creature.creature_id.is_empty(), "{name}");
        assert_eq!(creature.owner_user_id, "", "{name}");
        let original: Creature = serde_json::from_str(text).expect("fixture parses");
        assert_eq!(creature.name, original.name, "{name}");
        assert_eq!(creature.abilities, original.abilities, "{name}");
    }
}

#[tokio::test]
async fn importing_the_same_file_twice_replaces_and_keeps_the_id() {
    let server = start().await;
    let (_, text) = ANVIL[0];
    let first = gm_import(server.addr, text).await.result();
    let second = gm_import(server.addr, text).await.result();
    assert!(!first.replaced);
    assert!(second.replaced);
    assert_eq!(
        first.creature.expect("first").creature_id,
        second.creature.expect("second").creature_id
    );
    // A different file is a different creature.
    let other = gm_import(server.addr, ANVIL[1].1).await.result();
    assert!(!other.replaced);
}

#[tokio::test]
async fn a_client_chosen_id_and_owner_are_ignored() {
    let server = start().await;
    let mut v = json_of(ANVIL[3].1);
    v["creatureId"] = "chosen-by-client".into();
    v["ownerUserId"] = "alice".into();
    let r = gm_import(server.addr, &v.to_string()).await.result();
    let c = r.creature.expect("creature");
    assert_ne!(c.creature_id, "chosen-by-client");
    assert_eq!(c.owner_user_id, "");
}

#[tokio::test]
async fn bad_credentials_are_401_and_non_gm_is_403() {
    let server = start().await;
    let (_, text) = ANVIL[0];
    for (what, token) in [
        ("no header", None),
        ("bad dev token", Some("gm")),
        ("unknown member", Some("dev:ghost")),
    ] {
        let r = import(server.addr, token, "camp", text.as_bytes()).await;
        assert_eq!(r.status, 401, "{what}");
        assert_eq!(r.json_code(), "unauthorized", "{what}");
        r.assert_cors();
    }
    for who in ["dev:alice", "dev:spec"] {
        let r = import(server.addr, Some(who), "camp", text.as_bytes()).await;
        assert_eq!(r.status, 403, "{who}");
        assert_eq!(r.json_code(), "not_permitted", "{who}");
        r.assert_cors();
    }
    let r = import(server.addr, Some("dev:gm"), "other", text.as_bytes()).await;
    assert_eq!(r.status, 404);
    assert_eq!(r.json_code(), "unknown_campaign");
    r.assert_cors();
}

/// Auth must run before the body is read: the head promises a body that
/// never arrives, and the answer must still come back in time.
#[tokio::test]
async fn auth_is_checked_before_the_body_is_read() {
    let server = start().await;
    let len = MAX_IMPORT_BYTES.to_string();
    for (token, status, code) in [
        (None, 401, "unauthorized"),
        (Some("dev:alice"), 403, "not_permitted"),
    ] {
        let mut headers = vec![("Content-Length", len.clone())];
        if let Some(t) = token {
            headers.push(("Authorization", format!("Bearer {t}")));
        }
        let r = http(
            server.addr,
            "POST",
            "/campaigns/camp/imports",
            &headers,
            &[],
        )
        .await;
        assert_eq!((r.status, r.json_code().as_str()), (status, code));
    }
}

#[tokio::test]
async fn over_the_limit_is_413_by_length_and_when_chunked() {
    let server = start().await;
    let headers = [
        ("Content-Length", (MAX_IMPORT_BYTES + 1).to_string()),
        ("Authorization", "Bearer dev:gm".to_string()),
    ];
    let r = http(
        server.addr,
        "POST",
        "/campaigns/camp/imports",
        &headers,
        &[&[b' '; 1024]],
    )
    .await;
    assert_eq!(r.status, 413);
    assert_eq!(r.json_code(), "file_too_large");
    r.assert_cors();

    // No Content-Length: the limit is enforced while reading.
    let chunk = vec![b' '; 512 * 1024];
    let mut body: Vec<u8> = Vec::new();
    for _ in 0..2 {
        body.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        body.extend_from_slice(&chunk);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"1\r\nx\r\n0\r\n\r\n");
    let headers = [
        ("Transfer-Encoding", "chunked".to_string()),
        ("Authorization", "Bearer dev:gm".to_string()),
    ];
    let r = http(
        server.addr,
        "POST",
        "/campaigns/camp/imports",
        &headers,
        &[&body],
    )
    .await;
    assert_eq!(r.status, 413, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json_code(), "file_too_large");
}

#[tokio::test]
async fn a_cauldron_file_is_415_for_now() {
    let server = start().await;
    let r = gm_import(server.addr, CAULDRON).await;
    assert_eq!(r.status, 415);
    assert_eq!(r.json_code(), "format_not_supported_yet");
    r.assert_cors();
}

#[tokio::test]
async fn bad_creatures_are_400_and_change_nothing() {
    let server = start().await;
    let mut bad_dice = json_of(ANVIL[0].1);
    let attack_dice = bad_dice["abilities"]
        .as_array_mut()
        .expect("abilities")
        .iter_mut()
        .find_map(|a| a["steps"][0]["attack"]["damage"][0].as_object_mut())
        .expect("an attack ability");
    attack_dice.insert("dice".into(), "2d7+banana".into());
    let mut unknown_field = json_of(ANVIL[0].1);
    unknown_field["notAField"] = 1.into();
    let mut no_source = json_of(ANVIL[0].1);
    no_source["provenance"]["source"] = "".into();
    let mut hp_over = json_of(ANVIL[0].1);
    hp_over["hpCurrent"] = 999.into();
    let mut no_name = json_of(ANVIL[0].1);
    no_name["name"] = "".into();
    let cases: Vec<(&str, String)> = vec![
        ("bad dice", bad_dice.to_string()),
        ("unknown field", unknown_field.to_string()),
        ("no source", no_source.to_string()),
        ("hp over max", hp_over.to_string()),
        ("no name", no_name.to_string()),
        ("not JSON", "this is not json".to_string()),
        ("empty body", String::new()),
        ("an array", "[]".to_string()),
    ];
    for (what, body) in &cases {
        let r = gm_import(server.addr, body).await;
        assert_eq!(r.status, 400, "{what}");
        assert_eq!(r.json_code(), "bad_creature", "{what}");
        r.assert_cors();
    }

    // Nothing was stored or broadcast: the first good import is not a
    // replacement and is the first event of the campaign.
    let (mut gm, _) = Client::connect(server.addr, "dev:gm").await;
    let r = gm_import(server.addr, ANVIL[0].1).await.result();
    assert!(!r.replaced);
    let e = gm.recv_event().await;
    assert_eq!(e.seq, 1);
}

#[tokio::test]
async fn options_is_204_and_other_methods_are_405_json() {
    let server = start().await;
    let r = http(server.addr, "OPTIONS", "/campaigns/camp/imports", &[], &[]).await;
    assert_eq!(r.status, 204);
    assert_eq!(
        r.header("access-control-allow-headers"),
        Some("Authorization, Content-Type")
    );
    r.assert_cors();
    for method in ["GET", "PUT", "DELETE"] {
        let r = http(server.addr, method, "/campaigns/camp/imports", &[], &[]).await;
        assert_eq!(r.status, 405, "{method}");
        assert_eq!(r.json_code(), "method_not_allowed");
        r.assert_cors();
    }
}

// The 120 s body read timeout is shared with uploads and not tested end to
// end (it would need a two minute wait). The 503 `busy` answer is covered by
// the `UploadGate` unit test in assets.rs, which the import gate reuses.

// ----------------------------------------------------------- WebSocket tests

struct Client {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

impl Client {
    async fn connect(addr: SocketAddr, token: &str) -> (Client, Snapshot) {
        let (ws, _) = tokio::time::timeout(WAIT, connect_async(format!("ws://{addr}/ws")))
            .await
            .expect("connect in time")
            .expect("handshake");
        let mut c = Client { ws };
        c.send(client_message::Kind::Hello(Hello {
            campaign_id: "camp".into(),
            session_token: token.into(),
            last_seq: 0,
        }))
        .await;
        match c.recv().await.kind {
            Some(server_message::Kind::Snapshot(s)) => (c, s),
            other => panic!("expected a snapshot, got {other:?}"),
        }
    }

    async fn send(&mut self, kind: client_message::Kind) {
        let bytes = ClientMessage { kind: Some(kind) }.encode_to_vec();
        self.ws
            .send(Message::Binary(bytes.into()))
            .await
            .expect("send");
    }

    async fn send_intent(&mut self, id: &str, kind: intent::Kind) {
        self.send(client_message::Kind::Intent(Intent {
            client_intent_id: id.into(),
            kind: Some(kind),
        }))
        .await;
    }

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
}

fn upserted_creature(e: &Event) -> &Creature {
    match e.kind.as_ref() {
        Some(event::Kind::CreatureUpserted(u)) => u.creature.as_ref().expect("creature"),
        other => panic!("expected CreatureUpserted, got {other:?}"),
    }
}

#[tokio::test]
async fn the_gm_socket_gets_the_creature_and_a_player_gets_nothing() {
    let server = start().await;
    let (mut gm, _) = Client::connect(server.addr, "dev:gm").await;
    let (mut alice, _) = Client::connect(server.addr, "dev:alice").await;

    let result = gm_import(server.addr, ANVIL[3].1).await.result();
    let stored = result.creature.expect("creature");

    let e = gm.recv_event().await;
    assert_eq!(e.seq, 1);
    assert_eq!(e.actor_user_id, "gm");
    assert_eq!(e.client_intent_id, "");
    assert_eq!(e.scene_id, "s1");
    assert!(e.warnings.is_empty());
    assert_eq!(upserted_creature(&e), &stored);

    // The player's next message is the sentinel chat, not the creature.
    gm.send_intent(
        "c1",
        intent::Kind::Chat(ChatIntent {
            text: "sentinel".into(),
        }),
    )
    .await;
    let e = alice.recv_event().await;
    assert!(
        matches!(e.kind, Some(event::Kind::ChatPosted(_))),
        "the player received {e:?}"
    );
    assert_eq!(e.seq, 2);

    // A new player snapshot does not list it either.
    let (_, snap) = Client::connect(server.addr, "dev:alice").await;
    assert!(snap.creatures.is_empty());
    let (_, snap) = Client::connect(server.addr, "dev:gm").await;
    assert_eq!(snap.creatures, vec![stored]);
}

#[tokio::test]
async fn a_token_made_from_the_import_has_the_creatures_hit_points() {
    let server = start().await;
    let (mut gm, _) = Client::connect(server.addr, "dev:gm").await;
    let stored = gm_import(server.addr, ANVIL[1].1)
        .await
        .result()
        .creature
        .expect("creature");
    assert!(stored.hp_max > 0);
    gm.recv_event().await;

    gm.send_intent(
        "t1",
        intent::Kind::CreateToken(CreateTokenIntent {
            scene_id: "s1".into(),
            creature_id: stored.creature_id.clone(),
            position: Some(Vec2 { x: 105.0, y: 105.0 }),
            ..Default::default()
        }),
    )
    .await;
    let e = gm.recv_event().await;
    let token = match e.kind {
        Some(event::Kind::TokenUpserted(u)) => u.token.expect("token"),
        other => panic!("expected TokenUpserted, got {other:?}"),
    };
    assert_eq!(token.creature_id, stored.creature_id);
    assert_eq!(token.hp_max, Some(stored.hp_max));
    assert_eq!(token.hp_current, Some(stored.hp_current));
}
