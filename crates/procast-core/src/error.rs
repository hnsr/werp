use std::{io, path::PathBuf, process::ExitStatus, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum ProcastError {
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
    #[error("executable {0:?} was not found; install ffprobe or specify its path with --ffprobe")]
    MissingExecutable(PathBuf),
    #[error("could not run {program:?}: {source}")]
    ProcessIo { program: PathBuf, source: io::Error },
    #[error("{program:?} failed ({status}): {stderr}")]
    ProcessFailed {
        program: PathBuf,
        status: ExitStatus,
        stderr: String,
    },
    #[error("ffprobe metadata exceeded the {limit}-byte limit")]
    OutputLimit { limit: usize },
    #[error("media inspection timed out after {0:?}")]
    TimedOut(Duration),
    #[error("operation cancelled")]
    Cancelled,
    #[error("invalid ffprobe metadata: {0}")]
    InvalidMetadata(#[from] serde_json::Error),
    #[error("{cause}; additionally failed to stop/reap the child: {source}")]
    Cleanup { cause: Box<Self>, source: io::Error },
}
