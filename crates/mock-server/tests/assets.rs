//! Integration tests for the HTTP asset surface and `set_level_background`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream as StdTcpStream};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tt_contracts::v1::{
    client_message, event, intent, server_message, AssetRef, ClientMessage, Hello, Intent,
    IntentRejected, ServerMessage, SetLevelBackgroundIntent, Snapshot,
};
use tt_mock_server::dice::XorShift64Star;
use tt_mock_server::{MAX_ASSET_BYTES, MAX_STORED_ASSETS};

const WAIT: Duration = Duration::from_secs(5);
const SCENE: &str = "scene-mock";
const LEVEL: &str = "level-ground";

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

struct Reply {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    fn error_code(&self) -> String {
        let value: serde_json::Value = serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("error body is not JSON ({e}): {:?}", self.body));
        assert!(value["message"].is_string(), "error has a message");
        value["code"]
            .as_str()
            .expect("error has a code")
            .to_string()
    }

    fn asset_ref(&self) -> AssetRef {
        assert_eq!(self.status, 200, "upload failed: {:?}", self.body);
        AssetRef::decode(&self.body[..]).expect("body is an AssetRef")
    }
}

/// A tiny HTTP/1.1 client: one request, `Connection: close`, read to EOF.
/// Write errors are ignored because the server may answer 413 and close
/// before the whole body is sent.
async fn http(
    addr: SocketAddr,
    method: &'static str,
    path: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
) -> Reply {
    let path = path.to_string();
    let headers: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    tokio::task::spawn_blocking(move || {
        let mut stream = StdTcpStream::connect(addr).expect("connect");
        stream.set_read_timeout(Some(WAIT)).unwrap();
        stream.set_write_timeout(Some(WAIT)).unwrap();
        let mut head = format!("{method} {path} HTTP/1.1\r\n");
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
            head.push_str(&format!("Host: {addr}\r\n"));
        }
        for (k, v) in &headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        ));
        if stream.write_all(head.as_bytes()).is_ok() {
            for chunk in body.chunks(64 * 1024) {
                if stream.write_all(chunk).is_err() {
                    break;
                }
            }
        }
        let mut raw = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => raw.extend_from_slice(&buf[..n]),
            }
        }
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("response has a header block");
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .and_then(|l| l.split(' ').nth(1))
            .and_then(|s| s.parse().ok())
            .expect("status line");
        let headers = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        Reply {
            status,
            headers,
            body: raw[split + 4..].to_vec(),
        }
    })
    .await
    .expect("http task")
}

fn upload_path() -> &'static str {
    "/campaigns/campaign-mock/assets"
}

async fn upload_as(addr: SocketAddr, auth: &str, body: Vec<u8>) -> Reply {
    http(
        addr,
        "POST",
        upload_path(),
        &[("Authorization", auth)],
        body,
    )
    .await
}

async fn upload(addr: SocketAddr, body: Vec<u8>) -> Reply {
    upload_as(addr, "Bearer gm", body).await
}

fn png(seed: u32) -> Vec<u8> {
    let mut v = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    v.extend_from_slice(&seed.to_le_bytes());
    v.extend_from_slice(b"fake png body");
    v
}

fn jpeg() -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0];
    v.extend_from_slice(b"fake jpeg body");
    v
}

fn webp() -> Vec<u8> {
    let mut v = b"RIFF".to_vec();
    v.extend_from_slice(&[0x10, 0, 0, 0]);
    v.extend_from_slice(b"WEBPVP8 fake");
    v
}

fn assert_cors(reply: &Reply) {
    assert_eq!(reply.header("access-control-allow-origin"), Some("*"));
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

async fn set_background(ws: &mut Ws, id: &str, scene: &str, level: &str, asset: &str) {
    send(
        ws,
        client_message::Kind::Intent(Intent {
            client_intent_id: id.into(),
            kind: Some(intent::Kind::SetLevelBackground(SetLevelBackgroundIntent {
                scene_id: scene.into(),
                level_id: level.into(),
                asset_id: asset.into(),
            })),
        }),
    )
    .await;
}

async fn recv_rejected(ws: &mut Ws) -> IntentRejected {
    match recv(ws).await {
        server_message::Kind::IntentRejected(r) => r,
        other => panic!("expected a rejection, got {other:?}"),
    }
}

/// The next event must be `SceneUpserted`; returns the event and the first
/// scene's first level background.
async fn recv_scene_upserted(ws: &mut Ws) -> (tt_contracts::v1::Event, Option<AssetRef>) {
    let e = match recv(ws).await {
        server_message::Kind::Event(e) => e,
        other => panic!("expected an event, got {other:?}"),
    };
    let Some(event::Kind::SceneUpserted(u)) = e.kind.clone() else {
        panic!("expected scene_upserted, got {e:?}");
    };
    let scene = u.scene.expect("scene present");
    assert_eq!(scene.scene_id, SCENE);
    assert_eq!(e.scene_id, SCENE);
    (e, scene.levels[0].background.clone())
}

#[tokio::test]
async fn upload_without_authorization_is_401() {
    let addr = start_server().await;
    let r = http(addr, "POST", upload_path(), &[], png(1)).await;
    assert_eq!(r.status, 401);
    assert_eq!(r.error_code(), "unauthorized");
    assert_eq!(r.header("content-type"), Some("application/json"));
    assert_cors(&r);
}

#[tokio::test]
async fn upload_with_wrong_scheme_is_401() {
    let addr = start_server().await;
    let r = upload_as(addr, "Basic gm", png(1)).await;
    assert_eq!(r.status, 401);
    assert_eq!(r.error_code(), "unauthorized");
    assert_cors(&r);
}

#[tokio::test]
async fn upload_scheme_is_case_insensitive_and_allows_extra_spaces() {
    let addr = start_server().await;
    assert_eq!(upload_as(addr, "bEaReR   gm", png(1)).await.status, 200);
}

#[tokio::test]
async fn upload_with_player_token_is_403() {
    let addr = start_server().await;
    let r = upload_as(addr, "Bearer player", png(1)).await;
    assert_eq!(r.status, 403);
    assert_eq!(r.error_code(), "not_permitted");
    assert_cors(&r);
}

#[tokio::test]
async fn upload_empty_body_is_400() {
    let addr = start_server().await;
    let r = upload(addr, Vec::new()).await;
    assert_eq!(r.status, 400);
    assert_eq!(r.error_code(), "empty_file");
    assert_cors(&r);
}

#[tokio::test]
async fn upload_svg_is_400_unknown_format() {
    let addr = start_server().await;
    let r = upload(
        addr,
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(),
    )
    .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.error_code(), "unknown_format");
    assert_cors(&r);
    let gif = upload(addr, b"GIF89a....".to_vec()).await;
    assert_eq!(gif.status, 400);
    assert_eq!(gif.error_code(), "unknown_format");
}

#[tokio::test]
async fn upload_over_the_limit_is_413_json() {
    let addr = start_server().await;
    let mut body = png(1);
    body.resize(MAX_ASSET_BYTES + 1, 0);
    let r = upload(addr, body).await;
    assert_eq!(r.status, 413);
    assert_eq!(r.error_code(), "file_too_large");
    assert_cors(&r);
}

#[tokio::test]
async fn upload_of_exactly_the_limit_is_accepted() {
    let addr = start_server().await;
    let mut body = png(1);
    body.resize(MAX_ASSET_BYTES, 0);
    let asset = upload(addr, body).await.asset_ref();
    let r = http(
        addr,
        "GET",
        &format!("/assets/{}.png", asset.asset_id),
        &[],
        Vec::new(),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body.len(), MAX_ASSET_BYTES);
}

#[tokio::test]
async fn upload_url_falls_back_to_the_listener_for_a_bad_host() {
    let addr = start_server().await;
    for host in ["bad host", "h_st"] {
        let r = http(
            addr,
            "POST",
            upload_path(),
            &[("Authorization", "Bearer gm"), ("Host", host)],
            png(20),
        )
        .await;
        assert!(
            r.asset_ref()
                .url
                .starts_with(&format!("http://{addr}/assets/mock-")),
            "host {host:?}"
        );
    }
}

#[tokio::test]
async fn bad_percent_encoding_still_gets_json_and_cors() {
    let addr = start_server().await;
    let r = http(addr, "GET", "/assets/%FF.png", &[], Vec::new()).await;
    assert_eq!(r.status, 404);
    assert_eq!(r.error_code(), "unknown_asset");
    assert_eq!(r.header("content-type"), Some("application/json"));
    assert_cors(&r);
    let r = http(
        addr,
        "POST",
        "/campaigns/%FF/assets",
        &[("Authorization", "Bearer gm")],
        png(21),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_cors(&r);
}

#[tokio::test]
async fn unsupported_methods_get_json_405_with_cors() {
    let addr = start_server().await;
    for (method, path) in [
        ("PUT", upload_path()),
        ("GET", upload_path()),
        ("DELETE", "/assets/mock-0123456789abcdef.png"),
        ("POST", "/assets/mock-0123456789abcdef.png"),
    ] {
        let r = http(addr, method, path, &[], Vec::new()).await;
        assert_eq!(r.status, 405, "{method} {path}");
        assert_eq!(r.error_code(), "method_not_allowed");
        assert_eq!(r.header("content-type"), Some("application/json"));
        assert_cors(&r);
    }
}

#[tokio::test]
async fn upload_png_jpeg_and_webp_succeed() {
    let addr = start_server().await;
    for (body, media_type, ext) in [
        (png(7), "image/png", "png"),
        (jpeg(), "image/jpeg", "jpg"),
        (webp(), "image/webp", "webp"),
    ] {
        let r = upload_as(addr, "Bearer gm", body).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.header("content-type"), Some("application/x-protobuf"));
        assert_cors(&r);
        let asset = r.asset_ref();
        assert_eq!(asset.media_type, media_type);
        assert!(asset.asset_id.starts_with("mock-"));
        assert_eq!(asset.asset_id.len(), "mock-".len() + 16);
        assert_eq!((asset.width, asset.height), (0, 0));
        assert!(
            asset
                .url
                .ends_with(&format!("/assets/{}.{ext}", asset.asset_id)),
            "url {}",
            asset.url
        );
        assert!(asset.url.starts_with(&format!("http://{addr}/assets/")));
    }
}

#[tokio::test]
async fn upload_url_uses_the_host_header() {
    let addr = start_server().await;
    let r = http(
        addr,
        "POST",
        upload_path(),
        &[("Authorization", "Bearer gm"), ("Host", "mock.test:9000")],
        png(2),
    )
    .await;
    assert!(r
        .asset_ref()
        .url
        .starts_with("http://mock.test:9000/assets/mock-"));
}

#[tokio::test]
async fn same_bytes_give_the_same_asset_ref() {
    let addr = start_server().await;
    let a = upload(addr, png(3)).await.asset_ref();
    let b = upload(addr, png(3)).await.asset_ref();
    let c = upload(addr, png(4)).await.asset_ref();
    assert_eq!(a, b);
    assert_ne!(a.asset_id, c.asset_id);
}

#[tokio::test]
async fn get_returns_the_stored_bytes_with_headers() {
    let addr = start_server().await;
    let body = jpeg();
    let asset = upload(addr, body.clone()).await.asset_ref();
    let r = http(
        addr,
        "GET",
        &format!("/assets/{}.jpg", asset.asset_id),
        &[],
        Vec::new(),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, body);
    assert_eq!(r.header("content-type"), Some("image/jpeg"));
    assert_eq!(
        r.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    assert_cors(&r);
}

#[tokio::test]
async fn get_unknown_id_or_wrong_ext_is_404_json() {
    let addr = start_server().await;
    let asset = upload(addr, png(5)).await.asset_ref();
    for path in [
        "/assets/mock-0000000000000000.png".to_string(),
        format!("/assets/{}.jpg", asset.asset_id),
        format!("/assets/{}", asset.asset_id),
        format!("/assets/{}.png/extra", asset.asset_id),
    ] {
        let r = http(addr, "GET", &path, &[], Vec::new()).await;
        assert_eq!(r.status, 404, "{path}");
        assert_eq!(r.error_code(), "unknown_asset", "{path}");
        assert_eq!(r.header("content-type"), Some("application/json"));
        assert_cors(&r);
    }
}

#[tokio::test]
async fn options_preflight_answers_204_on_both_routes() {
    let addr = start_server().await;
    for path in [upload_path(), "/assets/mock-0000000000000000.png"] {
        let r = http(addr, "OPTIONS", path, &[], Vec::new()).await;
        assert_eq!(r.status, 204, "{path}");
        assert_cors(&r);
        assert_eq!(
            r.header("access-control-allow-methods"),
            Some("GET, POST, OPTIONS")
        );
        assert_eq!(
            r.header("access-control-allow-headers"),
            Some("Authorization, Content-Type")
        );
        assert_eq!(r.header("access-control-max-age"), Some("600"));
    }
}

#[tokio::test]
async fn storage_is_capped_but_reupload_still_works() {
    let addr = start_server().await;
    let first = upload(addr, png(0)).await.asset_ref();
    for seed in 1..MAX_STORED_ASSETS as u32 {
        assert_eq!(upload(addr, png(seed)).await.status, 200);
    }
    let full = upload(addr, png(MAX_STORED_ASSETS as u32)).await;
    assert_eq!(full.status, 507);
    assert_eq!(full.error_code(), "storage_full");
    assert_cors(&full);
    let again = upload(addr, png(0)).await;
    assert_eq!(again.status, 200);
    assert_eq!(again.asset_ref(), first);
}

#[tokio::test]
async fn gm_sets_background_and_everyone_sees_it() {
    let addr = start_server().await;
    let asset = upload(addr, png(10)).await.asset_ref();
    let (mut gm, _) = join(addr, "gm").await;
    let (mut player, _) = join(addr, "player").await;

    set_background(&mut gm, "bg1", SCENE, LEVEL, &asset.asset_id).await;
    for ws in [&mut gm, &mut player] {
        let (event, background) = recv_scene_upserted(ws).await;
        assert_eq!(background.as_ref(), Some(&asset));
        assert_eq!(event.seq, 1);
        assert_eq!(event.actor_user_id, "mock-gm");
        assert_eq!(event.client_intent_id, "bg1");
    }

    let (_late, snapshot) = join(addr, "player").await;
    assert_eq!(
        snapshot.scenes[0].levels[0].background.as_ref(),
        Some(&asset)
    );
}

#[tokio::test]
async fn empty_asset_id_clears_the_background() {
    let addr = start_server().await;
    let asset = upload(addr, png(11)).await.asset_ref();
    let (mut gm, _) = join(addr, "gm").await;
    set_background(&mut gm, "set", SCENE, LEVEL, &asset.asset_id).await;
    assert!(recv_scene_upserted(&mut gm).await.1.is_some());
    set_background(&mut gm, "clear", SCENE, LEVEL, "").await;
    let (event, background) = recv_scene_upserted(&mut gm).await;
    assert_eq!(background, None);
    assert_eq!(event.client_intent_id, "clear");
    let (_late, snapshot) = join(addr, "gm").await;
    assert_eq!(snapshot.scenes[0].levels[0].background, None);
}

#[tokio::test]
async fn player_cannot_set_background() {
    let addr = start_server().await;
    let asset = upload(addr, png(12)).await.asset_ref();
    let (mut player, _) = join(addr, "player").await;
    set_background(&mut player, "p1", SCENE, LEVEL, &asset.asset_id).await;
    let r = recv_rejected(&mut player).await;
    assert_eq!(r.code, "not_permitted");
    assert_eq!(r.client_intent_id, "p1");
}

#[tokio::test]
async fn background_rejections_name_the_right_problem() {
    let addr = start_server().await;
    let asset = upload(addr, png(13)).await.asset_ref();
    let (mut gm, _) = join(addr, "gm").await;
    let cases = [
        ("nope", LEVEL, asset.asset_id.as_str(), "unknown_scene"),
        (SCENE, "nope", asset.asset_id.as_str(), "unknown_level"),
        (SCENE, LEVEL, "mock-0000000000000000", "unknown_asset"),
    ];
    for (i, (scene, level, asset_id, code)) in cases.into_iter().enumerate() {
        let id = format!("bad{i}");
        set_background(&mut gm, &id, scene, level, asset_id).await;
        let r = recv_rejected(&mut gm).await;
        assert_eq!(r.code, code);
        assert_eq!(r.client_intent_id, id);
    }
}
