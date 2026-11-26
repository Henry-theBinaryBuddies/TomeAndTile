//! The HTTP side of asset upload (ADR 0014): `POST /campaigns/{id}/assets`
//! and `GET /assets/{id}.{ext}`. Thin I/O layer: auth and decisions reuse the
//! core (`auth::authenticate`, `Campaign::register_asset`), validation is the
//! pure `tt_asset_pipeline::inspect`, bytes go through [`AssetStore`].
//!
//! Every response, success or error, allows any origin. Every error is JSON
//! `{"code","message"}`. A token is never logged or echoed.

use std::error::Error as _;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::rejection::PathRejection;
use axum::extract::{DefaultBodyLimit, Path, State as AxumState};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use prost::Message as _;
use tt_asset_pipeline::{inspect, MAX_FILE_BYTES};
use tt_contracts::v1::{AssetRef, Hello, Role};

use crate::assets::{
    ext_for_media_type, media_type_for_ext, parse_file_name, AssetStore, UploadGate,
};
use crate::auth::{authenticate, AuthError};
use crate::net::Shared;
use crate::view::Viewer;

/// How long reading one upload body may take. A client that stalls (dropped
/// Wi-Fi, no FIN) must not hold an upload slot forever.
pub(crate) const BODY_READ_TIMEOUT: Duration = Duration::from_secs(120);

/// Everything the routes need when uploads are ON.
pub(crate) struct AssetState {
    pub store: Arc<dyn AssetStore>,
    pub public_url: Option<String>,
    pub gate: UploadGate,
}

/// The two asset routes, with CORS preflight and JSON 405 on each.
pub(crate) fn routes() -> Router<Arc<Shared>> {
    Router::new()
        .route(
            "/campaigns/{campaign_id}/assets",
            post(upload_asset)
                .options(preflight)
                .fallback(method_not_allowed)
                .layer(DefaultBodyLimit::max(MAX_FILE_BYTES as usize)),
        )
        .route(
            "/assets/{*file}",
            get(get_asset)
                .options(preflight)
                .fallback(method_not_allowed),
        )
}

pub(crate) fn cors(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

pub(crate) fn http_error(status: StatusCode, code: &str, message: &str) -> Response {
    let body = serde_json::json!({ "code": code, "message": message }).to_string();
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    cors(response)
}

pub(crate) async fn method_not_allowed() -> Response {
    http_error(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "this route does not support that method",
    )
}

pub(crate) async fn preflight() -> Response {
    let mut response = Response::new(Body::empty());
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

/// The bearer token from `Authorization: Bearer <token>`, scheme
/// case-insensitive.
pub(crate) fn bearer_token(headers: &HeaderMap) -> Option<&str> {
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

/// Base URL from the listener address. A wildcard address (0.0.0.0 or ::)
/// is not something a client can open, so it becomes loopback.
pub(crate) fn listener_base(addr: Option<SocketAddr>) -> String {
    match addr {
        Some(mut addr) => {
            if addr.ip().is_unspecified() {
                addr.set_ip(if addr.is_ipv4() {
                    std::net::Ipv4Addr::LOCALHOST.into()
                } else {
                    std::net::Ipv6Addr::LOCALHOST.into()
                });
            }
            format!("http://{addr}")
        }
        None => "http://127.0.0.1".to_string(),
    }
}

/// Base URL for links: the request's Host header (visible ASCII whitelist),
/// else the listener address.
pub(crate) fn base_url_from_host(headers: &HeaderMap, local_addr: Option<SocketAddr>) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .filter(|h| {
            !h.is_empty()
                && h.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
        });
    match host {
        Some(host) => format!("http://{host}"),
        None => listener_base(local_addr),
    }
}

/// Whether a body read failed because the limit was passed.
pub(crate) fn is_length_limit(err: &axum::Error) -> bool {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = err.source();
    while let Some(e) = cur {
        if e.to_string().contains("length limit exceeded") {
            return true;
        }
        cur = e.source();
    }
    false
}

fn too_large() -> Response {
    let mut response = http_error(
        StatusCode::PAYLOAD_TOO_LARGE,
        "file_too_large",
        &format!("file is larger than {MAX_FILE_BYTES} bytes"),
    );
    // The rest of the body is not read; do not wait for it.
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
}

pub(crate) fn read_timeout() -> Response {
    let mut response = http_error(
        StatusCode::REQUEST_TIMEOUT,
        "upload_timeout",
        "the upload took too long to arrive",
    );
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
}

fn storage_error(what: &str, err: &dyn std::fmt::Display) -> Response {
    eprintln!("tt-server: asset {what} failed: {err}");
    http_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "storage_error",
        "the server could not store the file",
    )
}

/// The Hello check for an HTTP request, GM only: the viewer, or the
/// ready-made 401, 403 or 404 response. Runs before any body byte is read.
pub(crate) fn authorize_gm(
    shared: &Shared,
    campaign_id: String,
    token: &str,
    forbidden_message: &str,
) -> Result<Viewer, Box<Response>> {
    let hello = Hello {
        session_token: token.to_string(),
        campaign_id,
        ..Default::default()
    };
    let auth = {
        let state = shared.lock();
        authenticate(&state.campaign, shared.dev_auth, &hello)
    };
    match auth {
        Ok(viewer) if viewer.role == Role::Gm => Ok(viewer),
        Ok(_) => Err(Box::new(http_error(
            StatusCode::FORBIDDEN,
            "not_permitted",
            forbidden_message,
        ))),
        Err(AuthError::WrongCampaign) => Err(Box::new(http_error(
            StatusCode::NOT_FOUND,
            "unknown_campaign",
            "no such campaign",
        ))),
        Err(AuthError::Disabled | AuthError::BadToken | AuthError::UnknownMember) => {
            Err(Box::new(http_error(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "the token is not accepted",
            )))
        }
    }
}

async fn upload_asset(
    AxumState(shared): AxumState<Arc<Shared>>,
    campaign_id: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some(assets) = shared.assets.as_ref() else {
        return http_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "uploads_off",
            "this server has asset upload turned off",
        );
    };
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

    // Same check as Hello, before a single body byte is read.
    if let Err(response) =
        authorize_gm(&shared, campaign_id, token, "only the GM may upload assets")
    {
        return *response;
    }

    // Held until this function returns.
    let Some(_permit) = assets.gate.try_enter() else {
        return http_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            "too many uploads in progress; try again shortly",
        );
    };

    let declared_too_big = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > MAX_FILE_BYTES);
    if declared_too_big {
        return too_large();
    }
    let read = tokio::time::timeout(
        BODY_READ_TIMEOUT,
        axum::body::to_bytes(body, MAX_FILE_BYTES as usize),
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

    let described = match inspect(&bytes) {
        Ok(a) => a,
        Err(e) => {
            return http_error(StatusCode::BAD_REQUEST, e.code(), &e.to_string());
        }
    };
    let Some(ext) = ext_for_media_type(&described.media_type) else {
        return http_error(
            StatusCode::BAD_REQUEST,
            "unknown_format",
            "only PNG, JPEG and WebP are accepted",
        );
    };

    let store = assets.store.clone();
    let id = described.asset_id.clone();
    let stored = tokio::task::spawn_blocking(move || store.put(&id, ext, &bytes)).await;
    match stored {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return storage_error("write", &e),
        Err(e) => return storage_error("write task", &e),
    }

    let base = match &assets.public_url {
        Some(url) => url.clone(),
        None => base_url_from_host(&headers, shared.local_addr),
    };
    let asset_ref = AssetRef {
        url: format!("{base}/assets/{}.{ext}", described.asset_id),
        ..described
    };
    let asset_ref = {
        let mut state = shared.lock();
        let provisional = state.provisional_assets.remove(&asset_ref.asset_id);
        match state.campaign.assets.get(&asset_ref.asset_id) {
            // A start-up guess of the link is replaced by the real one.
            Some(_) if provisional => {
                state
                    .campaign
                    .assets
                    .insert(asset_ref.asset_id.clone(), asset_ref.clone());
                asset_ref
            }
            Some(existing) => existing.clone(),
            None => {
                state.campaign.register_asset(asset_ref.clone());
                asset_ref
            }
        }
    };

    let mut response = Response::new(Body::from(asset_ref.encode_to_vec()));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-protobuf"),
    );
    cors(response)
}

fn unknown_asset() -> Response {
    http_error(StatusCode::NOT_FOUND, "unknown_asset", "no such asset")
}

async fn get_asset(
    AxumState(shared): AxumState<Arc<Shared>>,
    file: Result<Path<String>, PathRejection>,
) -> Response {
    let Some(assets) = shared.assets.as_ref() else {
        return unknown_asset();
    };
    let Ok(Path(file)) = file else {
        return unknown_asset();
    };
    let Some((id, ext)) = parse_file_name(&file) else {
        return unknown_asset();
    };
    let (id, store) = (id.to_string(), assets.store.clone());
    let read = tokio::task::spawn_blocking(move || store.get(&id, ext)).await;
    let bytes = match read {
        Ok(Ok(Some(bytes))) => bytes,
        Ok(Ok(None)) => return unknown_asset(),
        Ok(Err(e)) => return storage_error("read", &e),
        Err(e) => return storage_error("read task", &e),
    };
    let Some(media_type) = media_type_for_ext(ext) else {
        return unknown_asset();
    };
    let mut response = Response::new(Body::from(bytes));
    let h = response.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(media_type));
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    cors(response)
}

#[cfg(test)]
mod tests {
    use super::listener_base;

    // Checked here, not by binding 0.0.0.0 in an integration test: a test
    // that listens on every interface sets off the Windows firewall prompt.
    #[test]
    fn a_wildcard_listener_becomes_loopback() {
        let v4 = "0.0.0.0:8788".parse().ok();
        assert_eq!(listener_base(v4), "http://127.0.0.1:8788");
        let v6 = "[::]:8788".parse().ok();
        assert_eq!(listener_base(v6), "http://[::1]:8788");
        let lan = "192.168.1.20:8788".parse().ok();
        assert_eq!(listener_base(lan), "http://192.168.1.20:8788");
        assert_eq!(listener_base(None), "http://127.0.0.1");
    }
}
