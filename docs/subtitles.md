# Subtitles

The CLI and GUI share subtitle selection and rendering in the Rust core. This
guide covers their common behavior. See the [CLI guide](cli.md#subtitles) for
flags and configurable preferences, or the [GUI guide](gui.md#player-interaction)
for the subtitle picker and playback controls.

## Selection

> Configurable automatic selection and language preferences (`cli.subtitles.*`)
> apply only to the CLI. The GUI already preselects a subtitle in its dropdown
> using a fixed English-then-Dutch order; it does not read those CLI settings.
> You can review or change that selection before casting.

Automatic selection first looks for a supported embedded track matching the
ordered language preferences, then an exact-basename `.srt` beside the real
source file. If necessary it also checks beside a supplied alias. The extension
is case-insensitive; the basename must match exactly. Unrelated SRTs and language
variants with different basenames are not guessed.

Multiple matching SRT sidecars require explicit selection:
the CLI reports an error; the GUI leaves None selected with a warning.

Within the first matching language, prefer full dialogue over forced/signs-only
tracks, text over image tracks, non-SDH/CC over accessibility tracks, then a
default track, then the lowest stream index. Forced/SDH tracks remain selectable
and can be chosen automatically if they are the best available match. Metadata
language tags take precedence over track names; names are a fallback only for
missing/undefined language tags.

### Language names

English aliases include `en`, `eng`, `English`, and region variants such as
`en-GB`/`en_US`. Dutch aliases include `nl`, `nld`, `dut`, `Dutch`, `Nederlands`,
`Flemish`, and `Vlaams`. User preferences and otherwise untagged titles also accept
UK/GB as English shorthand. A media language tag `uk` is Ukrainian and is never
treated as English. CLI language preferences currently support only English and
Dutch; tracks in other languages can be selected explicitly in either frontend.

## Text subtitles

Embedded text (including SubRip, MP4 `mov_text`, ASS/SSA, and WebVTT) is extracted
from the original source and served as a temporary WebVTT track. Video need not
be re-encoded. External SRT, WebVTT, ASS, and SSA are supported in UTF-8 or
BOM-marked UTF-16. Conversion/extraction needs FFmpeg; external WebVTT does not.

Empty timed cues are skipped during preparation without modifying the original
file. Invalid timestamps and files with no non-empty cues produce an error.
ASS/SSA conversion warns that advanced fonts, positioning, and effects are lost;
this is readable caption support, not faithful ASS rendering.

## Image subtitles

Embedded PGS image subtitles use FFmpeg overlay and full video conversion.
They are permanently visible in that prepared video; they cannot be toggled off
on the receiver. Automatic mode selects full conversion and explains why. An
explicit CLI Direct/Remux/Audio mode fails because it disallows video encoding.
Selecting another track or disabling subtitles uses a distinct cache recipe, so
an output with burned captions is never reused for a different choice.
PGS burn-in passed local rendered-frame verification and user-confirmed TV
playback.

DVD/VobSub and DVB image subtitles, external image-subtitle files and OCR are not
supported. Burn-in on HDR-tagged sources uses experimental video transcoding
without tone mapping; audio-track limits still apply. See [media conversion](media-conversion.md).

## Subtitle delay

Delay is specified in signed milliseconds: positive values show captions later,
negative values earlier. The default is zero. Set it with the GUI's **Subtitle
delay** control or the CLI's `--subtitle-delay-ms` option before casting.

The delay applies when playback starts, including resume, and changes subtitle
timing without shifting audio or video. Text cues crossing the beginning are
clipped to zero; cues ending before or at zero are dropped. If none remain,
playback proceeds without a subtitle track. Originals are never modified. Image
subtitle burn-in also respects the delay and uses a separate cache recipe for
each offset.

## Convert-only

Convert-only output does not include subtitles. Open the original file in the
player to select them when casting; see the [converter guide](gui.md#convert-only-window).
