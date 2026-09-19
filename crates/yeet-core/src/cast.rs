//! Cast adapter. Third-party types stay private; Yeet owns the media session.
use std::{future::Future, net::SocketAddr, time::Duration};

use oxicast::{
    CastClient, CastEvent,
    types::{IdleReason, MediaStatus, PlayerState},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{CancellationToken, YeetError};

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
    operation: impl Future<Output = Result<T, YeetError>>,
) -> Result<T, YeetError> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(YeetError::Cancelled),
        result = tokio::time::timeout(timeout, operation) =>
            result.map_err(|_| YeetError::Cast(format!("operation timed out after {timeout:?}")))?,
    }
}

impl CastSession {
    pub async fn connect(
        address: SocketAddr,
        cancel: &CancellationToken,
    ) -> Result<Self, YeetError> {
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
    ) -> Result<Value, YeetError> {
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

    pub async fn launch(&mut self, cancel: &CancellationToken) -> Result<(), YeetError> {
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
                        .map_err(|e| YeetError::Cast(e.to_string()))?;
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
        Err(YeetError::Cast(
            "Default Media Receiver did not launch".into(),
        ))
    }

    pub async fn load(
        &mut self,
        video: &str,
        subtitles: Option<&str>,
        title: &str,
        start_position: f64,
        cancel: &CancellationToken,
    ) -> Result<Option<PlaybackSnapshot>, YeetError> {
        let app = self
            .app
            .as_ref()
            .ok_or_else(|| YeetError::Cast("receiver has not launched".into()))?;
        // Remember content before LOAD: cancellation may race with its reply.
        self.content_id = Some(video.into());
        self.media_id = None;
        let response = self
            .request(
                MEDIA,
                &app.transport_id,
                load_payload(&app.session_id, video, subtitles, title, start_position),
                cancel,
            )
            .await?;
        self.adopt_status(&response)
    }

    pub async fn status(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<Option<PlaybackSnapshot>, YeetError> {
        self.wait_for_status(Duration::ZERO, cancel).await
    }

    /// Consume terminal broadcasts while waiting for the next poll and its reply.
    /// A receiver may broadcast FINISHED only once, then return an empty status.
    pub async fn wait_for_status(
        &mut self,
        poll_delay: Duration,
        cancel: &CancellationToken,
    ) -> Result<Option<PlaybackSnapshot>, YeetError> {
        let app = self
            .app
            .as_ref()
            .ok_or_else(|| YeetError::Cast("receiver has not launched".into()))?;
        let value = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(YeetError::Cancelled),
            terminal = self.terminal_event() => return terminal.map(Some),
            value = async {
                tokio::time::sleep(poll_delay).await;
                self.request(
                MEDIA,
                &app.transport_id,
                json!({"type":"GET_STATUS"}),
                cancel,
                ).await
            } => value,
        };
        match value {
            Ok(value) => {
                let status = self.adopt_status(&value)?;
                // A receiver can keep returning stale BUFFERING or empty media
                // status after its app exits, without a CLOSE event. Confirm app
                // ownership outside the select so a media reply cannot cancel
                // this check after we have consumed the triggering event.
                let inactive = status.as_ref().is_none_or(|s| {
                    s.player_state == "BUFFERING"
                        || (s.player_state == "IDLE" && s.idle_reason.is_none())
                });
                if inactive && self.media_id.is_some() && self.receiver_app_ended(cancel).await? {
                    return Ok(Some(self.receiver_stopped_snapshot()));
                }
                Ok(status)
            }
            Err(error) => {
                // An app can disappear without a usable media terminal broadcast.
                // Ask the receiver itself; never turn a network timeout alone into success.
                if cancel.is_cancelled() {
                    return Err(YeetError::Cancelled);
                }
                if self.media_id.is_some() {
                    let ended = self.receiver_app_ended(cancel).await.unwrap_or(false);
                    if cancel.is_cancelled() {
                        return Err(YeetError::Cancelled);
                    }
                    if ended {
                        return Ok(Some(self.receiver_stopped_snapshot()));
                    }
                }
                Err(error)
            }
        }
    }

    async fn receiver_app_ended(&self, cancel: &CancellationToken) -> Result<bool, YeetError> {
        let Some(app) = &self.app else {
            return Ok(false);
        };
        let response = self
            .request(RECEIVER, "receiver-0", json!({"type":"GET_STATUS"}), cancel)
            .await?;
        let status = response
            .get("status")
            .and_then(Value::as_object)
            .ok_or_else(|| YeetError::Cast("receiver omitted its status object".into()))?;
        // Receivers may omit applications entirely when none are running.
        let Some(applications) = status.get("applications") else {
            let ended = status.contains_key("volume");
            tracing::debug!(ended, "receiver status has no application list");
            return Ok(ended);
        };
        let applications = applications
            .as_array()
            .ok_or_else(|| YeetError::Cast("receiver applications field is not an array".into()))?;
        for value in applications {
            let candidate: Application = serde_json::from_value(value.clone())
                .map_err(|e| YeetError::Cast(format!("invalid receiver application: {e}")))?;
            if candidate.app_id == app.app_id
                && candidate.session_id == app.session_id
                && candidate.transport_id == app.transport_id
            {
                tracing::debug!("owned receiver application is still running");
                return Ok(false);
            }
        }
        tracing::debug!(
            applications = applications.len(),
            "owned receiver application is no longer running"
        );
        Ok(true)
    }

    fn receiver_stopped_snapshot(&self) -> PlaybackSnapshot {
        PlaybackSnapshot {
            media_session_id: self
                .media_id
                .expect("only check app termination after adopting media"),
            player_state: "IDLE".into(),
            current_time: 0.0,
            // Internal adapter outcome, distinct from the protocol's FINISHED reason.
            idle_reason: Some("RECEIVER_STOPPED".into()),
            active_track_ids: vec![],
        }
    }

    async fn terminal_event(&self) -> Result<PlaybackSnapshot, YeetError> {
        loop {
            match self.client.next_event().await {
                Some(CastEvent::MediaStatusChanged(status)) => {
                    tracing::debug!(state = ?status.player_state, reason = ?status.idle_reason,
                        position = status.current_time, session = status.media_session_id,
                        "received media status event");
                    if let Some(content) = &self.content_id
                        && let Some(terminal) = owned_terminal(&status, content, self.media_id)
                    {
                        tracing::debug!(state = %terminal.player_state, reason = ?terminal.idle_reason, "received owned terminal media event");
                        return Ok(terminal);
                    }
                }
                Some(CastEvent::RawMessage {
                    namespace,
                    source,
                    payload,
                    ..
                }) if namespace == CONNECTION
                    && self.media_id.is_some()
                    && self
                        .app
                        .as_ref()
                        .is_some_and(|app| app.transport_id == source)
                    && serde_json::from_str::<Value>(&payload)
                        .ok()
                        .is_some_and(|value| value["type"] == "CLOSE") =>
                {
                    let token = CancellationToken::new();
                    if self.receiver_app_ended(&token).await? {
                        return Ok(self.receiver_stopped_snapshot());
                    }
                    return Err(YeetError::Cast(
                        "receiver closed the app channel while the application is still running"
                            .into(),
                    ));
                }
                // MediaSessionEnded lacks content identity. Use the accompanying
                // MediaStatusChanged event, which permits the full ownership check.
                Some(CastEvent::Disconnected(_)) | None => {
                    return Err(YeetError::Cast(
                        "receiver disconnected; playback was not restarted".into(),
                    ));
                }
                _ => {}
            }
        }
    }

    fn adopt_status(&mut self, response: &Value) -> Result<Option<PlaybackSnapshot>, YeetError> {
        let statuses = response
            .get("status")
            .and_then(Value::as_array)
            .ok_or_else(|| YeetError::Cast("receiver omitted the media status array".into()))?;
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
                    .map_err(|e| YeetError::Cast(e.to_string()))?;
                tracing::debug!(state = %status.player_state, reason = ?status.idle_reason,
                    position = status.current_time, session = status.media_session_id,
                    "received owned media status response");
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
            return Err(YeetError::Cast(
                "media session replaced by another sender".into(),
            ));
        }
        // Empty status and session-zero IDLE are transitional, not completion.
        tracing::debug!(
            entries = statuses.len(),
            "no owned media status in response"
        );
        Ok(None)
    }

    pub async fn command(
        &self,
        kind: &str,
        extra: Value,
        cancel: &CancellationToken,
    ) -> Result<PlaybackSnapshot, YeetError> {
        let app = self
            .app
            .as_ref()
            .ok_or_else(|| YeetError::Cast("receiver has not launched".into()))?;
        let id = self
            .media_id
            .ok_or_else(|| YeetError::Cast("no owned media session".into()))?;
        if !extra.is_object() || !matches!(kind, "PAUSE" | "PLAY" | "SEEK" | "EDIT_TRACKS_INFO") {
            return Err(YeetError::Cast("invalid prototype playback command".into()));
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
            return Err(YeetError::Cast("media session changed".into()));
        }
        Ok(status)
    }

    pub fn is_connected(&self) -> bool {
        self.client.is_connected()
    }

    /// Bounded best-effort remote STOP, followed by deterministic local shutdown.
    /// Uses our URL and session ID, never oxicast's mutable current-session ID.
    pub async fn close(self) -> Result<(), YeetError> {
        self.finish(true).await
    }

    /// Skip remote STOP after confirmed natural completion; still join transport.
    pub async fn finish(self, stop_owned: bool) -> Result<(), YeetError> {
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
            Ok::<_, YeetError>(())
        };
        let remote = tokio::time::timeout(Duration::from_secs(2), stop).await;
        // Always run this even when remote STOP fails or times out.
        self.client.disconnect().await.map_err(cast_error)?;
        match remote {
            Ok(result) => result,
            Err(_) => Err(YeetError::Cast(
                "remote STOP timed out; local connection closed".into(),
            )),
        }
    }
}

fn cast_error(error: oxicast::Error) -> YeetError {
    YeetError::Cast(error.to_string())
}

fn check_response(response: &Value) -> Result<(), YeetError> {
    match response.get("type").and_then(Value::as_str) {
        Some("MEDIA_STATUS" | "RECEIVER_STATUS") => Ok(()),
        _ => Err(YeetError::Cast(format!("receiver response: {response}"))),
    }
}

fn snapshot(response: &Value) -> Result<PlaybackSnapshot, YeetError> {
    let value = response
        .get("status")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .ok_or_else(|| YeetError::Cast(format!("no media status: {response}")))?;
    serde_json::from_value(value.clone()).map_err(|e| YeetError::Cast(e.to_string()))
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

fn load_payload(
    session: &str,
    video: &str,
    subtitles: Option<&str>,
    title: &str,
    start_position: f64,
) -> Value {
    let mut payload = json!({
        "type":"LOAD", "sessionId":session, "autoplay":true, "currentTime":start_position,
        "media":{
            "contentId":video, "contentType":"video/mp4", "streamType":"BUFFERED",
            "metadata":{"metadataType":0,"title":title}
        }
    });
    if let Some(subtitles) = subtitles {
        payload["activeTrackIds"] = json!([1]);
        payload["media"]["tracks"] = json!([{"trackId":1,"type":"TEXT","subtype":"SUBTITLES",
                "trackContentId":subtitles,"trackContentType":"text/vtt",
                "name":"Subtitles"}]);
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
        assert!(matches!(result, Err(YeetError::Cancelled)));
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
