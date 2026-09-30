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

## Player interaction

Opening a video or dropping one local file on the idle player does not start
playback. The selected-video panel stays visible through all three states:

1. **Selection:** devices, a combined embedded/exact-basename external subtitle
   list, a subtitle file picker, signed delay, and starting-position buttons.
   The picker starts beside the source. Discovery has a loading indicator.
2. **Preparation:** conversion/remux progress and Cancel; probing, subtitle work,
   connection and loading use an indeterminate indicator. Direct/cache-hit paths
   pass through this state briefly.
3. **Playing:** the selected-video panel shows the active subtitle track or external
   filename. Position, seek slider, pause/play and stop are aligned at the bottom.
   Space toggles pause/play without key-repeat. Stop waits for cleanup and returns
   to choices. Quit/Ctrl+Q/window close cancels active work and waits for helper exit.

The last-used eligible device is restored after discovery. Subtitles are preselected
using the shared `[subtitles]` settings and [selection rules](subtitles.md#selection):
English then Dutch by default, followed by exact-name SRT fallback. With
`subtitles.auto_load = false`, None is selected initially. Unsupported tracks are
skipped; ambiguous sidecars leave None selected with a warning. The combined list also includes matching VTT/ASS/SSA; an explicit picker file need
not match the video name. Manual choices survive stop/restart of the same video.
See the [subtitle guide](subtitles.md) for supported formats, rendering and limitations.

**Cast** starts at zero; **Cast from last position** appears when a usable
checkpoint exists. The UI saves progress but never resumes implicitly.
**Subtitle delay** is in signed milliseconds: positive later, negative earlier.
It restores the saved offset for each video/subtitle pair and survives stop/restart.
It is fixed for each session,
including burn-in. Subtitle selection/delay cannot change during preparation or
playback. Multiple-file, directory, remote-URL and active-session drops are ignored.

## Preferences and saved state

GUI preferences live in the shared [`config.toml`](configuration.md). Set
`gui.conversion.auto_close = false` to leave the converter open after success;
by default it closes after five seconds. Errors and cancellation stay open.

The player and converter remember their last device after an accepted start or
conversion. Merely changing the selection does not save it. This state lives in
`$XDG_STATE_HOME/werp/gui.toml` (normally `~/.local/state/werp/gui.toml`), separately
from configuration. Device selection and resume preferences under `cli.*` do not
change GUI behavior.

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
| `--help`, `--version` | Show usage or version information. |

Casting uses an OS-assigned HTTP port by default. For an existing firewall rule,
set the global `http_port = 8010` in
[`config.toml`](configuration.md). This setting applies
to both GUI and CLI casting and has no effect in convert-only. The GUI does not
create firewall rules.
CLI flags such as `--profile`, `--mode`, `--config` and `--no-config` are not GUI
options. Both apps do use the same [device override file](device-compatibility.md#user-overrides).

Build, local installation, development flags and automated checks are in
[DEVELOPMENT.md](../DEVELOPMENT.md).

Subtitle switching during playback, a settings window, single-instance
behavior, packaging and additional native frontends remain in [the roadmap](plan.md).
