# Procast

A Rust CLI and reusable backend for casting local videos and subtitles to
Chromecast. **The CLI now supports discovery, direct-play casting, and external
SRT/WebVTT subtitles.** The M2 CLI has passed automated tests, and the user has
verified visible subtitles and Ctrl+C shutdown with both formats on a KPN DIW7022.
Further hardware and discovery checks are tracked in the validation notes.

## Development setup

- Rust via rustup. `rust-toolchain.toml` pins Rust 1.96.0 with rustfmt and Clippy;
  rustup installs the pinned toolchain when needed.
- A working native linker/C compiler (such as Fedora's GCC).
- `ffprobe` on PATH for inspection and casting. `ffmpeg` is required for SRT
  conversion and optional test-fixture generation. Existing WebVTT needs no FFmpeg.
- Network access for the initial Cargo dependency download.

No FFmpeg development headers, Qt, or KDE libraries are needed. Cargo resolves
the Rust dependencies; `Cargo.lock` records tested versions.
The development baseline is Fedora 44 with FFmpeg/ffprobe 8.1.2. Codec support
depends on the installed FFmpeg build, not just its version.

## Run

```sh
cargo run --locked -- devices
cargo run --locked -- devices --json
cargo run --locked -- cast /path/to/movie.mp4 --device "Living Room" --subtitles /path/to/movie.srt
cargo run --locked -- inspect /path/to/video.mkv
cargo run --locked -- inspect /path/to/video.mkv --json
cargo run --locked -- --verbose inspect /path/to/video.mkv
cargo run --locked -- inspect /path/to/video.mkv --ffprobe /usr/bin/ffprobe --timeout 60
```

`devices` scans for five seconds by default (`--scan-seconds 1..60`). Choose an
exact friendly name or device ID; duplicate names require an ID. Omitting
`--device` selects automatically only when exactly one discovered receiver
advertises video support. Known audio-only devices are rejected, and unknown
capabilities require explicit selection. `--host 192.168.1.50` bypasses discovery;
use `--cast-port` only with `--host`. Discovery and casting currently use IPv4.

`cast` stays in the foreground until natural completion or interruption. It
replaces playback on the selected receiver. Ctrl+C or SIGTERM stops Procast's
owned media, closes its server and transport, reaps subprocesses, and removes
temporary subtitles. If another sender takes over, Procast exits without stopping
their session. Interactive playback controls are planned for M3.

### Current media support

M2 validates a conservative direct-play profile before contacting a receiver:

- MP4-family container reported by ffprobe, with one H.264 video stream.
- Baseline, Constrained Baseline, Main, or High profile; level up to 4.1,
  8-bit `yuv420p`, at most 1920×1080 and 30 fps; no HDR transfer function.
- No audio, or one mono/stereo AAC-LC track at up to 48 kHz.
- Optional external UTF-8 `.srt` or `.vtt`, up to 4 MiB, with valid timed cues.

The profile is a preflight policy, not a guarantee that every receiver can decode
every accepted file. Receiver rejection remains a runtime error. MKV, E-AC-3,
multichannel audio, embedded subtitle extraction, and remuxing/transcoding are
not implemented yet. Subtitles are used only when explicitly selected; source
files are never modified. SRT conversion has a 30-second limit, and temporary
files are session-owned. WebVTT cue timing is validated; styling semantics are
left to the receiver. Track activation and HTTP requests do not prove visual
rendering, which requires the hardware check.

Override executables with `cast --ffprobe PATH --ffmpeg PATH`; use
`--probe-timeout SECONDS` to change the probe limit.

### Network and session behaviour

The host must stay awake, and the receiver must be able to fetch files from it.
Procast chooses a local address using the route to the receiver and an OS-assigned
HTTP port. `--bind-address LOCAL_IP --http-port 8010` provides a fixed interface
and port for troubleshooting. Check firewall rules, VPN routing, mDNS, and Wi-Fi
client isolation if discovery or downloads fail. Procast does not edit firewall
rules. Only the selected video and optional prepared subtitles are served, under
random session URLs, with range requests and subtitle CORS support.

Cast connections use TLS without receiver identity verification, and media is
served over HTTP. Use a trusted LAN; see the [transport decision](docs/decisions/001-cast-library.md).
Transient empty/IDLE statuses are not completion. A lost connection fails without
automatically restarting the video. Inactive loading/buffering has a 30-second
limit; individual Cast requests have a 10-second limit. Cleanup attempts remote
STOP for at most two seconds, then disconnects and joins transport workers while
the HTTP server shuts down. Remote STOP cannot be guaranteed after a network loss.

### Inspection and output

Inspection reports the container, duration, stream codecs, video dimensions,
audio channels/sample rates, and subtitle languages/titles where available.
Unknown metadata remains unknown; inspection does not determine Cast compatibility.
JSON is normalized Procast metadata, not raw ffprobe output; its schema is not
yet a stable public API.

The input must be a regular local file. Paths containing spaces, Unicode, or shell
characters are passed directly as arguments. Source files are never modified.
Use `--` before a positional filename that begins with a dash.

Results go to stdout; errors and diagnostic logs go to stderr. `--verbose` enables
debug logging, and `RUST_LOG` can override the filter. Exit codes are 0 for success,
1 for operational errors, 2 for invalid arguments, 130 for Ctrl+C, and 143 for
SIGTERM on Linux. Cancellation waits for resource cleanup before returning.

Probing has a default 30-second runtime limit, a 4 MiB metadata limit, and retains
only the last 32 KiB of ffprobe's error output. Both output pipes are drained
concurrently. The backend accepts a cancellation token; callers must cancel and
await completion to guarantee cleanup.

For a standalone binary:

```sh
cargo build --release --locked
./target/release/procast inspect /path/to/video.mkv
```

## Checks

```sh
cargo fmt --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
# Explicit integration tests using installed ffmpeg/ffprobe:
cargo test --locked --workspace -- --ignored
```

Ordinary tests use fixtures and fake subprocesses; Linux lifecycle tests verify
that cancellation/timeouts leave no live or zombie child process. Unix tests use
the usual shell/coreutils tools. The ignored tests generate tiny temporary media
and exercise inspection plus complete casting sessions with SRT and VTT. A
missing FFmpeg or required encoder is a failure when explicitly requested. No
test contacts real Cast devices. Simulated receivers use local TLS and fetch the
served HTTP files, exercising errors, takeover, and cancellation as well as success.
The test environment must allow local sockets and normal process-signal delivery.

See [M2 validation and the remaining hardware check](docs/m2-validation.md).

## Cast feasibility example (M1)

Generate a silent 12-minute H.264/AAC MP4 and external WebVTT cues. This requires
FFmpeg with `libx264` and AAC encoding. Generated files stay in ignored `samples/`.
The script refuses to overwrite an existing fixture.

```sh
bash scripts/generate-m1-fixture.sh
cargo run --locked -p procast-core --example cast_probe -- --discover
cargo run --locked -p procast-core --example cast_probe -- \
  --device "Living Room" \
  --video samples/m1/test.mp4 --subtitles samples/m1/subtitles.vtt \
  --http-port 8010 --seconds 620 --exercise-controls
```

Choose an exact discovered name or ID. The test replaces playback on that receiver
and keeps serving for the requested duration. `--exercise-controls` pauses at
20 seconds, resumes at 25, seeks forward to 90 seconds at 40, and back to 20 at 60.
Without that flag it simply plays and reports status. Ctrl+C/SIGTERM triggers
cleanup. The example treats cancellation as a successful test exit; this is
separate from the production CLI exit-code contract above.

`--host IP --cast-port 8009` bypasses discovery. `--bind-address IP` can override
the local address chosen from the route to the receiver. The default HTTP port
is OS-assigned; the example above chooses a fixed port for firewall diagnosis.
The receiver must be able to reach this port. Procast does not edit firewall rules.
Only the two selected resources are served, under random session URLs.

This prototype uses IPv4 discovery, requires an already compatible MP4 and valid
WebVTT, and does not inspect/convert the inputs or extract embedded subtitles.
The host must remain awake. It uses encrypted Cast transport without device
authentication, for use on a trusted LAN; see the [M1 decision](docs/decisions/001-cast-library.md).

Prefer the production `procast devices` / `procast cast` commands for normal use.

Keep personal sample videos outside the repository or under the ignored
`samples/` directory.

See [the outline](docs/outline.md) and [implementation plan](docs/plan.md).
