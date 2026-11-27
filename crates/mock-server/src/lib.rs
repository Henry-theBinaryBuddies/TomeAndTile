//! Mock game server: speaks the real game-socket protocol from recorded data.
//!
//! It applies simplified rules and checks only the GM role and token control
//! (the README lists everything it skips). It redacts token hit points and
//! result cards (monster rolls, damage, sheet warnings) per
//! viewer and filters the creature sheets in the Snapshot, so the UI can
//! exercise the health-state view and the refusal paths; none of that is
//! security. It exists so the web UI and importers can be built before the
//! real server does. See the crate README.
//!
//! Besides the socket it serves a small HTTP asset surface (ADR 0014):
//! `POST /campaigns/{campaign_id}/assets` stores an image in memory and
//! `GET /assets/{asset_id}.{ext}` serves it back, both with CORS enabled. The
//! mock sniffs magic bytes only (PNG, JPEG, WebP); it does NOT run the real
//! pipeline's other checks (decoding, dimension limits, re-encoding, scanning),
//! so width and height are always 0. Ids are FNV-1a, not SHA-256; clients must
//! treat `asset_id` as opaque.

pub mod dice;

mod encounter;
mod tokens;
mod undo;
mod use_ability;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, PathRejection};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Path, State as AxumState};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use prost::Message as _;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tt_contracts::v1::{
    client_message, event, intent, server_message, AssetRef, ChatPosted, ClientMessage, Creature,
    DiceRolled, Encounter, Event, Hello, Intent, IntentRejected, Role, RollResult, SceneUpserted,
    ServerMessage, Snapshot, Token, TokenMoved, TokenUpserted, Vec2, Warning,
};

use dice::{Rng, XorShift64Star};
use undo::{Before, GroupRange, UndoGroup, UndoRecord};

/// The recorded game state every connection starts from: the pregen table seed.
const FIXTURE: &str = include_str!("../../../contracts/fixtures/seeds/pregen-table.json");

/// Largest accepted WebSocket message.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
/// Longest accepted chat text, in characters.
pub const MAX_CHAT_CHARS: usize = 2000;
/// Largest accepted asset upload; matches `tt_asset_pipeline::MAX_FILE_BYTES`.
pub const MAX_ASSET_BYTES: usize = 20 * 1024 * 1024;
/// Most distinct assets kept in memory; a new one beyond this is refused.
pub const MAX_STORED_ASSETS: usize = 32;
/// Events buffered per receiver before a slow client is dropped.
const BROADCAST_CAPACITY: usize = 256;

/// Parse the embedded fixture. Panics only if the committed fixture is broken,
/// which a unit test guards against.
pub fn fixture_snapshot() -> Snapshot {
    serde_json::from_str(FIXTURE)
        .expect("contracts/fixtures/seeds/pregen-table.json parses as Snapshot")
}

/// State shared by every connection in the process.
struct Game {
    /// The fixture, kept for scene data and members.
    template: Snapshot,
    tokens: Vec<Token>,
    /// Running encounters, at most one per scene.
    encounters: Vec<Encounter>,
    /// Sequence of the last event emitted; 0 before any.
    seq: u64,
    /// Count of resolutions so far; numbers `resolution_id` and `roll_id`.
    resolutions: u64,
    /// Count of tokens made by `CreateTokenIntent`; numbers `token-new-<n>`.
    next_token: u64,
    /// Count of encounters started; numbers `encounter-<n>`.
    next_encounter: u64,
    rng: Box<dyn Rng + Send>,
    /// Uploaded assets by id; memory only.
    assets: HashMap<String, StoredAsset>,
    /// The event group of every accepted intent, in seq order.
    groups: Vec<GroupRange>,
    /// Undoable groups not yet undone, oldest first. Only the last can be undone.
    undo_stack: Vec<UndoGroup>,
}

impl Game {
    fn new(template: Snapshot, rng: Box<dyn Rng + Send>) -> Self {
        Self {
            tokens: template.tokens.clone(),
            template,
            encounters: Vec::new(),
            seq: 0,
            resolutions: 0,
            next_token: 0,
            next_encounter: 0,
            rng,
            assets: HashMap::new(),
            groups: Vec::new(),
            undo_stack: Vec::new(),
        }
    }
}

struct StoredAsset {
    bytes: Bytes,
    media_type: &'static str,
    ext: &'static str,
    asset_ref: AssetRef,
}

struct Shared {
    // A std mutex is enough: it is never held across an await.
    game: Mutex<Game>,
    /// Events as the game made them, before per-viewer filtering.
    tx: broadcast::Sender<Arc<Event>>,
    /// Listener address, the fallback for building asset URLs.
    local_addr: Option<SocketAddr>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Game> {
        // Handlers do not panic while holding the lock; recover anyway.
        self.game.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Serve the mock protocol on `listener` until the task is dropped or the
/// listener fails. Randomness is seeded from the system clock.
pub async fn serve(listener: TcpListener) -> std::io::Result<()> {
    serve_with_rng(listener, Box::new(XorShift64Star::from_clock())).await
}

/// Like [`serve`] but with an injected random source, so tests are repeatable.
pub async fn serve_with_rng(
    listener: TcpListener,
    rng: Box<dyn Rng + Send>,
) -> std::io::Result<()> {
    let template = fixture_snapshot();
    let local_addr = listener.local_addr().ok();
    let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
    let shared = Arc::new(Shared {
        game: Mutex::new(Game::new(template, rng)),
        tx,
        local_addr,
    });
    let app = Router::new()
        .route(
            "/",
            get(|| async { "Tome & Tile mock server. WebSocket endpoint: /ws\n" }),
        )
        .route("/ws", get(ws_upgrade))
        .route(
            "/campaigns/{campaign_id}/assets",
            post(upload_asset)
                .options(preflight)
                .fallback(method_not_allowed)
                .layer(DefaultBodyLimit::max(MAX_ASSET_BYTES)),
        )
        .route(
            "/assets/{*file}",
            get(get_asset)
                .options(preflight)
                .fallback(method_not_allowed),
        )
        .with_state(shared);
    axum::serve(listener, app).await
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    AxumState(shared): AxumState<Arc<Shared>>,
) -> axum::response::Response {
    ws.max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| connection(socket, shared))
}

/// Build a JSON error response with CORS.
fn http_error(status: StatusCode, code: &str, message: &str) -> Response {
    let body = serde_json::json!({ "code": code, "message": message }).to_string();
    let mut response = Response::new(axum::body::Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    cors(response)
}

/// JSON 405 for unsupported methods on the asset routes.
async fn method_not_allowed() -> Response {
    http_error(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "this route does not support that method",
    )
}

/// Allow any origin to read this response.
fn cors(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

/// Answer a CORS preflight for the asset routes.
async fn preflight() -> Response {
    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    let h = response.headers_mut();
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Authorization, Content-Type"),
    );
    h.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("600"),
    );
    cors(response)
}

/// The bearer token from `Authorization: Bearer <token>`, scheme case-insensitive.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, rest) = value.split_once(' ')?;
    let token = rest.trim();
    if !scheme.eq_ignore_ascii_case("bearer")
        || token.is_empty()
        || token.contains(char::is_whitespace)
    {
        return None;
    }
    Some(token)
}

/// Media type and file extension from the leading magic bytes.
fn sniff(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(("image/png", "png"))
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("image/jpeg", "jpg"))
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    } else {
        None
    }
}

/// FNV-1a 64-bit. The real server uses SHA-256; this is only a mock id.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Base URL for asset links: the request's Host header, else the listener.
fn base_url(headers: &HeaderMap, local_addr: Option<SocketAddr>) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .filter(|h| {
            !h.is_empty()
                && h.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
        });
    match (host, local_addr) {
        (Some(host), _) => format!("http://{host}"),
        (None, Some(addr)) => format!("http://{addr}"),
        (None, None) => "http://127.0.0.1".to_string(),
    }
}

async fn upload_asset(
    AxumState(shared): AxumState<Arc<Shared>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return http_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "send Authorization: Bearer <token>",
        );
    };
    if token != "gm" {
        return http_error(
            StatusCode::FORBIDDEN,
            "not_permitted",
            "only the GM may upload assets",
        );
    }
    let bytes = match body {
        Ok(bytes) => bytes,
        Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return http_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "file_too_large",
                &format!("file is larger than {MAX_ASSET_BYTES} bytes"),
            );
        }
        Err(_) => {
            return http_error(
                StatusCode::BAD_REQUEST,
                "bad_body",
                "could not read the request body",
            );
        }
    };
    if bytes.is_empty() {
        return http_error(StatusCode::BAD_REQUEST, "empty_file", "the file is empty");
    }
    let Some((media_type, ext)) = sniff(&bytes) else {
        return http_error(
            StatusCode::BAD_REQUEST,
            "unknown_format",
            "the mock accepts PNG, JPEG and WebP only",
        );
    };
    let asset_id = format!("mock-{:016x}", fnv1a64(&bytes));
    let url = format!(
        "{}/assets/{asset_id}.{ext}",
        base_url(&headers, shared.local_addr)
    );
    let asset_ref = {
        let mut game = shared.lock();
        if let Some(existing) = game.assets.get(&asset_id) {
            existing.asset_ref.clone()
        } else if game.assets.len() >= MAX_STORED_ASSETS {
            drop(game);
            return http_error(
                StatusCode::INSUFFICIENT_STORAGE,
                "storage_full",
                &format!("the mock keeps at most {MAX_STORED_ASSETS} assets; restart it to reset"),
            );
        } else {
            let asset_ref = AssetRef {
                asset_id: asset_id.clone(),
                media_type: media_type.to_string(),
                url,
                width: 0,
                height: 0,
            };
            game.assets.insert(
                asset_id,
                StoredAsset {
                    bytes,
                    media_type,
                    ext,
                    asset_ref: asset_ref.clone(),
                },
            );
            asset_ref
        }
    };
    let mut response = Response::new(axum::body::Body::from(asset_ref.encode_to_vec()));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-protobuf"),
    );
    cors(response)
}

async fn get_asset(
    AxumState(shared): AxumState<Arc<Shared>>,
    file: Result<Path<String>, PathRejection>,
) -> Response {
    let Ok(Path(file)) = file else {
        return http_error(StatusCode::NOT_FOUND, "unknown_asset", "no such asset");
    };
    let found = file.split_once('.').and_then(|(id, ext)| {
        let game = shared.lock();
        game.assets
            .get(id)
            .filter(|a| a.ext == ext)
            .map(|a| (a.bytes.clone(), a.media_type))
    });
    let Some((bytes, media_type)) = found else {
        return http_error(StatusCode::NOT_FOUND, "unknown_asset", "no such asset");
    };
    let mut response = Response::new(axum::body::Body::from(bytes));
    let h = response.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(media_type));
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    cors(response)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn server_bytes(kind: server_message::Kind) -> Bytes {
    Bytes::from(ServerMessage { kind: Some(kind) }.encode_to_vec())
}

fn rejected(intent_id: &str, code: &str, message: &str) -> Bytes {
    server_bytes(server_message::Kind::IntentRejected(IntentRejected {
        client_intent_id: intent_id.to_string(),
        code: code.to_string(),
        message: message.to_string(),
    }))
}

/// Decode a binary frame as a `ClientMessage`; `None` means "close".
fn decode_client(bytes: &[u8]) -> Option<ClientMessage> {
    ClientMessage::decode(bytes).ok()
}

async fn close(mut socket: WebSocket) {
    let _ = socket.send(Message::Close(None)).await;
}

/// Who is on the other end of a socket.
#[derive(Clone, Copy)]
struct Viewer {
    user_id: &'static str,
    role: Role,
}

impl Viewer {
    /// The GM controls every token; a player the tokens that list them.
    fn controls(&self, token: &Token) -> bool {
        match self.role {
            Role::Gm => true,
            Role::Player => token.controller_user_ids.iter().any(|u| u == self.user_id),
            _ => false,
        }
    }
}

/// A copy of `token` with the hit points `viewer` may not see left unset.
/// Mirrors `redact_token` in `tt-server`; `health_state` and `conditions` stay.
fn redact_token(token: &Token, viewer: Viewer) -> Token {
    let mut t = token.clone();
    if !viewer.controls(token) {
        t.hp_current = None;
        t.hp_max = None;
        t.hp_temp = None;
    }
    t
}

/// Encode `event` for `viewer`: `None` when they may not receive it.
fn encode_event(shared: &Shared, event: &Event, viewer: Viewer) -> Option<Bytes> {
    let mut out = event.clone();
    match &event.kind {
        Some(event::Kind::TokenUpserted(u)) => {
            out.kind = Some(event::Kind::TokenUpserted(TokenUpserted {
                token: u.token.as_ref().map(|t| redact_token(t, viewer)),
            }));
            // Warnings on a token event describe the creature's sheet: only
            // for viewers who may see it (mirrors `tt-server`, #133).
            if viewer.role != Role::Gm {
                let game = shared.lock();
                let visible = u
                    .token
                    .as_ref()
                    .is_some_and(|t| game.token_sheet_visible(&t.token_id, viewer));
                if !visible {
                    out.warnings.clear();
                }
            }
        }
        // Result cards never show the stat block of a creature the viewer
        // cannot see (mirrors `tt-server`, #133).
        Some(event::Kind::ResolutionUpdated(_)) if viewer.role != Role::Gm => {
            shared.lock().redact_resolution_event(&mut out, viewer);
        }
        Some(event::Kind::DiceRolled(_)) if viewer.role != Role::Gm => {
            let game = shared.lock();
            if let Some(event::Kind::DiceRolled(d)) = out.kind.as_mut() {
                if let Some(roll) = d.roll.as_mut() {
                    game.redact_roll(roll, viewer);
                }
            }
        }
        // Sheets go only to viewers who may see them, as in the Snapshot.
        Some(event::Kind::CreatureUpserted(c)) => {
            let creature = c.creature.as_ref()?;
            if !shared
                .lock()
                .creature_visible(creature, viewer.user_id, viewer.role)
            {
                return None;
            }
        }
        _ => {}
    }
    Some(server_bytes(server_message::Kind::Event(out)))
}

async fn connection(mut socket: WebSocket, shared: Arc<Shared>) {
    // Phase 1: the first binary frame must be Hello.
    let hello = loop {
        match socket.recv().await {
            Some(Ok(Message::Binary(bytes))) => match decode_client(&bytes) {
                Some(ClientMessage {
                    kind: Some(client_message::Kind::Hello(hello)),
                }) => break hello,
                _ => return close(socket).await,
            },
            Some(Ok(Message::Text(_) | Message::Ping(_) | Message::Pong(_))) => continue,
            _ => return,
        }
    };
    let (viewer, snapshot, mut rx) = join(&shared, &hello);
    if socket.send(Message::Binary(snapshot)).await.is_err() {
        return;
    }

    // Phase 2: intents up, broadcast events down.
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let bytes = match incoming {
                    Some(Ok(Message::Binary(bytes))) => bytes,
                    Some(Ok(Message::Text(_) | Message::Ping(_) | Message::Pong(_))) => continue,
                    _ => return,
                };
                let Some(message) = decode_client(&bytes) else {
                    return close(socket).await;
                };
                if let Some(client_message::Kind::Intent(intent)) = message.kind {
                    if let Some(reply) = handle_intent(&shared, viewer.user_id, intent) {
                        if socket.send(Message::Binary(reply)).await.is_err() {
                            return;
                        }
                    }
                }
                // Hello after Hello and media_token_request are ignored.
            }
            outgoing = rx.recv() => match outgoing {
                Ok(event) => {
                    if let Some(bytes) = encode_event(&shared, &event, viewer) {
                        if socket.send(Message::Binary(bytes)).await.is_err() {
                            return;
                        }
                    }
                }
                // A client too slow to keep up is dropped; it can reconnect
                // and get a fresh Snapshot instead of a gapped event stream.
                Err(broadcast::error::RecvError::Lagged(_)) => return close(socket).await,
                Err(broadcast::error::RecvError::Closed) => return,
            },
        }
    }
}

/// Complete `Hello`: build the Snapshot and subscribe to events under one lock
/// so the client sees neither a gap nor a duplicate between the two.
fn join(shared: &Shared, hello: &Hello) -> (Viewer, Bytes, broadcast::Receiver<Arc<Event>>) {
    let game = shared.lock();
    let rx = shared.tx.subscribe();
    let (user_id, role) = identity(hello);
    let viewer = Viewer { user_id, role };
    let mut snapshot = game.template.clone();
    snapshot.seq = game.seq;
    snapshot.user_id = user_id.to_string();
    snapshot.role = role as i32;
    snapshot.tokens = game
        .tokens
        .iter()
        .map(|t| redact_token(t, viewer))
        .collect();
    snapshot.encounters = game.encounters.clone();
    snapshot.creatures = game
        .template
        .creatures
        .iter()
        .filter(|c| game.creature_visible(c, user_id, role))
        .cloned()
        .collect();
    let bytes = server_bytes(server_message::Kind::Snapshot(snapshot));
    (viewer, bytes, rx)
}

/// Mock identity rule: session token "gm" is the GM, anything else a player.
fn identity(hello: &Hello) -> (&'static str, Role) {
    if hello.session_token == "gm" {
        ("mock-gm", Role::Gm)
    } else {
        ("mock-player", Role::Player)
    }
}

/// Apply one intent. Returns a direct reply (a rejection) for the sender, or
/// `None` when the outcome was broadcast as events.
fn handle_intent(shared: &Shared, user_id: &str, intent: Intent) -> Option<Bytes> {
    let id = intent.client_intent_id.as_str();
    let mut game = shared.lock();
    let result = match intent.kind.as_ref() {
        Some(intent::Kind::MoveToken(m)) => game.move_token(m),
        Some(intent::Kind::Chat(c)) => game.chat(user_id, c.text.as_str()).map(Group::one),
        Some(intent::Kind::RollDice(r)) => game.roll_dice(user_id, r).map(Group::one),
        Some(intent::Kind::UseAbility(u)) => game.use_ability(user_id, u),
        Some(intent::Kind::StartEncounter(s)) => game.start_encounter(user_id, s),
        Some(intent::Kind::EndTurn(e)) => game.end_turn(user_id, e),
        Some(intent::Kind::SetInitiative(s)) => game.set_initiative(user_id, s),
        Some(intent::Kind::Undo(u)) => game.undo(user_id, u),
        Some(intent::Kind::SetHitPoints(s)) => game.set_hit_points(user_id, s),
        Some(intent::Kind::SetLevelBackground(b)) => {
            game.set_level_background(user_id, b).map(Group::one)
        }
        Some(intent::Kind::CreateToken(c)) => game.create_token(user_id, c),
        Some(intent::Kind::RemoveToken(r)) => game.remove_token(user_id, r),
        Some(intent::Kind::SetConditions(c)) => game.set_conditions(user_id, c),
        Some(intent::Kind::EndEncounter(e)) => game.end_encounter(user_id, e),
        _ => Err((
            "not_supported_by_mock",
            "the mock server does not support this intent".to_string(),
        )),
    };
    match result {
        Ok(group) => {
            let first = game.seq + 1;
            // Sent while the lock is held so every client sees events in seq
            // order, and one intent's events stay consecutive. No receivers
            // (nobody joined) is not an error.
            for (kind, warnings) in group.events {
                game.seq += 1;
                let scene_id = match &kind {
                    event::Kind::SceneUpserted(u) => u
                        .scene
                        .as_ref()
                        .map(|s| s.scene_id.clone())
                        .unwrap_or_default(),
                    _ => game
                        .template
                        .scenes
                        .first()
                        .map(|s| s.scene_id.clone())
                        .unwrap_or_default(),
                };
                let event = Event {
                    seq: game.seq,
                    at_ms: now_ms(),
                    scene_id,
                    actor_user_id: user_id.to_string(),
                    client_intent_id: id.to_string(),
                    warnings,
                    kind: Some(kind),
                };
                let _ = shared.tx.send(Arc::new(event));
            }
            let last = game.seq;
            game.groups.push(GroupRange {
                first,
                last,
                undoable: group.undo.is_some(),
                undone: false,
            });
            if let Some(record) = group.undo {
                game.undo_stack.push(UndoGroup {
                    first_seq: first,
                    record,
                });
            }
            None
        }
        Err((code, message)) => Some(rejected(id, code, &message)),
    }
}

type Rejection = (&'static str, String);

/// An event body plus the warnings that ride on `Event.warnings`.
type Outcome = (event::Kind, Vec<Warning>);

/// The events one intent produced, in order, and what undoing them restores.
struct Group {
    events: Vec<Outcome>,
    /// `Some` for an undoable group (`UndoIntent` in session.proto).
    undo: Option<UndoRecord>,
}

impl Group {
    /// A single event with no warnings, not undoable.
    fn one(kind: event::Kind) -> Self {
        Self {
            events: vec![(kind, Vec::new())],
            undo: None,
        }
    }
}

/// The GM's mock user id; any other user is a player.
const GM_USER: &str = "mock-gm";

fn warning(code: &str, message: String) -> Warning {
    Warning {
        code: code.to_string(),
        message,
    }
}

fn not_permitted(message: &str) -> Rejection {
    ("not_permitted", message.to_string())
}

impl Game {
    /// Refuse anyone but the GM.
    fn gm_only(&self, user_id: &str, message: &str) -> Result<(), Rejection> {
        if user_id == GM_USER {
            Ok(())
        } else {
            Err(not_permitted(message))
        }
    }

    /// Index of a token the sender may act with: any token for the GM, a
    /// visible token that lists them as a controller for a player. A player
    /// naming a missing token gets `not_permitted`, the GM `unknown_token`.
    fn controlled_token(&self, user_id: &str, token_id: &str) -> Result<usize, Rejection> {
        let found = self.tokens.iter().position(|t| t.token_id == token_id);
        match found {
            Some(i)
                if user_id == GM_USER
                    || (!self.tokens[i].hidden
                        && self.tokens[i]
                            .controller_user_ids
                            .iter()
                            .any(|c| c == user_id)) =>
            {
                Ok(i)
            }
            Some(_) => Err(not_permitted("you do not control that token")),
            None if user_id == GM_USER => Err(("unknown_token", format!("no token {token_id:?}"))),
            None => Err(not_permitted("you do not control that token")),
        }
    }
    /// May this viewer receive the full sheet of `creature`? Mirrors
    /// `creature_visible` in `tt-server`: the GM; the owner; and controllers of
    /// a visible token linked to it.
    fn creature_visible(&self, creature: &Creature, user_id: &str, role: Role) -> bool {
        match role {
            Role::Gm => true,
            Role::Player => {
                (!creature.owner_user_id.is_empty() && creature.owner_user_id == user_id)
                    || self.tokens.iter().any(|t| {
                        !t.hidden
                            && !t.creature_id.is_empty()
                            && t.creature_id == creature.creature_id
                            && t.controller_user_ids.iter().any(|u| u == user_id)
                    })
            }
            _ => false,
        }
    }

    /// Mirrors `token_sheet_visible` in `tt-server`.
    fn token_sheet_visible(&self, token_id: &str, viewer: Viewer) -> bool {
        self.tokens
            .iter()
            .find(|t| t.token_id == token_id)
            .and_then(|t| {
                self.template
                    .creatures
                    .iter()
                    .find(|c| c.creature_id == t.creature_id)
            })
            .is_some_and(|c| self.creature_visible(c, viewer.user_id, viewer.role))
    }

    /// Mirrors `redact_roll` in `tt-server`: a roll by a creature the viewer
    /// cannot see keeps its total and flags but loses dice, modifiers and
    /// notation.
    fn redact_roll(&self, roll: &mut RollResult, viewer: Viewer) {
        if roll.roller_creature_id.is_empty() {
            return;
        }
        let visible = self
            .template
            .creatures
            .iter()
            .find(|c| c.creature_id == roll.roller_creature_id)
            .is_some_and(|c| self.creature_visible(c, viewer.user_id, viewer.role));
        if !visible {
            roll.dice.clear();
            roll.modifiers.clear();
            roll.notation.clear();
        }
    }

    /// Mirrors `redact_resolution_event` in `tt-server` (#133): rolls by
    /// unseen creatures lose their dice; a target whose sheet is unseen
    /// loses `damage`; source-sheet warnings (`not_enough_resource`,
    /// `too_many_targets`, `out_of_range`, `ability_recharging`) need the
    /// source's sheet and `condition_immune` the named target's.
    fn redact_resolution_event(&self, out: &mut Event, viewer: Viewer) {
        let Some(event::Kind::ResolutionUpdated(ru)) = out.kind.as_mut() else {
            return;
        };
        let Some(res) = ru.resolution.as_mut() else {
            return;
        };
        let source_visible = self.token_sheet_visible(&res.source_token_id, viewer);
        for outcome in &mut res.outcomes {
            for roll in outcome
                .roll
                .iter_mut()
                .chain(outcome.damage_rolls.iter_mut())
            {
                self.redact_roll(roll, viewer);
            }
            let has_sheet = self
                .tokens
                .iter()
                .find(|t| t.token_id == outcome.token_id)
                .is_some_and(|t| !t.creature_id.is_empty());
            if has_sheet && !self.token_sheet_visible(&outcome.token_id, viewer) {
                outcome.damage.clear();
            }
        }
        let outcomes = res.outcomes.clone();
        let keep = |w: &Warning| match w.code.as_str() {
            "not_enough_resource" | "too_many_targets" | "out_of_range" | "ability_recharging" => {
                source_visible
            }
            "condition_immune" => outcomes
                .iter()
                .find(|o| w.message.starts_with(&format!("token {} ", o.token_id)))
                .is_some_and(|o| self.token_sheet_visible(&o.token_id, viewer)),
            _ => true,
        };
        res.warnings.retain(keep);
        out.warnings.retain(keep);
    }

    fn move_token(&mut self, m: &tt_contracts::v1::MoveTokenIntent) -> Result<Group, Rejection> {
        let Some(destination) = m.path.last().copied() else {
            return Err(("empty_path", "path has no waypoints".to_string()));
        };
        let (cell_size, feet_per_cell) = self
            .template
            .scenes
            .first()
            .and_then(|s| s.grid.as_ref())
            .map(|g| (g.cell_size, g.feet_per_cell))
            .unwrap_or((0.0, 0.0));
        let Some(index) = self.tokens.iter().position(|t| t.token_id == m.token_id) else {
            return Err(("unknown_token", format!("no token {:?}", m.token_id)));
        };
        let token = &mut self.tokens[index];
        let before = token.clone();
        let start = token.position.unwrap_or(m.path[0]);
        let distance_feet = path_distance_feet(&start, &m.path, cell_size, feet_per_cell);
        token.position = Some(destination);
        if !m.level_id.is_empty() {
            token.level_id = m.level_id.clone();
        }
        let mut group = Group::one(event::Kind::TokenMoved(TokenMoved {
            token_id: m.token_id.clone(),
            path: m.path.clone(),
            level_id: m.level_id.clone(),
            distance_feet,
        }));
        group.undo = Some(UndoRecord {
            moved: true,
            before: vec![Before {
                index,
                token_id: m.token_id.clone(),
                token: Some(before),
            }],
        });
        Ok(group)
    }

    fn chat(&mut self, user_id: &str, text: &str) -> Result<event::Kind, Rejection> {
        if text.is_empty() {
            return Err(("empty_chat", "chat text is empty".to_string()));
        }
        if text.chars().count() > MAX_CHAT_CHARS {
            return Err((
                "chat_too_long",
                format!("chat text is longer than {MAX_CHAT_CHARS} characters"),
            ));
        }
        Ok(event::Kind::ChatPosted(ChatPosted {
            user_id: user_id.to_string(),
            text: text.to_string(),
        }))
    }

    fn set_level_background(
        &mut self,
        user_id: &str,
        b: &tt_contracts::v1::SetLevelBackgroundIntent,
    ) -> Result<event::Kind, Rejection> {
        if user_id != "mock-gm" {
            return Err((
                "not_permitted",
                "only the GM may set a level background".to_string(),
            ));
        }
        let Some(scene) = self
            .template
            .scenes
            .iter_mut()
            .find(|s| s.scene_id == b.scene_id)
        else {
            return Err(("unknown_scene", format!("no scene {:?}", b.scene_id)));
        };
        let Some(level) = scene.levels.iter_mut().find(|l| l.level_id == b.level_id) else {
            return Err(("unknown_level", format!("no level {:?}", b.level_id)));
        };
        let background = if b.asset_id.is_empty() {
            None
        } else {
            match self.assets.get(&b.asset_id) {
                Some(asset) => Some(asset.asset_ref.clone()),
                None => return Err(("unknown_asset", format!("no asset {:?}", b.asset_id))),
            }
        };
        level.background = background;
        Ok(event::Kind::SceneUpserted(SceneUpserted {
            scene: Some(scene.clone()),
        }))
    }

    fn roll_dice(
        &mut self,
        user_id: &str,
        r: &tt_contracts::v1::RollDiceIntent,
    ) -> Result<event::Kind, Rejection> {
        let Some(notation) = dice::parse_notation(&r.notation) else {
            return Err((
                "bad_notation",
                "supported notation is NdM, NdM+K or NdM-K (N 1-100, M in 4,6,8,10,12,20,100)"
                    .to_string(),
            ));
        };
        let mode = tt_contracts::v1::RollMode::try_from(r.mode)
            .unwrap_or(tt_contracts::v1::RollMode::Unspecified);
        let roll_id = format!("roll-{}", self.seq + 1);
        let roll = dice::roll(
            self.rng.as_mut(),
            &notation,
            mode,
            roll_id,
            user_id,
            r.label.clone(),
        )
        .ok_or(("bad_notation", "total out of range".to_string()))?;
        Ok(event::Kind::DiceRolled(DiceRolled { roll: Some(roll) }))
    }
}

/// Sum of straight-line segment lengths from `start` through `path`, in feet.
fn path_distance_feet(start: &Vec2, path: &[Vec2], cell_size: f32, feet_per_cell: f32) -> f32 {
    if cell_size <= 0.0 {
        return 0.0;
    }
    let mut prev = (f64::from(start.x), f64::from(start.y));
    let mut units = 0.0_f64;
    for p in path {
        let next = (f64::from(p.x), f64::from(p.y));
        units += ((next.0 - prev.0).powi(2) + (next.1 - prev.1).powi(2)).sqrt();
        prev = next;
    }
    (units / f64::from(cell_size) * f64::from(feet_per_cell)) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_contracts::v1::{effect_step, EffectStep, HealthState, HitOutcome, UseAbilityIntent};

    #[test]
    fn fixture_parses_into_snapshot() {
        let snapshot = fixture_snapshot();
        assert_eq!(snapshot.seq, 0);
        assert_eq!(snapshot.members.len(), 2);
        assert_eq!(snapshot.scenes.len(), 1);
        assert_eq!(snapshot.scenes[0].scene_id, snapshot.active_scene_id);
        assert_eq!(snapshot.scenes[0].levels.len(), 1);
        assert!(snapshot.scenes[0].walls.is_empty());
        let grid = snapshot.scenes[0].grid.as_ref().unwrap();
        assert_eq!(grid.cell_size, 70.0);
        assert_eq!(grid.feet_per_cell, 5.0);
        assert_eq!(snapshot.tokens.len(), 8);
        assert_eq!(snapshot.creatures.len(), 7);
        let aria = &snapshot.tokens[0];
        assert_eq!(aria.name, "Aria");
        assert_eq!(aria.hp_current, Some(24));
        assert_eq!(aria.controller_user_ids, ["mock-player"]);
        let goblin = &snapshot.tokens[1];
        assert_eq!(goblin.name, "Goblin");
        assert_eq!(goblin.hp_current, Some(9));
        assert_eq!(goblin.hp_max, Some(9));
        assert!(goblin.controller_user_ids.is_empty());
        assert_eq!(goblin.health_state, HealthState::Healthy as i32);
    }

    /// Rolls every die as 1.
    struct Ones;

    impl Rng for Ones {
        fn next_u64(&mut self) -> u64 {
            0
        }
    }

    fn game() -> Game {
        Game::new(fixture_snapshot(), Box::new(Ones))
    }

    fn use_ability(game: &mut Game, source: &str, ability: &str, target: &str) -> Group {
        let u = UseAbilityIntent {
            source_token_id: source.to_string(),
            ability_id: ability.to_string(),
            target_token_ids: vec![target.to_string()],
            ..Default::default()
        };
        game.use_ability(GM_USER, &u).expect("the ability resolves")
    }

    #[test]
    fn a_target_with_no_hit_points_gets_a_warning_and_keeps_its_state() {
        let mut game = game();
        let goblin = game
            .tokens
            .iter_mut()
            .find(|t| t.token_id == "token-goblin")
            .unwrap();
        goblin.hp_current = None;
        goblin.hp_max = None;

        // Healing: shown on the card, not applied, so nothing is upserted.
        let group = use_ability(&mut game, "token-maribel", "cure-wounds", "token-goblin");
        assert_eq!(group.events.len(), 1);
        let (event::Kind::ResolutionUpdated(r), warnings) = &group.events[0] else {
            panic!("expected resolution_updated");
        };
        let codes: Vec<&str> = warnings.iter().map(|w| w.code.as_str()).collect();
        assert_eq!(codes, ["no_hit_points"]);
        let o = &r.resolution.as_ref().unwrap().outcomes[0];
        assert_eq!(o.healing, 8);

        // Damage: the damage is listed, the condition still lands, HP stays unset.
        let group = use_ability(&mut game, "token-ember-wisp", "choking-ash", "token-goblin");
        assert_eq!(group.events.len(), 2);
        let (event::Kind::ResolutionUpdated(r), warnings) = &group.events[0] else {
            panic!("expected resolution_updated");
        };
        assert_eq!(warnings.len(), 1);
        let o = &r.resolution.as_ref().unwrap().outcomes[0];
        assert_eq!(o.outcome, HitOutcome::SaveFailed as i32);
        assert_eq!(o.damage[0].applied, o.damage[0].rolled);
        assert!(!o.dropped_to_zero);
        let goblin = game
            .tokens
            .iter()
            .find(|t| t.token_id == "token-goblin")
            .unwrap();
        assert_eq!((goblin.hp_current, goblin.hp_max), (None, None));
        assert_eq!(goblin.health_state, HealthState::Healthy as i32);
        assert_eq!(goblin.conditions, ["blinded"]);
    }

    fn steps<'a>(game: &'a mut Game, creature: &str, ability: &str) -> &'a mut Vec<EffectStep> {
        let c = game
            .template
            .creatures
            .iter_mut()
            .find(|c| c.creature_id == creature)
            .unwrap();
        let a = c.abilities.iter_mut().find(|a| a.ability_id == ability);
        &mut a.unwrap().steps
    }

    #[test]
    fn plugin_steps_and_mixed_multi_attack_abilities_are_refused() {
        let mut game = game();
        steps(&mut game, "creature-brannoch", "longsword").push(EffectStep {
            step: Some(effect_step::Step::Plugin(Default::default())),
        });
        steps(&mut game, "creature-ilvaren", "scorching-ray").push(EffectStep {
            step: Some(effect_step::Step::Flat(Default::default())),
        });
        let attempt = |game: &mut Game, source: &str, ability: &str| {
            let u = UseAbilityIntent {
                source_token_id: source.to_string(),
                ability_id: ability.to_string(),
                target_token_ids: vec!["token-goblin".to_string()],
                ..Default::default()
            };
            game.use_ability(GM_USER, &u).err().map(|e| e.0)
        };
        assert_eq!(
            attempt(&mut game, "token-brannoch", "longsword"),
            Some("not_supported_by_mock")
        );
        assert_eq!(
            attempt(&mut game, "token-ilvaren", "scorching-ray"),
            Some("not_implemented")
        );
        // Nothing rolled or changed: no resolution number was spent.
        assert_eq!(game.resolutions, 0);
    }

    #[test]
    fn path_distance_counts_segments_in_feet() {
        let p = |x, y| Vec2 { x, y };
        let feet = path_distance_feet(&p(0.0, 0.0), &[p(70.0, 0.0), p(70.0, 140.0)], 70.0, 5.0);
        assert_eq!(feet, 15.0);
    }
}
