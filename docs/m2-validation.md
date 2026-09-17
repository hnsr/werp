# M2 validation

Date: 2026-09-17. Development baseline: Fedora 44, Rust 1.96.0,
FFmpeg/ffprobe 8.1.2. **Implementation and automated verification are complete.
The user verified visible video/subtitles and Ctrl+C shutdown with both external
WebVTT and SRT on the test receiver. Remaining hardware checks are listed below.**

Initial M2 development used simulated receivers only. The user subsequently
tested the production CLI on the KPN DIW7022 and reported the results below.

## Implemented boundary

- `procast devices` lists names, IDs, models, IPv4 endpoints, and advertised
  capabilities, with optional JSON output. Exact selectors resolve duplicate
  names safely. Automatic selection requires one confirmed video receiver.
- `procast cast` validates media and prepares optional external subtitles before
  receiver connection. It starts the HTTP server before LOAD and stays in the
  foreground until completion, failure, or a signal.
- `session::run` owns resources and emits Procast state through a watch channel.
  Its result reliably reports completion/error even if a frontend misses an
  intermediate state. Callers cancel the token and await the operation; dropping
  an in-progress future is not the supported cleanup contract.
- Only an owned positive media session with explicit `IDLE/FINISHED` completes
  successfully. Empty status and session-zero IDLE are transitional. A different
  session ends Procast without stopping the new sender. No automatic reconnect
  or replay is attempted.
- Requested subtitles must have valid timed cues, convert successfully if SRT,
  activate on the receiver, and receive an HTTP GET. These checks cannot verify
  visual rendering. Temporary resources are removed on all normal return paths,
  including failure and cancellation; abrupt process termination such as SIGKILL
  cannot run cleanup.

The media policy is deliberately narrow: MP4-family, H.264 Baseline/Main/High
up to level 4.1, 8-bit 4:2:0, at most 1920×1080/30 fps, no HDR, optional one
AAC-LC mono/stereo track at 8–48 kHz. ffprobe's format family also includes MOV
and related variants; acceptance is not proof of receiver compatibility. Missing
required metadata is an error. This is a Procast policy informed by
[Google's receiver-format documentation](https://developers.google.com/cast/docs/media),
not per-device capability negotiation. Video decoding and subtitle rendering
remain receiver-dependent. See [WebVTT's format specification](https://www.w3.org/TR/webvtt1/)
for cue syntax; Procast validates timed cues rather than implementing the full
styling parser.

## Checks completed

```sh
cargo fmt --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo test --locked --workspace -- --ignored
```

Results: **27 ordinary tests passed; both explicitly enabled FFmpeg tests passed.**
Local sockets and subprocess signal tests ran with normal host permissions;
restricted signal delivery inside the development sandbox is not representative.

Coverage includes:

- Valid, unsupported, and incomplete ffprobe metadata; ambiguous or audio-only
  device selection; argument conflicts and defaults.
- HTTP bytes, ranges, HEAD, CORS/preflight, unknown routes, and stalled client
  shutdown. Simulated TLS receivers fetch the video and prepared subtitle bytes.
- Natural completion, transient/empty IDLE, buffering, rejected LOAD, rejected
  text track, activated-but-unfetched subtitles, disconnect, and sender takeover.
- Cancellation while LOAD is outstanding and during playback; an owned STOP,
  closed HTTP listener, and closed transport after repeated sessions.
- SIGINT/SIGTERM during CLI preparation returning 130/143, with ffprobe reaped.
  Cancelled subtitle conversion reaps FFmpeg and removes its temporary directory.
- Native WebVTT works with a nonexistent FFmpeg executable. Invalid cues and
  non-UTF-8 data fail, and missing conversion executables are reported.
- Real FFmpeg generates a small H.264/AAC MP4; real probing and SRT or VTT
  preparation feed full sessions against a simulated receiver. Downloaded bytes
  match the source and expected subtitle text.

The supplied personal MKV was also checked with the CLI. It fails during media
validation before discovery and exits with code 1. Its container, multichannel
E-AC-3, and embedded subtitles need later media support. The source is untouched.

Remote cleanup has a two-second STOP budget. oxicast's disconnect has a separate
two-second send budget followed by aborting and joining its reader/writer/heartbeat
tasks. HTTP shutdown runs concurrently, with a two-second graceful budget before
abort/join. Normal responsive cancellation is expected within three seconds;
unresponsive transport cleanup can consume both remote budgets. OS-level stalled
filesystem operations are not covered by a strict wall-clock guarantee.

## User-confirmed hardware checks

Using the generated MP4 fixture in separate CLI runs, the user confirmed:

- External WebVTT: video played with visible subtitles; Ctrl+C shut down correctly.
- External SRT: video played with visible subtitles; Ctrl+C shut down correctly.
- The WebVTT run reported loading, playing, buffering, and a return to playing.
- Both casting runs resolved an IPv4 endpoint and reached playback.

The preceding `devices` command listed the video receiver and an audio-only
receiver, but both had empty IPv4 address lists. The follow-up investigation and
fix are recorded below.

Device names, IDs, local addresses, and personal media filenames are omitted.

## IPv4 discovery follow-up

The issue reproduced in JSON output: two of three initial read-only scans had
no IPv4 address for the video receiver. It was not a display-formatting bug.
`mdns-sd` 0.21.3 treats a service with only an IPv6/AAAA address as resolved. Its
normal service-resolution path skips the hostname address query once any address
record is cached. Procast filtered out IPv6 for its IPv4-only transport, leaving
an empty list unless an IPv4/A record arrived separately.

Procast now explicitly resolves the hostname when a service snapshot lacks IPv4.
New address records produce updated service snapshots through the existing
browse channel. Lookups are deduplicated per hostname, stay within the original
scan deadline, and have their reply consumers joined on timeout or cancellation.
Daemon shutdown stops the DNS queries. No previous scan's addresses are retained.

All eight read-only scans after the fix returned IPv4 addresses for both test
devices. Three diagnostic scans each showed the IPv6-only result, the explicit
lookup, and a later snapshot containing IPv4. No receiver application was
launched or controlled. This verifies the fallback on the current LAN; it does
not guarantee discovery through firewalls or for devices that only support IPv6.

Three new regression tests cover the IPv6-first sequence and duplicate events,
already-resolved IPv4, and unanswered lookups under deadline/cancellation. The
ordinary suite now has 30 passing tests; formatting and Clippy also pass. The
two FFmpeg tests passed previously and were not rerun for this discovery-only fix.

## Reproduction and remaining hardware acceptance

Use the existing ignored `samples/m1/test.mp4` with its matching VTT and SRT
sidecars. The silent 12-minute test pattern avoids changing volume and shows
changing subtitle cues.

```sh
cargo run --locked -- devices
cargo run --locked -- cast samples/m1/test.mp4 \
  --device "Living Room" --subtitles samples/m1/subtitles.vtt --http-port 8010
cargo run --locked -- cast samples/m1/test.mp4 \
  --device "Living Room" --subtitles samples/m1/subtitles.srt --http-port 8010
```

Replace `Living Room` with the selected receiver's name or ID from `devices`.
On a fresh checkout, generate the MP4/VTT with
`bash scripts/generate-m1-fixture.sh`, then create the SRT test sidecar with:

```sh
ffmpeg -nostdin -v error -n -i samples/m1/subtitles.vtt samples/m1/subtitles.srt
```

Still unverified on the M2 hardware path: playback through natural completion,
SIGTERM shutdown, interrupted loading, and explicit checks that the serving port
closes. These paths have automated coverage, but the user's report confirms
Ctrl+C and visible playback only. Add a full real-world compatible MP4 if
available; the generated clip does not establish broad format compatibility.
Complete those checks before marking all M2 acceptance complete. M3 remains
playback controls. The commands above are
manual reproduction instructions, not a scheduled hardware test.
