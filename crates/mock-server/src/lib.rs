//! Mock game server: speaks the real game-socket protocol from recorded data.
//!
//! It applies NO rules. It filters one thing per viewer, the creature sheets in
//! the Snapshot, and checks permission only for asset upload,
//! `set_level_background` and `use_ability`, so the UI can exercise those
//! refusal paths. `use_ability` rolls attack and damage for attack abilities
//! but changes no hit points. It exists so the web UI and importers can be
//! built before the real server does. See the crate README.
//!
//! Besides the socket it serves a small HTTP asset surface (ADR 0014):
//! `POST /campaigns/{campaign_id}/assets` stores an image in memory and
//! `GET /assets/{asset_id}.{ext}` serves it back, both with CORS enabled. The
//! mock sniffs magic bytes only (PNG, JPEG, WebP); it does NOT run the real
//! pipeline's other checks (decoding, dimension limits, re-encoding, scanning),
//! so width and height are always 0. Ids are FNV-1a, not SHA-256; clients must
//! treat `asset_id` as opaque.

pub mod dice;

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
    client_message, effect_step, event, intent, server_message, Ability, AssetRef, ChatPosted,
    ClientMessage, Creature, DiceRolled, Event, Hello, HitOutcome, Intent, IntentRejected,
    Resolution, ResolutionState, ResolutionUpdated, Role, RollMode, RollPurpose, RollResult,
    SceneUpserted, ServerMessage, Snapshot, TargetOutcome, TargetShape, Token, TokenMoved,
    UseAbilityIntent, Vec2, Warning,
};

use dice::{Rng, XorShift64Star};

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
    /// Sequence of the last event emitted; 0 before any.
    seq: u64,
    /// Count of resolutions so far; numbers `resolution_id` and `roll_id`.
    resolutions: u64,
    rng: Box<dyn Rng + Send>,
    /// Uploaded assets by id; memory only.
    assets: HashMap<String, StoredAsset>,
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
    tx: broadcast::Sender<Bytes>,
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
        game: Mutex::new(Game {
            tokens: template.tokens.clone(),
            template,
            seq: 0,
            resolutions: 0,
            rng,
            assets: HashMap::new(),
        }),
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
    let (user_id, snapshot, mut rx) = join(&shared, &hello);
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
                    if let Some(reply) = handle_intent(&shared, &user_id, intent) {
                        if socket.send(Message::Binary(reply)).await.is_err() {
                            return;
                        }
                    }
                }
                // Hello after Hello and media_token_request are ignored.
            }
            outgoing = rx.recv() => match outgoing {
                Ok(bytes) => {
                    if socket.send(Message::Binary(bytes)).await.is_err() {
                        return;
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
fn join(shared: &Shared, hello: &Hello) -> (String, Bytes, broadcast::Receiver<Bytes>) {
    let game = shared.lock();
    let rx = shared.tx.subscribe();
    let (user_id, role) = identity(hello);
    let mut snapshot = game.template.clone();
    snapshot.seq = game.seq;
    snapshot.user_id = user_id.to_string();
    snapshot.role = role as i32;
    snapshot.tokens = game.tokens.clone();
    snapshot.creatures = game
        .template
        .creatures
        .iter()
        .filter(|c| game.creature_visible(c, user_id, role))
        .cloned()
        .collect();
    let bytes = server_bytes(server_message::Kind::Snapshot(snapshot));
    (user_id.to_string(), bytes, rx)
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
/// `None` when the outcome was broadcast as an event.
fn handle_intent(shared: &Shared, user_id: &str, intent: Intent) -> Option<Bytes> {
    let id = intent.client_intent_id.as_str();
    let mut game = shared.lock();
    let result = match intent.kind.as_ref() {
        Some(intent::Kind::MoveToken(m)) => game.move_token(m).map(plain),
        Some(intent::Kind::Chat(c)) => game.chat(user_id, c.text.as_str()).map(plain),
        Some(intent::Kind::RollDice(r)) => game.roll_dice(user_id, r).map(plain),
        Some(intent::Kind::UseAbility(u)) => game.use_ability(user_id, u),
        Some(intent::Kind::SetLevelBackground(b)) => {
            game.set_level_background(user_id, b).map(plain)
        }
        _ => Err((
            "not_supported_by_mock",
            "the mock server does not support this intent".to_string(),
        )),
    };
    match result {
        Ok((kind, warnings)) => {
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
            // Sent while the lock is held so every client sees events in seq
            // order. No receivers (nobody joined) is not an error.
            let _ = shared
                .tx
                .send(server_bytes(server_message::Kind::Event(event)));
            None
        }
        Err((code, message)) => Some(rejected(id, code, &message)),
    }
}

type Rejection = (&'static str, String);

/// An event body plus the warnings that ride on `Event.warnings`.
type Outcome = (event::Kind, Vec<Warning>);

fn plain(kind: event::Kind) -> Outcome {
    (kind, Vec::new())
}

/// The GM's mock user id; any other user is a player.
const GM_USER: &str = "mock-gm";

fn warning(code: &str, message: String) -> Warning {
    Warning {
        code: code.to_string(),
        message,
    }
}

/// One attack step ready to roll: its dice parsed ahead of any die thrown.
struct PreparedAttack {
    step_index: u32,
    attack: dice::Notation,
    /// Each damage part as written, and with its dice doubled for a crit.
    damage: Vec<(dice::Notation, dice::Notation)>,
}

/// Parse an all-attack ability, or `None` if there are no steps, any step is
/// not an attack, or the dice text is outside what the mock parses.
fn prepare_attacks(ability: &Ability) -> Option<Vec<PreparedAttack>> {
    if ability.steps.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(ability.steps.len());
    for (i, step) in ability.steps.iter().enumerate() {
        let Some(effect_step::Step::Attack(a)) = &step.step else {
            return None;
        };
        let mut damage = Vec::with_capacity(a.damage.len());
        for part in &a.damage {
            let normal = dice::parse_notation(&part.dice)?;
            damage.push((normal, normal.doubled_dice()?));
        }
        out.push(PreparedAttack {
            step_index: u32::try_from(i).unwrap_or(u32::MAX),
            attack: dice::Notation::attack(a.to_hit_bonus),
            damage,
        });
    }
    Some(out)
}

impl Game {
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

    /// Roll an all-attack ability: attack and damage rolls per target, no
    /// hit point change. See the README for what is skipped.
    fn use_ability(&mut self, user_id: &str, u: &UseAbilityIntent) -> Result<Outcome, Rejection> {
        let is_gm = user_id == GM_USER;
        let not_permitted = || {
            (
                "not_permitted",
                "you do not control the source token".to_string(),
            )
        };
        let source_token = match self.tokens.iter().find(|t| t.token_id == u.source_token_id) {
            Some(t)
                if is_gm || (!t.hidden && t.controller_user_ids.iter().any(|c| c == user_id)) =>
            {
                t.clone()
            }
            Some(_) => return Err(not_permitted()),
            None if is_gm => {
                return Err(("unknown_token", format!("no token {:?}", u.source_token_id)))
            }
            None => return Err(not_permitted()),
        };
        let source = self
            .template
            .creatures
            .iter()
            .find(|c| {
                !source_token.creature_id.is_empty() && c.creature_id == source_token.creature_id
            })
            .cloned()
            .ok_or((
                "no_creature",
                "the source token has no creature".to_string(),
            ))?;
        let ability = source
            .abilities
            .iter()
            .find(|a| a.ability_id == u.ability_id)
            .cloned()
            .ok_or((
                "unknown_ability",
                format!("the creature has no ability {:?}", u.ability_id),
            ))?;
        let shape = ability
            .targeting
            .as_ref()
            .map(|t| TargetShape::try_from(t.shape).unwrap_or(TargetShape::Unspecified));
        if shape == Some(TargetShape::Creatures) && u.target_token_ids.is_empty() {
            return Err((
                "no_targets",
                "the ability needs at least one target".to_string(),
            ));
        }
        // (token id, armor class) per listed target.
        let mut targets: Vec<(String, i32)> = Vec::with_capacity(u.target_token_ids.len());
        for id in &u.target_token_ids {
            let creature = self
                .tokens
                .iter()
                .find(|t| &t.token_id == id && (is_gm || !t.hidden))
                .filter(|t| !t.creature_id.is_empty())
                .and_then(|t| {
                    self.template
                        .creatures
                        .iter()
                        .find(|c| c.creature_id == t.creature_id)
                });
            let Some(creature) = creature else {
                return Err((
                    "unknown_target",
                    format!("token {id:?} does not exist or has no creature"),
                ));
            };
            targets.push((id.clone(), creature.armor_class));
        }
        let area = matches!(
            shape,
            Some(TargetShape::Sphere | TargetShape::Cone | TargetShape::Line | TargetShape::Cube)
        );
        let steps = match prepare_attacks(&ability) {
            Some(steps) if !area => steps,
            _ => {
                return Err((
                    "not_supported_by_mock",
                    "the mock resolves only creature-targeted attack abilities".to_string(),
                ))
            }
        };

        let mut warnings = Vec::new();
        if steps.len() == 1 {
            let max = ability.targeting.as_ref().map_or(0, |t| t.max_targets);
            let limited = matches!(
                shape,
                Some(TargetShape::Creatures | TargetShape::Unspecified)
            );
            if limited && max > 0 && targets.len() > max as usize {
                warnings.push(warning(
                    "too_many_targets",
                    format!("{} targets listed, at most {max} allowed", targets.len()),
                ));
            }
        } else if targets.len() > steps.len() {
            warnings.push(warning(
                "extra_targets_ignored",
                format!(
                    "{} targets listed for {} attack steps; the extra are ignored",
                    targets.len(),
                    steps.len()
                ),
            ));
        }

        self.resolutions += 1;
        let n = self.resolutions;
        let mut roll_k = 0_u32;
        let pairs: Vec<(&PreparedAttack, &(String, i32))> = if steps.len() == 1 {
            targets.iter().map(|t| (&steps[0], t)).collect()
        } else {
            steps.iter().zip(targets.iter()).collect()
        };
        let mode = RollMode::try_from(u.mode).unwrap_or(RollMode::Unspecified);
        let mut outcomes = Vec::with_capacity(pairs.len());
        for (step, (token_id, armor_class)) in pairs {
            let attack = self.roll_for(
                &mut roll_k,
                n,
                user_id,
                &source.creature_id,
                &step.attack,
                mode,
                RollPurpose::Attack,
                format!("{} attack", ability.name),
            )?;
            let outcome = if attack.critical {
                HitOutcome::CriticalHit
            } else if attack.fumble || attack.total < *armor_class {
                HitOutcome::Miss
            } else {
                HitOutcome::Hit
            };
            let mut damage_rolls = Vec::new();
            if outcome != HitOutcome::Miss {
                for (normal, doubled) in &step.damage {
                    let notation = if outcome == HitOutcome::CriticalHit {
                        doubled
                    } else {
                        normal
                    };
                    damage_rolls.push(self.roll_for(
                        &mut roll_k,
                        n,
                        user_id,
                        &source.creature_id,
                        notation,
                        RollMode::Normal,
                        RollPurpose::Damage,
                        format!("{} damage", ability.name),
                    )?);
                }
            }
            outcomes.push(TargetOutcome {
                token_id: token_id.clone(),
                outcome: outcome as i32,
                roll: Some(attack),
                damage_rolls,
                damage: Vec::new(),
                healing: 0,
                effects_applied: Vec::new(),
                dropped_to_zero: false,
                step_index: step.step_index,
            });
        }
        let resolution = Resolution {
            resolution_id: format!("res-{n}"),
            source_token_id: source_token.token_id,
            ability_id: ability.ability_id,
            ability_name: ability.name,
            state: ResolutionState::Done as i32,
            outcomes,
            warnings: warnings.clone(),
        };
        Ok((
            event::Kind::ResolutionUpdated(ResolutionUpdated {
                resolution: Some(resolution),
            }),
            warnings,
        ))
    }

    /// One roll inside resolution `n`, id `roll-<n>-<k>` (the freeform ids are
    /// `roll-<seq>`, so the two never collide).
    #[allow(clippy::too_many_arguments)]
    fn roll_for(
        &mut self,
        roll_k: &mut u32,
        n: u64,
        user_id: &str,
        creature_id: &str,
        notation: &dice::Notation,
        mode: RollMode,
        purpose: RollPurpose,
        label: String,
    ) -> Result<RollResult, Rejection> {
        *roll_k += 1;
        let mut roll = dice::roll(
            self.rng.as_mut(),
            notation,
            mode,
            format!("roll-{n}-{roll_k}"),
            user_id,
            label,
        )
        .ok_or((
            "not_supported_by_mock",
            "a roll total was out of range".to_string(),
        ))?;
        roll.purpose = purpose as i32;
        roll.roller_creature_id = creature_id.to_string();
        Ok(roll)
    }

    fn move_token(
        &mut self,
        m: &tt_contracts::v1::MoveTokenIntent,
    ) -> Result<event::Kind, Rejection> {
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
        let Some(token) = self.tokens.iter_mut().find(|t| t.token_id == m.token_id) else {
            return Err(("unknown_token", format!("no token {:?}", m.token_id)));
        };
        let start = token.position.unwrap_or(m.path[0]);
        let distance_feet = path_distance_feet(&start, &m.path, cell_size, feet_per_cell);
        token.position = Some(destination);
        if !m.level_id.is_empty() {
            token.level_id = m.level_id.clone();
        }
        Ok(event::Kind::TokenMoved(TokenMoved {
            token_id: m.token_id.clone(),
            path: m.path.clone(),
            level_id: m.level_id.clone(),
            distance_feet,
        }))
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
    use tt_contracts::v1::HealthState;

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
        assert_eq!(snapshot.tokens.len(), 7);
        assert_eq!(snapshot.creatures.len(), 6);
        let aria = &snapshot.tokens[0];
        assert_eq!(aria.name, "Aria");
        assert_eq!(aria.hp_current, Some(24));
        assert_eq!(aria.controller_user_ids, ["mock-player"]);
        let goblin = &snapshot.tokens[1];
        assert_eq!(goblin.name, "Goblin");
        assert_eq!(goblin.hp_current, None);
        assert_eq!(goblin.hp_max, None);
        assert!(goblin.controller_user_ids.is_empty());
        assert_eq!(goblin.health_state, HealthState::Healthy as i32);
    }

    #[test]
    fn path_distance_counts_segments_in_feet() {
        let p = |x, y| Vec2 { x, y };
        let feet = path_distance_feet(&p(0.0, 0.0), &[p(70.0, 0.0), p(70.0, 140.0)], 70.0, 5.0);
        assert_eq!(feet, 15.0);
    }
}
