//! Small Linux sleep inhibitor; replace/extend this adapter for future frontends.
#[cfg(target_os = "linux")]
mod linux {
    use std::{io, process::Stdio, time::Duration};

    use tokio::{io::AsyncReadExt, process::Command, task::JoinHandle};

    use crate::{CancellationToken, YeetError};

    pub(crate) struct SleepInhibitor {
        stop: CancellationToken,
        worker: Option<JoinHandle<()>>,
    }

    impl SleepInhibitor {
        pub(crate) async fn acquire(cancel: &CancellationToken) -> Result<Self, YeetError> {
            let mut command = Command::new("systemd-inhibit");
            command.args([
                "--what=sleep",
                "--mode=block",
                "--who=Yeet",
                "--why=Preparing and casting local media",
                "--no-ask-password",
                "/bin/sh",
                "-c",
                // Fixed script, no user data. Readiness is emitted only after
                // systemd-inhibit acquires its lock and launches this command.
                // EOF releases the helper even if Yeet crashes/is killed.
                "printf R; read -r ignored || :",
            ]);
            Self::start(command, cancel, Duration::from_secs(3)).await
        }

        async fn start(
            mut command: Command,
            cancel: &CancellationToken,
            timeout: Duration,
        ) -> Result<Self, YeetError> {
            if cancel.is_cancelled() {
                return Err(YeetError::Cancelled);
            }
            let program = command.as_std().get_program().into();
            let io_error = |source| YeetError::ProcessIo {
                program: std::path::PathBuf::clone(&program),
                source,
            };
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                // Keep the inhibitor alive through Yeet's signal cleanup.
                .process_group(0)
                .kill_on_drop(true)
                .spawn()
                .map_err(io_error)?;
            let stdin = child.stdin.take();
            let mut stdout = child.stdout.take().expect("piped stdout");
            let ready = tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(YeetError::Cancelled),
                _ = tokio::time::sleep(timeout) => Err(YeetError::TimedOut(timeout)),
                result = stdout.read_u8() => match result {
                    Ok(b'R') => Ok(()),
                    Ok(_) => Err(io_error(io::Error::other("invalid sleep-inhibitor readiness response"))),
                    Err(e) => Err(io_error(e)),
                },
            };
            drop(stdout);
            if let Err(cause) = ready {
                drop(stdin);
                child.kill().await.map_err(io_error)?;
                return Err(cause);
            }
            let stop = CancellationToken::new();
            let stopping = stop.clone();
            let worker = tokio::spawn(async move {
                tokio::select! {
                    biased;
                    _ = stopping.cancelled() => {
                        drop(stdin);
                        match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
                            Ok(Ok(_)) => {}
                            Ok(Err(error)) => tracing::warn!(%error, "could not reap sleep inhibitor"),
                            Err(_) => {
                                if let Err(error) = child.kill().await {
                                    tracing::warn!(%error, "could not stop sleep inhibitor");
                                }
                            }
                        }
                    }
                    status = child.wait() => {
                        tracing::warn!(?status, "Sleep inhibitor exited unexpectedly; automatic sleep may interrupt casting");
                    }
                }
            });
            Ok(Self {
                stop,
                worker: Some(worker),
            })
        }

        pub(crate) async fn close(mut self) {
            self.stop.cancel();
            if let Some(worker) = self.worker.take()
                && let Err(error) = worker.await
            {
                tracing::warn!(%error, "sleep-inhibitor cleanup failed");
            }
        }
    }

    impl Drop for SleepInhibitor {
        fn drop(&mut self) {
            // The worker closes stdin and reaps the helper if a caller drops its
            // session future. Normal session shutdown also awaits it explicitly.
            self.stop.cancel();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn helper(path: &std::path::Path, script: &str) -> Command {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", script]).env("PID_FILE", path);
            command
        }

        async fn assert_reaped(path: &std::path::Path) {
            let pid = std::fs::read_to_string(path).unwrap();
            let proc = format!("/proc/{}", pid.trim());
            tokio::time::timeout(Duration::from_secs(3), async {
                while std::path::Path::new(&proc).exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("inhibitor helper must be reaped, not left running or zombie");
        }

        #[tokio::test]
        async fn explicit_close_and_drop_release_and_reap_helper() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("pid");
            for drop_only in [false, true] {
                let inhibitor = SleepInhibitor::start(
                    helper(
                        &path,
                        "echo $$ > \"$PID_FILE\"; printf R; read -r ignored || :",
                    ),
                    &CancellationToken::new(),
                    Duration::from_secs(2),
                )
                .await
                .unwrap();
                let pid = std::fs::read_to_string(&path).unwrap();
                assert!(std::path::Path::new(&format!("/proc/{}", pid.trim())).exists());
                if drop_only {
                    drop(inhibitor);
                } else {
                    inhibitor.close().await;
                }
                assert_reaped(&path).await;
            }
        }

        #[tokio::test]
        async fn acquisition_failure_timeout_and_cancellation_do_not_leak_helpers() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("pid");
            assert!(
                SleepInhibitor::start(
                    Command::new(dir.path().join("missing")),
                    &CancellationToken::new(),
                    Duration::from_secs(2),
                )
                .await
                .is_err()
            );
            for behavior in ["exit 1", "read -r ignored"] {
                let command = helper(&path, &format!("echo $$ > \"$PID_FILE\"; {behavior}"));
                assert!(
                    SleepInhibitor::start(
                        command,
                        &CancellationToken::new(),
                        Duration::from_millis(100)
                    )
                    .await
                    .is_err()
                );
                assert_reaped(&path).await;
            }
            let cancel = CancellationToken::new();
            let acquire = SleepInhibitor::start(
                helper(&path, "echo $$ > \"$PID_FILE\"; read -r ignored"),
                &cancel,
                Duration::from_secs(2),
            );
            // Wait for the new helper, then cancel while it has not announced readiness.
            std::fs::remove_file(&path).unwrap();
            let cancelling = async {
                while !path.exists() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                cancel.cancel();
            };
            let (result, ()) = tokio::join!(acquire, cancelling);
            assert!(matches!(result, Err(YeetError::Cancelled)));
            assert_reaped(&path).await;
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::SleepInhibitor;
