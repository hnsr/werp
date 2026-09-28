# Yeet

A Rust CLI, reusable backend, and native KDE frontend for casting local videos
and subtitles to Chromecast. `yeet FILE` selects direct play, remuxing, audio conversion, or full
conversion and reuses validated prepared files automatically. Automated checks
and the consolidated short-clip TV validation passed on the KPN DIW7022 receiver;
seeking and long-duration checks for the expanded paths are deferred.
Configuration, automatic embedded subtitles, preferred devices, and playback
resume are implemented. Automatic English embedded captions and interrupted
playback resume are confirmed on the TV, along with explicit Dutch selection,
restart, completion clearing the saved position, automatic external SRT loading,
subtitles off, and embedded PGS burn-in. Optional remaining checks are in the checklist.

## Development setup

- Rust via rustup. `rust-toolchain.toml` pins Rust 1.96.0 with rustfmt and Clippy;
  rustup installs the pinned toolchain when needed.
- A working native linker/C compiler (such as Fedora's GCC).
- `ffprobe` on PATH for inspection and casting. `ffmpeg` is required for SRT
  conversion, remuxing/transcoding on a cache miss, and test-fixture generation. Direct
  playback with existing WebVTT needs no FFmpeg. Forced transcoding requires
  `libx264` and AAC encoders plus a decoder for the input codecs.
- Network access for the initial Cargo dependency download.
- On Linux, `systemd-inhibit` for automatic sleep prevention during casting
  (already present on the Fedora KDE development machine). If unavailable or
  denied, Yeet warns and continues.

The CLI needs no FFmpeg development headers, Qt, or KDE libraries. Cargo resolves
the Rust dependencies; `Cargo.lock` records tested versions.
The development baseline is Fedora 44 with FFmpeg/ffprobe 8.1.2. Codec support
depends on the installed FFmpeg build, not just its version.

## KDE application

The C++/Qt 6 Widgets frontend starts its private Rust helper automatically.
Opening or dropping a local video preselects the last-used device when available
and suggests subtitles (English, then Dutch, then a matching SRT). These UI
defaults are independent of CLI settings and remain editable. Press **Yeet** to start
at zero or **Yeet from last position** to resume. A separate preparation screen
shows conversion/remuxing progress and cancellation before player controls appear.
Pause/play (also Space), seeking, and stop are available in the window.

```sh
cmake -S apps/yeet-kde -B target/kde -G Ninja -DCMAKE_BUILD_TYPE=Debug
cmake --build target/kde
./target/kde/yeet-kde samples/preferences/embedded.mkv --http-port 8010
```

The installed Fedora development packages are `gcc-c++ cmake ninja-build
extra-cmake-modules qt6-qtbase-devel kf6-kcoreaddons-devel kf6-ki18n-devel`.
See [build, installation, architecture, and TV checklist](docs/kde-ui.md) and the
[backend protocol](docs/backend-protocol.md). Automated frontend checks are local;
the new window's real-TV validation is pending.

Open a file with **Yeet (convert only)** to prepare a broadly compatible MP4
without starting playback, or run `yeet-kde --convert-only FILE`. Choose a discovered
video device to use its model rules and local overrides, or choose **Broad compatibility**
for conservative H.264/stereo AAC output. The last-used device is preselected when
available. Review the source/target preview, then click **Convert**. The separate
window shows progress and supports cancellation. Successful conversions
auto-close after five seconds by default; set `[conversion] autoClose=false` in
`~/.config/yeet/kde-ui.ini` to keep them open. Outputs use the shared reusable cache; see the [KDE guide](docs/kde-ui.md#convert-only-window).

## CLI

```sh
cargo run --locked -- devices
cargo run --locked -- devices --json
cargo run --locked -- /path/to/movie.mp4 --device "Living Room" --subtitles /path/to/movie.srt
cargo run --locked -- inspect /path/to/video.mkv
cargo run --locked -- inspect /path/to/video.mkv --json
cargo run --locked -- --verbose inspect /path/to/video.mkv
cargo run --locked -- inspect /path/to/video.mkv --ffprobe /usr/bin/ffprobe --timeout 60
```

`devices` scans for five seconds by default (`--scan-seconds 1..60`). Choose an
exact friendly name or device ID; duplicate names require an ID. Omitting
`--device` tries the configured preferred list, then selects automatically when
exactly one discovered receiver advertises video support. Known audio-only devices are rejected, and unknown
capabilities require explicit selection. `--host 192.168.1.50` bypasses discovery;
use `--cast-port` only with `--host`. Discovery and casting currently use IPv4.

`yeet FILE` stays in the foreground until natural completion or interruption. It
replaces playback on the selected receiver. Ctrl+C or SIGTERM stops Yeet's
owned media, closes its server and transport, reaps subprocesses, and removes
temporary media and subtitles. Completed reusable outputs are retained. If another sender takes over, Yeet exits without stopping
their session. Interactive terminal controls remain parked in M3; the KDE window has player controls.

Casting is the default action: `yeet FILE`. Use `yeet devices` to list receivers
and `yeet inspect FILE` to inspect media. For a file named `devices`, `inspect`,
or `help`, use a path such as `./devices` or put `--` before the filename.

### Preferences and resume

CLI automation preferences are read from `~/.config/yeet/config.toml` (or
`$XDG_CONFIG_HOME/yeet/config.toml`). See [the example](docs/config.example.toml)
and [configuration, subtitles, and resume](docs/preferences-and-subtitles.md).
Defaults enable automatic subtitles, prefer English then Dutch, and save playback
position. Configure `[cli.devices] preferred` with ordered exact names or stable IDs.
Explicit `--device`/`--host` overrides preferences. `--config PATH` selects another
file; `--no-config` uses built-in defaults. All automatic preferences are under
`[cli.devices]`, `[cli.subtitles]`, and `[cli.playback]`; the UI does not apply them.
Model capabilities can be adjusted or extended in `~/.config/yeet/devices.toml`;
see [user device overrides](docs/device-compatibility.md#user-overrides).
They apply to automatic casting in CLI and KDE. Explicit CLI profiles bypass
model rules; `--no-config` skips local overrides but retains the bundled database.
`--config PATH` changes only CLI preferences. Global compatibility flags are removed.
`[cli.playback] auto_resume = false` disables automatic resume while still saving
progress for explicit resume later.

An interrupted video resumes on its next cast, five seconds before the last
checkpoint. Normal completion clears the checkpoint. Use `--restart` to start
over while saving progress or `--no-resume` to disable reading/writing position
for a session. Positions live in the user's state directory, outside the media
library. This does not automatically reconnect a failed session.

### Automatic playback and overrides

```sh
yeet /path/to/movie.mkv --device "Living Room"
yeet /path/to/movie.mkv --device "Living Room" --subtitles /path/to/movie.srt
```

Yeet inspects the source, discovers/selects the receiver, and explains its
choice before preparing anything or launching the Cast application:

1. Serve the original MP4 if its streams fit the receiver profile.
2. Copy supported video/audio from MKV or MP4 into MP4 when only a remux is needed.
3. Copy supported video and encode incompatible audio to stereo AAC in MP4.
4. Encode unsupported SDR video to H.264 and audio to stereo AAC.

Selected image subtitles require burn-in and therefore full video conversion.

All preparation finishes before playback; seeking uses the existing HTTP range
server. Selection is based on metadata and a receiver profile. Yeet does not
blindly retry a failing cast or network connection with more encoding, and cannot
automatically detect silent audio or incorrect colours on the TV.

| Option | Meaning |
| --- | --- |
| `--mode auto` | Default: choose the least preparation needed |
| `--mode direct` | Require original-file playback; fail if the profile rejects it |
| `--mode remux` | Require stream-copy MP4 preparation; no encoding |
| `--mode audio` | Require copied video and stereo AAC audio conversion |
| `--mode transcode` | Require full SDR H.264/stereo AAC preparation |
| `--profile auto` | Default: bundled device database (KPN DIW7022 includes observed 1080p50/Level 4.2); Baseline for unknown models or `--host` |
| `--profile baseline` | MP4 H.264 up to 1080p30/level 4.1, 8-bit 4:2:0, optional mono/stereo AAC-LC |
| `--profile extended` | Also allow HEVC Main/Main 10 up to level 4.0/1080p30 and AAC-LC through six channels |
| `--profile experimental` | Additionally allow H.264/AC-3 trials; audible output is not guaranteed |
| `--no-cache` | Prepare afresh and delete the session output afterward |
| `--cache-dir PATH` | Store prepared files in this directory instead of beside the source |

Device rules and their evidence live in the bundled [compatibility database](docs/device-compatibility.md).
Extended support is based on observations on the development receiver, not full
capability negotiation or a guarantee for every file/device. Baseline can be
selected explicitly on a receiver whose audio or video support differs. Dolby
and HE-AAC audio convert to AAC-LC under the normal profiles. Full encoding uses
libx264 veryfast/CRF 20, up to 1080p30, and stereo AAC 192 kbps/48 kHz.

Original MP4 H.264 High Level 4.2 at 1080p50 with stereo AAC-LC also passed a
user playback check on the development KPN receiver and is now in its bundled policy.
See the [compatibility notes](docs/automatic-playback.md#h264-1080p50-direct-play-result);
59.94/60 fps remains unverified on hardware.

Known PQ/HLG/Dolby Vision, ambiguous multiple audio/video tracks, and missing
required duration/frame-rate information produce clear errors. HDR tone mapping,
hardware acceleration, on-the-fly conversion, and track selection remain deferred.
An unsupported audio decoder or required encoder produces an FFmpeg error; a
valid cache hit does not require FFmpeg for media preparation.

### Persistent prepared files

Prepared files are stored beside the **canonical original file**, following
symlinks. Names look like `movie.yeet-<12-hex-key-tag>-<8-hex-generation>.mp4`, with a small
`.mp4.json` completion record. Only registered media/subtitles are served; the
sidecar is not exposed. If the source directory is not writable, Yeet announces
a fallback to `$XDG_CACHE_HOME/yeet` or `$HOME/.cache/yeet`.
The full source stem is preserved unless it exceeds the filename budget; Unicode
characters are never split. Short tags are backed by full SHA-256 validation and
no-overwrite publication with collision retries. Older prepared filenames remain
reusable when the recipe is unchanged. The updated stereo downmix uses a new
audio recipe, so older audio encodes are retained but not reused. See [naming and collision handling](docs/automatic-playback.md#storage-and-reuse).

Reuse verifies a full SHA-256 fingerprint of the source, its canonical path,
the preparation mode/profile/recipe version, the output digest, and output media
metadata. This reads files from disk but avoids encoding. Changed or damaged files
are regenerated under new names without overwriting existing files. Subtitles
are prepared separately for text tracks, so changing an external text subtitle
does not invalidate video. Image-subtitle burn-in includes the selected track in
the cache recipe.

Incomplete session work is deleted on handled failures/cancellation. Validated
outputs survive playback completion, cancellation, or a receiver connection error.
Publication never replaces an existing file; concurrent first-time requests may
produce duplicate valid generations. There is no automatic eviction yet: obsolete
results, duplicates, or crash leftovers can be removed manually when not in use.
Delete a prepared MP4 together with its `.mp4.json` sidecar. Source files are never
modified. With `--no-cache`, the previous temporary-output cleanup behavior applies.

### Subtitles and batch validation

Automatic loading first selects an embedded track matching the ordered language
preferences, then falls back to an exact-name `.srt` beside the video. Explicit
`--subtitles FILE`, `--subtitle-track INDEX`, and `--no-subtitles` override this;
`--auto-subtitles` enables it when disabled in config. Use `inspect` for stream indexes.

Embedded text is extracted to WebVTT. External SRT/VTT/ASS/SSA accepts UTF-8 or
BOM-marked UTF-16. ASS/SSA conversion loses advanced styling and reports that
limitation. Embedded PGS/DVD/DVB images are burned into a fully converted video;
they cannot be toggled on the receiver. Text conversion/extraction needs FFmpeg;
an external WebVTT file does not. Unselected tracks, attachments, titles, and
chapters are omitted during preparation.

The new [preferences/subtitle/resume checklist](docs/preferences-and-subtitles.md#verification-and-tv-checklist)
uses a short generated fixture and does not require phone controls.

The earlier M5 batch remains available for regression checks:

```sh
bash scripts/validate-m5.sh "Living Room"
```

This creates short neutral clips under ignored `samples/`, then casts them in
sequence. It checks selected modes, natural completion, cache reuse without
FFmpeg, and cancellation/HTTP cleanup; it asks for picture, sound, timing, and
caption confirmation. It targets the development receiver's Extended profile.
Allow roughly 15–20 minutes. No hardware test runs during `cargo test`.
See [automatic playback and reuse validation](docs/automatic-playback.md).

### Network and session behaviour

The host must stay awake, and the receiver must be able to fetch files from it.
On Linux, `yeet FILE` holds a `systemd-inhibit` sleep lock during preparation, playback
(including pauses), and cleanup. KDE PowerDevil honours this lock. Screen dimming
and locking remain enabled. The lock is released when the session ends; acquisition
failure produces a warning rather than preventing playback. `--no-inhibit-sleep`
disables it for a session. This is a small initial Linux adapter; broader desktop
integration will be revisited with UI work. See [sleep inhibition](docs/sleep-inhibition.md)
for verification and limits.

Yeet chooses a local address using the route to the receiver and an OS-assigned
HTTP port. `--bind-address LOCAL_IP --http-port 8010` provides a fixed interface
and port for troubleshooting. Check firewall rules, VPN routing, mDNS, and Wi-Fi
client isolation if discovery or downloads fail. Yeet does not edit firewall
rules. Only the selected video and optional prepared subtitles are served, under
random session URLs, with range requests and subtitle CORS support.

Cast connections use TLS without receiver identity verification, and media is
served over HTTP. Use a trusted LAN; see the [transport decision](docs/decisions/001-cast-library.md).
Yeet consumes terminal broadcasts alongside status polls, so a one-time
FINISHED notification completes the session even if later polls would be empty.
An owned CANCELLED status or confirmed termination of the receiver application
ends the session normally with `Playback stopped on receiver.`; Yeet does not
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
JSON is normalized Yeet metadata, not raw ffprobe output; its schema is not
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
./target/release/yeet inspect /path/to/video.mkv
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
cargo run --locked -- samples/short/test.mp4 \
  --device "Living Room" --subtitles samples/short/subtitles.vtt --http-port 8010
```

Replace `Living Room` with your selected receiver. Use `subtitles.srt` to check
SRT conversion. Let playback finish without interrupting it; the CLI should
report `Playback completed.` and exit. The original 12-minute fixture is retained.

For a separate SIGTERM check, start casting again. While it plays, use another
terminal to run `pgrep -a -x yeet`, identify that casting process, and run
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
cargo run --locked -p yeet-core --example cast_probe -- --discover
cargo run --locked -p yeet-core --example cast_probe -- \
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
The receiver must be able to reach this port. Yeet does not edit firewall rules.
Only the two selected resources are served, under random session URLs.

This prototype uses IPv4 discovery, requires an already compatible MP4 and valid
WebVTT, and does not inspect/convert the inputs or extract embedded subtitles.
The host must remain awake. It uses encrypted Cast transport without device
authentication, for use on a trusted LAN; see the [M1 decision](docs/decisions/001-cast-library.md).

Prefer the production `yeet devices` / `yeet` commands for normal use.

Keep personal sample videos outside the repository or under the ignored
`samples/` directory.

See [the outline](docs/outline.md) and [implementation plan](docs/plan.md).
