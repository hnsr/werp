use std::{future::Future, io, path::PathBuf, process::Stdio, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{ChildStdout, Command},
};

use crate::{CancellationToken, WerpError};

const STDOUT_LIMIT: usize = 4 * 1024 * 1024;
const STDERR_LIMIT: usize = 32 * 1024;

/// Drain both pipes while waiting. Cancellation is cooperative: callers cancel
/// the token and await the result so the child is explicitly killed and reaped.
pub(crate) async fn capture(
    command: &mut Command,
    cancellation: &CancellationToken,
    timeout: Duration,
) -> Result<Vec<u8>, WerpError> {
    let program = PathBuf::from(command.as_std().get_program());
    run(command, cancellation, timeout, |stdout| async move {
        read_metadata(stdout, &program).await
    })
    .await
}

/// Stream stdout through a bounded consumer while retaining the same cancellation,
/// diagnostic, and reaping guarantees as metadata capture.
pub(crate) async fn run<T, F, Fut>(
    command: &mut Command,
    cancellation: &CancellationToken,
    timeout: Duration,
    consume: F,
) -> Result<T, WerpError>
where
    F: FnOnce(ChildStdout) -> Fut,
    Fut: Future<Output = Result<T, WerpError>>,
{
    if cancellation.is_cancelled() {
        return Err(WerpError::Cancelled);
    }
    let program = PathBuf::from(command.as_std().get_program());
    let io_error = |source| WerpError::ProcessIo {
        program: program.clone(),
        source,
    };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                WerpError::MissingExecutable(program.clone())
            } else {
                io_error(source)
            }
        })?;
    tracing::debug!(pid = child.id(), executable = ?program, "started media process");
    let stdout = child.stdout.take().expect("stdout was configured as piped");
    let stderr = child.stderr.take().expect("stderr was configured as piped");
    let result = {
        let operation = async {
            let (stdout, stderr, status) = tokio::try_join!(
                consume(stdout),
                async { read_tail(stderr, STDERR_LIMIT).await.map_err(io_error) },
                async { child.wait().await.map_err(io_error) },
            )?;
            if !status.success() {
                let tail = String::from_utf8_lossy(&stderr).trim().to_owned();
                return Err(WerpError::ProcessFailed {
                    program: program.clone(),
                    status,
                    stderr: if tail.is_empty() {
                        "no diagnostic output".into()
                    } else {
                        tail
                    },
                });
            }
            Ok(stdout)
        };
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(WerpError::Cancelled),
            _ = tokio::time::sleep(timeout) => Err(WerpError::TimedOut(timeout)),
            result = operation => result,
        }
    };
    if let Err(cause) = result {
        // kill() also waits/reaps, and succeeds if wait() already collected it.
        if let Err(source) = child.kill().await {
            return Err(WerpError::Cleanup {
                cause: Box::new(cause),
                source,
            });
        }
        return Err(cause);
    }
    tracing::debug!("media process finished");
    result
}

async fn read_metadata(
    reader: impl AsyncRead + Unpin,
    program: &std::path::Path,
) -> Result<Vec<u8>, WerpError> {
    let mut bytes = Vec::new();
    reader
        .take((STDOUT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|source| WerpError::ProcessIo {
            program: program.into(),
            source,
        })?;
    if bytes.len() > STDOUT_LIMIT {
        return Err(WerpError::OutputLimit {
            limit: STDOUT_LIMIT,
        });
    }
    Ok(bytes)
}

async fn read_tail(mut reader: impl AsyncRead + Unpin, limit: usize) -> io::Result<Vec<u8>> {
    let mut tail = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let length = reader.read(&mut buffer).await?;
        if length == 0 {
            return Ok(tail);
        }
        if length >= limit {
            tail.clear();
            tail.extend_from_slice(&buffer[length - limit..length]);
        } else {
            let excess = (tail.len() + length).saturating_sub(limit);
            if excess > 0 {
                tail.drain(..excess);
            }
            tail.extend_from_slice(&buffer[..length]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn keeps_only_the_end_of_diagnostic_output() {
        let bytes = b"an arbitrarily long diagnostic ending in tail";
        assert_eq!(read_tail(&bytes[..], 4).await.unwrap(), b"tail");
        assert_eq!(read_tail(&bytes[..], 0).await.unwrap(), b"");
    }
}
