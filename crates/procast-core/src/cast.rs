//! Cast adapter. Third-party types stay private; Procast owns the media session.
use std::{future::Future, net::SocketAddr, time::Duration};

use oxicast::{
    CastClient, CastEvent,
    types::{IdleReason, MediaStatus, PlayerState},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{CancellationToken, ProcastError};

const RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
const MEDIA: &str = "urn:x-cast:com.google.cast.media";
const CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
const DEFAULT_RECEIVER: &str = "CC1AD845";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackSnapshot {
    pub media_session_id: i64,
    pub player_state: String,
    #[serde(default)]
    pub current_time: f64,
    pub idle_reason: Option<String>,
    #[serde(default)]
    pub active_track_ids: Vec<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Application {
    app_id: String,
    session_id: String,
    transport_id: String,
}

pub struct CastSession {
    client: CastClient,
    app: Option<Application>,
    media_id: Option<i64>,
    content_id: Option<String>,
}

pub(crate) async fn bounded<T>(
    cancel: &CancellationToken,
    timeout: Duration,
    operation: impl Future<Output = Result<T, ProcastError>>,
) -> Result<T, ProcastError> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(ProcastError::Cancelled),
        result = tokio::time::timeout(timeout, operation) =>
            result.map_err(|_| ProcastError::Cast(format!("operation timed out after {timeout:?}")))?,
    }
}

impl CastSession {
    pub async fn connect(
        address: SocketAddr,
        cancel: &CancellationToken,
    ) -> Result<Self, ProcastError> {
        // oxicast's tokio-rustls dependency enables aws-lc as well as ring.
        // Select a default only if the embedding application has not done so.
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        let client = bounded(cancel, REQUEST_TIMEOUT, async {
            CastClient::builder(address.ip().to_string(), address.port())
                .auto_reconnect(false)
                .request_timeout(REQUEST_TIMEOUT)
                .verify_tls(false)
                .connect()
                .await
                .map_err(cast_error)
        })
        .await?;
        Ok(Self {
            client,
            app: None,
            media_id: None,
            content_id: None,
        })
    }

    async fn request(
        &self,
        namespace: &str,
        destination: &str,
        payload: Value,
        cancel: &CancellationToken,
    ) -> Result<Value, ProcastError> {
        bounded(cancel, REQUEST_TIMEOUT, async {
            let response = self
                .client
                .send_raw(namespace, destination, payload)
                .await
                .map_err(cast_error)?;
            check_response(&response)?;
            Ok(response)
        })
        .await
    }

    pub async fn launch(&mut self, cancel: &CancellationToken) -> Result<(), ProcastError> {
        // Use raw launch so oxicast never adopts a mutable "current application".
        // Its disconnect() then only shuts down transport; STOP is owned below.
        let response = self
            .request(
                RECEIVER,
                "receiver-0",
                json!({"type":"LAUNCH", "appId":DEFAULT_RECEIVER}),
                cancel,
            )
            .await?;
        let mut response = response;
        for _ in 0..15 {
            if let Some(apps) = response
                .pointer("/status/applications")
                .and_then(Value::as_array)
            {
                for value in apps {
                    let app: Application = serde_json::from_value(value.clone())
                        .map_err(|e| ProcastError::Cast(e.to_string()))?;
                    if app.app_id == DEFAULT_RECEIVER {
                        bounded(cancel, REQUEST_TIMEOUT, async {
                            self.client
                                .send_raw_no_reply(
                                    CONNECTION,
                                    &app.transport_id,
                                    json!({"type":"CONNECT", "origin":{}}),
                                )
                                .await
                                .map_err(cast_error)
                        })
                        .await?;
                        self.app = Some(app);
                        return Ok(());
                    }
                }
            }
            bounded(cancel, Duration::from_secs(2), async {
                tokio::time::sleep(Duration::from_millis(500)).await;
                Ok(())
            })
            .await?;
            response = self
                .request(RECEIVER, "receiver-0", json!({"type":"GET_STATUS"}), cancel)
                .await?;
        }
        Err(ProcastError::Cast(
            "Default Media Receiver did not launch".into(),
        ))
    }

    pub async fn load(
        &mut self,
        video: &str,
        subtitles: Option<&str>,
        title: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<PlaybackSnapshot>, ProcastError> {
        let app = self
            .app
            .as_ref()
            .ok_or_else(|| ProcastError::Cast("receiver has not launched".into()))?;
        // Remember content before LOAD: cancellation may race with its reply.
        self.content_id = Some(video.into());
        self.media_id = None;
        let response = self
            .request(
                MEDIA,
                &app.transport_id,
                load_payload(&app.session_id, video, subtitles, title),
                cancel,
            )
            .await?;
        self.adopt_status(&response)
    }

    pub async fn status(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<Option<PlaybackSnapshot>, ProcastError> {
        self.wait_for_status(Duration::ZERO, cancel).await
    }

    /// Consume terminal broadcasts while waiting for the next poll and its reply.
    /// A receiver may broadcast FINISHED only once, then return an empty status.
    pub async fn wait_for_status(
        &mut self,
        poll_delay: Duration,
        cancel: &CancellationToken,
    ) -> Result<Option<PlaybackSnapshot>, ProcastError> {
        let app = self
            .app
            .as_ref()
            .ok_or_else(|| ProcastError::Cast("receiver has not launched".into()))?;
        let value = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ProcastError::Cancelled),
            terminal = self.terminal_event() => return terminal.map(Some),
            value = async {
                tokio::time::sleep(poll_delay).await;
                self.request(
                MEDIA,
                &app.transport_id,
                json!({"type":"GET_STATUS"}),
                cancel,
                ).await
            } => value?,
        };
        self.adopt_status(&value)
    }

    async fn terminal_event(&self) -> Result<PlaybackSnapshot, ProcastError> {
        loop {
            match self.client.next_event().await {
                Some(CastEvent::MediaStatusChanged(status)) => {
                    if let Some(content) = &self.content_id
                        && let Some(terminal) = owned_terminal(&status, content, self.media_id)
                    {
                        tracing::debug!(state = %terminal.player_state, reason = ?terminal.idle_reason, "received owned terminal media event");
                        return Ok(terminal);
                    }
                }
                // MediaSessionEnded lacks content identity. Use the accompanying
                // MediaStatusChanged event, which permits the full ownership check.
                Some(CastEvent::Disconnected(_)) | None => {
                    return Err(ProcastError::Cast(
                        "receiver disconnected; playback was not restarted".into(),
                    ));
                }
                _ => {}
            }
        }
    }

    fn adopt_status(&mut self, response: &Value) -> Result<Option<PlaybackSnapshot>, ProcastError> {
        let statuses = response
            .get("status")
            .and_then(Value::as_array)
            .ok_or_else(|| ProcastError::Cast("receiver omitted the media status array".into()))?;
        for value in statuses {
            let id = value
                .get("mediaSessionId")
                .and_then(Value::as_i64)
                .filter(|id| *id > 0);
            if let Some(content) = &self.content_id
                && id.is_some()
                && owns_status(value, content, self.media_id)
            {
                let status: PlaybackSnapshot = serde_json::from_value(value.clone())
                    .map_err(|e| ProcastError::Cast(e.to_string()))?;
                self.media_id = id;
                return Ok(Some(status));
            }
        }
        if self.media_id.is_some()
            && statuses.iter().any(|s| {
                s.get("mediaSessionId")
                    .and_then(Value::as_i64)
                    .is_some_and(|id| id > 0)
            })
        {
            return Err(ProcastError::Cast(
                "media session replaced by another sender".into(),
            ));
        }
        // Empty status and session-zero IDLE are transitional, not completion.
        Ok(None)
    }

    pub async fn command(
        &self,
        kind: &str,
        extra: Value,
        cancel: &CancellationToken,
    ) -> Result<PlaybackSnapshot, ProcastError> {
        let app = self
            .app
            .as_ref()
            .ok_or_else(|| ProcastError::Cast("receiver has not launched".into()))?;
        let id = self
            .media_id
            .ok_or_else(|| ProcastError::Cast("no owned media session".into()))?;
        if !extra.is_object() || !matches!(kind, "PAUSE" | "PLAY" | "SEEK" | "EDIT_TRACKS_INFO") {
            return Err(ProcastError::Cast(
                "invalid prototype playback command".into(),
            ));
        }
        let mut payload = extra;
        payload["type"] = json!(kind);
        payload["mediaSessionId"] = json!(id);
        let response = self
            .request(MEDIA, &app.transport_id, payload, cancel)
            .await?;
        let status = snapshot(&response)?;
        let value = &response["status"][0];
        if status.media_session_id != id
            || !self
                .content_id
                .as_ref()
                .is_some_and(|content| owns_status(value, content, Some(id)))
        {
            return Err(ProcastError::Cast("media session changed".into()));
        }
        Ok(status)
    }

    pub fn is_connected(&self) -> bool {
        self.client.is_connected()
    }

    /// Bounded best-effort remote STOP, followed by deterministic local shutdown.
    /// Uses our URL and session ID, never oxicast's mutable current-session ID.
    pub async fn close(self) -> Result<(), ProcastError> {
        self.finish(true).await
    }

    /// Skip remote STOP after confirmed natural completion; still join transport.
    pub async fn finish(self, stop_owned: bool) -> Result<(), ProcastError> {
        let stop = async {
            if stop_owned && let (Some(app), Some(content)) = (&self.app, &self.content_id) {
                let token = CancellationToken::new();
                let response = self
                    .request(
                        MEDIA,
                        &app.transport_id,
                        json!({"type":"GET_STATUS"}),
                        &token,
                    )
                    .await?;
                if let Some(status) = response
                    .get("status")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    && owns_status(status, content, self.media_id)
                    && status.get("playerState").and_then(Value::as_str) != Some("IDLE")
                    && let Some(id) = status
                        .get("mediaSessionId")
                        .and_then(Value::as_i64)
                        .filter(|id| *id > 0)
                {
                    self.request(
                        MEDIA,
                        &app.transport_id,
                        json!({"type":"STOP", "mediaSessionId":id}),
                        &token,
                    )
                    .await?;
                }
            }
            Ok::<_, ProcastError>(())
        };
        let remote = tokio::time::timeout(Duration::from_secs(2), stop).await;
        // Always run this even when remote STOP fails or times out.
        self.client.disconnect().await.map_err(cast_error)?;
        match remote {
            Ok(result) => result,
            Err(_) => Err(ProcastError::Cast(
                "remote STOP timed out; local connection closed".into(),
            )),
        }
    }
}

fn cast_error(error: oxicast::Error) -> ProcastError {
    ProcastError::Cast(error.to_string())
}

fn check_response(response: &Value) -> Result<(), ProcastError> {
    match response.get("type").and_then(Value::as_str) {
        Some("MEDIA_STATUS" | "RECEIVER_STATUS") => Ok(()),
        _ => Err(ProcastError::Cast(format!("receiver response: {response}"))),
    }
}

fn snapshot(response: &Value) -> Result<PlaybackSnapshot, ProcastError> {
    let value = response
        .get("status")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .ok_or_else(|| ProcastError::Cast(format!("no media status: {response}")))?;
    serde_json::from_value(value.clone()).map_err(|e| ProcastError::Cast(e.to_string()))
}

fn owns_status(status: &Value, content: &str, id: Option<i64>) -> bool {
    let reported_content = status.pointer("/media/contentId").and_then(Value::as_str);
    let same_id =
        id.is_some_and(|id| status.get("mediaSessionId").and_then(Value::as_i64) == Some(id));
    (reported_content == Some(content) && id.is_none_or(|_| same_id))
        || (reported_content.is_none() && same_id)
}

fn owned_terminal(
    status: &MediaStatus,
    content: &str,
    id: Option<i64>,
) -> Option<PlaybackSnapshot> {
    let reported_id = i64::from(status.media_session_id);
    if reported_id <= 0 || status.player_state != PlayerState::Idle {
        return None;
    }
    let reason = match status.idle_reason? {
        IdleReason::Finished => "FINISHED",
        IdleReason::Cancelled => "CANCELLED",
        IdleReason::Interrupted => "INTERRUPTED",
        IdleReason::Error => "ERROR",
        _ => return None,
    };
    let mut identity = json!({"mediaSessionId":reported_id});
    if let Some(media) = &status.media {
        identity["media"] = json!({"contentId":media.content_id});
    }
    if !owns_status(&identity, content, id) {
        return None;
    }
    Some(PlaybackSnapshot {
        media_session_id: reported_id,
        player_state: "IDLE".into(),
        current_time: status.current_time,
        idle_reason: Some(reason.into()),
        // oxicast's typed events omit tracks. Confirmation comes from the raw
        // LOAD/EDIT/status replies, not an invented track state at completion.
        active_track_ids: vec![],
    })
}

fn load_payload(session: &str, video: &str, subtitles: Option<&str>, title: &str) -> Value {
    let mut payload = json!({
        "type":"LOAD", "sessionId":session, "autoplay":true, "currentTime":0,
        "media":{
            "contentId":video, "contentType":"video/mp4", "streamType":"BUFFERED",
            "metadata":{"metadataType":0,"title":title}
        }
    });
    if let Some(subtitles) = subtitles {
        payload["activeTrackIds"] = json!([1]);
        payload["media"]["tracks"] = json!([{"trackId":1,"type":"TEXT","subtype":"SUBTITLES",
                "trackContentId":subtitles,"trackContentType":"text/vtt",
                "name":"External subtitles"}]);
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_load_errors_and_does_not_own_another_senders_media() {
        assert!(check_response(&json!({"type":"LOAD_FAILED","detailedErrorCode":104})).is_err());
        let ours = json!({"mediaSessionId":7,"media":{"contentId":"ours"}});
        assert!(owns_status(&ours, "ours", Some(7)));
        assert!(!owns_status(&ours, "ours", Some(8)));
        assert!(!owns_status(&ours, "theirs", Some(7)));
        assert!(snapshot(&json!({"type":"MEDIA_STATUS","status":[]})).is_err());
    }
    #[tokio::test]
    async fn pending_request_is_cancellable_and_bounded() {
        let token = CancellationToken::new();
        let cancel = async {
            tokio::task::yield_now().await;
            token.cancel();
        };
        let (result, ()) = tokio::join!(
            bounded::<()>(&token, Duration::from_secs(1), std::future::pending()),
            cancel
        );
        assert!(matches!(result, Err(ProcastError::Cancelled)));
        assert!(
            bounded::<()>(
                &CancellationToken::new(),
                Duration::from_millis(10),
                std::future::pending()
            )
            .await
            .is_err()
        );
    }
}
