# Cast transport for M1

Date: 2026-09-17
Status: accepted for M1; hardware and failure-path validation passed.

## Decision

Use **oxicast 0.0.3**, pinned exactly, behind `procast-core::cast`. Use its TLS,
framing, heartbeat, and request-correlation implementation. Procast owns receiver
launch, media/track payloads, playback state, and session ownership. No dependency
fork or patch was needed. The library remains replaceable and its types are not
part of Procast's public API.

The published `rust_cast` 0.21.0 source was evaluated first. It uses blocking I/O,
hides the socket inside `CastDevice`, and reads frame headers/payloads using local
buffers and `read_exact`. Adding ordinary read timeouts alone would not preserve
partial frames. Subtitle fields are also missing from its media/load types.
Its optional `thread_safe` feature changes synchronization, but does not solve
blocking reads or expose transport cancellation. Making this fit the desired
lifecycle would require more transport changes than the alternative.

oxicast already separates a persistent async reader from outgoing commands.
Cancelling a Procast request does not restart that reader or lose partial bytes.
Shutdown cancels and joins its tasks. Its media types also lack text tracks, but
`send_raw` supports correlated JSON requests, including LOAD and EDIT_TRACKS_INFO.
Procast explicitly supplies WebVTT track metadata and activates track ID 1.

## Adapter constraints

- Automatic reconnection is disabled. Connection loss fails clearly; playback
  is never silently restarted.
- Connect and requests have ten-second bounds and cooperative cancellation.
  Callers cancel and await, then explicitly close the session.
- Procast stores its original receiver transport/session, media ID, and unique
  content URL. Cleanup checks those before STOP, with a two-second remote deadline.
  Local shutdown runs even if the receiver cannot be reached.
- oxicast's high-level `disconnect()` stops its internally tracked current media
  session. Procast therefore uses raw LAUNCH/CONNECT and never populates its
  high-level application handle. This leaves disconnect as transport cleanup;
  Procast sends STOP itself for the owned session. Re-evaluate this assumption
  before upgrading oxicast. A transport-only close API would be a useful upstream
  improvement; no upstream message or PR has been sent.
- HTTP serving uses axum routing and tower-http file/range handling. Procast owns
  the HTTP connection tasks and joins/aborts them during shutdown, including
  stalled clients. Only registered media and subtitle URLs are available.
- Discovery uses mdns-sd 0.21.3 directly and currently selects IPv4 addresses.
  Exact names and IDs work; duplicate names require an ID. Audio-only receivers
  are listed but rejected for the video test when their capability bit is known.

## TLS behaviour

Cast devices use certificates that do not follow ordinary public-web trust.
The prototype explicitly uses oxicast's `verify_tls(false)`: the connection is
encrypted, and handshake signatures are verified, but the device certificate
identity is not authenticated. Cast device-auth is not implemented by oxicast.
This is a trusted-LAN prototype, not an authenticated remote-control service.

The dependency graph enables both rustls ring and aws-lc providers. Procast selects
ring only when the embedding application has not already installed a provider,
avoiding rustls's ambiguous-provider panic. Both native dependency trees currently
build; simplifying the upstream feature selection is later dependency cleanup.

## Hardware evidence

Target discovered by mDNS: a receiver advertising model **KPN DIW7022**,
Cast TCP port 8009. Firmware version was not obtained. The receiver advertised
video capability. A Google Nest Mini was also discovered but was not cast to.

Development host: Fedora 44, Rust 1.96.0, FFmpeg/ffprobe 8.1.2. Default Media Receiver
application ID: `CC1AD845`. Existing host firewall settings already allowed the
chosen HTTP port; no firewall configuration was changed.

Fixture: generated 640×360, 15 fps H.264 Constrained Baseline video, silent stereo AAC-LC at
48 kHz, 720 seconds, MP4 with faststart. Separate UTF-8 WebVTT has a visibly
numbered cue every five seconds. The generator is committed; media is ignored.

Observed:

- Discovery, receiver launch, video fetch, and subtitle fetch succeeded.
- The user confirmed both moving video and changing subtitle text on the TV.
- Receiver status reports `PLAYING` and `activeTrackIds: [1]`.
- Pause held the reported position; resume advanced it again.
- Forward seek to 90 seconds and backward seek to 20 seconds succeeded, with
  further HTTP requests and subtitle track 1 still active.
- The sustained run lasted 622 seconds (including a five-second pause), with no
  reported disconnect or failed status request. Subtitles remained active.
- Timed shutdown received a successful remote STOP response and completed local
  cleanup in 155 ms.
- Two subsequent runs on the latest build verified SIGINT and SIGTERM during
  playback. They exited in 83 ms and 88 ms respectively, with successful remote
  STOP and the HTTP port closed. The same port was reusable between runs.
- Formatting and Clippy passed. All 18 ordinary automated tests passed (the
  additional real-FFmpeg fixture test remains opt-in and previously passed in M0).

Automated checks use a generated TLS certificate and a loopback fake receiver.
They exercise a refused connection, a stalled TLS handshake deadline,
cancellation during TLS negotiation, disconnection while awaiting LAUNCH, and
cancellation with an incomplete Cast frame. A correlated-response test ignores
an unrelated reply and confirms disconnect does not send STOP after a foreign
media-status broadcast. HTTP tests check full/ranged reads,
HEAD, invalid ranges, MIME/CORS/preflight, unregistered paths, and stalled-client
shutdown. No ordinary test discovers or controls real TVs.

## Limits and next step

This is evidence for one receiver and one prepared media profile. It does not
establish all Chromecast generations, all codecs, accurate subtitle rendering
for every file, or HDR support. The user's representative MKV was inspected in
M0; it has not been cast in M1. Embedded text extraction, audio compatibility,
remuxing/transcoding, and SRT conversion are subsequent work.

M2 should build the user-facing commands and lifecycle around this adapter,
including media validation, actionable network errors, and more state-transition
and takeover tests. The adapter is a feasibility API, not a stable public API.

## Sources inspected

- [rust_cast 0.21.0 source](https://docs.rs/crate/rust_cast/0.21.0/source/src/)
- [oxicast 0.0.3 source](https://docs.rs/crate/oxicast/0.0.3/source/src/)
- [Cast load request / active tracks](https://developers.google.com/cast/docs/reference/web_receiver/cast.framework.messages.LoadRequestData)
- [Cast track fields](https://developers.google.com/cast/docs/reference/web_receiver/cast.framework.messages.Track)
