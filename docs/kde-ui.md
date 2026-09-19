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
The CLI owns its configurable device/subtitle automation and automatic resume.
The UI has independent convenience defaults: the last-used device and suggested
subtitles. Opening a file in the regular player never launches playback or conversion.
The separate convert-only entry starts offline preparation immediately.

CLI configuration uses `[cli.devices]`, `[cli.subtitles]`, and `[cli.playback]`.
The last uses `auto_resume`; saving checkpoints is an independent shared capability.
No compatibility aliases are required. Existing personal config is updated locally.
The helper reads only shared `[compatibility]` settings when starting work; it
does not apply CLI configuration. The KDE app stores the last device ID
in `$XDG_CONFIG_HOME/yeet/kde-ui.ini` (normally `~/.config/yeet/kde-ui.ini`), separate
from `config.toml`. It records the device when a start request is accepted and
restores it after discovery only if it is still an available video-capable target.
Changing a selection without starting playback does not overwrite the last device.

## Interaction

On startup, inspect the file and discover receivers asynchronously. Show a device
selector that restores the last-used device if available, a single subtitle list containing None,
embedded tracks, and exact-basename external SRT, WebVTT, ASS, and SSA files. A separate file picker
starts beside the source and can choose supported external subtitle files.
On opening a new video, preselect the backend's recommendation using the CLI's
existing ranking with fixed English-then-Dutch language order, then a matching
SRT fallback. Unsupported tracks are skipped; ambiguous sidecars leave None selected
and show a warning. Manual subtitle choices, including None, survive stopping and
restarting the same video. CLI configuration and flags do not affect these defaults.

A single local video can also be dragged onto the idle window. Dropping opens it
without playback; drops during preparation/playback, multiple files, directories,
and remote URLs are ignored. The source file is never moved.

Yeet starts at zero. Yeet from last position appears when a usable checkpoint
exists. Both require a valid selected receiver. There are three separate screens:

1. **Selection:** choose the receiver, subtitle track/file, and starting position.
2. **Preparation:** show conversion/remuxing progress and Cancel. While probing,
   preparing subtitles, connecting, or loading, show an indeterminate indicator.
   Direct playback and cache hits pass through this screen briefly.
3. **Playing:** show pause/play, stop, position, and a seek slider. Space toggles
   pause/play while the window is active, without repeating when held down.

In the regular player, no conversion starts before pressing a Yeet button. Progress is a structured
fraction from the backend; the frontend does not parse FFmpeg or CLI output.
Disable subtitle selection during preparation and playback. Stop waits for cleanup
and returns to startup choices. The helper stays available for another session.
The bottom-row Quit button and Ctrl+Q use the normal window-close cleanup path.
Ctrl+Q also works during preparation and playback. Closing the UI or losing its input pipe cancels work, releases resources, and
exits the helper. Helper failure must be visible in the UI.

## Convert-only window

Open a video with **Yeet (convert only)**, or run:

```sh
./target/kde/yeet-kde --convert-only /path/to/video.mkv
```

Without a filename this mode opens a file picker. It starts preparation immediately
and shows the filename, source/target formats, progress, and Cancel. The default
target is the conservative profile: MP4, SDR H.264 up to 1080p30/level 4.1, and optional
mono/stereo AAC-LC. Compatible streams are copied when possible; an already
compatible MP4 needs no conversion. Encoded audio is stereo AAC at 192 kbps/48 kHz.
No receiver is discovered or contacted, and no HTTP listener or resume checkpoint
is created. The helper acquires the same best-effort sleep inhibitor used by casting.
Shared `[compatibility]` settings can independently allow bounded SDR HEVC and
AAC-LC surround; both default to false. The window displays the target reported
by the backend. CLI automation preferences and the last-used receiver do not
influence this target. See [configuration](preferences-and-subtitles.md#configuration).

Completed output is validated and kept beside the canonical source, with the
existing user-cache fallback if the directory is not writable. Existing conversions
with the same recipe are reused. The original is kept. Subtitles are not copied or
burned into this offline output: open the original in the player to retain Yeet's
subtitle selection. The result path is displayed on completion.

Cancel waits for cleanup before becoming Close. Closing the window or Ctrl+Q also
cancels active work and waits for helper exit. On success, Close is available and
a visible five-second countdown closes the window automatically. Errors and
cancellation stay open. An already-compatible source and cache reuse are successes.

The regular player still chooses its recipe for the selected receiver, which may
be different from the convert-only target. Opening the prepared MP4 directly needs no
conversion; opening the original reuses it only when the player's recipe matches.

## Transport contract

Use UTF-8 newline-delimited JSON over private stdin/stdout pipes. Logs go to stderr.
Every request has an integer `id` and a `method`, with typed `params` as needed.
Responses echo the id and carry either a result or a structured error. Progress
is an asynchronous event with a session identifier, phase, position, and message.
A protocol-version handshake rejects incompatible versions. Messages and queues
are bounded. No shell command construction or parsing of CLI prose is involved.

Operations: handshake, inspect (metadata/subtitles/checkpoint), discover, start
(explicit file/device/subtitle/position), pause, play, seek, stop, and shutdown.
Only one playback session or offline conversion runs per helper. Commands for expired sessions must not
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
build), and both application/Open With desktop entries. Ensure the
install's `bin` directory is in the desktop session's PATH. Installation does not
change the default association for video files.

### Register the development build with KDE

A user-local `org.yeet.Yeet.desktop` entry now points directly to the current
`target/kde/yeet-kde` executable, using HTTP port 8010. Yeet appears in the
application launcher and Dolphin's **Open With** menu. This does not replace the
default video player. It advertises MP4/M4V, MKV, WebM, AVI, MOV, MPEG/TS, FLV,
WMV, and Ogg video; playback remains subject to codec support and preparation.

The development launcher is outside Git. To register it again after moving the
checkout, run these commands from the repository root after building:

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

The [version 1 protocol reference](backend-protocol.md) defines the frontend boundary.
Developer overrides `--backend PATH` and `--no-discovery` allow offline UI checks.

## Automated checks

```sh
cargo fmt --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace -- --include-ignored
/usr/bin/ctest --test-dir target/kde --output-on-failure
```

The Rust suite covers the helper protocol, shared subtitle ranking, conversion,
and casting. Native window tests cover the player and convert-only lifecycle; the native checks run with both Fusion and KDE Breeze styles. Formatting,
Clippy, the CMake build, desktop-entry validation, and installation into a temporary
prefix also passed. The Rust checks include real FFmpeg fixtures and local simulated Cast receivers.
The new coverage verifies explicit controls, invalid seeks, ownership/cleanup,
protocol framing, version rejection, ignored CLI config, read-only inspection,
and EOF cancelling/reaping an active probe. The native tests exercise the three
screens, device persistence, suggested/manual subtitle choices, explicit resume,
Space controls, drag-and-drop, repeated sessions,
stale events, cancellation, and helper shutdown. Another native check inspects
through the actual Rust helper. These checks do not cast to a TV.

Set `YEET_UI_SCREENSHOTS` to a local output directory when running CTest to save
selection, preparation, and playing screenshots from the simulated UI test, including convert-only progress and completion.

## Convert-only checks

Automated checks use short generated media to exercise remuxing, audio conversion,
full video conversion, reuse, already-compatible input, and unchanged sources.
Protocol checks cancel active encoders by explicit cancellation, shutdown, and EOF,
verify child reaping and partial-output removal, and reject concurrent operations.
Native tests cover progress, terminal errors, cancellation, early cancellation,
window close, and the success countdown. None of these tests contacts a TV.

For a manual check, open a video via Dolphin's new entry and confirm the formats,
progress, output location and auto-close. Cancel a second conversion and confirm
that it stays open with Close after cleanup. Real listening remains useful for
subjective dialogue clarity; synthetic channel tests cannot judge a movie's mix.

## TV checklist

1. Open `samples/preferences/embedded.mkv`. Confirm no playback starts, English is selected, and
   the last-used TV is selected if available (choose it on the first run).
   Then press **Yeet**. Confirm picture, sound, and captions.
2. Pause and resume using the window buttons and Space. Seek forwards and backwards; also seek
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

6. Reopen the app and verify the last-used device is restored after discovery.
   Drop another local video onto the idle window; it should open with suggested
   subtitles while waiting for **Yeet** to be pressed.

Successful prepared outputs remain reusable, as in the CLI. Subtitle switching
during playback, a settings window, packaging, and non-KDE frontends are deferred.
