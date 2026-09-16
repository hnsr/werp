# Procast

A Rust CLI and reusable backend for casting local videos and subtitles to
Chromecast. **Media inspection is available in the CLI; casting is currently an
explicit M1 hardware-test example.**

## Development setup

- Rust via rustup. `rust-toolchain.toml` pins Rust 1.96.0 with rustfmt and Clippy;
  rustup installs the pinned toolchain when needed.
- A working native linker/C compiler (such as Fedora's GCC).
- `ffprobe` on PATH for media inspection. FFmpeg itself is also needed to generate
  the optional integration-test fixture and for future conversion features.
- Network access for the initial Cargo dependency download.

No FFmpeg development headers, Qt, KDE libraries, or Cast hardware are needed for
M0. Cargo resolves the Rust dependencies; `Cargo.lock` records tested versions.
The development baseline is Fedora 44 with FFmpeg/ffprobe 8.1.2. Codec support
depends on the installed FFmpeg build, not just its version.

## Run

```sh
cargo run --locked -- inspect /path/to/video.mkv
cargo run --locked -- inspect /path/to/video.mkv --json
cargo run --locked -- --verbose inspect /path/to/video.mkv
cargo run --locked -- inspect /path/to/video.mkv --ffprobe /usr/bin/ffprobe --timeout 60
```

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
SIGTERM on Linux. Cancellation stops and reaps ffprobe before returning.

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
# Explicit integration test using installed ffmpeg/ffprobe:
cargo test --locked -p procast-core --test probe -- --ignored
```

Ordinary tests use fixtures and fake subprocesses; Linux lifecycle tests verify
that cancellation/timeouts leave no live or zombie child process. Unix tests use
the usual shell/coreutils tools. The ignored integration test generates a tiny
temporary video with audio and subtitles, then removes it. A missing FFmpeg is a
failure when this test is explicitly requested. No test contacts Cast devices.
Cast transport tests use generated certificates and fake receivers on loopback.
The test environment must allow local sockets and normal process-signal delivery.

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
separate from the production `inspect` exit-code contract above.

`--host IP --cast-port 8009` bypasses discovery. `--bind-address IP` can override
the local address chosen from the route to the receiver. The default HTTP port
is OS-assigned; the example above chooses a fixed port for firewall diagnosis.
The receiver must be able to reach this port. Procast does not edit firewall rules.
Only the two selected resources are served, under random session URLs.

This prototype uses IPv4 discovery, requires an already compatible MP4 and valid
WebVTT, and does not inspect/convert the inputs or extract embedded subtitles.
The host must remain awake. It uses encrypted Cast transport without device
authentication, for use on a trusted LAN; see the [M1 decision](docs/decisions/001-cast-library.md).

The final `procast devices` / `procast cast` commands belong to M2.

Keep personal sample videos outside the repository or under the ignored
`samples/` directory.

See [the outline](docs/outline.md) and [implementation plan](docs/plan.md).
