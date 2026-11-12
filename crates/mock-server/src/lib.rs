//! Mock game server: speaks the real game-socket protocol from recorded data.
//!
//! It applies NO rules and NO permissions and does no per-viewer filtering.
//! It exists so the web UI and importers can be built before the real server
//! does. See the crate README.

pub mod dice;

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State as AxumState;
use axum::routing::get;
use axum::Router;
use prost::Message as _;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tt_contracts::v1::{
    client_message, event, intent, server_message, ChatPosted, ClientMessage, DiceRolled, Event,
    Hello, Intent, IntentRejected, Role, ServerMessage, Snapshot, Token, TokenMoved, Vec2,
};

use dice::{Rng, XorShift64Star};

/// The recorded game state every connection starts from.
const FIXTURE: &str = include_str!("../../../contracts/fixtures/mock/snapshot.json");

/// Largest accepted WebSocket message.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
/// Longest accepted chat text, in characters.
pub const MAX_CHAT_CHARS: usize = 2000;
/// Events buffered per receiver before a slow client is dropped.
const BROADCAST_CAPACITY: usize = 256;

/// Parse the embedded fixture. Panics only if the committed fixture is broken,
/// which a unit test guards against.
pub fn fixture_snapshot() -> Snapshot {
    serde_json::from_str(FIXTURE).expect("contracts/fixtures/mock/snapshot.json parses as Snapshot")
}

/// State shared by every connection in the process.
struct Game {
    /// The fixture, kept for scene data and members.
    template: Snapshot,
    tokens: Vec<Token>,
    /// Sequence of the last event emitted; 0 before any.
    seq: u64,
    rng: Box<dyn Rng + Send>,
}

struct Shared {
    // A std mutex is enough: it is never held across an await.
    game: Mutex<Game>,
    tx: broadcast::Sender<Bytes>,
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
    let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
    let shared = Arc::new(Shared {
        game: Mutex::new(Game {
            tokens: template.tokens.clone(),
            template,
            seq: 0,
            rng,
        }),
        tx,
    });
    let app = Router::new()
        .route(
            "/",
            get(|| async { "Tome & Tile mock server. WebSocket endpoint: /ws\n" }),
        )
        .route("/ws", get(ws_upgrade))
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
        Some(intent::Kind::MoveToken(m)) => game.move_token(m),
        Some(intent::Kind::Chat(c)) => game.chat(user_id, c.text.as_str()),
        Some(intent::Kind::RollDice(r)) => game.roll_dice(user_id, r),
        _ => Err((
            "not_supported_by_mock",
            "the mock server does not support this intent".to_string(),
        )),
    };
    match result {
        Ok(kind) => {
            game.seq += 1;
            let scene_id = game
                .template
                .scenes
                .first()
                .map(|s| s.scene_id.clone())
                .unwrap_or_default();
            let event = Event {
                seq: game.seq,
                at_ms: now_ms(),
                scene_id,
                actor_user_id: user_id.to_string(),
                client_intent_id: id.to_string(),
                warnings: Vec::new(),
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

impl Game {
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
        assert_eq!(snapshot.tokens.len(), 2);
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
