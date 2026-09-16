# Procast

A Rust CLI and reusable backend for casting local videos and subtitles to
Chromecast. **M0 implements media inspection; casting is not implemented yet.**

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

Keep personal sample videos outside the repository or under the ignored
`samples/` directory.

See [the outline](docs/outline.md) and [implementation plan](docs/plan.md).
