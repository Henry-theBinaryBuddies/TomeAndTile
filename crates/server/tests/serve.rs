//! End-to-end check of the HTTP side of `serve`, over a raw TCP stream.
//! (The WebSocket loop is not tested here: no WebSocket client is approved.)

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tt_contracts::v1::Snapshot;
use tt_server::{serve, Config};

async fn get(addr: std::net::SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let request = format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut out))
        .await
        .expect("response in time")
        .expect("read");
    String::from_utf8_lossy(&out).into_owned()
}

#[tokio::test]
async fn healthz_and_banner_answer() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(serve(
        listener,
        Config {
            seed: Snapshot::default(),
            dev_auth: false,
        },
    ));
    let health = get(addr, "/healthz").await;
    assert!(health.starts_with("HTTP/1.1 200"), "{health}");
    assert!(health.ends_with("ok"), "{health}");
    let banner = get(addr, "/").await;
    assert!(banner.contains("Tome & Tile game server"), "{banner}");
    server.abort();
}
