//! Tome & Tile mock server binary. See the crate README.

use tokio::net::TcpListener;

const DEFAULT_PORT: u16 = 8787;

#[tokio::main]
async fn main() {
    let port = match std::env::var("TT_MOCK_PORT") {
        Ok(value) => match value.trim().parse::<u16>() {
            Ok(port) => port,
            Err(_) => {
                eprintln!("TT_MOCK_PORT must be a port number 0-65535, got {value:?}");
                std::process::exit(2);
            }
        },
        Err(_) => DEFAULT_PORT,
    };
    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("cannot bind 127.0.0.1:{port}: {err}");
            std::process::exit(1);
        }
    };
    let addr = listener
        .local_addr()
        .expect("bound listener has an address");
    println!("Tome & Tile mock server listening on ws://{addr}/ws");
    if let Err(err) = tt_mock_server::serve(listener).await {
        eprintln!("server error: {err}");
        std::process::exit(1);
    }
}
