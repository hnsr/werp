# Automatic playback and reusable preparation

Current media selection and cache behavior, followed by scoped hardware evidence.
The short-clip matrix passed on KPN DIW7022; expanded-path seeking and long-duration
checks remain deferred. KDE controls are now available for future checks.

## Default behavior and overrides

`yeet FILE --device "Living Room"` inspects the file and discovers the
receiver, then selects the first suitable path:

1. Serve the original MP4.
2. Copy compatible video and audio into an MP4 (remux).
3. Copy compatible video and convert audio to stereo AAC in an MP4.
4. Convert SDR video/audio to H.264/stereo AAC in an MP4.

Preparation finishes before playback begins. Selection lives in the reusable
backend; the CLI reports the selected path and its reason. Receiver discovery
happens before preparation, but the receiver application is only launched once
the media and external subtitles are ready.

`--mode direct|remux|audio|transcode` requires that path; it fails if the input
cannot meet its constraints. `--mode auto` is the default. There is no automatic
retry or escalation after a network/receiver failure: a successful Cast status
does not establish that audio was audible.

`--profile auto` uses the bundled [device database](device-compatibility.md)
merged with optional local model overrides in `~/.config/yeet/devices.toml`.
KPN DIW7022 starts with Extended support plus the observed H.264 Level 4.2/1080p50
limit; unknown models and `--host` use Baseline. Explicit `--profile baseline`,
`extended`, or `experimental` bypasses model rules. This is an observation-based
model mapping, not receiver capability negotiation or a guarantee for every unit.

| Profile | Supported video for direct/copy paths | Supported copied audio |
| --- | --- | --- |
| Baseline | H.264, 8-bit 4:2:0, up to level 4.1/1080p30 | Optional mono/stereo AAC-LC |
| Extended | Baseline plus HEVC Main/Main 10, up to level 4.0/1080p30 | Optional AAC-LC through six channels |
| Experimental | Extended bounds | Also H.264/AC-3 passthrough for explicit trials |

The normal profiles convert Dolby and HE-AAC audio. Copy paths accept MP4/MKV;
original-file playback requires MP4. Full conversion targets the Baseline profile
regardless of the requested receiver profile. Known HDR, ambiguous multiple
audio/video tracks, and missing required duration/frame-rate metadata remain clear
errors. Tone mapping and track selection are separate work.

Subtitle preferences select an embedded track or matching external SRT;
explicit `--subtitles`, `--subtitle-track`, and `--no-subtitles` override this.
Text is served as WebVTT, while image tracks require full video burn-in. See
[preferences and subtitles](preferences-and-subtitles.md) for scope and validation.
Unselected tracks, attachments, titles, and chapters are omitted during preparation.

Model-specific permissions can be changed in the optional
[device override file](device-compatibility.md#user-overrides); there are no global
codec opt-ins. Convert-only uses the same rules for its selected device, or
Baseline for Broad compatibility. [KDE behavior](kde-ui.md#convert-only-window).

Remuxing produces a complete faststart MP4 using FFmpeg's demuxer/muxer;
no audio/video encoders are required. Generated-fixture checks verified unchanged
encoded payload hashes, preserved relative audio/video timing within 3 ms,
mono and silent inputs, source preservation, and cancellation/partial-file cleanup.

## Preparation limits

Full encoding uses libx264 veryfast/CRF 20, at most 1080p30, an 8 Mbps maximum
video rate/16 Mbit buffer, and AAC-LC 192 kbps/48 kHz. Smaller inputs are not
intentionally enlarged; silent input remains silent. Copied streams are not
subjected to encoder bitrate caps.

Preparation has a 24-hour default deadline. Space estimates use source size for
copy paths or bounded output rates for encoding, with muxing overhead and 64 MiB
headroom checked before/during work. This does not reserve disk space. Progress
reaches completion only after validation/publication. FFmpeg errors are bounded
and child processes are reaped on handled cancellation.

## Storage and reuse

By default, preparation places an MP4 and `.mp4.json` completion record beside the
canonical source, following symlinks. A name has this form:

```text
movie.yeet-<12-hex-key-tag>-<8-hex-generation>.mp4
movie.yeet-<12-hex-key-tag>-<8-hex-generation>.mp4.json
```

The source stem (without its old extension) is preserved unless too long:
both the MP4 and its metadata sidecar stay within a 255-byte filename budget,
leaving up to 219 UTF-8 bytes for the source stem without splitting a character.
This fits the usual Linux filename limit, including
[ext4's 255-byte limit](https://docs.kernel.org/filesystems/ext4/directory.html).

The visible key tag is 12 hexadecimal characters (48 bits); the generation is a
random 8-character hexadecimal ID (32 bits), not another content hash. Neither is
a correctness guarantee on its own. The metadata retains the full SHA-256 cache
key and output digest. A short-tag collision cannot select the wrong cached file:
the full key must match before reuse. Publishing uses atomic no-replace operations
for both files; an occupied media or metadata name triggers a fresh random ID,
up to 32 attempts, without repeating the conversion or overwriting existing data.
Existing files with the older naming scheme remain reusable under their current
names; renaming them is unnecessary.

The key includes canonical source path, full source SHA-256, preparation mode,
resolved profile, selected image track/delay for burn-in, and recipe versions.
The base recipe is version 1; encoded audio also uses the stereo-matrix recipe
below. Output-affecting changes must bump the relevant version. External text
subtitle changes do not invalidate prepared video.

A cache hit verifies the completion record, output size, full output SHA-256, and
fresh ffprobe metadata against the profile. Reuse reads the source and output
from disk, but does not encode or require FFmpeg for media preparation. ffprobe
remains required. Missing, corrupt, or mismatched entries are ignored; new outputs
get new names without replacing them.

If the source folder is unwritable, Yeet announces a fallback to
`$XDG_CACHE_HOME/yeet` or `$HOME/.cache/yeet`. Valid adjacent output can still
be reused from a read-only directory. `--cache-dir PATH` explicitly chooses a
storage directory and reports an error if it cannot be used. `--no-cache` bypasses
reuse and retention, preparing temporary output under the user cache or explicit
cache directory and removing it when the session ends.

Preparation uses a private temporary directory, validates output, and checks that
the source has not changed before publishing. A completed MP4 is published using
an atomic, non-replacing hard link; its completion record is also published
without replacing a file. Only the record makes the output eligible for reuse.
Concurrent first-time producers may create duplicate valid generations.

Completed outputs survive natural completion, cancellation, and connection errors.
Partial session work is removed on handled failures/cancellation. There is no
automatic eviction or crash garbage collection yet. Obsolete generations and
crash leftovers require manual removal when not in use; remove a prepared MP4
with its matching sidecar. Original files are never modified.

## Local verification

See [README checks](../README.md#checks) for the current test commands. Automated
coverage distinguishes simulated/local verification from TV observations.

Coverage includes automatic selection and overrides; source/recipe invalidation;
corrupt output/metadata; symlink destinations; read-only fallback; concurrent
preparation; retention versus temporary cleanup; and cancellation. Real FFmpeg
tests verify copied encoded payloads and relative audio/video timing for H.264
with AC-3, HEVC Main 10 with multichannel AAC, and HEVC Main 10 with E-AC-3. The
existing subtitle, HTTP, simulated receiver, and lifecycle checks still pass.

The following additional checks used 30-second stream-copy clips from neutral
library aliases. Every row selected the expected mode, prepared and probed
successfully where needed, and reached the deliberately unavailable loopback
receiver. Each prepared case then reused the output with a nonexistent FFmpeg
path. These checks establish local preparation and reuse, **not TV playback**.

| Sample | Input | Profile | Selected mode |
| --- | --- | --- | --- |
| 009 | H.264 / stereo AAC-LC / MP4 | Extended | Direct |
| 004 | H.264 / six-channel AAC-LC / MP4 | Extended | Direct |
| 005 | HEVC Main 10 / six-channel AAC-LC / MP4 | Extended | Direct |
| 006 | H.264 / stereo AAC-LC / MKV | Extended | Remux |
| 051 | HEVC Main / six-channel AAC-LC / MKV | Extended | Remux |
| 008 | HEVC Main 10 / six-channel AAC-LC / MKV | Extended | Remux |
| 010 | HEVC Main 10 / six-channel HE-AAC / MKV | Extended | Audio |
| 001 | H.264 / six-channel E-AC-3 / MKV | Extended | Audio |
| 022 | HEVC Main 10 / six-channel E-AC-3 / MKV | Extended | Audio |
| 041 | H.264 / six-channel AC-3 / MP4 | Extended | Audio |
| 007 | AV1 / six-channel E-AC-3 / MKV | Extended | Transcode |
| 005 | HEVC Main 10 / six-channel AAC-LC / MP4 | Baseline | Transcode |

HE-AAC decoding/conversion was checked with sample 010. The installed fixture
encoder could not generate HE-AAC, so this case is not claimed as a generated
HE-AAC integration fixture. No real receiver was contacted by local verification.

## Hardware batch result

On 2026-09-18, the user completed the guided batch on the KPN DIW7022 receiver
and reported that all clips looked and sounded correct, with no issues noted.
The supplied final transcript contains `All batch checks passed.` and shows
the final Audio/Extended case reaching playback, stopping, and reporting
`Cancelled; cleanup completed.`

This confirms the short-clip matrix above on this receiver: automatic direct play,
remuxing, audio conversion, and full conversion; the Baseline override; external
SRT/WebVTT captions; natural completion; and persistent reuse with FFmpeg
unavailable. The script's final success also confirms its SIGINT exit-130,
temporary-output removal, and HTTP-port-closure assertions passed. Visual/audio
confirmation comes from the user's observations, separately from those assertions.

The batch used generated clips and adjacent prepared outputs under ignored
`samples/`. Read-only-folder fallback remains covered by automated tests, not a
separate hardware scenario. These results do not establish long-duration drift,
seek behavior on the expanded paths, or compatibility with other receivers.

## H.264 1080p50 direct-play result

The user reported successful original-file playback on the development KPN receiver after
enabling the former global high-frame-rate opt-in (now replaced by model rules). The tested MP4 contains
1920×1080 H.264 High Level 4.2, 50 fps, 8-bit 4:2:0 (`yuv420p`), BT.709 transfer,
and stereo AAC-LC at 48 kHz. An attached JPEG cover image is ignored when
assessing the video stream.

Previously, the 30 fps/Level 4.1 policy selected full transcoding. Local
verification with the opt-in enabled returned “Already compatible; no conversion
needed” with FFmpeg unavailable; the subsequent receiver playback confirmation
comes from the user's report. No source filename or personal device identifier
is recorded here.

This establishes a successful 1080p50/Level 4.2 trial on this receiver. The bundled
device database admits that case automatically for KPN DIW7022. A local model
override can raise the ceiling to 60 fps, but 59.94/60 fps has not yet been
hardware-validated. This report does not establish full-duration playback, seeking,
or subtitle behavior for this sample.

## Reproduce the guided hardware batch

From the repository, run:

```sh
bash scripts/validate-m5.sh "Living Room"
```

Replace the example device name. The optional second argument is an HTTP port
(default 8010). The script requires the neutral library aliases listed above,
FFmpeg/ffprobe, and the development receiver's Extended support. It builds the
CLI and generates short clips, external SRT/VTT, and logs under an ignored
`samples/m5-batch.*` directory. It does not modify original library files or store
prepared outputs beside those originals. `--prepare-only` creates the fixtures
without casting; this path and shell syntax were verified locally.

Allow roughly 15–20 minutes. For each clip, let it finish and confirm picture,
sound, timing and captions. Prepared cases repeat with FFmpeg unavailable to
verify reuse. The last case disables caching and asks for Enter during playback;
the script sends SIGINT and checks exit 130, temporary-file removal and port
closure. Failures stop the batch and preserve logs. No run is scheduled by these
instructions. Phone controls became unavailable during earlier tests; KDE now
provides seek/pause controls for separate long-play/sync checks, which remain
unconfirmed. Protocol success is never substituted for visual/audio confirmation.

## Stereo downmix

Audio encoding uses an explicit layout-aware FFmpeg stereo matrix. Centre and
surround coefficients are 0.70710678 before normalization (the usual -3 dB
contribution); centre reaches both left and right, and side/back surrounds go to
the corresponding side. LFE is not added to ordinary stereo. Matrix encoding is
`none`, and `rematrix_maxval=1` normalizes the sum to avoid overload at the mixing
stage. Existing mono/stereo audio is not given a dialogue boost. The audio remains
AAC-LC, 192 kbps, 48 kHz; stream-copy/remux paths do not alter samples.

This guards against dropped dialogue channels and overload during downmixing.
It does not guarantee subjective intelligibility, fix a poor original mix, or
limit every reconstructed AAC peak. No loudness compression, EQ or dialogue
enhancement is applied. See [FFmpeg's resampler controls](https://www.ffmpeg.org/ffmpeg-resampler.html).

Encoded-audio cache recipes include `stereo-matrix-v1`; older audio conversions
remain on disk but are not reused for the updated recipe. Remux and silent-video
recipes are unchanged. Synthetic 5.1, 5.1(side), and 7.1 tests verify centre,
surround routing and overload protection; generated media tests also decode the
resulting AAC and check centre-only audio is audible in both channels.
