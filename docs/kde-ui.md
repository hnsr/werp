# KDE frontend

The C++/Qt 6 Widgets application launches a private Rust helper through QProcess.
The user opens one app, not a separately managed service. This avoids Rust GUI
bindings and lets other native frontends reuse the [versioned protocol](backend-protocol.md).
The core owns media/Cast/lifecycle logic; the UI owns presentation and choices.
See [project decisions](plan.md#decisions-and-rationale).

The user confirmed initial KDE playback and subsequent UI improvements. Local
native/protocol tests pass; the complete TV checklist below is not recorded as passed.

## Player interaction

Opening a video or dropping one local file on the idle player does not start
playback. The selected-video panel stays visible through all three states:

1. **Selection:** devices, a combined embedded/exact-basename external subtitle
   list, a subtitle file picker, signed delay, and starting-position buttons.
   The picker starts beside the source. Discovery has a loading indicator.
2. **Preparation:** conversion/remux progress and Cancel; probing, subtitle work,
   connection and loading use an indeterminate indicator. Direct/cache-hit paths
   pass through this state briefly.
3. **Playing:** position, seek slider, pause/play and stop, aligned at the bottom.
   Space toggles pause/play without key-repeat. Stop waits for cleanup and returns
   to choices. Quit/Ctrl+Q/window close cancels active work and waits for helper exit.

The last-used eligible device is restored after discovery. Subtitles default to
English then Dutch using shared ranking, then exact-name SRT fallback. Unsupported
tracks are skipped; ambiguous sidecars leave None selected with a warning. The
combined list also includes matching VTT/ASS/SSA; an explicit picker file need
not match the video name. Manual choices survive stop/restart of the same video.
CLI preferences do not change these UI defaults.

**Yeet** starts at zero; **Yeet from last position** appears when a usable
checkpoint exists. The UI saves progress but never resumes implicitly.
**Subtitle delay** is in signed milliseconds: positive later, negative earlier.
It resets for a new file, survives stop/restart, and is fixed for each session,
including burn-in. Subtitle selection/delay cannot change during preparation or
playback. Multiple-file, directory, remote-URL and active-session drops are ignored.

## GUI preferences

`$XDG_CONFIG_HOME/yeet/kde-ui.ini` (normally `~/.config/yeet/kde-ui.ini`) stores
`lastDeviceId` in the General group. The player and converter share it; an accepted
start/convert with a device records it. Merely changing a selection does not.
The helper snapshots [device rules](device-compatibility.md) per operation and
never reads `[cli.*]` preferences.

```ini
[conversion]
autoClose=true
```

Auto-close defaults to five seconds after successful conversion, including reuse
or already-compatible input. Set false to retain the window with **Auto-close
disabled** shown. Errors/cancellation stay open. The setting is read when the
window opens and has no CLI effect.

## Convert-only window

```sh
./target/kde/yeet-kde --convert-only /path/to/video.mkv
```

Without a filename, a native picker opens. Choose a discovered video receiver or
**Broad compatibility (no device)**, review the source/target format preview,
and click **Convert**. The last-used device is preselected if present; audio-only
receivers are omitted. Refresh rescans, and empty/failed discovery still permits
Broad compatibility. Changing the target updates a read-only preview; stale
responses cannot overwrite a newer choice.

A device uses its advertised model and bundled-plus-user database rules, exactly
as automatic casting does. Unknown models/Broad compatibility use conservative
H.264/stereo AAC MP4. Conversion resolves the policy and inspects again, so changes
since preview are validated. Malformed overrides fail visibly; Broad compatibility
bypasses them. The selected device need not remain online during conversion.

**Source** and **Target** are bold left-aligned headings; Container, Video,
Resolution and Audio labels/values are regular weight. Planned and probed output
use the same rows. The framed selected-video panel has consistent padding and a
bold heading, filename below, and a full-path tooltip. Long labels wrap without
clipping. Controls stay at the bottom.

Once work starts, target/Refresh are disabled and Cancel waits for cleanup before
becoming Close. Window close/Ctrl+Q also waits for cleanup. Preview has no encoder,
cache publication or sleep lock; conversion holds sleep inhibition and keeps
validated output with the usual adjacent-file/user-cache behavior. Neither
conversion nor preview starts playback, connects to the receiver, serves HTTP,
selects subtitles or writes resume checkpoints.

Subtitles are not copied or burned into convert-only output. Open the original
in the player to select them. Casting and convert-only reuse the same recipe for
the same resolved policy; another device or bitmap burn-in may need another recipe.

## Build and launch

Fedora development packages: `gcc-c++ cmake ninja-build extra-cmake-modules
qt6-qtbase-devel kf6-kcoreaddons-devel kf6-ki18n-devel`. CMake requires Qt >= 6.6,
KDE Frameworks >= 6 and C++17. From the repository root:

```sh
cmake -S apps/yeet-kde -B target/kde -G Ninja -DCMAKE_BUILD_TYPE=Debug
cmake --build target/kde
./target/kde/yeet-kde /path/to/video.mkv --http-port 8010
```

CMake builds/copies the Rust helper beside the UI; rebuild after Rust or C++ changes.
Omit the file to use Open video; omit `--http-port` for an OS-assigned serving port.
A fixed port helps with an existing firewall rule; the app does not create one.
`--backend PATH` selects a test helper. `--no-discovery` skips the initial scan in
both windows; Refresh remains available. The Rust CLI has no Qt dependency;
Python is used only by the native test fixture.

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

## Automated checks

Use [README checks](../README.md#checks). The CTest target runs offscreen with
Fusion; focused native tests have also passed with KDE Breeze. Set
`YEET_UI_SCREENSHOTS` to save test screenshots. Native tests use a Python mock
helper, plus a read-only inspection check through the real Rust helper.

Coverage includes explicit start, preview/device persistence, subtitle choices,
resume, Space, drag-and-drop, preparation, controls, stale events, cancellation,
helper exit, format rows and auto-close. Rust tests exercise protocol framing,
version rejection, bounded requests, seek/ownership/lifecycle behavior and real
FFmpeg conversion/cache fixtures. Automated tests never contact TVs.

## Convert-only checks

Open a video through Dolphin's convert-only entry. Confirm no work starts before
Convert, the last-used device is selected if available, and switching to Broad
compatibility updates the preview. Convert and check the result path; reopen the
original to verify reuse. Cancel a second uncached conversion and confirm Close
appears after cleanup. Auto-close depends on the GUI setting above. Device-targeted
conversion/cache sharing and discovery-failure fallback have automated coverage;
subjective dialogue quality still requires listening.

## TV checklist

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
   output remains. The local PGS fixture is not shipped; see [subtitle checks](preferences-and-subtitles.md#verification-and-tv-checklist).
6. Reopen, confirm device persistence, and drop a local video onto the idle player.
   It should refresh choices without starting playback.

Subtitle switching during playback, a settings window, MPRIS, single-instance
behavior, packaging and non-KDE frontends remain in [the roadmap](plan.md).
