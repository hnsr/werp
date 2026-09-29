# Media conversion

Werp prepares media for the selected receiver while avoiding unnecessary encoding.
The Rust backend shares this behavior between CLI casting, GUI casting and
convert-only. Receiver capabilities and observations belong in the
[device database](device-compatibility.md).

## Choosing a conversion

Automatic mode inspects the source and selects the first suitable path:

| Path | Action |
| --- | --- |
| Direct | Serve the original compatible MP4. |
| Remux | Copy compatible video/audio from MP4 or MKV into MP4 without quality loss. |
| Audio | Copy compatible video and encode audio as stereo AAC in MP4. |
| Transcode | Encode video as 8-bit H.264 and audio as stereo AAC in MP4; HDR-tagged input is experimental and has no tone mapping. |

Preparation produces a complete faststart MP4 before playback, keeping HTTP
seeking and reuse straightforward. There is no live encoding. The receiver app
launches only after media and subtitles are ready; a failed cast does not trigger
an automatic retry with different encoding.

CLI `--mode direct|remux|audio|transcode` requires that path or fails if its
constraints cannot be met. `--mode auto` is the default.

The CLI also has a [development-only bypass](../DEVELOPMENT.md#development-launch-options)
for testing original files outside these compatibility rules.

`--profile auto` uses the discovered model's bundled rules plus optional
[user overrides](device-compatibility.md#user-overrides). Unknown models and
`--host` use Baseline. Explicit profiles bypass model rules:

| Profile | Video admitted for direct/copy paths | Copied audio |
| --- | --- | --- |
| Baseline | H.264, 8-bit 4:2:0, up to Level 4.1/1080p30 | Mono/stereo AAC-LC |
| Extended | Baseline plus HEVC Main/Main 10, up to Level 4.0/1080p30 | AAC-LC through six channels |
| Experimental | Extended bounds | Also H.264/AC-3 passthrough for explicit trials |

Model rules can allow higher H.264 frame rates/levels. Normal profiles convert
Dolby and HE-AAC audio. Full encoding always targets Baseline, regardless of the
selected profile. Silent inputs remain silent.

PQ (`smpte2084`) and HLG (`arib-std-b67`) colour tags do not prevent direct
playback, remuxing or audio-only conversion. These paths preserve the video and
its colour signalling; the receiver handles rendering. We allow tags globally
because an H.264/PQ-tagged sample played without visible issues on the KPN DIW7022;
this is not a guarantee of HDR rendering on every receiver. No device flag is
required.

Video encoding of PQ/HLG-tagged sources is also allowed experimentally, including
automatic fallback, forced transcoding and subtitle burn-in. It uses the existing
8-bit H.264 pipeline without tone mapping; accepting tags does not guarantee HDR
preservation or correct SDR rendering. We removed the guard to permit practical
trials, and will revisit colour handling if playback issues arise.

Dolby Vision, ambiguous multiple audio/video tracks, and missing required metadata
remain errors. Tone mapping and explicit audio-track selection are not implemented.

[GUI convert-only](gui.md#convert-only-window) uses the selected device's rules,
or Baseline for **Broad compatibility**, without launching a receiver. It leaves
subtitles with the original file. During casting, text subtitles are served as
WebVTT; selected image subtitles require full video encoding with burn-in.
See [subtitle selection and delay](preferences-and-subtitles.md). Preparation
omits unselected tracks, attachments, source titles and chapters.

## Encoding and progress

FFmpeg runs as a subprocess; remuxing needs demuxer/muxer support but no encoders.
Full encoding uses libx264 veryfast/CRF 20, H.264 High/Level 4.1, 8-bit 4:2:0,
at most 1080p30, and an 8 Mbps maximum rate/16 Mbit buffer. Smaller inputs are not
intentionally enlarged. Encoded audio is AAC-LC at 192 kbps/48 kHz. Copied streams
retain their encoded payloads and are not subject to these encoder bitrate caps.

Preparation reports progress and has a 24-hour default deadline. Disk-space
estimates use source size for copy paths or bounded output rates for encoding,
plus muxing overhead and 64 MiB headroom. Checks before/during work do not reserve
space. Completion is reported only after output validation and publication;
handled cancellation/failure reaps FFmpeg and removes partial work.

## Stereo downmix

Audio encoding uses an explicit layout-aware FFmpeg matrix. Centre and surround
coefficients are 0.70710678 (−3 dB) before normalization: centre feeds both sides,
side/back surrounds feed the corresponding side, and LFE is omitted. Matrix
encoding is `none`; `rematrix_maxval=1` normalizes the sum to avoid mix overload.
Existing mono/stereo is not given a dialogue boost. Stream copying does not alter
samples.

This preserves dialogue-channel contributions without promising subjective
intelligibility or limiting every reconstructed AAC peak. No compression, EQ or
dialogue enhancement is applied. See [FFmpeg's resampler controls](https://www.ffmpeg.org/ffmpeg-resampler.html).

## Storage and reuse

Prepared files live beside the canonical source, following symlinks:

```text
movie.werp-<12-hex-key-tag>-<8-hex-generation>.mp4
movie.werp-<12-hex-key-tag>-<8-hex-generation>.mp4.json
```

The source stem is preserved up to 219 UTF-8 bytes, keeping filenames within a
255-byte budget. The short key tag and random generation keep names manageable;
reuse checks the full SHA-256 key and output digest, so a short-name collision
cannot select the wrong file. Existing files using older names remain reusable.

The cache key includes canonical source path, source SHA-256, mode, resolved
profile, image-subtitle track/delay for burn-in, and recipe versions. Encoded audio
includes `stereo-matrix-v1`; recipe versions distinguish output-affecting changes. External text-subtitle changes do not invalidate prepared video.

Reuse validates the completion record, output size/SHA-256 and fresh ffprobe
metadata against the profile. It reads source/output data from disk but avoids
encoding; FFmpeg is unnecessary for reusing media, while ffprobe remains required.
Missing, corrupt or mismatched entries are ignored.

If the source folder is unwritable, storage falls back to `$XDG_CACHE_HOME/werp`
(or `~/.cache/werp`); valid adjacent outputs can still be reused. `--cache-dir PATH`
selects a directory explicitly and fails if unusable. `--no-cache` skips reuse and
retention, using temporary output under the user cache or explicit directory.

New outputs are prepared privately, validated and published without replacing
existing files, after checking that the source has not changed. The completion
sidecar makes an output eligible for reuse. Concurrent producers may create
multiple valid generations. Originals are never modified.

Completed cached outputs survive completion, cancellation and connection errors;
partial work is removed on handled failure. There is no automatic eviction or
crash cleanup. Remove unused prepared MP4s together with their sidecars when they
are not in use.
