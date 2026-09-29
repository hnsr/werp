# Development

Build, local installation, contributor workflows and validation live here.
For normal usage, see [README](README.md). Run commands from the repository root.

- [Setup and builds](#setup-and-builds)
- [Local KDE installation](#local-kde-installation)
- [Development launch options](#development-launch-options)
- [Automated checks](#automated-checks)
- [Hardware checks](#hardware-checks)
- [Contributor guidelines](#contributor-guidelines)

## Setup and builds

- Rust via rustup; `rust-toolchain.toml` pins Rust 1.96.0 with rustfmt and Clippy
  for the Rust 2024 workspace.
- A native linker/C compiler, and network access for the initial Cargo download.
- `ffprobe` on PATH. `ffmpeg` is needed for text subtitle conversion/extraction
  (except external WebVTT), remuxing and encoding on a cache miss. Full encoding
  needs libx264/AAC plus input decoders. Codec availability depends on the build.
- Linux `systemd-inhibit` for best-effort sleep prevention; Yeet warns and continues
  if it is unavailable or denied.

Fedora 44 with FFmpeg/ffprobe 8.1.2 is the recorded development baseline. The CLI
needs no Qt, FFmpeg development headers or Python runtime. `Cargo.lock` records
Rust dependencies; release packaging/clean-system validation remains open. These
versions record the development baseline, not support for every Fedora/codec build.

### Rust CLI and backend

```sh
cargo build --locked --workspace
cargo run --locked -- --help
cargo run --locked -- inspect /path/to/video.mkv --json
cargo build --release --locked
./target/release/yeet --help
```

### KDE frontend

Fedora development packages: `gcc-c++ cmake ninja-build extra-cmake-modules
qt6-qtbase-devel kf6-kcoreaddons-devel kf6-ki18n-devel`. CMake requires Qt >= 6.6,
KDE Frameworks >= 6 and C++17. From the repository root:

```sh
cmake -S apps/yeet-kde -B target/kde -G Ninja -DCMAKE_BUILD_TYPE=Debug
cmake --build target/kde
./target/kde/yeet-kde /path/to/video.mkv --http-port 8010
```

CMake builds/copies the Rust helper beside the UI; rebuild after Rust or C++ changes.
The Rust CLI has no Qt dependency; Python is used only by the native test fixture.

## Local KDE installation

For local installation, configure `-DCMAKE_INSTALL_PREFIX="$HOME/.local"`, rebuild
and run `cmake --install target/kde`. CMake installs the UI in `bin`, the helper in
the KDE libexec directory, and both desktop entries. Ensure the install's `bin` is
in the desktop session's PATH. This does not change the default video player.
RPM packaging/clean-system validation is still open.

### Register the development build with KDE

The development setup uses user-local desktop entries pointing at the checkout.
To register or repair them after moving it, run from the repository root:

```sh
desktop-file-install --dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications" \
  --set-key=Exec --set-value="\"$PWD/target/kde/yeet-kde\" --http-port 8010 %f" \
  apps/yeet-kde/org.yeet.Yeet.desktop
desktop-file-install --dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications" \
  --set-key=Exec --set-value="\"$PWD/target/kde/yeet-kde\" --convert-only %f" \
  apps/yeet-kde/org.yeet.Yeet.ConvertOnly.desktop
update-desktop-database "${XDG_DATA_HOME:-$HOME/.local/share}/applications"
kbuildsycoca6
```

The entries advertise the MIME types in the committed desktop files, including
MP4 and MKV. An Open With association is not a codec-compatibility guarantee.

## Development launch options

The GUI accepts `--backend PATH` to select a helper executable and
`--no-discovery` to skip the initial device scan in either window; Refresh still
works. These are development/testing flags, separate from the
[normal GUI options](docs/kde-ui.md#launch-options).

`yeet-backend` is normally launched by the GUI, not managed as a separate app.
For integration/testing it accepts `--http-port PORT` (default `0`, OS-assigned),
`--ffprobe PATH`, `--ffmpeg PATH` and `--no-inhibit-sleep`. GUI launch options are
not arbitrary helper/CLI pass-through arguments. Frontend implementations use the
[private protocol](docs/backend-protocol.md).

## Automated checks

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

### GUI tests

The CTest target runs offscreen with Fusion; focused native tests have also passed with KDE Breeze. Set
`YEET_UI_SCREENSHOTS` to save test screenshots. Native tests use a Python mock
helper, plus a read-only inspection check through the real Rust helper.

Coverage includes explicit start, preview/device persistence, subtitle choices,
resume, Space, drag-and-drop, preparation, controls, stale events, cancellation,
helper exit, format rows and auto-close. Rust tests exercise protocol framing,
version rejection, bounded requests, seek/ownership/lifecycle behavior and real
FFmpeg conversion/cache fixtures. Automated tests never contact TVs.

### Subtitle and resume tests

Local tests cover TOML defaults/validation/overrides, language ranking and explicit
selection, exact sidecar lookup, UTF-16, device preferences/errors, cancellation,
state identity/locking, restart, disabled resume, and normal completion. Complete
simulated sessions verify saved offsets in subsequent Cast LOAD messages after
cancellation, failed LOAD, and disconnect. Real FFmpeg tests exercise generated
SubRip/MP4 text/ASS tracks and preserve cue times. A local PGS sample was prepared
and a rendered frame visually confirmed its caption; this is not a TV result.

## Hardware checks

Use an explicitly chosen receiver: casting replaces its current playback.
Confirm picture, sound and captions on the TV; protocol success is not a visual
or audio pass. The [device database](crates/yeet-core/data/devices.toml) records
observed support and limits. The complete GUI checklist and expanded-path
long-duration/seek validation remain outstanding.

| Check | Instructions |
| --- | --- |
| Completion, SRT/WebVTT, signal cleanup | [CLI cleanup](#cli-cleanup-check) |
| Preparation matrix and reuse | [Conversion batch](#conversion-batch) |
| Language selection, delay and resume | [Subtitles and resume](#subtitles-and-resume) |
| GUI player and converter | [GUI playback](#gui-playback), [convert-only](#convert-only) |
| Transport diagnosis | [Feasibility probe](#feasibility-probe) |

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

### Conversion batch

```sh
bash scripts/validate-m5.sh "Living Room"
```

The optional second argument sets the HTTP port (default 8010). This script
requires the ignored neutral library aliases and a receiver supporting the
Extended profile. It builds the CLI, creates short clips under `samples/m5-batch.*`,
and checks playback, external captions, completion, reuse and cancellation.
Follow its prompts and inspect the TV. `--prepare-only` generates fixtures
without casting; it does not establish hardware compatibility.

### Subtitles and resume

Generate a short, audible fixture without contacting a TV:

```sh
bash scripts/generate-subtitle-fixture.sh samples/preferences 60
```

The fixture includes full English at stream 3, Dutch at stream 4, and deliberately
default/forced English at stream 2. It also creates an exact-name external SRT
case and an alternate Dutch-first config. Run these checks when ready, substituting
a selected receiver where an explicit device is needed:

1. `cargo run --locked -- samples/preferences/embedded.mkv`: the configured
   preferred receiver should play full English captions, not the forced track.
   Interrupt after at least 20 seconds, then repeat the same command. Confirm the
   resume notice and TV starting near that point with correctly timed captions.
   Let it finish; the next run should start at zero.
2. Add `--subtitle-track 4 --restart`: Dutch captions should appear from the start.
   The generated `samples/preferences/dutch.toml` can also be selected with
   `--config ... --device "Living Room"` to test automatic Dutch preference.
3. Cast `samples/preferences/external.mp4 --no-resume`: the same-name external SRT
   should be selected automatically. Repeat with `--no-subtitles` to confirm none.
4. Optional image-caption check on the local prepared fixture:
   `cargo run --locked -- samples/subtitle-check/embedded-pgs-cues.mkv --subtitle-track 4 --no-resume`.
   The CLI should select burn-in and reuse its prepared output; confirm visible
   captions during the short clip. This fixture is local-only.

Use `--subtitle-delay-ms 1500` and then `-1500` for a signed-delay regression;
text/image delays have local test coverage, not a complete hardware sync report.
Long-duration and expanded-path seeking remain deferred. This checklist uses
Ctrl+C for interruption; the GUI provides controls for later seek checks.

### Convert-only

Open a video through Dolphin's convert-only entry. Confirm no work starts before
Convert, the last-used device is selected if available, and switching to Broad
compatibility updates the preview. Convert and check the result path; reopen the
original to verify reuse. Cancel a second uncached conversion and confirm Close
appears after cleanup. Auto-close depends on the
[GUI preference](docs/kde-ui.md#kde-frontend-preferences). Device-targeted conversion/cache sharing and discovery-failure fallback have automated coverage;
subjective dialogue quality still requires listening.

### GUI playback

1. Generate `samples/preferences/embedded.mkv` with
   `bash scripts/generate-subtitle-fixture.sh samples/preferences 60`. Open it;
   confirm English and the last-used eligible TV are selected, without playback.
   Press **Yeet** and confirm picture, sound and captions.
2. Pause/resume with buttons and Space. Seek both directions, including while
   paused; confirm caption/audio sync. Longer copied-HEVC checks are separate.
3. Stop after at least 20 seconds; confirm the TV stops and choices return.
   Try **Yeet from last position**, then **Yeet** to verify resume versus restart.
4. Choose an external subtitle and test positive/negative delay. Close during
   playback and confirm owned playback/helper resources stop.
5. With an uncached input requiring conversion (or a selected PGS track), confirm
   progress before playback. Cancel a fresh preparation and check no partial
   output remains. The local PGS fixture is not shipped; see [subtitle checks](#subtitles-and-resume).
6. Reopen, confirm device persistence, and drop a local video onto the idle player.
   It should refresh choices without starting playback.

### Feasibility probe

The original feasibility example is retained for transport diagnosis. It requires
an already compatible MP4/WebVTT; prefer `yeet FILE` for normal use.

```sh
bash scripts/generate-m1-fixture.sh
cargo run --locked -p yeet-core --example cast_probe -- --discover
cargo run --locked -p yeet-core --example cast_probe -- \
  --device "Living Room" \
  --video samples/m1/test.mp4 --subtitles samples/m1/subtitles.vtt \
  --http-port 8010 --seconds 620 --exercise-controls
```

The generator creates an ignored 12-minute silent H.264/AAC clip and numbered
captions. Choose an intended receiver: the example replaces its playback.
`--exercise-controls` pauses at 20 seconds, resumes at 25, seeks forward to 90
at 40, then back to 20 at 60. Ctrl+C/SIGTERM triggers cleanup; this prototype
counts cancellation as test success, unlike the CLI's 130/143 contract.
The host must stay awake and allow the chosen serving port. No firewall rule is
changed automatically. Generated media stays outside Git.

## Contributor guidelines

Use focused tests for media selection, subprocesses, HTTP ranges/CORS, Cast
ownership, IPC, resume/cache identity and UI lifecycle. Opt-in FFmpeg tests use
small generated fixtures. Automated tests use loopback receivers and must not
contact TVs. Hardware runs are explicit and record what the user saw/heard,
separately from protocol/log results. Do not maintain rolling test counts in docs.

Add CI when repository hosting and supported build environments are settled.

### Implementation constraints

- Do not modify original files. Publish completed output without replacing
  unrelated files; remove partial work on handled cancellation/errors.
- Invoke tools with argument arrays, drain bounded diagnostics, and reap children.
  Cancellation means cancel **and await** the operation, not just drop its future.
- Serve only registered media/subtitle URLs, with byte ranges and subtitle CORS.
  Choose the local address using the receiver route; never change firewall rules.
- Stop only Yeet's owned media. A takeover ends ownership; a network failure must
  still release local resources even if remote STOP fails.
- Use a trusted LAN: Cast TLS currently does not authenticate receiver identity,
  and media is served over HTTP. See the transport decision for limits.
- Reject known HDR, ambiguous audio/video tracks and missing required metadata
  rather than guessing a destructive conversion. No HDR tone mapping yet.
- Keep personal device names, IDs, addresses, source filenames and symlink targets
  out of commits. Public model identifiers and neutral sample IDs are suitable.

Output-affecting conversion changes must update the relevant cache recipe version.
When upgrading oxicast, recheck that transport disconnection cannot stop a foreign
session; see the [transport decision](docs/decisions/001-cast-library.md).

### Device database maintenance

Edit `crates/yeet-core/data/devices.toml` and rebuild the app. Add canonical model
IDs, explicit aliases and bounded permissions using the
[device schema](docs/device-compatibility.md#observation-fields). New codec
capabilities require core support as well as a database edit.

Keep observations scoped to what was actually seen/heard. Local conversion or a
successful protocol exchange alone does not establish playback support. Tests do
not upgrade untested formats to hardware passes. Validate changes with:

```sh
cargo test --locked -p yeet-core --lib devices::tests
```

This checks schema, matching, override behavior and evidence links. Supporting
links may target README or documents under `docs/`, with optional heading fragments.

### Local samples and privacy

Keep private media, neutral symlinks and detailed local artifacts under ignored
`samples/` or outside the repository. Do not force-add symlinks: their targets
contain personal source paths. Only public model identifiers, neutral aliases and
technical metadata belong in committed reports; see the
[historical inventory](docs/media-inventory.md). Preserve separate original-file,
remux and encoding outcomes when recording results.
