//! Asset upload over real HTTP and the WebSocket (ADR 0014). A tiny raw
//! HTTP/1.1 client over `TcpStream` keeps the test dependencies as they are.
//! Every network wait has a timeout so a bug fails a test instead of hanging.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tt_asset_pipeline::{inspect, MAX_FILE_BYTES};
use tt_contracts::v1::{
    client_message, event, intent, server_message, AssetRef, ClientMessage, Event, Hello, Intent,
    IntentRejected, Level, Member, Role, Scene, ServerMessage, SetLevelBackgroundIntent, Snapshot,
};
use tt_server::assets::AssetConfig;
use tt_server::{serve, Config};

const WAIT: Duration = Duration::from_secs(10);

const PNG: &[u8] = include_bytes!("../../../integrations/asset-pipeline/tests/samples/tiny.png");
const JPG: &[u8] = include_bytes!("../../../integrations/asset-pipeline/tests/samples/map.jpg");
const WEBP: &[u8] =
    include_bytes!("../../../integrations/asset-pipeline/tests/samples/icon-lossless.webp");

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

/// A unique temp directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("tt-assets-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
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

async fn start_with(dev_auth: bool, assets: Option<AssetConfig>) -> Server {
    start_bound("127.0.0.1:0", dev_auth, assets).await
}

async fn start_bound(bind: &str, dev_auth: bool, assets: Option<AssetConfig>) -> Server {
    let listener = TcpListener::bind(bind).await.expect("bind");
    let mut addr = listener.local_addr().expect("addr");
    if addr.ip().is_unspecified() {
        addr.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
    }
    let handle = tokio::spawn(serve(
        listener,
        Config {
            seed: seed(),
            dev_auth,
            media: None,
            assets,
            rolls: None,
        },
    ));
    Server { addr, handle }
}

async fn start(dir: &TempDir) -> Server {
    start_with(
        true,
        Some(AssetConfig {
            dir: dir.0.clone(),
            public_url: None,
        }),
    )
    .await
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

    fn asset_ref(&self) -> AssetRef {
        assert_eq!(self.status, 200, "{}", String::from_utf8_lossy(&self.body));
        assert_eq!(self.header("content-type"), Some("application/x-protobuf"));
        AssetRef::decode(&self.body[..]).expect("AssetRef")
    }

    fn assert_cors(&self) {
        assert_eq!(self.header("access-control-allow-origin"), Some("*"));
    }
}

/// Send one request with `Connection: close`. `body_chunks` are written one
/// after another; a write error (the server answering early and closing, as
/// for 413) is tolerated because the response is read concurrently.
async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body_chunks: &[&[u8]],
) -> Resp {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let mut head = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        head.push_str("Host: test.local:1234\r\n");
    }
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

async fn upload(addr: SocketAddr, token: Option<&str>, campaign: &str, body: &[u8]) -> Resp {
    let mut headers = vec![("Content-Length", body.len().to_string())];
    if let Some(t) = token {
        headers.push(("Authorization", format!("Bearer {t}")));
    }
    http(
        addr,
        "POST",
        &format!("/campaigns/{campaign}/assets"),
        &headers,
        &[body],
    )
    .await
}

async fn upload_with_host(addr: SocketAddr, host: &str, body: &[u8]) -> Resp {
    let headers = [
        ("Host", host.to_string()),
        ("Content-Length", body.len().to_string()),
        ("Authorization", "Bearer dev:gm".to_string()),
    ];
    http(addr, "POST", "/campaigns/camp/assets", &headers, &[body]).await
}

async fn gm_upload(addr: SocketAddr, body: &[u8]) -> AssetRef {
    upload(addr, Some("dev:gm"), "camp", body).await.asset_ref()
}

fn hex_ext_name(a: &AssetRef) -> String {
    let ext = match a.media_type.as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        other => panic!("unexpected {other}"),
    };
    format!("{}.{ext}", a.asset_id)
}

// ---------------------------------------------------------------- HTTP tests

#[tokio::test]
async fn uploads_off_answers_503_and_get_404() {
    let server = start_with(true, None).await;
    let r = upload(server.addr, Some("dev:gm"), "camp", PNG).await;
    assert_eq!(r.status, 503);
    assert_eq!(r.json_code(), "uploads_off");
    r.assert_cors();
    let name = format!("{}.png", "a".repeat(64));
    let r = http(server.addr, "GET", &format!("/assets/{name}"), &[], &[]).await;
    assert_eq!(r.status, 404);
    assert_eq!(r.json_code(), "unknown_asset");
    r.assert_cors();
}

#[tokio::test]
async fn bad_credentials_are_401() {
    let dir = TempDir::new("auth401");
    let server = start(&dir).await;
    let len = PNG.len().to_string();
    let basic = vec![
        ("Content-Length", len.clone()),
        ("Authorization", "Basic dev:gm".to_string()),
    ];
    let cases: Vec<(&str, Resp)> = vec![
        ("no header", upload(server.addr, None, "camp", PNG).await),
        (
            "wrong scheme",
            http(
                server.addr,
                "POST",
                "/campaigns/camp/assets",
                &basic,
                &[PNG],
            )
            .await,
        ),
        (
            "bad dev token",
            upload(server.addr, Some("gm"), "camp", PNG).await,
        ),
        (
            "unknown member",
            upload(server.addr, Some("dev:ghost"), "camp", PNG).await,
        ),
    ];
    for (what, r) in cases {
        assert_eq!(r.status, 401, "{what}");
        assert_eq!(r.json_code(), "unauthorized", "{what}");
        r.assert_cors();
    }
    // Nothing was stored.
    assert_eq!(std::fs::read_dir(&dir.0).expect("dir").count(), 0);

    // With dev auth off no token is accepted.
    let dir2 = TempDir::new("auth401-off");
    let off = start_with(
        false,
        Some(AssetConfig {
            dir: dir2.0.clone(),
            public_url: None,
        }),
    )
    .await;
    let r = upload(off.addr, Some("dev:gm"), "camp", PNG).await;
    assert_eq!(r.status, 401);
}

/// Auth must run before the body is read: the head promises a full-size body
/// that never arrives, and the answer must still come back in time.
#[tokio::test]
async fn auth_is_checked_before_the_body_is_read() {
    let dir = TempDir::new("auth-first");
    let server = start(&dir).await;
    let len = MAX_FILE_BYTES.to_string();
    for (token, status, code) in [
        (None, 401, "unauthorized"),
        (Some("dev:nobody"), 401, "unauthorized"),
        (Some("dev:alice"), 403, "not_permitted"),
    ] {
        let mut headers = vec![("Content-Length", len.clone())];
        if let Some(t) = token {
            headers.push(("Authorization", format!("Bearer {t}")));
        }
        let r = http(server.addr, "POST", "/campaigns/camp/assets", &headers, &[]).await;
        assert_eq!((r.status, r.json_code().as_str()), (status, code));
    }
}

#[tokio::test]
async fn non_gm_is_403_and_wrong_campaign_is_404() {
    let dir = TempDir::new("auth403");
    let server = start(&dir).await;
    for who in ["dev:alice", "dev:spec"] {
        let r = upload(server.addr, Some(who), "camp", PNG).await;
        assert_eq!(r.status, 403, "{who}");
        assert_eq!(r.json_code(), "not_permitted");
        r.assert_cors();
    }
    let r = upload(server.addr, Some("dev:gm"), "other", PNG).await;
    assert_eq!(r.status, 404);
    assert_eq!(r.json_code(), "unknown_campaign");
    r.assert_cors();
    assert_eq!(std::fs::read_dir(&dir.0).expect("dir").count(), 0);
}

#[tokio::test]
async fn pipeline_rejections_are_400_with_the_pipeline_code() {
    let dir = TempDir::new("reject400");
    let server = start(&dir).await;
    let svg: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>";
    let cases: [(&str, &[u8], &str); 3] = [
        ("empty", b"", "empty_file"),
        ("svg", svg, "disallowed_format"),
        ("garbage", b"this is not an image at all", "unknown_format"),
    ];
    for (what, body, code) in cases {
        let r = upload(server.addr, Some("dev:gm"), "camp", body).await;
        assert_eq!(r.status, 400, "{what}");
        assert_eq!(r.json_code(), code, "{what}");
        r.assert_cors();
    }
    assert_eq!(std::fs::read_dir(&dir.0).expect("dir").count(), 0);
}

#[tokio::test]
async fn over_the_limit_is_413_by_length_and_when_chunked() {
    let dir = TempDir::new("big413");
    let server = start(&dir).await;
    // Declared over the limit: refused from the headers alone, so the test
    // sends only a little of the body (the server never reads it).
    let declared = (MAX_FILE_BYTES + 1).to_string();
    let headers = [
        ("Content-Length", declared),
        ("Authorization", "Bearer dev:gm".to_string()),
    ];
    let r = http(
        server.addr,
        "POST",
        "/campaigns/camp/assets",
        &headers,
        &[&[0u8; 1024]],
    )
    .await;
    assert_eq!(r.status, 413);
    assert_eq!(r.json_code(), "file_too_large");
    r.assert_cors();

    // No Content-Length: the limit is enforced while reading.
    let chunk = vec![b'a'; 1024 * 1024];
    let mut body: Vec<u8> = Vec::new();
    for _ in 0..20 {
        body.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        body.extend_from_slice(&chunk);
        body.extend_from_slice(b"\r\n");
    }
    // One byte past the limit, then the end marker.
    body.extend_from_slice(b"1\r\nx\r\n0\r\n\r\n");
    let headers = [
        ("Transfer-Encoding", "chunked".to_string()),
        ("Authorization", "Bearer dev:gm".to_string()),
    ];
    let r = http(
        server.addr,
        "POST",
        "/campaigns/camp/assets",
        &headers,
        &[&body],
    )
    .await;
    assert_eq!(r.status, 413, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json_code(), "file_too_large");
}

#[tokio::test]
async fn exactly_at_the_limit_is_not_rejected_for_size() {
    let dir = TempDir::new("limit");
    let server = start(&dir).await;
    // A PNG signature padded with zeros: the pipeline may refuse it, but not
    // for size.
    let mut body = vec![0u8; MAX_FILE_BYTES as usize];
    body[..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let r = upload(server.addr, Some("dev:gm"), "camp", &body).await;
    assert_ne!(r.status, 413);
    if r.status != 200 {
        assert_ne!(r.json_code(), "file_too_large");
    }
}

#[tokio::test]
async fn png_jpeg_and_webp_are_stored_and_served_back() {
    let dir = TempDir::new("ok200");
    let server = start(&dir).await;
    for (bytes, media_type) in [
        (PNG, "image/png"),
        (JPG, "image/jpeg"),
        (WEBP, "image/webp"),
    ] {
        let want = inspect(bytes).expect("sample is valid");
        // Lower-case scheme and a Host header: link is built from Host.
        let r = upload(server.addr, Some("dev:gm"), "camp", bytes).await;
        r.assert_cors();
        let got = r.asset_ref();
        assert_eq!(got.media_type, media_type);
        assert_eq!(got.asset_id, want.asset_id);
        assert_eq!((got.width, got.height), (want.width, want.height));
        let name = hex_ext_name(&got);
        assert_eq!(got.url, format!("http://test.local:1234/assets/{name}"));
        assert!(dir.0.join(&name).is_file(), "{name} on disk");
        assert_eq!(std::fs::read(dir.0.join(&name)).expect("read"), bytes);

        // Same file again: same AssetRef, still one file.
        let again = upload(server.addr, Some("dev:gm"), "camp", bytes)
            .await
            .asset_ref();
        assert_eq!(again, got);
        // Same file via another Host: the registered AssetRef, link included.
        let other = upload_with_host(server.addr, "lan.example:99", bytes)
            .await
            .asset_ref();
        assert_eq!(other, got);

        // Serve it back.
        let g = http(server.addr, "GET", &format!("/assets/{name}"), &[], &[]).await;
        assert_eq!(g.status, 200);
        assert_eq!(g.body, bytes);
        assert_eq!(g.header("content-type"), Some(media_type));
        assert_eq!(
            g.header("cache-control"),
            Some("public, max-age=31536000, immutable")
        );
        assert_eq!(g.header("x-content-type-options"), Some("nosniff"));
        g.assert_cors();
    }
    let files: Vec<_> = std::fs::read_dir(&dir.0)
        .expect("dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files.len(), 3, "{files:?}");
    assert!(files.iter().all(|f| !f.ends_with(".tmp")), "{files:?}");
}

#[tokio::test]
async fn scheme_is_case_insensitive_and_public_url_is_used() {
    let dir = TempDir::new("public");
    let server = start_with(
        true,
        Some(AssetConfig {
            dir: dir.0.clone(),
            public_url: Some("https://cdn.example.org/base".into()),
        }),
    )
    .await;
    let headers = [
        ("Content-Length", PNG.len().to_string()),
        ("Authorization", "bEaReR dev:gm".to_string()),
    ];
    let r = http(
        server.addr,
        "POST",
        "/campaigns/camp/assets",
        &headers,
        &[PNG],
    )
    .await;
    let got = r.asset_ref();
    assert_eq!(
        got.url,
        format!("https://cdn.example.org/base/assets/{}.png", got.asset_id)
    );
}

#[tokio::test]
async fn get_with_bad_names_is_404_json_with_cors() {
    let dir = TempDir::new("badnames");
    let server = start(&dir).await;
    let got = gm_upload(server.addr, PNG).await;
    let id = got.asset_id.clone();
    let names = [
        format!("{}.png", id.to_uppercase()),
        format!("{}.png", &id[1..]),
        format!("{id}.gif"),
        format!("{id}.jpg.png"),
        format!("..%2F{id}.png"),
        "..%2F..%2FCargo.toml".to_string(),
        format!("%2e%2e/{id}.png"),
        format!("{}.png", "0".repeat(64)),
        "nope".to_string(),
    ];
    for name in names {
        let r = http(server.addr, "GET", &format!("/assets/{name}"), &[], &[]).await;
        assert_eq!(r.status, 404, "{name}");
        assert_eq!(r.json_code(), "unknown_asset", "{name}");
        r.assert_cors();
    }
    // The real one still works.
    let r = http(
        server.addr,
        "GET",
        &format!("/assets/{}", hex_ext_name(&got)),
        &[],
        &[],
    )
    .await;
    assert_eq!(r.status, 200);
}

#[tokio::test]
async fn options_is_204_and_other_methods_are_405_json() {
    let dir = TempDir::new("methods");
    let server = start(&dir).await;
    for path in ["/campaigns/camp/assets", "/assets/x.png"] {
        let r = http(server.addr, "OPTIONS", path, &[], &[]).await;
        assert_eq!(r.status, 204, "{path}");
        assert_eq!(
            r.header("access-control-allow-methods"),
            Some("GET, POST, OPTIONS")
        );
        assert_eq!(
            r.header("access-control-allow-headers"),
            Some("Authorization, Content-Type")
        );
        assert_eq!(r.header("access-control-max-age"), Some("600"));
        r.assert_cors();
    }
    for (method, path) in [
        ("DELETE", "/campaigns/camp/assets"),
        ("GET", "/campaigns/camp/assets"),
        ("POST", "/assets/x.png"),
        ("PUT", "/assets/x.png"),
    ] {
        let r = http(server.addr, method, path, &[], &[]).await;
        assert_eq!(r.status, 405, "{method} {path}");
        assert_eq!(r.json_code(), "method_not_allowed");
        r.assert_cors();
    }
}

// A stalled body read is bounded by BODY_READ_TIMEOUT (120 s) in upload.rs so
// a dead client cannot keep an upload slot; not tested end to end because it
// would need a two minute wait or injected time.
// The 503 `busy` answer is covered by the unit test of `UploadGate` in
// assets.rs: holding four real uploads open at once over HTTP would make the
// test depend on socket timing.

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

    async fn set_background(&mut self, id: &str, scene: &str, level: &str, asset: &str) {
        self.send(client_message::Kind::Intent(Intent {
            client_intent_id: id.into(),
            kind: Some(intent::Kind::SetLevelBackground(SetLevelBackgroundIntent {
                scene_id: scene.into(),
                level_id: level.into(),
                asset_id: asset.into(),
            })),
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

    async fn recv_rejection(&mut self) -> IntentRejected {
        match self.recv().await.kind {
            Some(server_message::Kind::IntentRejected(r)) => r,
            other => panic!("expected a rejection, got {other:?}"),
        }
    }
}

fn background_in(e: &Event) -> Option<AssetRef> {
    match e.kind.as_ref() {
        Some(event::Kind::SceneUpserted(u)) => u
            .scene
            .as_ref()
            .expect("scene")
            .levels
            .iter()
            .find(|l| l.level_id == "l1")
            .expect("level l1")
            .background
            .clone(),
        other => panic!("expected SceneUpserted, got {other:?}"),
    }
}

fn snapshot_background(s: &Snapshot) -> Option<AssetRef> {
    s.scenes[0].levels[0].background.clone()
}

#[tokio::test]
async fn upload_then_set_background_reaches_every_viewer() {
    let dir = TempDir::new("ws-flow");
    let server = start(&dir).await;
    let (mut gm, _) = Client::connect(server.addr, "dev:gm").await;
    let (mut alice, _) = Client::connect(server.addr, "dev:alice").await;

    let asset = gm_upload(server.addr, PNG).await;
    gm.set_background("i1", "s1", "l1", &asset.asset_id).await;
    for client in [&mut gm, &mut alice] {
        let e = client.recv_event().await;
        assert_eq!(e.scene_id, "s1");
        assert_eq!(background_in(&e), Some(asset.clone()));
    }

    // A new viewer's Snapshot has it.
    let (_, snap) = Client::connect(server.addr, "dev:alice").await;
    assert_eq!(snapshot_background(&snap), Some(asset.clone()));

    // Empty id clears it for everyone.
    gm.set_background("i2", "s1", "l1", "").await;
    for client in [&mut gm, &mut alice] {
        let e = client.recv_event().await;
        assert_eq!(background_in(&e), None);
    }
    let (_, snap) = Client::connect(server.addr, "dev:gm").await;
    assert_eq!(snapshot_background(&snap), None);
}

#[tokio::test]
async fn set_background_rejections_have_stable_codes() {
    let dir = TempDir::new("ws-reject");
    let server = start(&dir).await;
    let (mut gm, _) = Client::connect(server.addr, "dev:gm").await;
    let (mut alice, _) = Client::connect(server.addr, "dev:alice").await;
    let asset = gm_upload(server.addr, PNG).await;

    alice
        .set_background("p1", "s1", "l1", &asset.asset_id)
        .await;
    let r = alice.recv_rejection().await;
    assert_eq!(
        (r.code.as_str(), r.client_intent_id.as_str()),
        ("not_permitted", "p1")
    );

    let cases = [
        ("nope", "l1", asset.asset_id.as_str(), "unknown_scene"),
        ("s1", "nope", asset.asset_id.as_str(), "unknown_level"),
        ("s1", "l1", "deadbeef", "unknown_asset"),
    ];
    for (scene, level, id, code) in cases {
        gm.set_background("g", scene, level, id).await;
        assert_eq!(gm.recv_rejection().await.code, code);
    }

    // Nothing changed: a new snapshot has no background.
    let (_, snap) = Client::connect(server.addr, "dev:gm").await;
    assert_eq!(snapshot_background(&snap), None);
}

#[tokio::test]
async fn an_asset_registered_by_an_earlier_run_survives_a_restart() {
    let dir = TempDir::new("restart");
    let asset = {
        let first = start(&dir).await;
        gm_upload(first.addr, JPG).await
    };
    // Leftovers that must be ignored: a foreign file, a mismatched name.
    std::fs::write(dir.0.join("notes.txt"), b"hello").expect("write");
    std::fs::write(dir.0.join(format!("{}.png", "1".repeat(64))), PNG).expect("write");

    let second = start(&dir).await;
    let (mut gm, _) = Client::connect(second.addr, "dev:gm").await;
    gm.set_background("r1", "s1", "l1", &asset.asset_id).await;
    let e = gm.recv_event().await;
    let bg = background_in(&e).expect("background set");
    assert_eq!(bg.asset_id, asset.asset_id);
    assert_eq!(bg.media_type, "image/jpeg");
    assert_eq!(
        bg.url,
        format!("http://{}/assets/{}", second.addr, hex_ext_name(&asset))
    );

    // The mismatched file was not registered.
    gm.set_background("r2", "s1", "l1", &"1".repeat(64)).await;
    assert_eq!(gm.recv_rejection().await.code, "unknown_asset");
}

#[tokio::test]
async fn a_restart_link_is_provisional_and_a_reupload_replaces_it() {
    let dir = TempDir::new("provisional");
    let asset = {
        let first = start(&dir).await;
        gm_upload(first.addr, JPG).await
    };
    let second = start(&dir).await;
    let name = hex_ext_name(&asset);
    // The scan registered a guess from the listener address.
    let (mut gm, _) = Client::connect(second.addr, "dev:gm").await;
    gm.set_background("a", "s1", "l1", &asset.asset_id).await;
    let bg = background_in(&gm.recv_event().await).expect("bg");
    assert_eq!(bg.url, format!("http://{}/assets/{name}", second.addr));
    // Uploading again gives the Host-based link and keeps it from then on.
    let fixed = upload_with_host(second.addr, "lan.example:99", JPG)
        .await
        .asset_ref();
    assert_eq!(fixed.url, format!("http://lan.example:99/assets/{name}"));
    let later = upload_with_host(second.addr, "other.example", JPG)
        .await
        .asset_ref();
    assert_eq!(later, fixed);
    gm.set_background("b", "s1", "l1", &asset.asset_id).await;
    assert_eq!(background_in(&gm.recv_event().await), Some(fixed));
}

#[tokio::test]
async fn a_file_whose_extension_does_not_match_its_format_is_not_registered() {
    let dir = TempDir::new("ext-mismatch");
    // Real JPEG bytes under their own hash but a .png name.
    let id = inspect(JPG).expect("valid").asset_id;
    std::fs::write(dir.0.join(format!("{id}.png")), JPG).expect("write");
    let server = start(&dir).await;
    let (mut gm, _) = Client::connect(server.addr, "dev:gm").await;
    gm.set_background("m", "s1", "l1", &id).await;
    assert_eq!(gm.recv_rejection().await.code, "unknown_asset");
}
