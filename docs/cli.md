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

## Subtitle selection and rendering

With automatic loading enabled, Werp first looks for a supported embedded
track matching the ordered language preferences, then an exact-basename `.srt`
beside the real source file. If necessary it also checks beside a supplied alias.
The extension is case-insensitive; the basename must match exactly. Multiple
matching sidecars produce an explicit-selection error. Unrelated SRTs and language
variants with different basenames are not guessed.

Within the first matching language, prefer full dialogue over forced/signs-only
tracks, text over image tracks, non-SDH/CC over accessibility tracks, then a
default track, then the lowest stream index. The selected file/index is printed.
Forced/SDH tracks remain selectable and can be chosen automatically if they are
the best available match. Metadata language tags take precedence over track names;
names are a fallback only for missing/undefined language tags.

English aliases include `en`, `eng`, `English`, and region variants such as
`en-GB`/`en_US`. Dutch aliases include `nl`, `nld`, `dut`, `Dutch`, `Nederlands`,
`Flemish`, and `Vlaams`. User preferences and otherwise untagged titles also accept
UK/GB as English shorthand. A media language tag `uk` is Ukrainian and is never
treated as English. Other preferred languages are currently rejected in config.

Overrides are mutually exclusive:

| Option | Behavior |
| --- | --- |
| `--subtitles FILE` | Use that external file, regardless of automatic loading settings |
| `--subtitle-track INDEX` | Use an embedded subtitle's absolute stream index from `inspect` |
| `--no-subtitles` | Disable subtitles for this session |
| `--auto-subtitles` | Enable automatic selection even if disabled in config |

Embedded text (including SubRip, MP4 `mov_text`, ASS/SSA, and WebVTT) is extracted
from the original source and served as a temporary WebVTT track. Video need not
be re-encoded. External SRT, WebVTT, ASS, and SSA are supported in UTF-8 or
BOM-marked UTF-16. Conversion/extraction needs FFmpeg; external WebVTT does not.
Empty timed cues are skipped during preparation without modifying the original
file. Invalid timestamps and files with no non-empty cues still produce an error.
Use `--subtitle-delay-ms 1500` to show captions 1.5 seconds later, or
`--subtitle-delay-ms -1500` to show them earlier. The default is zero. The GUI
selection screen offers the same setting as **Subtitle delay**, in milliseconds.
It applies when playback starts, including resume, and changes subtitle timing
without shifting audio or video. Text cues crossing the beginning are clipped to
zero; cues ending before or at zero are dropped. If none remain, playback proceeds
without a subtitle track. Originals are never modified. Image subtitle burn-in
also respects the delay and uses a separate cache recipe for each offset.
ASS/SSA conversion warns that advanced fonts, positioning, and effects are lost;
this is readable caption support, not faithful ASS rendering.

Embedded PGS image subtitles use FFmpeg overlay and full video conversion.
They are permanently visible in that prepared video; they cannot be toggled off
on the receiver. Automatic mode selects full conversion and explains why. An
explicit Direct/Remux/Audio mode fails rather than violating its no-video-encoding
constraint. Selecting another track or disabling subtitles uses a distinct cache
recipe, so an output with burned captions is never reused for a different choice.
PGS burn-in passed local rendered-frame verification and user-confirmed TV
playback. External image-subtitle files and OCR are not implemented.
Burn-in on HDR-tagged sources uses experimental video transcoding without tone
mapping; audio-track limits still apply.

Further subtitle expansion is considered when a real file exposes a gap.

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
