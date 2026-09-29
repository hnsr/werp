# Cast transport decision

Use **oxicast 0.0.3**, pinned exactly, behind `werp-core::cast`. Use its TLS,
framing, heartbeat, and request-correlation implementation. Werp owns receiver
launch, media/track payloads, playback state, and session ownership. No dependency
fork or patch was needed. The library remains replaceable and its types are not
part of Werp's public API.

The published `rust_cast` 0.21.0 source was evaluated first. It uses blocking I/O,
hides the socket inside `CastDevice`, and reads frame headers/payloads using local
buffers and `read_exact`. Adding ordinary read timeouts alone would not preserve
partial frames. Subtitle fields are also missing from its media/load types.
Its optional `thread_safe` feature changes synchronization, but does not solve
blocking reads or expose transport cancellation. Making this fit the desired
lifecycle would require more transport changes than the alternative.

oxicast already separates a persistent async reader from outgoing commands.
Cancelling a Werp request does not restart that reader or lose partial bytes.
Shutdown cancels and joins its tasks. Its media types also lack text tracks, but
`send_raw` supports correlated JSON requests, including LOAD and EDIT_TRACKS_INFO.
Werp explicitly supplies WebVTT track metadata and activates track ID 1.

## Adapter constraints

- Automatic reconnection is disabled. Connection loss fails clearly; playback
  is never silently restarted.
- Connect and requests have ten-second bounds and cooperative cancellation.
  Callers cancel and await, then explicitly close the session.
- Werp stores its original receiver transport/session, media ID, and unique
  content URL. Cleanup checks those before STOP, with a two-second remote deadline.
  Local shutdown runs even if the receiver cannot be reached.
- oxicast's high-level `disconnect()` stops its internally tracked current media
  session. Werp therefore uses raw LAUNCH/CONNECT and never populates its
  high-level application handle. This leaves disconnect as transport cleanup;
  Werp sends STOP itself for the owned session. This assumption matters when upgrading oxicast. A transport-only close API
  would be a useful upstream
  improvement; no upstream message or PR has been sent.
- HTTP serving uses axum routing and tower-http file/range handling. Werp owns
  the HTTP connection tasks and joins/aborts them during shutdown, including
  stalled clients. Only registered media and subtitle URLs are available.
- Discovery uses mdns-sd 0.21.3 directly and currently selects IPv4 addresses.
  Exact names and IDs work; duplicate names require an ID. Audio-only receivers
  are listed but rejected for the video test when their capability bit is known.

## Follow-up fixes

An IPv6-first mDNS result could leave device listings without IPv4: mdns-sd
0.21.3 considered the service resolved and suppressed its usual hostname query.
Werp explicitly resolves hostnames missing IPv4, deduplicates queries and keeps
work within the scan deadline. Eight follow-up LAN scans returned IPv4 for both
test devices; regressions cover IPv6-first, duplicate and unanswered lookups.
This does not establish IPv6 transport or discovery across isolated networks.

A short clip initially finished on the TV but timed out in the CLI because its
single unsolicited `IDLE/FINISHED` event was ignored and later polls were empty.
The adapter now consumes broadcasts alongside replies, checking session/content
ownership for both. FINISHED establishes completion; empty status, elapsed time,
foreign events and session-zero IDLE do not. A follow-up 30-second run exited 0
and closed its HTTP port. Simulated tests cover terminal events during polling,
immediate disconnect, errors and takeover.

Phone-initiated stop initially timed out despite successful local cleanup.
App-channel CLOSE, failed polls and stale BUFFERING/empty replies now trigger
checks for the owned receiver application's exit. Confirmed exit or owned
CANCELLED media status ends normally and skips redundant STOP; a timeout alone
never proves a normal stop. An unqueryable receiver or an app still running
remains an error. Simulated tests cover foreign closes and still-running apps.
This improves lifecycle handling, but is not a proven fix for the separate
intermittent startup media exit recorded in the device evidence.

## TLS behaviour

Cast devices use certificates that do not follow ordinary public-web trust.
Werp uses oxicast's `verify_tls(false)`: the connection is
encrypted, and handshake signatures are verified, but the device certificate
identity is not authenticated. Cast device-auth is not implemented by oxicast.
Werp is intended for trusted LANs; it is not an authenticated remote-control service.

The dependency graph enables both rustls ring and aws-lc providers. Werp selects
ring only when the embedding application has not already installed a provider,
avoiding rustls's ambiguous-provider panic. Both native dependency trees currently
build; simplifying the upstream feature selection is later dependency cleanup.

## Sources inspected

- [rust_cast 0.21.0 source](https://docs.rs/crate/rust_cast/0.21.0/source/src/)
- [oxicast 0.0.3 source](https://docs.rs/crate/oxicast/0.0.3/source/src/)
- [Cast load request / active tracks](https://developers.google.com/cast/docs/reference/web_receiver/cast.framework.messages.LoadRequestData)
- [Cast track fields](https://developers.google.com/cast/docs/reference/web_receiver/cast.framework.messages.Track)
