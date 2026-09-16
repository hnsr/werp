use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};

use clap::{Parser, Subcommand};
use procast_core::{
    CancellationToken, ProcastError,
    media::{self, MediaInfo, ProbeOptions},
};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "procast",
    version,
    about = "Inspect local media for Procast (casting is coming next)"
)]
struct Cli {
    /// Show debug diagnostics on stderr
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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
    // Register before starting the probe, including in noninteractive runs.
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
    let Commands::Inspect {
        file,
        json,
        ffprobe,
        timeout,
    } = cli.command;
    let cancellation = CancellationToken::new();
    let options = ProbeOptions {
        executable: ffprobe,
        timeout: Duration::from_secs(timeout),
    };
    let inspection = media::inspect(&file, &options, &cancellation);
    tokio::pin!(inspection);
    let result = tokio::select! {
        result = &mut inspection => result,
        received = signal => {
            cancellation.cancel();
            let cleanup = inspection.await;
            if let Err(error) = cleanup
                && !matches!(error, ProcastError::Cancelled) {
                return Err(error.into());
            }
            let code = received?;
            eprintln!("Inspection cancelled.");
            return Ok(ExitCode::from(code));
        }
    }?;
    let mut output = io::stdout().lock();
    if json {
        serde_json::to_writer_pretty(&mut output, &result)?;
        writeln!(output)?;
    } else {
        write_summary(&mut output, &result)?;
    }
    Ok(ExitCode::SUCCESS)
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
