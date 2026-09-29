# M2 validation

Historical evidence from 2026-09-17 and follow-ups, on Fedora 44 with Rust 1.96.0
and FFmpeg/ffprobe 8.1.2. This records the original direct-play CLI and its fixes;
current selection, subtitles and retained storage are described in
[automatic playback](automatic-playback.md) and [preferences](preferences-and-subtitles.md).

## Implemented boundary

M2 introduced IPv4 discovery, explicit device selection, MP4-family H.264/stereo
AAC playback, external SRT/WebVTT, HTTP range/CORS serving and a reusable session
coordinator. Its narrow media guard was later expanded by model rules and media
preparation. Rejection of an MKV during M2 is not a current format limitation.

The lifecycle decisions still apply: callers cancel and await cleanup, only owned
media is stopped, empty status is not completion, and failed remote cleanup does
not prevent releasing local resources. SIGKILL cannot run application cleanup.

## Checks completed

Automated tests passed for HTTP bytes/ranges/HEAD/CORS, simulated TLS receivers,
LOAD and subtitle failures, transient states, completion, takeover, disconnect,
and cancellation. Real FFmpeg fixtures exercised SRT/WebVTT sessions and source
preservation. Signal tests verified exit 130/143 and child reaping. Socket/signal
checks required normal host permissions rather than the restricted sandbox.

## User-confirmed hardware checks

On KPN DIW7022, the user confirmed visible video/subtitles and correct Ctrl+C
shutdown in separate external WebVTT and SRT runs. SIGTERM during WebVTT playback
reported `Cancelled; cleanup completed.` and exit 143; that transcript did not
include a separate port check. Casting resolved IPv4 even while the preceding
standalone device listing sometimes showed empty address lists.

After the completion fix below, the WebVTT short clip exited successfully and
closed its port. Later [M5 batch evidence](automatic-playback.md#hardware-batch-result)
adds short-clip SRT/WebVTT completion and cancellation/port checks across preparation
paths; it should not be confused with an unperformed repeat of every M2 scenario.

## IPv4 discovery follow-up

This was resolution behavior, not a display bug. mdns-sd 0.21.3 could consider a
service resolved after only an IPv6/AAAA record, suppressing its usual hostname
query. Filtering that result to IPv4 left an empty address list.

Yeet explicitly resolves hostnames when service snapshots lack IPv4, deduplicates
queries, and keeps them within the original scan deadline. Reply consumers are
joined and the daemon is shut down on completion/cancellation. All eight follow-up
read-only LAN scans returned IPv4 for both test devices. Regression tests cover
IPv6-first, duplicate/already-resolved events and unanswered lookups. This does
not establish IPv6 transport or discovery through network isolation/firewalls.

## Natural-completion follow-up

The user saw the 30-second WebVTT clip finish normally, but Yeet returned to
Loading and timed out. The receiver sent one unsolicited owned `IDLE/FINISHED`
status; later polls were empty. The adapter had been reading only request replies.

The adapter now consumes broadcasts alongside polls, applying the same session
and content-ownership checks. FINISHED is completion; elapsed time, empty status,
foreign events and session-zero IDLE are not. A subsequent run printed
`Playback completed.`, returned 0 after 30.712 seconds, and closed the HTTP port.
The terminal event-to-exit delay was about 2 ms. This establishes lifecycle behavior;
the user's earlier report provides the visual evidence. Simulated regressions
cover terminal events before/during polls, immediate disconnect, errors and takeover.

## Reproduction and remaining hardware acceptance

Generate a short fixture (the script refuses to overwrite existing output):

```sh
bash scripts/generate-m1-fixture.sh samples/short 30
cargo run --locked -- samples/short/test.mp4 \
  --device "Living Room" --subtitles samples/short/subtitles.vtt --no-resume --http-port 8010
```

Replace the receiver and repeat with `subtitles.srt`. Let the clip finish; expect
`Playback completed.` and exit 0. For a separate SIGTERM run, identify the casting
PID with `pgrep -a -x yeet` in another terminal and send `kill -TERM PID`. Expect
cleanup and exit 143. Check the selected port with `ss -H -ltn 'sport = :8010'`.
Use Ctrl+C for the separate SIGINT/exit-130 path; do not use SIGKILL for a cleanup test.

Interrupted loading and signal-port closure were not individually reported for
all original M2 runs. Later batch and automated evidence cover broader lifecycle
cases. Remaining project validation is tracked in [the roadmap](plan.md), rather
than treating this historical checklist as a release gate.
