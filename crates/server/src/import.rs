//! The HTTP side of creature import (ADR 0015):
//! `POST /campaigns/{campaign_id}/imports`. Same shape as asset upload
//! (`upload.rs`): auth before the body is read, GM only, CORS on every
//! response, JSON `{"code","message"}` errors, 120 s body read timeout. The
//! decisions (validation, re-import, ids, the event) are in
//! [`Campaign::import_creature`]; nothing is written to disk.
//!
//! Accepted today: one `Creature` in Protobuf JSON (the native format, what
//! Adventurer's Anvil exports). A Creature Cauldron export (a JSON object
//! with an `"app"` key) answers 415 `format_not_supported_yet` until the
//! importer crate (#105) is wired in.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::rejection::PathRejection;
use axum::extract::{DefaultBodyLimit, Path, State as AxumState};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use prost::Message as _;
use tt_contracts::v1::{Creature, ImportResult};

use crate::net::{now_ms, Shared};
use crate::upload::{
    authorize_gm, bearer_token, cors, http_error, is_length_limit, method_not_allowed, preflight,
    read_timeout, BODY_READ_TIMEOUT,
};

/// Largest accepted body for the native format: 1 MiB.
pub const MAX_IMPORT_BYTES: u64 = 1024 * 1024;
/// Most imports being received or processed at once.
pub(crate) const MAX_CONCURRENT_IMPORTS: usize = 4;
/// Longest parser message echoed back, in characters.
const MAX_ECHOED_CHARS: usize = 300;

pub(crate) fn routes() -> Router<Arc<Shared>> {
    Router::new().route(
        "/campaigns/{campaign_id}/imports",
        post(import_creature)
            .options(preflight)
            .fallback(method_not_allowed)
            .layer(DefaultBodyLimit::max(MAX_IMPORT_BYTES as usize)),
    )
}

fn too_large() -> Response {
    let mut response = http_error(
        StatusCode::PAYLOAD_TOO_LARGE,
        "file_too_large",
        &format!("file is larger than {MAX_IMPORT_BYTES} bytes"),
    );
    // The rest of the body is not read; do not wait for it.
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
}

fn bad_creature(message: &str) -> Response {
    let shown: String = message.chars().take(MAX_ECHOED_CHARS).collect();
    http_error(StatusCode::BAD_REQUEST, "bad_creature", &shown)
}

/// Parse the body: a Creature Cauldron export is refused for now, anything
/// else must be exactly one `Creature` in Protobuf JSON (unknown fields are
/// errors).
fn parse_native(bytes: &[u8]) -> Result<Creature, Box<Response>> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| Box::new(bad_creature(&format!("not JSON: {e}"))))?;
    if value.as_object().is_some_and(|o| o.contains_key("app")) {
        return Err(Box::new(http_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "format_not_supported_yet",
            "Creature Cauldron import arrives with #105; send a Creature in Protobuf JSON",
        )));
    }
    serde_json::from_value::<Creature>(value)
        .map_err(|e| Box::new(bad_creature(&format!("not a valid Creature: {e}"))))
}

async fn import_creature(
    AxumState(shared): AxumState<Arc<Shared>>,
    campaign_id: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Ok(Path(campaign_id)) = campaign_id else {
        return http_error(
            StatusCode::NOT_FOUND,
            "unknown_campaign",
            "no such campaign",
        );
    };
    let Some(token) = bearer_token(&headers) else {
        return http_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "send Authorization: Bearer <token>",
        );
    };
    let viewer = match authorize_gm(&shared, campaign_id, token, "only the GM may import") {
        Ok(viewer) => viewer,
        Err(response) => return *response,
    };

    // Held until this function returns.
    let Some(_permit) = shared.import_gate.try_enter() else {
        return http_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            "too many imports in progress; try again shortly",
        );
    };

    let declared_too_big = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > MAX_IMPORT_BYTES);
    if declared_too_big {
        return too_large();
    }
    let read = tokio::time::timeout(
        BODY_READ_TIMEOUT,
        axum::body::to_bytes(body, MAX_IMPORT_BYTES as usize),
    )
    .await;
    let bytes = match read {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(e)) if is_length_limit(&e) => return too_large(),
        Err(_) => return read_timeout(),
        Ok(Err(_)) => {
            return http_error(
                StatusCode::BAD_REQUEST,
                "bad_body",
                "could not read the request body",
            )
        }
    };

    let creature = match parse_native(&bytes) {
        Ok(c) => c,
        Err(response) => return *response,
    };

    let result = {
        let mut guard = shared.lock();
        let state = &mut *guard;
        let outcome = state.campaign.import_creature(
            &viewer.user_id,
            creature,
            Vec::new(),
            now_ms(),
            &mut state.ids,
        );
        outcome.map(|o| {
            // Sent while the lock is held so every client sees seq order.
            // No receivers is not an error.
            let _ = state.tx.send(Arc::new(o.event));
            ImportResult {
                creature: Some(o.creature),
                replaced: o.replaced,
                notes: Vec::new(),
            }
        })
    };
    match result {
        Ok(result) => {
            let mut response = Response::new(Body::from(result.encode_to_vec()));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/x-protobuf"),
            );
            cors(response)
        }
        Err(r) => {
            let status = match r.code {
                "bad_creature" => StatusCode::BAD_REQUEST,
                "too_many_creatures" => StatusCode::CONFLICT,
                "not_permitted" => StatusCode::FORBIDDEN,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            http_error(status, r.code, &r.message)
        }
    }
}
