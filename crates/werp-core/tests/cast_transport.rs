//! Loopback fake Cast endpoint. Never discovers or touches a real receiver.
use std::{sync::Arc, time::Duration};

use prost::Message;
use rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};
use tokio_rustls::TlsAcceptor;
use werp_core::{CancellationToken, WerpError, cast::CastSession};

#[derive(Clone, PartialEq, Message)]
struct Envelope {
    #[prost(int32, required, tag = "1")]
    protocol_version: i32,
    #[prost(string, required, tag = "2")]
    source_id: String,
    #[prost(string, required, tag = "3")]
    destination_id: String,
    #[prost(string, required, tag = "4")]
    namespace: String,
    #[prost(int32, required, tag = "5")]
    payload_type: i32,
    #[prost(string, optional, tag = "6")]
    payload_utf8: Option<String>,
}

#[tokio::test]
async fn raw_launch_correlates_replies_and_disconnect_does_not_stop_foreign_media() {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.cert.der().clone()],
                PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into(),
            )
            .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (ready_tx, ready_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut stream = TlsAcceptor::from(Arc::new(config))
            .accept(socket)
            .await
            .unwrap();
        let mut ready_tx = Some(ready_tx);
        while let Ok(length) = stream.read_u32().await {
            assert!(length <= 65536);
            let mut bytes = vec![0; length as usize];
            stream.read_exact(&mut bytes).await.unwrap();
            let message = Envelope::decode(bytes.as_slice()).unwrap();
            let payload: serde_json::Value =
                serde_json::from_str(message.payload_utf8.as_deref().unwrap()).unwrap();
            assert_ne!(
                payload["type"], "STOP",
                "disconnect must not stop foreign media"
            );
            if payload["type"] == "LAUNCH" {
                let id = payload["requestId"].as_u64().unwrap();
                let unrelated = serde_json::json!({"type":"LAUNCH_ERROR","requestId":id+100,"reason":"unrelated"});
                let correct = serde_json::json!({"type":"RECEIVER_STATUS","requestId":id,"status":{"applications":[{"appId":"CC1AD845","sessionId":"test-session","transportId":"test-transport"}]}});
                for payload in [unrelated, correct] {
                    let response = Envelope {
                        protocol_version: 0,
                        source_id: "receiver-0".into(),
                        destination_id: "sender-0".into(),
                        namespace: message.namespace.clone(),
                        payload_type: 0,
                        payload_utf8: Some(payload.to_string()),
                    }
                    .encode_to_vec();
                    stream.write_u32(response.len() as u32).await.unwrap();
                    // Split the frame across writes; a request must not confuse
                    // framing with response correlation.
                    stream.write_all(&response[..2]).await.unwrap();
                    tokio::task::yield_now().await;
                    stream.write_all(&response[2..]).await.unwrap();
                    stream.flush().await.unwrap();
                }
            }
            if payload["type"] == "CONNECT" && message.destination_id == "test-transport" {
                let response = Envelope { protocol_version:0, source_id:"test-transport".into(), destination_id:"sender-0".into(), namespace:"urn:x-cast:com.google.cast.media".into(), payload_type:0,
                    payload_utf8:Some(serde_json::json!({"type":"MEDIA_STATUS","status":[{"mediaSessionId":999,"playerState":"PLAYING","currentTime":42,"media":{"contentId":"another-sender","contentType":"video/mp4"}}]}).to_string()) }.encode_to_vec();
                stream.write_u32(response.len() as u32).await.unwrap();
                stream.write_all(&response).await.unwrap();
                stream.flush().await.unwrap();
                ready_tx.take().unwrap().send(()).unwrap();
            }
        }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        let token = CancellationToken::new();
        let mut session = CastSession::connect(address, &token).await.unwrap();
        session.launch(&token).await.unwrap();
        ready_rx.await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        session.close().await.unwrap();
        server.await.unwrap();
    })
    .await
    .expect("raw launch / transport-only close did not complete");
}

#[tokio::test]
async fn cancelled_tls_handshake_closes_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let token = CancellationToken::new();
    let server = async {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 8192];
        assert!(socket.read(&mut buffer).await.unwrap() > 0);
        token.cancel();
        assert_eq!(socket.read(&mut buffer).await.unwrap(), 0);
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(CastSession::connect(address, &token), server)
    })
    .await
    .unwrap();
    assert!(matches!(result, Err(WerpError::Cancelled)));
}

#[tokio::test]
async fn unreachable_endpoint_and_stalled_handshake_fail_within_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let token = CancellationToken::new();
    assert!(CastSession::connect(address, &token).await.is_err());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        socket.read_to_end(&mut bytes).await.unwrap();
        assert!(!bytes.is_empty());
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(12), async {
        tokio::join!(CastSession::connect(address, &token), server)
    })
    .await
    .expect("connect deadline did not close the socket");
    assert!(matches!(result, Err(WerpError::Cast(message)) if message.contains("timed out")));
}

#[tokio::test]
async fn pending_request_handles_disconnect_and_cancelled_partial_frame() {
    for cancel_partial in [false, true] {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![cert.cert.der().clone()],
                    PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into(),
                )
                .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (ready_tx, ready_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut stream = TlsAcceptor::from(Arc::new(config))
                .accept(socket)
                .await
                .unwrap();
            loop {
                let length = stream.read_u32().await.unwrap();
                assert!(length <= 65536);
                let mut bytes = vec![0; length as usize];
                stream.read_exact(&mut bytes).await.unwrap();
                let message = Envelope::decode(bytes.as_slice()).unwrap();
                let payload: serde_json::Value =
                    serde_json::from_str(message.payload_utf8.as_deref().unwrap()).unwrap();
                if payload["type"] == "LAUNCH" {
                    break;
                }
            }
            if cancel_partial {
                // A real TLS connection with an incomplete Cast frame. Cancelling
                // must close it; the partial reader must never be reused.
                stream.write_all(&[0, 0]).await.unwrap();
                stream.flush().await.unwrap();
                ready_tx.send(()).unwrap();
                let mut bytes = Vec::new();
                let _ = stream.read_to_end(&mut bytes).await;
            } else {
                ready_tx.send(()).unwrap();
                // Dropping the stream simulates a lost connection mid-request.
            }
        });
        let token = CancellationToken::new();
        let mut session = CastSession::connect(address, &token).await.unwrap();
        let control = async {
            ready_rx.await.unwrap();
            if cancel_partial {
                token.cancel();
            }
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(session.launch(&token), control)
        })
        .await
        .expect("pending request did not terminate");
        if cancel_partial {
            assert!(matches!(result, Err(WerpError::Cancelled)));
        } else {
            assert!(result.is_err());
            assert!(!session.is_connected());
        }
        tokio::time::timeout(Duration::from_secs(3), session.close())
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }
}
