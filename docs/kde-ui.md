# KDE frontend and backend interface

## Agreed design

The KDE application is C++ with Qt 6 Widgets and KDE Frameworks 6. It starts a
private Rust helper automatically through QProcess. The user opens one app;
there is no separately managed service. Future GTK or other native frontends
can implement the same versioned message protocol without Rust bindings.

`yeet-core` owns probing, discovery, subtitle enumeration/preparation, compatibility,
conversion/cache, Cast sessions, controls, sleep inhibition, and checkpoint storage.
The CLI owns automatic device/subtitle selection and automatic resume. The UI
makes explicit choices; opening a file never launches playback or conversion.

Configuration moves to `[cli.devices]`, `[cli.subtitles]`, and `[cli.playback]`.
The last uses `auto_resume`; saving checkpoints is an independent shared capability.
No compatibility aliases are required. Existing personal config is updated locally.
The helper does not load CLI configuration.

## Interaction

On startup, inspect the file and discover receivers asynchronously. Show a device
selector with no automatic selection, a single subtitle list containing None,
embedded tracks, and exact-basename external SRT files. A separate file picker
starts beside the source and can choose supported external subtitle files.

Yeet starts at zero. Yeet from last position appears when a usable checkpoint
exists. Both require an explicitly selected receiver. Preparation shows progress
and cancellation. Once playing, show pause/play, stop, position, and seeking.
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

## Implementation order and validation

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
