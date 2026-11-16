//! Game server entry point. Configuration comes from the environment:
//!
//! * `TT_PORT` (default 8788), `TT_BIND` (default 127.0.0.1)
//! * `TT_SEED`: path to a Snapshot JSON file; unset uses the embedded dev seed
//! * `TT_DEV_AUTH=1`: enable `dev:<user_id>` login. DEV ONLY, NOT SECURITY.
//! * `TT_LIVEKIT_URL`, `TT_LIVEKIT_API_KEY`, `TT_LIVEKIT_API_SECRET`: all
//!   three enable video and voice, none leaves it off, anything in between
//!   is a start-up error. Values are never printed.

use std::process::ExitCode;

use tt_contracts::v1::Snapshot;
use tt_server::media::MediaConfig;
use tt_server::{serve, Config};

/// The dev seed, used when `TT_SEED` is unset.
const DEV_SEED: &str = include_str!("../../../contracts/fixtures/mock/snapshot.json");

fn load_seed() -> Result<Snapshot, String> {
    let text = match std::env::var("TT_SEED") {
        Ok(path) => std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read TT_SEED {path}: {e}"))?,
        Err(_) => {
            eprintln!("tt-server: TT_SEED not set; using the embedded dev seed");
            DEV_SEED.to_string()
        }
    };
    serde_json::from_str(&text).map_err(|e| format!("seed is not a valid Snapshot: {e}"))
}

/// Read the three LiveKit variables. Errors name variables, never values.
fn load_media() -> Result<Option<MediaConfig>, String> {
    const NAMES: [&str; 3] = [
        "TT_LIVEKIT_URL",
        "TT_LIVEKIT_API_KEY",
        "TT_LIVEKIT_API_SECRET",
    ];
    let mut values: Vec<Option<String>> = Vec::new();
    for name in NAMES {
        match std::env::var(name) {
            Ok(v) => values.push(Some(v)),
            Err(std::env::VarError::NotPresent) => values.push(None),
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(format!("{name} is not valid text"))
            }
        }
    }
    if values.iter().all(Option::is_none) {
        return Ok(None);
    }
    let missing: Vec<&str> = NAMES
        .iter()
        .zip(&values)
        .filter(|(_, v)| v.is_none())
        .map(|(n, _)| *n)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "video and voice need all of TT_LIVEKIT_URL, TT_LIVEKIT_API_KEY and \
             TT_LIVEKIT_API_SECRET; missing: {}",
            missing.join(", ")
        ));
    }
    let mut it = values.into_iter().flatten();
    let (url, key, secret) = (
        it.next().unwrap_or_default(),
        it.next().unwrap_or_default(),
        it.next().unwrap_or_default(),
    );
    MediaConfig::new(url, key, secret)
        .map(Some)
        .map_err(|e| format!("invalid LiveKit settings ({e}); check the TT_LIVEKIT_* variables"))
}

#[tokio::main]
async fn main() -> ExitCode {
    let seed = match load_seed() {
        Ok(seed) => seed,
        Err(message) => {
            eprintln!("tt-server: {message}");
            return ExitCode::FAILURE;
        }
    };
    let dev_auth = std::env::var("TT_DEV_AUTH").is_ok_and(|v| v == "1");
    if dev_auth {
        eprintln!(
            "tt-server: WARNING dev auth is ON: anyone who can reach the socket can sign in \
             as any member with a `dev:<user_id>` token. This is NOT security."
        );
    } else {
        eprintln!(
            "tt-server: no login method is enabled (set TT_DEV_AUTH=1 for dev auth); \
             every connection will be refused"
        );
    }
    let media = match load_media() {
        Ok(m) => m,
        Err(message) => {
            eprintln!("tt-server: {message}");
            return ExitCode::FAILURE;
        }
    };
    match &media {
        Some(m) => eprintln!("tt-server: video and voice ON (LiveKit at {})", m.sfu_url()),
        None => eprintln!("tt-server: video and voice OFF (TT_LIVEKIT_* not set)"),
    }
    let port = match std::env::var("TT_PORT") {
        Ok(p) => match p.parse::<u16>() {
            Ok(p) => p,
            Err(_) => {
                eprintln!("tt-server: TT_PORT {p:?} is not a port number");
                return ExitCode::FAILURE;
            }
        },
        Err(_) => 8788,
    };
    let bind = std::env::var("TT_BIND").unwrap_or_else(|_| "127.0.0.1".to_string());
    let listener = match tokio::net::TcpListener::bind((bind.as_str(), port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("tt-server: cannot listen on {bind}:{port}: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("tt-server: listening on {bind}:{port}");
    match serve(
        listener,
        Config {
            seed,
            dev_auth,
            media,
        },
    )
    .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tt-server: {e}");
            ExitCode::FAILURE
        }
    }
}
