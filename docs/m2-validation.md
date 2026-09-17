# M2 validation

Date: 2026-09-17. Development baseline: Fedora 44, Rust 1.96.0,
FFmpeg/ffprobe 8.1.2. **Implementation and automated verification are complete;
the M2 CLI's real-TV acceptance is pending.**

No real receiver was discovered, launched, or stopped during this work. The user
was asleep. M1 already established visible MP4/WebVTT playback on the test receiver,
but that is evidence for the prototype, not full acceptance of the new CLI.

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

## Remaining hardware acceptance

When the user is available, use the existing ignored `samples/m1/test.mp4` with
its matching VTT and the SRT sidecar prepared during M2. The silent 12-minute
test pattern avoids changing volume and shows changing subtitle cues.

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

For each format, confirm visible changing subtitles and playback through natural
completion. Repeat with Ctrl+C during playback and SIGTERM in another run;
confirm the TV stops and the serving port closes. Check interrupted loading when
practical. Add a full real-world compatible MP4 if available; the generated clip
does not establish broad format compatibility. Then mark M2 verified and proceed
to M3 controls. Do not interpret the commands above as a scheduled hardware test.
