# Local media inventory

This is the initial inventory and early trial history. Commands use current CLI
spellings; historical compatibility limits and test status below are superseded
by [automatic playback and reuse](automatic-playback.md).

Scanned 2026-09-17 using ffprobe 8.1.2. Recursively found and successfully probed
**75 videos (88.63 GiB): 45 Matroska/MKV and 30 MP4**. No probe failures.
Also found **230 external SRT files**; their contents and matching video/language
associations were not checked. Archives were not unpacked. Source media was not
modified, copied, decoded in full, or cast to a receiver during the scan.
Subsequent user playback checks are recorded below.

## Two provisional sets

| Set | Files | Meaning |
| --- | ---: | --- |
| `samples/library/no-transcode-candidates/` | 73 | Worth testing without video or audio encoding: 28 original MP4 candidates and 45 MKV candidates that may need stream-copy remuxing. Includes one low-confidence AV1 case. |
| `samples/library/likely-transcode-or-review/` | 2 | Unusual 8-bit H.264 with PQ/BT.2020 signalling. Review colour correctness before choosing conversion or a metadata repair. |

These sets are metadata-based hypotheses, not blanket playback guarantees.
The CSV records the original scan assessment; subsequent playback evidence is
recorded separately below.
No file is proven to require transcoding by this scan. The second set identifies
the strongest video-conversion concerns, not an unconditional instruction to encode.
Files can move between sets after actual receiver tests. A receiver without HEVC
or AV1 support would move those video codecs into its conversion set; audio
support also depends on the output configuration.

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

| Container | Video | Audio | Count | First route to test | Representative |
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
  with E-AC-3 surround audio. Keep it as an experimental no-encoding candidate;
  its Cast decoder support has not been established on the test receiver.
- All videos are at most 1920×1080, about 23.976 or 24 fps, with one video and
  one audio stream. No 4K, high-frame-rate, DTS, or TrueHD cases were found.
- **Audio:** 47 AAC-LC (20 stereo, 27 six-channel), 14 HE-AAC six-channel,
  9 AC-3 six-channel, and 5 E-AC-3 six-channel (3 report Dolby Atmos).
  All are sampled at 48 kHz. A codec listed in metadata does not establish
  successful receiver output, correct downmix, or preservation of Atmos.
- **Embedded subtitles:** 145 SubRip tracks, 16 mov_text tracks, and 11 PGS
  tracks across 53 files. Each of the 11 files containing PGS also has SubRip.
  Text extraction/conversion can avoid video encoding; selecting only a bitmap
  track could require burn-in later. No subtitle was selected in this scan.

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

Yeet's current M2 guard is intentionally narrower: MP4-family H.264 with
mono/stereo AAC-LC and conservative video limits. Only **6** library files fit
that apparent format envelope; three now have user-confirmed picture and sound.
Inventory
membership does not bypass that guard. The experimental direct-play path
(`--mode direct --profile experimental`) additionally admits one 3–6 channel AAC-LC track and HEVC Main/Main 10
MP4 up to level 4.0 at 1080p30, with 8/10-bit 4:2:0 pixels and no known HDR
signalling. It serves original bytes and reports receiver-dependent support.
H.264/AC-3 MP4 trials are also enabled for one 1–6 channel audio track at 32,
44.1, or 48 kHz; `sample-041` produced video but no audible sound on the test setup.
HEVC/AC-3, E-AC-3, MKV,
and HDR cases remain outside the experimental policy. The user has reported successful
experimental HEVC playback of `sample-005` on the test receiver.

## User playback results

On 2026-09-17, the user confirmed good video and sound for all three suggested
MP4 H.264/AAC-stereo baseline samples using the existing direct-play path:

| Sample | Video dimensions / frame rate | Picture | Sound |
| --- | --- | --- | --- |
| `sample-009` | 1920×816 / 24 fps | Confirmed good | Confirmed good |
| `sample-014` | 1280×690 / 24 fps | Confirmed good | Confirmed good |
| `sample-017` | 1920×800 / approximately 23.976 fps | Confirmed good | Confirmed good |

This establishes real-file picture/audio playback for these three samples on the
existing test receiver. No remuxing or transcoding was needed. It does not establish
support on other receivers or validate the three remaining files in this group.
Subtitles, full-duration playback, explicit A/V synchronization checks, natural
completion, and shutdown were not reported for these runs. Earlier generated
fixture results remain separate evidence.

On 2026-09-18, the user reported that `sample-004` also works with
`--mode direct --profile experimental` on the existing KPN DIW7022 receiver. This serves
the original MP4 containing H.264 High, 1920×1040 at approximately 23.976 fps,
and six-channel AAC-LC, without remuxing or transcoding. The requested check
covered picture, audible dialogue, and A/V synchronization; the user reported
general success without separate measurements. This does not establish discrete
surround output, long-duration synchronization, or compatibility with all files
in the group or other receivers. No external subtitle was selected for this run.

The user also reported success on 2026-09-18 for original-file playback of
`sample-005` with `--mode direct --profile experimental` on the KPN DIW7022: MP4 with
HEVC Main 10, 10-bit 4:2:0, 1920×1080 at approximately 23.976 fps, and six-channel
AAC-LC. The suggested checks covered picture/colours, dialogue, synchronization,
and phone-controlled seeking/pause/resume. The user reported general success
without separate measurements or per-check details. No subtitles were selected.
This is evidence for this file/receiver combination, not universal HEVC support
or confirmation of discrete surround output or full-duration playback.

On 2026-09-18, the user tested original-file playback of `sample-041` with
`--mode direct --profile experimental` on the same KPN DIW7022 setup: H.264 High,
1920×804 at 24 fps, with six-channel AC-3 at 48 kHz in MP4. Video played, but
there was no audible sound. This is a failed audio-output trial, not successful
direct playback. It does not isolate the cause between receiver decoding,
passthrough/output configuration, or the particular file. Other receivers and
the remaining AC-3 files have not been tested. Keep the original scan's candidate
classification as a hypothesis; do not promote AC-3 into the default policy.
The subsequent `--mode audio` trial produced audible sound for roughly
one or two seconds, then returned to the Google TV home screen. Picture was not
observed. The CLI reported Playing, then Buffering, and eventually a 30-second
playable-state timeout. This is not a successful playback result; the cause of
the receiver exit remains unknown. A locally recreated output preserves the full
encoded video payload, has matching duration and stereo AAC, and decodes its first
minute without FFmpeg errors.

The user subsequently reported no issues casting the saved H.264/stereo AAC
output directly with verbose logging. The inspected log shows successful HTTP
206 responses, PLAYING positions through approximately 298.6 seconds, and
pause/resume transitions without warnings or errors. At inspection the session
was paused; natural completion and cleanup were not established by this log.
This supports playback of the video-copy/audio-converted output on this setup;
it is not a repeat of the integrated preparation step. The user reports prior
intermittent startup cutouts on this device. That is a plausible explanation
for the earlier failure, but its cause remains unresolved and the diagnostic
changes are not proven to have fixed it.

A later integrated `--mode audio` run of `sample-041` also passed: the user
reported good sound and no issues, then interrupted playback with Ctrl+C.
The CLI reported cleanup completed and exit code 130; the dedicated preparation
directory was empty and the HTTP port was closed. Natural completion and
receiver-initiated stop remain separate, unverified hardware checks for this mode.

The user also reported no problems playing MKV H.264/AAC-stereo `sample-006`
with `--mode remux` on the KPN DIW7022. The suggested check covered picture, sound,
synchronization, and Ctrl+C; no separate measurements or cleanup transcript were
provided. This confirms an initial successful remux trial for this sample/setup.
No embedded or external subtitles were selected. See [remux validation](remux-validation.md).

## Suggested M5 test order

1. MP4 H.264/AAC-stereo picture and sound now pass for three real files above.
   External subtitles and lifecycle checks on these files remain to be exercised.
2. Original MP4 H.264/AAC-surround now has a successful trial with `sample-004`.
   MP4 HEVC Main 10/AAC also has a successful trial with `sample-005`.
   H.264/AC-3 `sample-041` played video but had no audible sound. A saved output
   with copied video and stereo AAC now has a successful playback retry after
   an unexplained initial startup failure. Integrated audio conversion, playback,
   and Ctrl+C cleanup now also pass. Natural completion and receiver-initiated
   stop remain unverified for this mode.
   Confirm picture, colour, audible audio, timing, completion, and cancellation.
3. MKV H.264/AAC-stereo `sample-006` now has a successful `--mode remux` hardware
   trial, alongside the earlier local preparation and cleanup check.
   Then evaluate HEVC Main/Main 10 with AAC/HE-AAC and H.264/HEVC with Dolby audio.
   These broader profiles and original MKV delivery are not enabled by `--mode remux`. Preserve selected audio/video and handle
   subtitle extraction separately rather than copying every stream blindly.
4. Try the AV1/E-AC-3 sample as a separate low-confidence case.
5. Inspect the two HDR review samples before considering any conversion. If an
   otherwise compatible case fails only on audio, prefer audio-only conversion
   when encoding work resumes.

A failed LOAD is evidence to diagnose (container, codec, server requests, and
receiver response), not enough by itself to conclude video encoding is required.
No automatic encoding fallback is wanted during the initial compatibility work.
Record original-file and remuxed-file outcomes separately by receiver model.

## Local samples and privacy

There is one neutral symlink per video, including codec/profile, bit depth,
dimensions, frame rate, and audio format in its name. These are references to the
original files, not copies or anonymized media. Moving an original breaks its link.
The numeric IDs identify this scan and do not promise stable rescanning order.

All symlinks and detailed local artifacts live under Git-ignored `samples/`.
**Do not force-add the symlinks:** their targets contain personal source paths.
The shareable report and CSV contain only neutral aliases and technical metadata;
no personal device name, original filename, or source path is included.
