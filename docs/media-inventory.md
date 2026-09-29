# Local media inventory

This is the initial inventory and early trial history. It is not a live inventory of the Downloads directory. Current policy is described
in [media conversion](media-conversion.md); original hypotheses below are
retained separately from observed outcomes.

Scanned 2026-09-17 using ffprobe 8.1.2. Recursively found and successfully probed
**75 videos (88.63 GiB): 45 Matroska/MKV and 30 MP4**. No probe failures.
Also found **230 external SRT files**; their contents and matching video/language
associations were not checked. Archives were not unpacked. Source media was not
modified, copied, decoded in full, or cast to a receiver during the scan.
Subsequent user playback checks are recorded below.

## Original candidate sets

| Set | Files | Meaning |
| --- | ---: | --- |
| `samples/library/no-transcode-candidates/` | 73 | Worth testing without video or audio encoding: 28 original MP4 candidates and 45 MKV candidates that may need stream-copy remuxing. Includes one low-confidence AV1 case. |
| `samples/library/likely-transcode-or-review/` | 2 | Unusual 8-bit H.264 with PQ/BT.2020 signalling. Review colour correctness before choosing conversion or a metadata repair. |

These sets are metadata-based hypotheses, not blanket playback guarantees.
The CSV records the original scan assessment; subsequent playback evidence is
recorded separately below.
No file is proven to require transcoding by this scan. The second set identifies
the strongest video-conversion concerns, not an unconditional instruction to encode.
The directory names retain the original hypotheses; they do not drive runtime
selection. Current rules may require audio or video encoding for a file in the
no-transcode-candidates set.

**Direct play** serves the original file. **Remuxing** changes its container while
copying encoded streams, without quality loss from re-encoding, but still requires
preparation time and output space. **Transcoding** re-encodes video and/or audio.
The first set includes direct-play and remux candidates to reflect the priority
of avoiding encoding. Remuxing alone cannot fix an unsupported codec.

## Format combinations

Each row groups container, video codec/profile/bit depth, HDR signalling, and audio
codec/profile/channel count. Different dimensions, frame rates, and codec levels
remain visible per file in [the complete CSV](media-inventory.csv). Sample links
work only on the development machine and point to ignored local symlinks.

| Container | Video | Audio | Count | Original trial hypothesis | Representative |
| --- | --- | --- | ---: | --- | --- |
| MKV | AV1 Main, 10-bit | EAC3, 6 ch | 1 | Original trial / copy-remux; AV1 uncertain | [sample-007](../samples/library/no-transcode-candidates/sample-007_av1-main-10bit_1620x1080_23.976fps_eac3-6ch.mkv) |
| MKV | H264 High, 8-bit | AAC LC, 2 ch | 13 | Original trial / copy-remux | [sample-006](../samples/library/no-transcode-candidates/sample-006_h264-high-8bit_720x480_23.976fps_aac-lc-2ch.mkv) |
| MKV | H264 High, 8-bit | AC3, 6 ch | 1 | Original trial / copy-remux | [sample-024](../samples/library/no-transcode-candidates/sample-024_h264-high-8bit_1790x1080_24fps_ac3-6ch.mkv) |
| MKV | H264 High, 8-bit | EAC3 Dolby Digital Plus + Dolby Atmos, 6 ch | 3 | Original trial / copy-remux | [sample-001](../samples/library/no-transcode-candidates/sample-001_h264-high-8bit_1920x1080_24fps_eac3-atmos-6ch.mkv) |
| MKV | HEVC Main, 8-bit | AAC LC, 6 ch | 10 | Original trial / copy-remux | [sample-051](../samples/library/no-transcode-candidates/sample-051_hevc-main-8bit_1920x1080_23.976fps_aac-lc-6ch.mkv) |
| MKV | HEVC Main 10, 10-bit | AAC HE-AAC, 6 ch | 14 | Original trial / copy-remux | [sample-010](../samples/library/no-transcode-candidates/sample-010_hevc-main-10-10bit_1920x800_23.976fps_aac-he-aac-6ch.mkv) |
| MKV | HEVC Main 10, 10-bit | AAC LC, 6 ch | 2 | Original trial / copy-remux | [sample-008](../samples/library/no-transcode-candidates/sample-008_hevc-main-10-10bit_1624x1080_23.976fps_aac-lc-6ch.mkv) |
| MKV | HEVC Main 10, 10-bit | EAC3, 6 ch | 1 | Original trial / copy-remux | [sample-022](../samples/library/no-transcode-candidates/sample-022_hevc-main-10-10bit_1920x1080_23.976fps_eac3-6ch.mkv) |
| MP4 | H264 High, 8-bit | AAC LC, 2 ch | 6 | Original MP4 | [sample-009](../samples/library/no-transcode-candidates/sample-009_h264-high-8bit_1920x816_24fps_aac-lc-2ch.mp4) |
| MP4 | H264 High, 8-bit | AAC LC, 6 ch | 7 | Original MP4 | [sample-004](../samples/library/no-transcode-candidates/sample-004_h264-high-8bit_1920x1040_23.976fps_aac-lc-6ch.mp4) |
| MP4 | H264 High, 8-bit | AC3, 6 ch | 8 | Original MP4 | [sample-041](../samples/library/no-transcode-candidates/sample-041_h264-high-8bit_1920x804_24fps_ac3-6ch.mp4) |
| MP4 | H264 High, 8-bit **PQ/BT.2020** | AAC LC, 2 ch | 1 | Review HDR; possible encoding | [sample-021](../samples/library/likely-transcode-or-review/sample-021_h264-high-8bit_1280x720_23.976fps_aac-lc-2ch_pq-bt2020.mp4) |
| MP4 | H264 High, 8-bit **PQ/BT.2020** | AAC LC, 6 ch | 1 | Review HDR; possible encoding | [sample-073](../samples/library/likely-transcode-or-review/sample-073_h264-high-8bit_1920x1038_23.976fps_aac-lc-6ch_pq-bt2020.mp4) |
| MP4 | HEVC Main 10, 10-bit | AAC LC, 6 ch | 7 | Original MP4 | [sample-005](../samples/library/no-transcode-candidates/sample-005_hevc-main-10-10bit_1920x1080_23.976fps_aac-lc-6ch.mp4) |

## What the metadata tells us

- **40 H.264 files:** all High profile, 8-bit 4:2:0, level 3.0–4.1.
  Two have the unusual HDR signalling described below; 38 do not.
- **34 HEVC files:** 10 Main/8-bit and 24 Main 10/10-bit, all level 4.0.
  None reports PQ or HLG transfer characteristics. Missing colour metadata is
  not proof of SDR, and 10-bit alone is not proof of HDR.
- **1 AV1 file:** Main profile, 10-bit 4:2:0, level 4.0, 1620×1080,
  with E-AC-3 surround audio. It was an experimental no-encoding hypothesis;
  native Cast support remains unverified and current automatic mode converts it.
- All videos are at most 1920×1080, about 23.976 or 24 fps, with one video and
  one audio stream. No 4K, high-frame-rate, DTS, or TrueHD cases were found.
- **Audio:** 47 AAC-LC (20 stereo, 27 six-channel), 14 HE-AAC six-channel,
  9 AC-3 six-channel, and 5 E-AC-3 six-channel (3 report Dolby Atmos).
  All are sampled at 48 kHz. A codec listed in metadata does not establish
  successful receiver output, correct downmix, or preservation of Atmos.
- **Embedded subtitles:** 145 SubRip tracks, 16 mov_text tracks, and 11 PGS
  tracks across 53 files. Each of the 11 files containing PGS also has SubRip.
  Text extraction avoids video encoding; selecting PGS now uses implemented
  burn-in. No subtitle was selected during the original scan.

The CSV preserves ffprobe's raw codec-specific `level` values: H.264 uses ten
units per level, HEVC uses thirty, and AV1 uses a level index (8 means 4.0).
Bitrate values are container averages, not measured peaks. The local
`samples/library/inventory.json` retains whitelisted stream metadata, including
language tags, dispositions, and any reported side data, without source names.

### The two HDR review cases

`sample-021` (1280×720, AAC stereo) and `sample-073` (1920×1038, AAC six-channel)
combine H.264 High/yuv420p with `smpte2084` transfer and BT.2020 primaries.
They need an explicit colour check. If the pixels genuinely represent HDR and
the receiver cannot render that combination, tone mapping plus video encoding
may be necessary. If the signalling is wrong, a metadata/bitstream correction
might suffice. Do not apply tone mapping or strip HDR metadata automatically.
HDR conversion remains deferred work.

## Receiver evidence and limits

Google lists MP4 and WebM containers, but not general Matroska/MKV. H.264 has broad
coverage; HEVC and AV1 vary by device generation. Dolby audio passthrough depends
on the receiving setup. Therefore MKV files are container experiments or remux
candidates, and HEVC/AV1/Dolby files need receiver-specific validation.
[Google Cast media support](https://developers.google.com/cast/docs/media).

KPN documents AAC/HE-AAC, Dolby audio and HDR capabilities for the DIW7022, but
warns that hardware capabilities need not be available in every application.
This supports trying compatible streams before encoding; it does not establish
Default Media Receiver compatibility or AV1 support.
[KPN TV+ Box specifications](https://community.kpn.com/kennisbank-kpn-tv-box-149/specificaties-kpn-tv-box-573098).

At the time of the scan, only six files appeared to fit the narrow M2 guard.
Current KPN model rules also admit bounded HEVC and multichannel AAC-LC, remux
MKV to MP4, convert unsupported audio, and fully convert AV1. Native AC-3 failed
in the trial below and is not promoted into the normal policy. Native AV1, native
MKV, 4K/HDR and untested audio passthrough are not established by this inventory.

## User playback results

These early observations concern the KPN DIW7022 development setup. Direct trials
served original MP4 bytes; the wider cases initially used explicit experimental
permissions, now represented by bounded model rules.

| Sample | Format/path | User observation | Evidence limits |
| --- | --- | --- | --- |
| 009 / 014 / 017 | H.264/stereo AAC MP4; 1920×816/24, 1280×690/24, 1920×800/~23.976 | Good picture and sound for all three. | No full-duration, subtitle or cleanup report for these runs. |
| 004 | H.264 1920×1040/~23.976, six-channel AAC; original MP4 | General success for requested picture/dialogue/sync check. | Discrete surround and long-duration sync not established. |
| 005 | HEVC Main 10 1920×1080/~23.976, six-channel AAC; original MP4 | General success; requested checks included colours, dialogue, sync and phone controls. | No separate per-check measurements; no subtitles selected. |
| 041 | H.264 1920×804/24, six-channel AC-3; original MP4 | Picture, but no audible sound. | Cause not isolated between decoding, output setup and file; other AC-3 files untested. |
| 041 | Copied video/stereo AAC preparation | First attempt exited to the TV home screen after brief sound. Later saved-output playback and integrated conversion succeeded. | Startup failure cause remains unresolved. |
| 006 | H.264/stereo AAC MKV → MP4 remux | General success for picture/sound/sync/Ctrl+C request. | No per-check measurements or cleanup transcript. |

For sample-041, the successful saved-output log had HTTP 206, positions through
about 298.6 seconds and pause/resume, ending paused. The later integrated run
reported good sound, exit 130 after Ctrl+C, an empty preparation directory and a
closed HTTP port. The user had seen similar intermittent receiver startup issues,
but this is not proof of the earlier failure's cause or of a fix. See the
[receiver-stop handling](decisions/001-cast-library.md#follow-up-fixes).

The [device observations](../crates/yeet-core/data/devices.toml) later confirmed the
short-clip direct/remux/audio/full-conversion matrix with captions and reuse.
This supersedes the original proposed test order. Expanded-path seeking,
long-duration sync and the two HDR review cases remain open; full-conversion
success for AV1 does not establish native AV1 playback. Original-file, remux and encoding results are distinct observations.

## Local samples and privacy

There is one neutral symlink per video, including codec/profile, bit depth,
dimensions, frame rate, and audio format in its name. These are references to the
original files, not copies or anonymized media. Moving an original breaks its link.
The numeric IDs identify this scan and do not promise stable rescanning order.

All symlinks and detailed local artifacts live under Git-ignored `samples/`.
Symlink targets contain personal source paths; see the
[contributor privacy rules](../DEVELOPMENT.md#local-samples-and-privacy).
The shareable report and CSV contain only neutral aliases and technical metadata;
no personal device name, original filename, or source path is included.
