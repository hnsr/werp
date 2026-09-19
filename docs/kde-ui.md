# KDE frontend and backend interface

The first implementation is available in `apps/yeet-kde` and
`crates/yeet-backend`. Local automated checks cover the native window, protocol,
and controlled sessions. The user confirmed the initial frontend test worked;
the full TV checklist remains pending.

## Design

The KDE application is C++ with Qt 6 Widgets and KDE Frameworks 6. It starts a
private Rust helper automatically through QProcess. The user opens one app;
there is no separately managed service. Future GTK or other native frontends
can implement the same versioned message protocol without Rust bindings.

`yeet-core` owns probing, discovery, subtitle enumeration/preparation, compatibility,
conversion/cache, Cast sessions, controls, sleep inhibition, and checkpoint storage.
The CLI owns automatic device/subtitle selection and automatic resume. The UI
makes explicit choices; opening a file never launches playback or conversion.

CLI configuration uses `[cli.devices]`, `[cli.subtitles]`, and `[cli.playback]`.
The last uses `auto_resume`; saving checkpoints is an independent shared capability.
No compatibility aliases are required. Existing personal config is updated locally.
The helper does not load CLI configuration.

## Interaction

On startup, inspect the file and discover receivers asynchronously. Show a device
selector with no automatic selection, a single subtitle list containing None,
embedded tracks, and exact-basename external SRT, WebVTT, ASS, and SSA files. A separate file picker
starts beside the source and can choose supported external subtitle files.

Yeet starts at zero. Yeet from last position appears when a usable checkpoint
exists. Both require an explicitly selected receiver. There are three separate screens:

1. **Selection:** choose the receiver, subtitle track/file, and starting position.
2. **Preparation:** show conversion/remuxing progress and Cancel. While probing,
   preparing subtitles, connecting, or loading, show an indeterminate indicator.
   Direct playback and cache hits pass through this screen briefly.
3. **Playing:** show pause/play, stop, position, and a seek slider.

No conversion starts before pressing a Yeet button. Progress is a structured
fraction from the backend; the frontend does not parse FFmpeg or CLI output.
Disable subtitle selection during preparation and playback. Stop waits for cleanup
and returns to startup choices. The helper stays available for another session.
Closing the UI or losing its input pipe cancels work, releases resources, and
exits the helper. Helper failure must be visible in the UI.

## Transport contract

Use UTF-8 newline-delimited JSON over private stdin/stdout pipes. Logs go to stderr.
Every request has an integer `id` and a `method`, with typed `params` as needed.
Responses echo the id and carry either a result or a structured error. Progress
is an asynchronous event with a session identifier, phase, position, and message.
A protocol-version handshake rejects incompatible versions. Messages and queues
are bounded. No shell command construction or parsing of CLI prose is involved.

Operations: handshake, inspect (metadata/subtitles/checkpoint), discover, start
(explicit file/device/subtitle/position), pause, play, seek, stop, and shutdown.
Only one playback session runs per helper. Commands for expired sessions must not
affect a later session. Queries never trigger playback. Seek validation and
receiver ownership checks belong in the backend. Lifecycle acknowledgements and
session events distinguish a request being accepted from playback actually starting.

## Implementation and validation

1. Separate CLI policy/configuration from explicit backend requests; preserve CLI
   defaults and expose read-only checkpoint lookup and subtitle enumeration.
2. Add a controllable core session and the private Rust helper. Exercise the
   protocol and lifecycle against simulated receivers, including cleanup on EOF.
3. Add the Qt/KDE window, native file dialogs, and desktop Open With entry.
4. Build on Fedora, exercise the GUI locally without contacting a receiver, and
   provide a short user-run TV checklist. Keep original private names out of Git.

Development packages on Fedora: `gcc-c++ cmake ninja-build extra-cmake-modules
qt6-qtbase-devel kf6-kcoreaddons-devel kf6-ki18n-devel`.

References: [QProcess](https://doc.qt.io/qt-6/qprocess.html),
[Qt Widgets](https://doc.qt.io/qt-6/qtwidgets-index.html).

## Build and launch

From the repository root:

```sh
cmake -S apps/yeet-kde -B target/kde -G Ninja -DCMAKE_BUILD_TYPE=Debug
cmake --build target/kde
./target/kde/yeet-kde samples/preferences/embedded.mkv --http-port 8010
```

CMake also builds and copies the Rust helper beside the frontend. Re-run the build
after either Rust or C++ changes. Launch without a file to use the native picker.
The `--http-port 8010` override uses a fixed port for an existing firewall rule;
omit it to let the OS choose a port. No firewall rules are changed automatically.
The Rust CLI remains `yeet FILE` and has no Qt dependency. `yeet-kde FILE` is the UI.
There is no Python runtime dependency for either application; the Qt tests use a
small Python mock helper.

For an optional local install, configure with `-DCMAKE_INSTALL_PREFIX="$HOME/.local"`
and run `cmake --install target/kde`. This installs the frontend under `bin`, the
helper under the KDE libexec directory (`lib64/libexec/yeet` on this Fedora
build), and an application/Open With desktop entry. Ensure the
install's `bin` directory is in the desktop session's PATH. Installation does not
change the default association for video files.

The [version 1 protocol reference](backend-protocol.md) defines the frontend boundary.
Developer overrides `--backend PATH` and `--no-discovery` allow offline UI checks.

## Automated checks

```sh
cargo fmt --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace -- --include-ignored
/usr/bin/ctest --test-dir target/kde --output-on-failure
```

All 72 Rust tests and both native window test cases passed on Fedora. Formatting,
Clippy, the CMake build, desktop-entry validation, and installation into a temporary
prefix also passed. The Rust checks include real FFmpeg fixtures and local simulated Cast receivers.
The new coverage verifies explicit controls, invalid seeks, ownership/cleanup,
protocol framing, version rejection, ignored CLI config, read-only inspection,
and EOF cancelling/reaping an active probe. The native tests exercise the three
screens, explicit device/subtitle/resume choices, controls, repeated sessions,
stale events, cancellation, and helper shutdown. Another native check inspects
through the actual Rust helper. These checks do not cast to a TV.

Set `YEET_UI_SCREENSHOTS` to a local output directory when running CTest to save
selection, preparation, and playing screenshots from the simulated UI test.

## TV checklist

1. Open `samples/preferences/embedded.mkv`. Confirm no playback starts and both
   device and subtitle selection remain explicit. Select the TV and English,
   then press **Yeet**. Confirm picture, sound, and captions.
2. Pause and resume using the window. Seek forwards and backwards; also seek
   while paused. Confirm playback and captions stay in sync.
3. After at least 20 seconds, stop. Confirm the TV stops and selection returns.
   Use **Yeet from last position**, then stop and use **Yeet** to verify the
   difference between resuming and starting at zero.
4. Select an external subtitle with the picker. It should open beside the video.
   Confirm captions and then close the window during playback; the helper and
   owned playback should stop.
5. To see actual conversion progress, use an uncached conversion candidate or
   copy the generated PGS fixture `samples/subtitle-check/embedded-pgs-cues.mkv`
   into a fresh temporary folder under `samples`, open that copy, and select its
   PGS track. Confirm the preparation screen advances before playback starts.
   Repeat with another fresh copy and press Cancel during conversion; confirm
   selection returns and no incomplete prepared output remains.

Successful prepared outputs remain reusable, as in the CLI. Subtitle switching
during playback, a settings window, packaging, and non-KDE frontends are deferred.
