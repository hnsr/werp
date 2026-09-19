# Preferences, subtitles, device selection, and resume

Implemented 2026-09-19. Local and simulated-receiver verification is described
below. The user confirmed automatic English embedded subtitles and interrupted
playback resume on the TV, followed by explicit Dutch selection, restart, and
normal completion clearing the saved position. Automatic external SRT loading and
subtitles off also passed, followed by embedded PGS burn-in. Optional remaining
TV checks are listed below. No TV
was contacted during implementation. Earlier seeking and long-duration checks remain deferred.

## Configuration

Yeet reads `$XDG_CONFIG_HOME/yeet/config.toml`, falling back to
`~/.config/yeet/config.toml` when XDG_CONFIG_HOME is unset or not absolute.
A missing default file uses built-in defaults. `--config PATH` selects an explicit
file; a missing explicit file is an error. `--no-config` uses built-in defaults.
Malformed values and unknown keys are errors, so spelling mistakes do not silently
change behavior. CLI overrides take precedence over file preferences.

See [the example TOML](config.example.toml). Supported settings:

```toml
[compatibility]
allow_hevc = false
allow_aac_surround = false

[cli.subtitles]
auto_load = true
languages = ["en", "nl"]

[cli.devices]
preferred = ["Living Room", "Bedroom"]

[cli.playback]
auto_resume = true
```

The built-in device preference list is empty. Personal device names belong in
the user's configuration, never in tracked examples or commits.

`[compatibility]` is shared by the CLI and KDE helper. Both flags default to
false. `allow_hevc` admits SDR HEVC Main/Main 10 up to level 4.0/1080p30;
`allow_aac_surround` admits AAC-LC with up to six channels. The flags are
independent: HEVC alone still converts multichannel audio to stereo. They do not
admit HDR, larger dimensions/frame rates, unsupported profiles, or ambiguous
tracks. Dolby/HE-AAC audio still converts to AAC-LC. Enable only formats known to
work on the intended receiver; these are global preferences, not device negotiation.

Convert-only adds these permissions to its conservative target. Automatic casting
adds them to the existing receiver-model profile. Existing known-model support
is preserved when flags are false. Explicit CLI `--profile baseline`, `extended`,
or `experimental` overrides these preferences; `--mode transcode` still forces
H.264/stereo AAC. CLI `--no-config` disables file preferences, while model detection
still applies. The helper reads the shared section for each new conversion or
cast; it ignores CLI-only values and validates the shared section. Malformed TOML
or invalid shared fields fail the new operation visibly. Inspect/discovery remain
independent of configuration. No settings change an operation already in progress.

## Subtitle selection and rendering

With automatic loading enabled, Yeet first looks for a supported embedded
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
ASS/SSA conversion warns that advanced fonts, positioning, and effects are lost;
this is readable caption support, not faithful ASS rendering.

Embedded PGS/DVD/DVB image subtitles use FFmpeg overlay and full video conversion.
They are permanently visible in that prepared video; they cannot be toggled off
on the receiver. Automatic mode selects full conversion and explains why. An
explicit Direct/Remux/Audio mode fails rather than violating its no-video-encoding
constraint. Selecting another track or disabling subtitles uses a distinct cache
recipe, so an output with burned captions is never reused for a different choice.
PGS burn-in passed local rendered-frame verification and user-confirmed TV
playback. DVD/DVB use the same route but remain unverified with representative
samples. External image-subtitle files and OCR are
not implemented. Existing SDR/HDR and audio-track limits still apply.

The library scan found 53 of 75 videos with embedded subtitles: 145 SubRip,
16 MP4 text, and 11 PGS streams. All PGS examples also had English text
alternatives. Common metadata included `eng`, `dut`, `English`, `English SDH`,
`Forced`, and `Nederlands`; these informed the ranking above. Private filenames
and raw metadata remain under ignored `samples/`.

The user considers this subtitle scope sufficient for now: all subtitle codec
types found in the provided sample set are supported, including external SRT.
Further format expansion and optional subtitle checks are parked until a real
file exposes a gap. This is format coverage, not a claim that every sample and
subtitle stream has been played through on the receiver.

## Device selection

Explicit `--device NAME/ID` or `--host IP` wins. Otherwise, Yeet tries the
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

Checkpoints live under `$XDG_STATE_HOME/yeet/resume` or
`~/.local/state/yeet/resume`. Keys hash the canonical source path, size, and
modification time; moving/replacing a source starts fresh. Records contain only
version, duration, and position, with private file permissions. Atomic replacement
avoids partial JSON. Per-source locks prevent competing sessions from overwriting
one checkpoint; inability to access/lock state produces a warning and leaves
casting available. Empty lock files may remain after completion; they are not
active locks. Invalid records are ignored. A crash can lose up to the last
checkpoint interval of reported progress. No state is stored in Git or in media.

## Verification and TV checklist

The user ran the generated embedded-subtitle fixture on the KPN DIW7022 and
confirmed English captions and successful resume after interruption. A subsequent
check confirmed explicit Dutch track selection with restart, and playback starting
from zero after normal completion without the restart flag. The external fixture
also passed automatic same-name SRT loading and a second run with no captions
when `--no-subtitles` was supplied. The user also confirmed embedded PGS burn-in
on the TV using the short image-caption fixture. Automatic Dutch preference has
local test coverage only. This report does
not establish subtitle synchronization after arbitrary seeking.

`cargo test --locked --workspace -- --include-ignored` passed all 68 tests,
including the real FFmpeg fixtures. Formatting, Clippy with warnings denied,
and `git diff --check` passed. Local CLI checks also confirmed English/Dutch
selection, external fallback, subtitles off, image-caption cache reuse, and
refusal to reuse that captioned output when subtitles are disabled.

Local tests cover TOML defaults/validation/overrides, language ranking and explicit
selection, exact sidecar lookup, UTF-16, device preferences/errors, cancellation,
state identity/locking, restart, disabled resume, and normal completion. Complete
simulated sessions verify saved offsets in subsequent Cast LOAD messages after
cancellation, failed LOAD, and disconnect. Real FFmpeg tests exercise generated
SubRip/MP4 text/ASS tracks and preserve cue times. A local PGS sample was prepared
and a rendered frame visually confirmed its caption; this is not a TV result.

Generate a short, audible fixture without contacting a TV:

```sh
bash scripts/generate-subtitle-fixture.sh samples/preferences 60
```

The fixture includes full English at stream 3, Dutch at stream 4, and deliberately
default/forced English at stream 2. It also creates an exact-name external SRT
case and an alternate Dutch-first config. On the development machine these files
have already been generated. Run these checks when ready, substituting a selected
receiver where an explicit device is needed:

1. `cargo run --locked -- samples/preferences/embedded.mkv`: the configured
   preferred receiver should play full English captions, not the forced track.
   Interrupt after at least 20 seconds, then repeat the same command. Confirm the
   resume notice and TV starting near that point with correctly timed captions.
   Let it finish; the next run should start at zero.
2. Add `--subtitle-track 4 --restart`: Dutch captions should appear from the start.
   The generated `samples/preferences/dutch.toml` can also be selected with
   `--config ... --device "Living Room"` to test automatic Dutch preference.
3. Cast `samples/preferences/external.mp4 --no-resume`: the same-name external SRT
   should be selected automatically. Repeat with `--no-subtitles` to confirm none.
4. Optional image-caption check on the local prepared fixture:
   `cargo run --locked -- samples/subtitle-check/embedded-pgs-cues.mkv --subtitle-track 4 --no-resume`.
   The CLI should select burn-in and reuse its prepared output; confirm visible
   captions during the short clip. This fixture is local-only.

The earlier long-duration and general seeking checks remain deferred. No phone
controls are required for this checklist; Ctrl+C is sufficient.
