//! Test-only helpers shared across modules.

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

/// Serves one fixed 200 response and then closes. Avoids a dev-dependency just
/// to stand up a feed for the scheduler's dry-run gate tests.
pub async fn spawn_single_response_server(body: &'static str) -> String {
    spawn_single_response_server_with(200, "application/rss+xml", body).await
}

/// Same, with an arbitrary status and content type, for the `/feedcheck`
/// classification tests (404 vs unparseable vs empty).
pub async fn spawn_single_response_server_with(
    status: u16,
    content_type: &'static str,
    body: &'static str,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        loop {
            let n = socket.read(&mut buf).await.unwrap();
            if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let response = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
    });
    format!("http://{addr}/feed")
}
