use std::{
    io::{self, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};

use clap::{Parser, Subcommand};
use procast_core::{
    CancellationToken, ProcastError, discovery,
    media::{self, MediaInfo, ProbeOptions},
    session::{self, CastRequest, Phase, Target},
};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "procast",
    version,
    about = "Cast local videos and subtitles to a Chromecast"
)]
struct Cli {
    /// Show debug diagnostics on stderr
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum ModeArg {
    Auto,
    Direct,
    Remux,
    Audio,
    Transcode,
}
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum ProfileArg {
    Auto,
    Baseline,
    Extended,
    Experimental,
}

impl From<ModeArg> for procast_core::playback::Mode {
    fn from(mode: ModeArg) -> Self {
        match mode {
            ModeArg::Auto => Self::Auto,
            ModeArg::Direct => Self::Direct,
            ModeArg::Remux => Self::Remux,
            ModeArg::Audio => Self::Audio,
            ModeArg::Transcode => Self::Transcode,
        }
    }
}
impl From<ProfileArg> for procast_core::playback::Profile {
    fn from(profile: ProfileArg) -> Self {
        match profile {
            ProfileArg::Auto => Self::Auto,
            ProfileArg::Baseline => Self::Baseline,
            ProfileArg::Extended => Self::Extended,
            ProfileArg::Experimental => Self::Experimental,
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// List Cast receivers on the local network (does not start playback)
    Devices {
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=60))]
        scan_seconds: u64,
        #[arg(long)]
        json: bool,
    },
    /// Cast a local video, with optional preparation and external SRT/WebVTT subtitles
    Cast {
        file: PathBuf,
        /// Exact friendly name or stable device ID
        #[arg(long, conflicts_with = "host")]
        device: Option<String>,
        /// Explicit receiver IPv4 address; bypasses discovery
        #[arg(long, conflicts_with = "device")]
        host: Option<Ipv4Addr>,
        #[arg(long, default_value_t = 8009, requires = "host", value_parser = clap::value_parser!(u16).range(1..))]
        cast_port: u16,
        #[arg(long)]
        subtitles: Option<PathBuf>,
        /// Select automatically, or require one preparation path
        #[arg(long, value_enum, default_value_t = ModeArg::Auto)]
        mode: ModeArg,
        /// Receiver compatibility profile (auto uses discovered model; unknown/--host uses baseline)
        #[arg(long, value_enum, default_value_t = ProfileArg::Auto)]
        profile: ProfileArg,
        /// Do not reuse or retain prepared files; remove them after this session
        #[arg(long)]
        no_cache: bool,
        /// Allow normal automatic sleep during this casting session
        #[arg(long)]
        no_inhibit_sleep: bool,
        /// Store prepared files here instead of beside the source
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        /// Reachable local IP to advertise to the receiver
        #[arg(long)]
        bind_address: Option<IpAddr>,
        /// Local serving port (0 lets the OS choose)
        #[arg(long, default_value_t = 0)]
        http_port: u16,
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=60))]
        scan_seconds: u64,
        #[arg(long, default_value = "ffprobe")]
        ffprobe: PathBuf,
        #[arg(long, default_value = "ffmpeg")]
        ffmpeg: PathBuf,
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..))]
        probe_timeout: u64,
    },
    /// Show container, duration, and video/audio/subtitle tracks
    Inspect {
        file: PathBuf,
        /// Emit normalized metadata as JSON
        #[arg(long)]
        json: bool,
        /// Path to the ffprobe executable
        #[arg(long, default_value = "ffprobe")]
        ffprobe: PathBuf,
        /// Maximum probe runtime, in seconds
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..))]
        timeout: u64,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(if cli.verbose {
            "procast_core=debug"
        } else {
            "warn"
        })
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .with_ansi(false)
        .without_time()
        .init();
    match run(cli).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode, Box<dyn std::error::Error>> {
    // Register before starting any work, including in noninteractive runs.
    #[cfg(unix)]
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let signal = async {
        #[cfg(unix)]
        {
            tokio::select! { _ = interrupt.recv() => Ok::<u8, io::Error>(130), _ = terminate.recv() => Ok(143) }
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await?;
            Ok::<u8, io::Error>(130)
        }
    };
    let cancellation = CancellationToken::new();
    let operation = execute(cli.command, &cancellation);
    tokio::pin!(operation);
    tokio::select! {
        result = &mut operation => { result?; Ok(ExitCode::SUCCESS) },
        received = signal => {
            cancellation.cancel();
            if let Err(error) = operation.await
                && !matches!(error.downcast_ref::<ProcastError>(), Some(ProcastError::Cancelled)) {
                return Err(error);
            }
            eprintln!("Cancelled; cleanup completed.");
            Ok(ExitCode::from(received?))
        }
    }
}

async fn execute(
    command: Commands,
    cancel: &CancellationToken,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Commands::Inspect {
            file,
            json,
            ffprobe,
            timeout,
        } => {
            let options = ProbeOptions {
                executable: ffprobe,
                timeout: Duration::from_secs(timeout),
            };
            let result = media::inspect(&file, &options, cancel).await?;
            let mut output = io::stdout().lock();
            if json {
                serde_json::to_writer_pretty(&mut output, &result)?;
                writeln!(output)?;
            } else {
                write_summary(&mut output, &result)?;
            }
        }
        Commands::Devices { scan_seconds, json } => {
            let devices = discovery::discover(Duration::from_secs(scan_seconds), cancel).await?;
            let mut output = io::stdout().lock();
            if json {
                serde_json::to_writer_pretty(&mut output, &devices)?;
                writeln!(output)?;
            } else if devices.is_empty() {
                writeln!(
                    output,
                    "No Cast devices found. Check the LAN, mDNS/firewall settings, or use --host IP."
                )?;
            } else {
                for device in devices {
                    let capability = match device.capabilities {
                        Some(bits) if bits & 1 != 0 => "video",
                        Some(_) => "audio-only",
                        None => "capabilities unknown",
                    };
                    writeln!(
                        output,
                        "{:?} | {} | model={:?} | {:?}:{} | {}",
                        device.name,
                        device.id,
                        device.model,
                        device.addresses,
                        device.port,
                        capability
                    )?;
                }
            }
        }
        Commands::Cast {
            file,
            device,
            host,
            cast_port,
            subtitles,
            mode,
            profile,
            no_cache,
            no_inhibit_sleep,
            cache_dir,
            bind_address,
            http_port,
            scan_seconds,
            ffprobe,
            ffmpeg,
            probe_timeout,
        } => {
            let mut request = CastRequest::new(file);
            request.target = if let Some(host) = host {
                Target::Host(SocketAddr::new(host.into(), cast_port))
            } else if let Some(device) = device {
                Target::Device(device)
            } else {
                Target::Auto
            };
            request.subtitles = subtitles;
            request.mode = mode.into();
            request.profile = profile.into();
            request.cache.enabled = !no_cache;
            request.inhibit_sleep = !no_inhibit_sleep;
            request.cache.directory = cache_dir;
            request.bind_address = bind_address;
            request.http_port = http_port;
            request.scan_duration = Duration::from_secs(scan_seconds);
            request.probe = ProbeOptions {
                executable: ffprobe,
                timeout: Duration::from_secs(probe_timeout),
            };
            request.ffmpeg = ffmpeg;
            let (progress, mut updates) =
                tokio::sync::watch::channel(session::SessionState::default());
            let monitor = tokio::spawn(async move {
                let mut last = None;
                let mut notices_printed = 0;
                loop {
                    let state = updates.borrow_and_update().clone();
                    for message in state.notices.iter().skip(notices_printed) {
                        eprintln!("{}", message.escape_debug());
                    }
                    notices_printed = state.notices.len();
                    let key = (state.phase, state.message.clone());
                    if last.as_ref() != Some(&key)
                        && !matches!(
                            state.phase,
                            Phase::Completed | Phase::Stopped | Phase::Cancelled | Phase::Failed
                        )
                    {
                        eprintln!(
                            "{}{}",
                            phase_label(state.phase),
                            state
                                .message
                                .as_ref()
                                .map(|m| format!(": {}", m.escape_debug()))
                                .unwrap_or_default()
                        );
                    }
                    last = Some(key);
                    if updates.changed().await.is_err() {
                        break;
                    }
                }
                updates.borrow().phase
            });
            let result = session::run(request, progress, cancel).await;
            let phase = monitor.await?;
            result?;
            writeln!(
                io::stdout().lock(),
                "{}",
                if phase == Phase::Stopped {
                    "Playback stopped on receiver."
                } else {
                    "Playback completed."
                }
            )?;
        }
    }
    Ok(())
}

fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Preparing => "Preparing",
        Phase::Discovering => "Discovering devices",
        Phase::Connecting => "Connecting",
        Phase::Loading => "Loading",
        Phase::Playing => "Playing",
        Phase::Paused => "Paused",
        Phase::Buffering => "Buffering",
        Phase::Stopping => "Stopping",
        Phase::Completed => "Completed",
        Phase::Stopped => "Stopped",
        Phase::Cancelled => "Cancelled",
        Phase::Failed => "Failed",
    }
}

fn write_summary(output: &mut impl Write, info: &MediaInfo) -> io::Result<()> {
    writeln!(output, "File: {}", info.path.display())?;
    writeln!(output, "Container: {}", info.container)?;
    match info.duration_seconds {
        Some(seconds) => writeln!(output, "Duration: {seconds:.3} s")?,
        None => writeln!(output, "Duration: unknown")?,
    }
    writeln!(output, "Streams:")?;
    if info.streams.is_empty() {
        writeln!(output, "  none reported")?;
    }
    for stream in &info.streams {
        write!(
            output,
            "  #{} {}: {}",
            stream.index,
            stream.kind,
            stream.codec.as_deref().unwrap_or("unknown codec")
        )?;
        if let Some(profile) = &stream.profile {
            write!(output, " ({profile})")?;
        }
        if let (Some(width), Some(height)) = (stream.width, stream.height) {
            write!(output, ", {width}×{height}")?;
        }
        if let Some(channels) = stream.channels {
            write!(output, ", {channels} channels")?;
        }
        if let Some(rate) = stream.sample_rate_hz {
            write!(output, ", {rate} Hz")?;
        }
        if let Some(language) = &stream.language {
            write!(output, ", {language}")?;
        }
        if let Some(title) = &stream.title {
            write!(output, ", {title:?}")?;
        }
        if stream.default {
            write!(output, " [default]")?;
        }
        if stream.forced {
            write!(output, " [forced]")?;
        }
        if stream.attached_picture {
            write!(output, " [cover art]")?;
        }
        writeln!(output)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cast_target_defaults_and_conflicts() {
        for extra in [
            vec![],
            vec!["--device", "Living Room"],
            vec!["--host", "127.0.0.1"],
            vec!["--mode", "direct", "--profile", "experimental"],
            vec![
                "--mode",
                "remux",
                "--profile",
                "extended",
                "--cache-dir",
                "/tmp",
            ],
            vec!["--mode", "audio", "--profile", "baseline", "--no-cache"],
            vec!["--mode", "transcode", "--cache-dir", "/tmp"],
            vec!["--mode", "auto", "--profile", "auto"],
        ] {
            let args = ["procast", "cast", "movie.mp4"].into_iter().chain(extra);
            assert!(Cli::try_parse_from(args).is_ok());
        }
        for extra in [
            vec!["--device", "TV", "--host", "127.0.0.1"],
            vec!["--cast-port", "8009"],
            vec!["--scan-seconds", "0"],
            vec!["--probe-timeout", "0"],
            vec!["--mode", "invalid"],
            vec!["--profile", "invalid"],
            vec!["--mode", "direct", "--mode", "audio"],
            vec!["--force-transcode"],
            vec!["--transcode-audio"],
            vec!["--remux"],
            vec!["--experimental-direct-play"],
            vec!["--transcode-dir", "/tmp"],
        ] {
            let args = ["procast", "cast", "movie.mp4"].into_iter().chain(extra);
            assert!(Cli::try_parse_from(args).is_err());
        }
    }
}
