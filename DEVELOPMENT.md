# Development

Build, local installation, contributor workflows and validation live here.
For normal usage, see [README](README.md). Run commands from the repository root.

- [Setup and builds](#setup-and-builds)
- [Local KDE installation](#local-kde-installation)
- [Development launch options](#development-launch-options)
- [Automated checks](#automated-checks)
- [Hardware validation policy](#hardware-validation-policy)
- [Contributor guidelines](#contributor-guidelines)

## Setup and builds

- Rust via rustup; `rust-toolchain.toml` pins Rust 1.96.0 with rustfmt and Clippy
  for the Rust 2024 workspace.
- A native linker/C compiler, and network access for the initial Cargo download.
- `ffprobe` on PATH. `ffmpeg` is needed for text subtitle conversion/extraction
  (except external WebVTT), remuxing and encoding on a cache miss. Full encoding
  needs libx264/AAC plus input decoders. Codec availability depends on the build.
- Linux `systemd-inhibit` for best-effort sleep prevention; Werp warns and continues
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
./target/release/werp --help
```

### KDE frontend

Fedora development packages: `gcc-c++ cmake ninja-build extra-cmake-modules
qt6-qtbase-devel kf6-kcoreaddons-devel kf6-ki18n-devel`. CMake requires Qt >= 6.6,
KDE Frameworks >= 6 and C++17. From the repository root:

```sh
cmake -S apps/werp-kde -B target/kde -G Ninja -DCMAKE_BUILD_TYPE=Debug
cmake --build target/kde
./target/kde/werp-kde /path/to/video.mkv --http-port 8010
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
  --set-key=Exec --set-value="\"$PWD/target/kde/werp-kde\" --http-port 8010 %f" \
  apps/werp-kde/org.werp.Werp.desktop
desktop-file-install --dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications" \
  --set-key=Exec --set-value="\"$PWD/target/kde/werp-kde\" --convert-only %f" \
  apps/werp-kde/org.werp.Werp.ConvertOnly.desktop
update-desktop-database "${XDG_DATA_HOME:-$HOME/.local/share}/applications"
kbuildsycoca6
```

The entries advertise the MIME types in the committed desktop files, including
MP4 and MKV. An Open With association is not a codec-compatibility guarantee.

## Development launch options

The CLI's `--force-direct` serves the original video without format compatibility
checks, media conversion or prepared-file reuse. Use it to trial formats outside
the device database, including HDR-tagged files; it does not change colour metadata
or guarantee correct rendering. File inspection, text subtitles, HTTP range
requests, playback controls and cleanup still apply. Image subtitle burn-in is
rejected; select a text track or use `--no-subtitles`.

```sh
werp /path/to/video.mp4 --force-direct --device "Living Room" --no-subtitles --no-resume
```

This flag conflicts with `--mode` and `--profile`, never falls back to conversion,
and is unavailable in the GUI or its backend protocol. Normal `--mode direct`
continues to enforce compatibility checks. Cache options have no effect on a
forced trial. Unknown containers are sent as `application/octet-stream`.

The GUI accepts `--backend PATH` to select a helper executable and
`--no-discovery` to skip the initial device scan in either window; Refresh still
works. These are development/testing flags, separate from the
[normal GUI options](docs/gui.md#launch-options).

`werp-backend` is normally launched by the GUI, not managed as a separate app.
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
`WERP_UI_SCREENSHOTS` to save test screenshots. Native tests use a Python mock
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

## Hardware validation policy

The previous manual acceptance checklists are retired. Future playback problems
will be handled as bugs with targeted reproduction. Existing device observations
retain their original scope; retiring a check does not mark it as passed.

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
- Stop only Werp's owned media. A takeover ends ownership; a network failure must
  still release local resources even if remote STOP fails.
- Use a trusted LAN: Cast TLS currently does not authenticate receiver identity,
  and media is served over HTTP. See the transport decision for limits.
- Reject Dolby Vision, ambiguous audio/video tracks and missing required metadata.
  PQ/HLG-tagged video encoding is experimental and uses the ordinary 8-bit H.264
  pipeline; no tone mapping or guaranteed HDR preservation is implemented.
- Keep personal device names, IDs, addresses, source filenames and symlink targets
  out of commits. Public model identifiers and neutral sample IDs are suitable.

Output-affecting conversion changes must update the relevant cache recipe version.
When upgrading oxicast, recheck that transport disconnection cannot stop a foreign
session; see the [transport decision](docs/decisions/001-cast-library.md).

### Device database maintenance

Edit `crates/werp-core/data/devices.toml` and rebuild the app. Add canonical model
IDs, explicit aliases and bounded permissions using the
[device schema](docs/device-compatibility.md#observation-fields). New codec
capabilities require core support as well as a database edit.

Keep observations scoped to what was actually seen/heard. Local conversion or a
successful protocol exchange alone does not establish playback support. Tests do
not upgrade untested formats to hardware passes. Validate changes with:

```sh
cargo test --locked -p werp-core --lib devices::tests
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
