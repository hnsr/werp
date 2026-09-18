# Procast

A Rust CLI and reusable backend for casting local videos and subtitles to
Chromecast. **The CLI supports discovery, direct-play casting, optional forced
transcoding, and external SRT/WebVTT subtitles.** The M2 CLI has passed automated tests, and the user has
verified visible subtitles and Ctrl+C shutdown with both formats on a KPN DIW7022.
Further hardware checks are tracked in the validation notes.

## Development setup

- Rust via rustup. `rust-toolchain.toml` pins Rust 1.96.0 with rustfmt and Clippy;
  rustup installs the pinned toolchain when needed.
- A working native linker/C compiler (such as Fedora's GCC).
- `ffprobe` on PATH for inspection and casting. `ffmpeg` is required for SRT
  conversion, forced transcoding, and optional test-fixture generation. Direct
  playback with existing WebVTT needs no FFmpeg. Forced transcoding requires
  `libx264` and AAC encoders plus a decoder for the input codecs.
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
temporary media and subtitles. If another sender takes over, Procast exits without stopping
their session. Interactive playback controls are planned for M3.

### Current media support

M2 validates a conservative direct-play profile before contacting a receiver:

- MP4-family container reported by ffprobe, with one H.264 video stream.
- Baseline, Constrained Baseline, Main, or High profile; level up to 4.1,
  8-bit `yuv420p`, at most 1920×1080 and 30 fps; no HDR transfer function.
- No audio, or one mono/stereo AAC-LC track at up to 48 kHz.
- Optional external UTF-8 `.srt` or `.vtt`, up to 4 MiB, with valid timed cues.

The profile is a preflight policy, not a guarantee that every receiver can decode
every accepted file. Receiver rejection remains a runtime error. For an explicit
trial of **3–6 channel AAC-LC**, **HEVC Main/Main 10**, or **H.264 with AC-3**
in MP4, use:

```sh
cargo run --locked -- cast /path/to/surround.mp4 \
  --device "Living Room" --experimental-direct-play --http-port 8010
```

The experimental flag serves the original file without audio/video conversion
and reports receiver-dependent support. HEVC trials are limited to level 4.0,
8/10-bit 4:2:0, and 1080p30, with zero or one AAC-LC track of up to six channels.
Check picture, colours, audible dialogue, synchronization, and correct downmix or
surround output on your actual setup. H.264/AC-3 trials permit one 1–6 channel
track at 32, 44.1, or 48 kHz; HEVC/AC-3 is not enabled. Dolby audio output depends
on the receiver and connected equipment. It does not enable E-AC-3, MKV, known
HDR signalling, unknown required metadata, or multiple audio tracks. Omitting
the flag keeps the conservative policy. The reusable backend exposes the policy
and assessment independently of the CLI. See the [media inventory and hardware
results](docs/media-inventory.md) for current evidence.

The tested H.264/AC-3 sample played video but produced no audible sound on the
KPN DIW7022 setup. AC-3 remains an experimental trial, with no automatic fallback.

Embedded subtitle extraction and automatic conversion fallback are not implemented
yet. Explicit stream-copy preparation is available with `--remux`.
Subtitles are used only when explicitly selected; source
files are never modified. SRT conversion has a 30-second limit, and temporary
files are session-owned. WebVTT cue timing is validated; styling semantics are
left to the receiver. Track activation and HTTP requests do not prove visual
rendering, which requires the hardware check.

Override executables with `cast --ffprobe PATH --ffmpeg PATH`; use
`--probe-timeout SECONDS` to change the probe limit.

### Force transcoding

```sh
cargo run --locked -- cast /path/to/movie.mkv --device "Living Room" \
  --force-transcode --subtitles /path/to/movie.srt --http-port 8010
```

`--force-transcode` re-encodes the video and any audio even if the original file
could play directly. It prepares a complete MP4 before connecting to the receiver:
H.264 High/level 4.1, 8-bit 4:2:0, at most 1920×1080 and 30 fps, and stereo AAC
at 48 kHz. Smaller inputs are not deliberately enlarged; scaling preserves aspect
ratio. Audio-free inputs remain silent. Encoding uses the CPU (`libx264`,
`veryfast`, CRF 20, maximum video rate 8 Mbps; AAC 192 kbps).

Preparation shows percentage progress and remains cancellable with Ctrl+C or
SIGTERM. The source is never overwritten. Output is probed and checked before
serving, with metadata at the front of the MP4 for HTTP playback. Playback cannot
start until preparation finishes; quality loss and extra disk space are expected.

Private session directories default to `$XDG_CACHE_HOME/procast`, falling back to
`$HOME/.cache/procast`. Use `--transcode-dir /path/to/disk/cache` to choose another
parent. Space is estimated before encoding and checked during progress updates;
this is not a disk reservation. Avoid RAM-backed `/tmp` for long videos. Outputs
are deleted on normal completion, errors, or handled cancellation; a machine
crash or SIGKILL can leave a `session-*` directory to remove after Procast stops.
Outputs are not reused across sessions. The encoding deadline is 24 hours,
independent of the short probe timeout.

Initial limits: one video and at most one audio track, known duration/frame rate,
and SDR input. Known PQ/HLG and Dolby Vision signalling is rejected because tone
mapping is not implemented; missing HDR metadata cannot be detected reliably.
External SRT/WebVTT keeps the existing path; embedded subtitles are not selected
or burned in. `--force-transcode` and `--experimental-direct-play` are mutually
exclusive. Hardware acceleration and on-the-fly transcoding are deferred.

See [forced-transcoding validation](docs/transcode-validation.md). The user has
confirmed real-file picture/audio playback and seeking, pause/resume, and stop
from a phone, plus short-fixture SRT/WebVTT captions and natural completion.
Temporary-file/server cleanup also passed after natural completion and phone stop.
Improved phone-stop reporting also passed its hardware retest with exit code 0.
Detailed real-file synchronization remains to be checked.

### Convert only the audio

For H.264 MP4 whose picture works but audio does not, preserve the encoded video
and convert the single audio track to stereo AAC:

```sh
cargo run --locked -- cast /path/to/movie.mp4 --device "Living Room" \
  --transcode-audio --http-port 8010
```

This prepares a complete MP4 with copied video and 192 kbps, 48 kHz stereo AAC.
It avoids video encoding and its quality loss, but still reads and writes the
whole file before playback. Video must already meet the conservative H.264
profile above; HEVC, MKV, known HDR, missing audio, and multiple audio tracks
are rejected in this initial version. FFmpeg needs an input audio decoder and
the AAC encoder; this mode does not need a video encoder.

External `--subtitles`, progress, cancellation, `--transcode-dir`, output
validation, and session cleanup use the same preparation path described above.
The disk estimate includes the source size plus new audio and muxing overhead.
`--remux`, `--transcode-audio`, `--force-transcode`, and
`--experimental-direct-play` are mutually exclusive. Without these flags the conservative direct-play policy
remains unchanged; there is no automatic conversion fallback.

### Remux MKV to MP4 without encoding

```sh
cargo run --locked -- cast /path/to/movie.mkv --device "Living Room" \
  --remux --http-port 8010
```

`--remux` copies the selected encoded video and audio into a complete, seekable
MP4 before connecting. There is no audio/video re-encoding or associated quality
loss. The initial profile accepts Matroska/MKV or MP4 with one H.264 video stream
within the conservative limits above, and zero or one mono/stereo AAC-LC track.
HEVC, surround AAC, Dolby audio, known HDR, and multiple audio tracks are rejected
for this mode. An incompatible codec needs a different path; changing containers
alone cannot fix it. FFmpeg is required but no audio/video encoders are needed.

Embedded subtitles, attachments, chapters, and source metadata are omitted.
External SRT/WebVTT still works with `--subtitles`; embedded subtitle extraction
remains deferred. Preparation reads and writes the entire file, reports progress,
and shares the existing cancellation and cleanup behavior. `--transcode-dir`
chooses the preparation directory for remuxing too. Disk checks estimate source
size plus overhead and headroom. Default direct play remains unchanged.

The first hardware trial with `sample-006` succeeded; see
[remux validation and remaining checks](docs/remux-validation.md).

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
Procast consumes terminal broadcasts alongside status polls, so a one-time
FINISHED notification completes the session even if later polls would be empty.
An owned CANCELLED status or confirmed termination of the receiver application
ends the session normally with `Playback stopped on receiver.`; Procast does not
send a redundant STOP. An app-channel close or failed media request is checked
against receiver status, so transport loss alone is not reported as a normal stop.
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

## Short playback fixture

For a quick natural-completion check, generate a separate 30-second moving test
pattern with silent audio and matching WebVTT/SRT cues every five seconds:

```sh
bash scripts/generate-m1-fixture.sh samples/short 30
cargo run --locked -- cast samples/short/test.mp4 \
  --device "Living Room" --subtitles samples/short/subtitles.vtt --http-port 8010
```

Replace `Living Room` with your selected receiver. Use `subtitles.srt` to check
SRT conversion. Let playback finish without interrupting it; the CLI should
report `Playback completed.` and exit. The original 12-minute fixture is retained.

For a separate SIGTERM check, start casting again. While it plays, use another
terminal to run `pgrep -a -x procast`, identify that casting process, and run
`kill -TERM PID` with its numeric PID. SIGTERM is a normal termination request;
Ctrl+C sends SIGINT instead. Both should stop the owned playback and clean up.
After SIGTERM, `echo $?` in the casting terminal should report 143. Do not use
`kill -9`, which prevents cleanup.

## Cast feasibility example (M1)

Generate a silent 12-minute H.264/AAC MP4 and external WebVTT/SRT cues. This requires
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
