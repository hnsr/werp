//! Explicit hardware test, not part of normal `cargo test` and not the final CLI.
use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use clap::Parser;
use serde_json::json;
use yeet_core::{
    CancellationToken, YeetError,
    cast::CastSession,
    discovery,
    serve::{MediaServer, address_toward},
};

#[derive(Parser)]
#[command(
    about = "M1 hardware prototype: compatible MP4 + WebVTT; replaces selected receiver playback"
)]
struct Args {
    #[arg(long)]
    discover: bool,
    #[arg(long, conflicts_with = "host")]
    device: Option<String>,
    #[arg(long, conflicts_with = "device")]
    host: Option<IpAddr>,
    #[arg(long, default_value_t = 8009)]
    cast_port: u16,
    #[arg(long)]
    video: Option<PathBuf>,
    #[arg(long)]
    subtitles: Option<PathBuf>,
    #[arg(long, default_value_t = 0)]
    http_port: u16,
    #[arg(long)]
    bind_address: Option<IpAddr>,
    #[arg(long, default_value_t = 620)]
    seconds: u64,
    #[arg(long)]
    exercise_controls: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "yeet_core=info,oxicast=warn".into()),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let token = CancellationToken::new();
    let shutdown = token.clone();
    // Register both before discovery/connect; no blocking terminal reader.
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let signals = tokio::spawn(async move {
        tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
        shutdown.cancel();
    });
    let result = run(args, &token).await;
    signals.abort();
    let _ = signals.await;
    match result {
        Err(YeetError::Cancelled) => {
            eprintln!("Cancelled; cleanup completed.");
            Ok(())
        }
        result => result.map_err(Into::into),
    }
}

async fn run(args: Args, token: &CancellationToken) -> Result<(), YeetError> {
    let receiver = if let Some(host) = args.host {
        SocketAddr::new(host, args.cast_port)
    } else {
        let devices = discovery::discover(Duration::from_secs(5), token).await?;
        for device in &devices {
            println!(
                "{} | {} | {} | {:?}:{} | capabilities={:?}",
                device.name,
                device.model,
                device.id,
                device.addresses,
                device.port,
                device.capabilities
            );
        }
        if args.discover {
            return Ok(());
        }
        let selector = args
            .device
            .as_deref()
            .ok_or_else(|| YeetError::Discovery("provide --device NAME/ID or --host IP".into()))?;
        let device = discovery::select(&devices, selector)?;
        if device.capabilities.is_some_and(|bits| bits & 1 == 0) {
            return Err(YeetError::Discovery(
                "selected receiver does not advertise video output".into(),
            ));
        }
        SocketAddr::new(
            (*device
                .addresses
                .first()
                .ok_or_else(|| YeetError::Discovery("no IPv4 address".into()))?)
            .into(),
            device.port,
        )
    };
    let video = args
        .video
        .ok_or_else(|| YeetError::Serve("provide --video compatible.mp4".into()))?;
    let subtitles = args
        .subtitles
        .ok_or_else(|| YeetError::Serve("provide --subtitles valid.vtt".into()))?;
    let bind = match args.bind_address {
        Some(bind) => bind,
        None => address_toward(receiver).await?,
    };
    let server = MediaServer::start(
        SocketAddr::new(bind, args.http_port),
        &video,
        Some(&subtitles),
    )
    .await?;
    eprintln!("Serving {} and {:?}", server.video_url, server.subtitle_url);
    let connected = CastSession::connect(receiver, token).await;
    let mut session = match connected {
        Ok(session) => session,
        Err(error) => {
            server.shutdown().await?;
            return Err(error);
        }
    };
    let playback = async {
        session.launch(token).await?;
        eprintln!("Default Media Receiver launched.");
        let initial = session
            .load(
                &server.video_url,
                server.subtitle_url.as_deref(),
                "Yeet M1 test",
                0.0,
                token,
            )
            .await?;
        eprintln!("LOAD + subtitle activation: {initial:?}");
        let start = Instant::now();
        let mut actions = 0;
        while start.elapsed() < Duration::from_secs(args.seconds) {
            tokio::select! {
                _ = token.cancelled() => return Err(YeetError::Cancelled),
                _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            }
            if !session.is_connected() {
                return Err(YeetError::Cast(
                    "connection lost; automatic reconnect disabled".into(),
                ));
            }
            let Some(status) = session.status(token).await? else {
                continue;
            };
            println!(
                "elapsed={}s state={} position={:.1}s subtitles={:?} HTTP={}",
                start.elapsed().as_secs(),
                status.player_state,
                status.current_time,
                status.active_track_ids,
                server.requests.load(Ordering::Relaxed)
            );
            if status.player_state == "IDLE" {
                return if status.idle_reason.as_deref() == Some("FINISHED") {
                    Ok(())
                } else {
                    Err(YeetError::Cast(format!("unexpected idle: {status:?}")))
                };
            }
            if args.exercise_controls {
                let elapsed = start.elapsed().as_secs();
                let action = match actions {
                    0 if elapsed >= 20 => Some(("PAUSE", json!({}))),
                    1 if elapsed >= 25 => Some(("PLAY", json!({}))),
                    2 if elapsed >= 40 => Some((
                        "SEEK",
                        json!({"currentTime":90,"resumeState":"PLAYBACK_START"}),
                    )),
                    3 if elapsed >= 60 => Some((
                        "SEEK",
                        json!({"currentTime":20,"resumeState":"PLAYBACK_START"}),
                    )),
                    _ => None,
                };
                if let Some((kind, extra)) = action {
                    eprintln!("{kind}: {:?}", session.command(kind, extra, token).await?);
                    actions += 1;
                }
            }
        }
        Ok(())
    }
    .await;
    let cleanup_start = Instant::now();
    let remote_cleanup = session.close().await;
    let local_cleanup = server.shutdown().await;
    eprintln!(
        "Cleanup finished in {:?}; remote={remote_cleanup:?}",
        cleanup_start.elapsed()
    );
    local_cleanup?;
    playback?;
    remote_cleanup
}
