use std::{fs, time::Duration};

use procast_core::serve::MediaServer;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

async fn request(url: &str, method: &str, headers: &str) -> String {
    let (host, path) = url
        .strip_prefix("http://")
        .unwrap()
        .split_once('/')
        .unwrap();
    let mut stream = TcpStream::connect(host).await.unwrap();
    stream
        .write_all(
            format!(
                "{method} /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n{headers}\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    String::from_utf8(bytes).unwrap()
}

#[tokio::test]
async fn serves_selected_resources_ranges_head_and_subtitle_cors() {
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("video");
    let subtitles = dir.path().join("captions");
    fs::write(&video, b"0123456789").unwrap();
    fs::write(&subtitles, b"WEBVTT\n\n").unwrap();
    let server = MediaServer::start("127.0.0.1:0".parse().unwrap(), &video, Some(&subtitles))
        .await
        .unwrap();
    let full = request(&server.video_url, "GET", "").await;
    assert!(full.starts_with("HTTP/1.1 200"));
    assert!(full.ends_with("0123456789"));
    let range = request(&server.video_url, "GET", "Range: bytes=2-5\r\n").await;
    assert!(range.starts_with("HTTP/1.1 206"));
    assert!(range.contains("content-range: bytes 2-5/10"));
    assert!(range.ends_with("2345"));
    let head = request(&server.video_url, "HEAD", "").await;
    assert!(head.contains("content-length: 10"));
    assert!(head.ends_with("\r\n\r\n"));
    assert!(
        request(&server.video_url, "GET", "Range: bytes=99-100\r\n")
            .await
            .starts_with("HTTP/1.1 416")
    );
    let vtt = request(
        server.subtitle_url.as_deref().unwrap(),
        "GET",
        "Origin: https://www.gstatic.com\r\n",
    )
    .await;
    assert!(vtt.contains("content-type: text/vtt"));
    assert!(vtt.contains("access-control-allow-origin: *"));
    let options = request(server.subtitle_url.as_deref().unwrap(), "OPTIONS", "Origin: https://www.gstatic.com\r\nAccess-Control-Request-Method: GET\r\nAccess-Control-Request-Headers: range\r\n").await;
    assert!(options.contains("access-control-allow-methods: GET,HEAD,OPTIONS"));
    let wrong = server.video_url.replace("video.mp4", "../../etc/passwd");
    assert!(request(&wrong, "GET", "").await.starts_with("HTTP/1.1 404"));
    let address = server
        .video_url
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    // Leave an incomplete request connected: shutdown must close the socket,
    // not merely stop accepting new clients.
    let mut stalled = TcpStream::connect(&address).await.unwrap();
    stalled
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    server.shutdown().await.unwrap();
    let mut remaining = Vec::new();
    let result =
        tokio::time::timeout(Duration::from_secs(1), stalled.read_to_end(&mut remaining)).await;
    assert!(result.is_ok(), "stalled HTTP connection survived shutdown");
    assert!(TcpStream::connect(address).await.is_err());
}
