use std::{io, path::PathBuf, process::ExitStatus, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum YeetError {
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("media preparation failed: {0}")]
    Transcode(String),
    #[error("unsupported media: {0}")]
    UnsupportedMedia(String),
    #[error("invalid subtitles: {0}")]
    Subtitles(String),
    #[error("Cast operation failed: {0}")]
    Cast(String),
    #[error("device discovery failed: {0}")]
    Discovery(String),
    #[error("local media server failed: {0}")]
    Serve(String),
    #[error("cannot access input {path:?}: {source}")]
    InputIo { path: PathBuf, source: io::Error },
    #[error("input is not a regular file: {0:?}")]
    NotRegularFile(PathBuf),
    #[error(
        "executable {0:?} was not found; install it or provide its path with --ffprobe / --ffmpeg"
    )]
    MissingExecutable(PathBuf),
    #[error("could not run {program:?}: {source}")]
    ProcessIo { program: PathBuf, source: io::Error },
    #[error("{program:?} failed ({status}): {stderr}")]
    ProcessFailed {
        program: PathBuf,
        status: ExitStatus,
        stderr: String,
    },
    #[error("subprocess output exceeded the {limit}-byte limit")]
    OutputLimit { limit: usize },
    #[error("subprocess timed out after {0:?}")]
    TimedOut(Duration),
    #[error("operation cancelled")]
    Cancelled,
    #[error("invalid ffprobe metadata: {0}")]
    InvalidMetadata(#[from] serde_json::Error),
    #[error("{cause}; additionally failed to stop/reap the child: {source}")]
    Cleanup { cause: Box<Self>, source: io::Error },
}
