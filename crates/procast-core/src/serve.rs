//! Session-scoped, selected-file HTTP serving for the feasibility prototype.
use std::{
    net::{IpAddr, SocketAddr},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::{Router, http::Method, middleware};
use tokio::{
    net::{TcpListener, UdpSocket},
    task::{JoinHandle, JoinSet},
};
use tower_http::{
    cors::{Any, CorsLayer},
    services::ServeFile,
};

use crate::{CancellationToken, ProcastError};

pub struct MediaServer {
    pub video_url: String,
    pub subtitle_url: String,
    pub requests: Arc<AtomicU64>,
    cancel: CancellationToken,
    task: JoinHandle<Result<(), std::io::Error>>,
}

impl MediaServer {
    pub async fn start(
        address: SocketAddr,
        video: &Path,
        subtitles: &Path,
    ) -> Result<Self, ProcastError> {
        for path in [video, subtitles] {
            let meta = tokio::fs::metadata(path)
                .await
                .map_err(|e| ProcastError::Serve(e.to_string()))?;
            if !meta.is_file() {
                return Err(ProcastError::NotRegularFile(path.into()));
            }
        }
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|e| ProcastError::Serve(e.to_string()))?;
        let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let video_path = format!("/{token}/video.mp4");
        let subtitle_path = format!("/{token}/subtitles.vtt");
        let requests = Arc::new(AtomicU64::new(0));
        let counter = requests.clone();
        let router = Router::new()
            .route_service(&video_path, ServeFile::new_with_mime(video, &"video/mp4".parse().unwrap()))
            .route_service(&subtitle_path, ServeFile::new_with_mime(subtitles, &"text/vtt".parse().unwrap()))
            .layer(CorsLayer::new().allow_origin(Any).allow_methods([Method::GET, Method::HEAD, Method::OPTIONS]).allow_headers(Any))
            .layer(middleware::from_fn(move |request: axum::extract::Request, next: middleware::Next| {
                let counter = counter.clone();
                async move {
                    let method = request.method().clone();
                    let path = request.uri().path().to_owned();
                    let response = next.run(request).await;
                    counter.fetch_add(1, Ordering::Relaxed);
                    tracing::info!(%method, %path, status = %response.status(), "media HTTP request");
                    response
                }
            }));
        let listener = TcpListener::bind(address)
            .await
            .map_err(|e| ProcastError::Serve(e.to_string()))?;
        let address = listener
            .local_addr()
            .map_err(|e| ProcastError::Serve(e.to_string()))?;
        let cancel = CancellationToken::new();
        let shutdown = cancel.clone();
        let task = tokio::spawn(async move {
            // Own every connection task so cancellation closes even stalled
            // downloads. Aborting a detached graceful server alone is not enough.
            let mut connections = JoinSet::new();
            let result = loop {
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break Ok(()),
                    Some(_) = connections.join_next(), if !connections.is_empty() => {},
                    accepted = listener.accept() => {
                        let (socket, _) = match accepted { Ok(pair) => pair, Err(error) => break Err(error) };
                        let service = hyper_util::service::TowerToHyperService::new(router.clone());
                        connections.spawn(async move {
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(hyper_util::rt::TokioIo::new(socket), service).await;
                        });
                    }
                }
            };
            connections.shutdown().await;
            result
        });
        Ok(Self {
            video_url: format!("http://{address}{video_path}"),
            subtitle_url: format!("http://{address}{subtitle_path}"),
            requests,
            cancel,
            task,
        })
    }

    pub async fn shutdown(mut self) -> Result<(), ProcastError> {
        self.cancel.cancel();
        match tokio::time::timeout(Duration::from_secs(2), &mut self.task).await {
            Ok(result) => result
                .map_err(|e| ProcastError::Serve(e.to_string()))?
                .map_err(|e| ProcastError::Serve(e.to_string()))?,
            Err(_) => {
                self.task.abort();
                let _ = (&mut self.task).await;
            }
        }
        Ok(())
    }
}

impl Drop for MediaServer {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

pub async fn address_toward(receiver: SocketAddr) -> Result<IpAddr, ProcastError> {
    let socket = UdpSocket::bind(if receiver.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })
    .await
    .map_err(|e| ProcastError::Serve(e.to_string()))?;
    socket
        .connect(receiver)
        .await
        .map_err(|e| ProcastError::Serve(e.to_string()))?;
    Ok(socket
        .local_addr()
        .map_err(|e| ProcastError::Serve(e.to_string()))?
        .ip())
}
