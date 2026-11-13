//! Thin socket layer: bytes in, bytes out. All decisions live in the core.
//!
//! Shape follows the mock server: first frame is `Hello`, reply `Snapshot`,
//! then intents up and events down. The server broadcasts each event
//! UNFILTERED as an `Arc<Event>`; every connection filters it for its own
//! viewer before encoding.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State as AxumState;
use axum::routing::get;
use axum::Router;
use prost::Message as _;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tt_contracts::v1::{
    client_message, server_message, ClientMessage, Event, Hello, Intent, IntentRejected,
    ServerMessage, Snapshot,
};

use crate::auth::authenticate;
use crate::campaign::{Campaign, Rejection};
use crate::ids::Uuid7Ids;
use crate::rng::OsRolls;
use crate::view::Viewer;

/// Largest accepted WebSocket message (same as the mock).
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
/// Events buffered per receiver before a slow client is dropped.
const BROADCAST_CAPACITY: usize = 256;
/// How long a new socket may take to send its Hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// What the server needs to start.
pub struct Config {
    /// Full, unfiltered starting state of the one campaign.
    pub seed: Snapshot,
    /// DEV ONLY, not security: accept `dev:<user_id>` tokens. See `auth`.
    pub dev_auth: bool,
}

struct State {
    campaign: Campaign,
    rng: OsRolls,
    ids: Uuid7Ids,
    tx: broadcast::Sender<Arc<Event>>,
}

struct Shared {
    // A std mutex is enough: it is never held across an await.
    state: Mutex<State>,
    dev_auth: bool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Handlers never panic while holding the lock; recover anyway.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Serve the game socket on `listener` until the task is dropped or the
/// listener fails.
pub async fn serve(listener: TcpListener, config: Config) -> std::io::Result<()> {
    let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            campaign: Campaign::from_snapshot(config.seed),
            rng: OsRolls::new(),
            ids: Uuid7Ids::new(),
            tx,
        }),
        dev_auth: config.dev_auth,
    });
    let app = Router::new()
        .route(
            "/",
            get(|| async { "Tome & Tile game server. WebSocket endpoint: /ws\n" }),
        )
        .route("/healthz", get(|| async { "ok" }))
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
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Encode a server message. Public so tests can check the wire round trip.
pub fn encode(kind: server_message::Kind) -> Bytes {
    Bytes::from(ServerMessage { kind: Some(kind) }.encode_to_vec())
}

/// The reply sent to the sender alone when an intent is refused.
pub fn rejection_message(client_intent_id: &str, r: &Rejection) -> ServerMessage {
    ServerMessage {
        kind: Some(server_message::Kind::IntentRejected(IntentRejected {
            client_intent_id: client_intent_id.to_string(),
            code: r.code.to_string(),
            message: r.message.clone(),
        })),
    }
}

fn decode_client(bytes: &[u8]) -> Option<ClientMessage> {
    ClientMessage::decode(bytes).ok()
}

async fn close(mut socket: WebSocket) {
    let _ = socket.send(Message::Close(None)).await;
}

/// Wait for the first binary frame and require it to be a Hello.
async fn read_hello(socket: &mut WebSocket) -> Option<Hello> {
    loop {
        match socket.recv().await {
            Some(Ok(Message::Binary(bytes))) => {
                return match decode_client(&bytes) {
                    Some(ClientMessage {
                        kind: Some(client_message::Kind::Hello(hello)),
                    }) => Some(hello),
                    _ => None,
                }
            }
            Some(Ok(Message::Text(_) | Message::Ping(_) | Message::Pong(_))) => continue,
            _ => return None,
        }
    }
}

async fn connection(mut socket: WebSocket, shared: Arc<Shared>) {
    // Phase 1: the first binary frame must be a Hello that authenticates.
    let hello = match tokio::time::timeout(HELLO_TIMEOUT, read_hello(&mut socket)).await {
        Ok(Some(hello)) => hello,
        _ => return close(socket).await,
    };
    let Some((viewer, snapshot, mut rx)) = join(&shared, &hello) else {
        return close(socket).await;
    };
    if socket.send(Message::Binary(snapshot)).await.is_err() {
        return;
    }

    // Phase 2: intents up, filtered events down.
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
                // A second Hello and media_token_request are ignored.
                if let Some(client_message::Kind::Intent(intent)) = message.kind {
                    if let Some(reply) = handle_intent(&shared, &viewer, intent) {
                        if socket.send(Message::Binary(reply)).await.is_err() {
                            return;
                        }
                    }
                }
            }
            outgoing = rx.recv() => match outgoing {
                Ok(event) => {
                    let filtered = shared.lock().campaign.event_for(&event, &viewer);
                    if let Some(event) = filtered {
                        let bytes = encode(server_message::Kind::Event(event));
                        if socket.send(Message::Binary(bytes)).await.is_err() {
                            return;
                        }
                    }
                }
                // Too slow: dropped; reconnecting gets a fresh Snapshot.
                Err(broadcast::error::RecvError::Lagged(_)) => return close(socket).await,
                Err(broadcast::error::RecvError::Closed) => return,
            },
        }
    }
}

/// Authenticate, then build this viewer's Snapshot and subscribe to events
/// under one lock so the client sees neither a gap nor a duplicate.
fn join(
    shared: &Shared,
    hello: &Hello,
) -> Option<(Viewer, Bytes, broadcast::Receiver<Arc<Event>>)> {
    let state = shared.lock();
    let viewer = authenticate(&state.campaign, shared.dev_auth, hello).ok()?;
    let rx = state.tx.subscribe();
    let snapshot = state.campaign.snapshot_for(&viewer);
    Some((viewer, encode(server_message::Kind::Snapshot(snapshot)), rx))
}

/// Apply one intent. Returns a direct reply (a rejection) for the sender, or
/// `None` when the outcome was broadcast as events.
fn handle_intent(shared: &Shared, viewer: &Viewer, intent: Intent) -> Option<Bytes> {
    let client_intent_id = intent.client_intent_id.clone();
    let mut guard = shared.lock();
    let state = &mut *guard;
    match state.campaign.apply(
        &viewer.user_id,
        intent,
        now_ms(),
        &mut state.rng,
        &mut state.ids,
    ) {
        Ok(events) => {
            // Sent while the lock is held so every client sees seq order.
            // No receivers is not an error.
            for event in events {
                let _ = state.tx.send(Arc::new(event));
            }
            None
        }
        Err(r) => rejection_message(&client_intent_id, &r).kind.map(encode),
    }
}
