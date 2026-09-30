# CLI guide

```sh
werp devices
werp inspect /path/to/video.mkv --json
werp /path/to/video.mkv --device "Living Room"
werp /path/to/video.mkv --device "Living Room" --subtitles /path/to/captions.srt
werp --help
```

`werp FILE` stays in the foreground while serving. It replaces playback on the
selected receiver. Ctrl+C/SIGTERM cleans up owned playback, child processes,
HTTP serving and temporary files; successful reusable outputs remain. Another
sender's takeover ends Werp's ownership without stopping that sender's media.
Interactive terminal playback controls remain parked; the GUI has controls.

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
| `--config PATH`, `--no-config` | Override shared settings and CLI preferences, or ignore them and local model overrides |

Configuration paths, defaults and settings are in the [configuration reference](configuration.md).
Device capabilities use the separate [device database](device-compatibility.md).
GUI interaction is covered in the [GUI guide](gui.md).

## Subtitles

Automatic selection is enabled by default, using English then Dutch. Configure
`subtitles.auto_load` and `subtitles.languages` in
[`config.toml`](configuration.md) to change this behavior. The GUI uses the same settings for dropdown
preselection. The selected file or stream index is printed before playback.
See the [shared subtitle guide](subtitles.md) for selection rules, supported
formats, rendering and limitations.

Overrides are mutually exclusive:

| Option | Behavior |
| --- | --- |
| `--subtitles FILE` | Use that external file, regardless of automatic loading settings |
| `--subtitle-track INDEX` | Use an embedded subtitle's absolute stream index from `inspect` |
| `--no-subtitles` | Disable subtitles for this session |
| `--auto-subtitles` | Enable automatic selection even if disabled in config |

Use `--subtitle-delay-ms 1500` to show captions 1.5 seconds later, or
`--subtitle-delay-ms -1500` to show them earlier. The default is zero; see
[delay behavior](subtitles.md#subtitle-delay).

## Device selection

Explicit `--device NAME/ID` or `--host IP` wins. Otherwise, Werp tries the
configured preferred list in order among reachable confirmed video receivers,
skipping absent and audio-only devices. If no preference matches, it selects the
sole eligible video receiver. Multiple candidates require a preference or explicit
selection; duplicate friendly names require a stable ID. Unknown capabilities
still require explicit selection. Selection errors include detected names/IDs and
mark known audio-only devices. This does not alter the separate `devices` scan.

## Playback positions

CLI automatic resume is controlled by `[cli.playback] auto_resume`. Turning off
automatic resume still records progress; `--no-resume` disables both reading and
writing for that CLI invocation. The UI records progress but only resumes when
the user chooses its resume button.

Resume is enabled by default. Positions come from owned PLAYING/PAUSED receiver
status, not elapsed wall time. Changed positions are saved at most every five
seconds and flushed on ordinary interruption/error/receiver stop. The next cast
of the same source requests the saved position minus five seconds for context.
Checkpoints before ten seconds or within five seconds of the end start from zero.
Normal FINISHED completion clears the checkpoint. This resumes a subsequent cast;
it does not automatically reconnect or relaunch after a network failure.

`--restart` starts at zero while recording new progress. `--no-resume` neither
reads nor writes position state for that run. `--resume` enables it even if config
disables it. These flags are mutually exclusive.

Checkpoints live under `$XDG_STATE_HOME/werp/resume` or
`~/.local/state/werp/resume`. Keys hash the canonical source path, size, and
modification time; moving/replacing a source starts fresh. Records contain only
version, duration, and position, with private file permissions. Atomic replacement
avoids partial JSON. Per-source locks prevent competing sessions from overwriting
one checkpoint; inability to access/lock state produces a warning and leaves
casting available. Empty lock files may remain after completion; they are not
active locks. Invalid records are ignored. A crash can lose up to the last
checkpoint interval of reported progress. No state is stored in Git or in media.
