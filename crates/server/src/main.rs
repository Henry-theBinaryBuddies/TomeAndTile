//! Game server entry point. Configuration comes from the environment:
//!
//! * `TT_PORT` (default 8788), `TT_BIND` (default 127.0.0.1)
//! * `TT_SEED`: path to a Snapshot JSON file; unset uses the embedded dev seed
//! * `TT_DEV_AUTH=1`: enable `dev:<user_id>` login. DEV ONLY, NOT SECURITY.

use std::process::ExitCode;

use tt_contracts::v1::Snapshot;
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
    match serve(listener, Config { seed, dev_auth }).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tt-server: {e}");
            ExitCode::FAILURE
        }
    }
}
