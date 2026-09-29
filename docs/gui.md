# GUI

This guide describes the GUI interaction model and the current KDE implementation.
GTK and other native frontends can use the same backend contract; they are not
implemented yet.

The KDE frontend uses C++/Qt 6 Widgets and launches a private Rust helper through
QProcess.
The user opens one app, not a separately managed service. This avoids Rust GUI
bindings and lets other native frontends reuse the [versioned protocol](backend-protocol.md).
The core owns media/Cast/lifecycle logic; the UI owns presentation and choices.
See [project decisions](plan.md#decisions-and-rationale).

The user confirmed initial KDE playback and subsequent UI improvements. Future
playback issues are handled as bugs; there is no pending manual acceptance checklist.

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

**Werp** starts at zero; **Werp from last position** appears when a usable
checkpoint exists. The UI saves progress but never resumes implicitly.
**Subtitle delay** is in signed milliseconds: positive later, negative earlier.
It resets for a new file, survives stop/restart, and is fixed for each session,
including burn-in. Subtitle selection/delay cannot change during preparation or
playback. Multiple-file, directory, remote-URL and active-session drops are ignored.

## KDE frontend preferences

`$XDG_CONFIG_HOME/werp/kde-ui.ini` (normally `~/.config/werp/kde-ui.ini`) stores
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
werp-kde --convert-only /path/to/video.mkv
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

## Launch options

`werp-kde [OPTIONS] [FILE]` has its own arguments; it does not forward arbitrary
`werp` CLI options to the backend. Omit FILE to choose a video in the GUI.

| GUI option | Effect |
| --- | --- |
| `--convert-only` | Open the converter instead of the player. |
| `--http-port PORT` | Fix the player's serving port; default `0` chooses automatically. Has no effect in convert-only. Also supported by the CLI. |
| `--help`, `--version` | Show usage or version information. |

A fixed port helps with an existing firewall rule; the GUI does not create one.
CLI flags such as `--profile`, `--mode`, `--config` and `--no-config` are not GUI
options. Both apps do use the same [device override file](device-compatibility.md#user-overrides).

Build, local installation, development flags and automated checks are in
[DEVELOPMENT.md](../DEVELOPMENT.md).

Subtitle switching during playback, a settings window, MPRIS, single-instance
behavior, packaging and additional native frontends remain in [the roadmap](plan.md).
