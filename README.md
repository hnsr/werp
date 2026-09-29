# Yeet

Cast local videos and subtitles to a Chromecast from a Rust CLI or native KDE app.
Yeet chooses direct play, MP4 remuxing, audio-only conversion or full SDR conversion
and reuses validated prepared files. Conversion finishes before playback starts.
For MKV files with compatible video/audio, remuxing copies the streams into MP4
without re-encoding or quality loss. H.264/AAC and device-permitted HEVC/surround
are supported; subtitles are handled separately. See [media conversion](docs/media-conversion.md)
for selection, encoding and reuse.

The KDE player provides device/subtitle selection, preparation progress, resume,
pause/play/seek/stop and Dolphin **Open With** integration. Convert-only previews
output for a selected device without starting playback. Recorded hardware coverage
centres on **KPN DIW7022**; other models use conservative defaults unless configured.

See the [project direction and roadmap](docs/plan.md) for decisions and open work,
and the [device database](docs/device-compatibility.md) for verified support.

## Development setup

- Rust via rustup; `rust-toolchain.toml` pins Rust 1.96.0 with rustfmt and Clippy.
- A native linker/C compiler, and network access for the initial Cargo download.
- `ffprobe` on PATH. `ffmpeg` is needed for text subtitle conversion/extraction
  (except external WebVTT), remuxing and encoding on a cache miss. Full encoding
  needs libx264/AAC plus input decoders. Codec availability depends on the build.
- Linux `systemd-inhibit` for best-effort sleep prevention; Yeet warns and continues
  if it is unavailable or denied.

Fedora 44 with FFmpeg/ffprobe 8.1.2 is the recorded development baseline. The CLI
needs no Qt, FFmpeg development headers or Python runtime. `Cargo.lock` records
Rust dependencies; release packaging/clean-system validation remains open.

## KDE application

Install the development packages `gcc-c++ cmake ninja-build extra-cmake-modules
qt6-qtbase-devel kf6-kcoreaddons-devel kf6-ki18n-devel`, then:

```sh
cmake -S apps/yeet-kde -B target/kde -G Ninja -DCMAKE_BUILD_TYPE=Debug
cmake --build target/kde
./target/kde/yeet-kde /path/to/video.mkv
./target/kde/yeet-kde --convert-only /path/to/video.mkv
```

The app launches its Rust helper automatically. The player preselects the last-used
device and suggests English, then Dutch subtitles. Review choices and press
**Yeet** or **Yeet from last position**. Space toggles pause/play; Ctrl+Q closes
with cleanup. Opening/dropping a file does not start playback.

Convert-only lets you choose a discovered device (using its model rules and local
overrides) or **Broad compatibility** for conservative H.264/stereo AAC output.
Review the source/target preview and press **Convert**. Successful output is
retained; auto-close defaults to five seconds and is configurable in the GUI INI.

See the [KDE guide](docs/kde-ui.md) for installation, file associations, settings
and manual checks, and the [private protocol](docs/backend-protocol.md) for frontend development.

## CLI

```sh
cargo run --locked -- devices
cargo run --locked -- inspect /path/to/video.mkv --json
cargo run --locked -- /path/to/video.mkv --device "Living Room"
cargo run --locked -- /path/to/video.mkv --device "Living Room" --subtitles /path/to/captions.srt
cargo build --release --locked
./target/release/yeet --help
```

`yeet FILE` stays in the foreground while serving. It replaces playback on the
selected receiver. Ctrl+C/SIGTERM cleans up owned playback, child processes,
HTTP serving and temporary files; successful reusable outputs remain. Another
sender's takeover ends Yeet's ownership without stopping that sender's media.
Interactive terminal playback controls remain parked; KDE has controls.

`--device` accepts an exact friendly name or ID; duplicate names require an ID.
Without it, CLI preferences apply, then the sole eligible video receiver is chosen.
`--host IP` bypasses IPv4 discovery and model detection. Use `--help` for diagnostic
options, or a path such as `./devices` for filenames that match subcommands.

| Options | Purpose |
| --- | --- |
| `--mode auto/direct/remux/audio/transcode` | Choose automatically (default), or require a preparation path |
| `--profile auto/baseline/extended/experimental` | Model rules (default), conservative support, bounded HEVC/AAC surround, or explicit AC-3 trials |
| `--subtitles FILE`, `--subtitle-track INDEX`, `--no-subtitles` | Override automatic subtitle selection |
| `--subtitle-delay-ms N` | Signed delay: positive is later, negative earlier |
| `--restart`, `--resume`, `--no-resume` | Control starting position/checkpoint behavior |
| `--cache-dir PATH`, `--no-cache` | Override storage or disable output reuse/retention |
| `--config PATH`, `--no-config` | Override CLI preferences, or ignore preferences and local model overrides |

CLI preferences live in `~/.config/yeet/config.toml`: automatic subtitles
(English then Dutch), preferred devices and automatic resume. The GUI does not
apply these CLI settings. Device rules live separately in `devices.toml`; GUI
preferences use `kde-ui.ini`. XDG locations are supported. See
[configuration and resume](docs/preferences-and-subtitles.md) and
[device overrides](docs/device-compatibility.md#user-overrides).

Prepared MP4s and completion sidecars live beside the canonical source, falling
back to the user cache if unwritable. Originals are never changed. Reuse validates
source/output fingerprints and the recipe; this costs disk reads but avoids
encoding. There is no automatic eviction. See [storage and reuse](docs/media-conversion.md#storage-and-reuse).

## Limits and networking

Known HDR/Dolby Vision, multiple tracks requiring explicit audio/video selection,
and missing required metadata remain errors. Live encoding, hardware acceleration,
HDR tone mapping and broader platform support are deferred. Subtitle format scope
is accepted for the sample set; advanced ASS styling and external bitmap files
remain limited. See [subtitles](docs/preferences-and-subtitles.md).

The host must stay awake and be reachable from the receiver. Yeet chooses the
local address from the receiver route and an OS-assigned HTTP
port. For an existing firewall rule, use `--http-port 8010` (CLI or GUI);
`--bind-address` is a CLI diagnostic override. Check firewall, VPN routes, mDNS and
Wi-Fi isolation if discovery or downloads fail. Yeet never edits firewall rules.

Only registered media/subtitle resources are served, with ranges and subtitle CORS.
Use a trusted LAN: Cast TLS does not authenticate receiver identity, and media
uses HTTP. Lost connections fail rather than automatically restarting playback.
See [transport rationale](docs/decisions/001-cast-library.md).

CLI results go to stdout; diagnostics go to stderr. Use `--verbose` or `RUST_LOG`
for debug output. Exit codes: 0 success, 1 operation failure, 2 argument error,
130 Ctrl+C and 143 SIGTERM on Linux. Inspection JSON is Yeet metadata, not raw
ffprobe output or a stable public API.

## Sleep prevention

On Linux, Yeet uses `systemd-inhibit` to block sleep during casting and conversion,
including preparation, paused playback and cleanup. Merely opening the GUI or
browsing devices does not acquire a lock. Screen dimming/locking remains enabled.
CLI/helper `--no-inhibit-sleep` opts out. Acquisition failures warn and let work
continue; forced sleep or power-policy overrides may still interrupt playback.
This small adapter avoids desktop bindings; broader platform support is deferred.

Locally verified on Fedora KDE: the lock appeared in logind and KDE's active
inhibitions, and disappeared after SIGINT cleanup. Automated tests cover helper
failure, cancellation, release and signal cleanup. Actual suspend and long-video
validation remain outstanding; standby was only a suspected cause of an earlier
interruption. To inspect an active lock, use the KDE power applet or
`systemd-inhibit --list --no-pager`; Yeet's entry should disappear after cleanup.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
# Explicit tests requiring installed FFmpeg/ffprobe and encoders:
cargo test --locked --workspace -- --ignored
# Build KDE first; native tests use a Python mock helper:
/usr/bin/ctest --test-dir target/kde --output-on-failure
```

Automated tests use generated media, fake subprocesses and loopback TLS receivers;
they never discover or control TVs. They need local sockets and process-signal
permissions. An explicitly requested FFmpeg test fails if its prerequisites are
missing. Hardware checks are separate and require an intended receiver.

| Manual check | Guide |
| --- | --- |
| Short clip, SRT/WebVTT, SIGTERM cleanup | [CLI cleanup check](#cli-cleanup-check) |
| Automatic preparation matrix and reuse | [Batch script](scripts/validate-m5.sh) (requires local sample aliases; explicitly casts to the named device) |
| Subtitle selection, delay and resume | [Subtitle checklist](docs/preferences-and-subtitles.md#verification-and-tv-checklist) |
| KDE controls and convert-only | [KDE checklists](docs/kde-ui.md#convert-only-checks) |
| Original feasibility probe | [Transport decision](docs/decisions/001-cast-library.md#reproduction) |

Keep private media and local symlinks under ignored `samples/` or outside the
repository. The [historical inventory](docs/media-inventory.md) uses neutral aliases.

### CLI cleanup check

Generate a short fixture (existing output is never overwritten), then choose an
intended receiver:

```sh
bash scripts/generate-m1-fixture.sh samples/short 30
cargo run --locked -- samples/short/test.mp4 \
  --device "Living Room" --subtitles samples/short/subtitles.vtt --no-resume --http-port 8010
```

Repeat with `subtitles.srt`. Natural completion should exit 0; Ctrl+C should
finish cleanup and exit 130. For a separate SIGTERM run, find the casting PID
with `pgrep -a -x yeet` in another terminal and send `kill -TERM PID`; expect 143.
After shutdown, `ss -H -ltn 'sport = :8010'` should show no listener. Cancellation
can also be checked during loading. SIGKILL cannot run application cleanup.
