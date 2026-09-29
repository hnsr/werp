# Transcoding validation

Historical evidence from the explicit full/audio-conversion work of 2026-09-17–18.
Current automatic selection and persistent outputs supersede the initial
restricted profiles and temporary-only storage; see [automatic playback](automatic-playback.md).
Commands below use current CLI spellings.

## Full conversion: decision and local evidence

Full conversion was added as an explicit escape hatch before automatic selection.
It prepares a complete MP4 before receiver launch, keeping HTTP seeking simple.
The established SDR target is H.264 High/level 4.1, 8-bit 4:2:0, up to 1080p30,
libx264 veryfast/CRF 20, 8 Mbps maximum rate/16 Mbit buffer, and stereo AAC
192 kbps/48 kHz when audio exists. Silent inputs remain silent; smaller images
are not deliberately enlarged. Known HDR and ambiguous tracks fail.

Real FFmpeg checks converted generated FFV1/PCM surround media, downscaled a
1922×1082/60 fps input, verified faststart/profile/duration and unchanged sources,
and exercised SRT/WebVTT through loopback receivers. Mock and CLI checks covered
encoder availability, space estimates, bounded progress/errors, invalid output,
timeouts and SIGINT/SIGTERM child/partial-output cleanup. The initial short-clip
local preparation/failed-loopback run took about 0.63 seconds; this was not a
full-length performance estimate or TV playback test.

Current space/deadline and downmix behavior is documented in the playback guide.
Originally all outputs were temporary; successful prepared files are now retained
unless `--no-cache` is requested. Neither original source media nor unrelated files
are overwritten.

## Audio-only preparation

Audio conversion was added after an H.264/AC-3 MP4 produced picture but silence.
The first implementation copied H.264 from MP4 and encoded stereo AAC; current
profiles also support bounded HEVC and MKV inputs. Copying video avoids the cost
and quality loss of unnecessary video encoding.

A generated H.264/AC-3 fixture with B-frames and delayed audio verified unchanged
encoded-video SHA-256, non-silent AAC, faststart, duration and relative audio/video
start times within 50 ms. Local recreation of sample-041 preserved its entire
video payload and decoded the first minute without errors. These checks do not
establish perceptual sync on the receiver.

The first sample-041 integrated run briefly played sound, then exited to the TV
home screen; picture was not observed and the CLI eventually timed out. A later
cast of the prepared output had no reported issues; logs showed HTTP 206,
PLAYING positions through about 298.6 seconds and pause/resume. That log ended
paused, so did not establish completion or cleanup. A subsequent integrated
conversion/playback run passed with good sound, Ctrl+C, exit 130, an empty
preparation directory and a closed HTTP port.

The user reported similar intermittent receiver startup issues. Their relation
to this failure is unproven. During investigation, stale BUFFERING/empty media
replies gained receiver-application presence checks: a confirmed app exit now
ends as receiver-stopped, while an unqueryable receiver or an app still running
remains an error. This improves diagnosis; it is not a proven fix for the original
media exit. Tests cover both exited and still-running apps.

## Hardware feedback and remaining checks

All observations below concern the KPN DIW7022 development setup.

| Trial | Observed result | Limits |
| --- | --- | --- |
| Generated short full-conversion fixture | User reported successful playback. | Initial path confirmation only. |
| sample-004, full conversion from H.264/six-channel AAC | Video/audio, phone-controlled seek, pause/resume and stop passed. | No subtitles selected; no separate channel-balance, full-duration or cleanup report. |
| 30-second forced conversion, SRT and WebVTT | User reported captions, natural completion and exit 0. | Filesystem/port checks were separate. |
| Natural completion and phone stop cleanup | Preparation directory empty; no HTTP listener. | The first phone stop still returned timeout errors/exit 1. |
| Phone stop after lifecycle fix | `Playback stopped on receiver.`, exit 0, no timeout/cleanup warnings. | Filesystem/port checks were not repeated in this transcript. |
| Integrated sample-041 audio conversion | Good sound, Ctrl+C cleanup, exit 130, empty directory and closed port. | Receiver-initiated stop and long-duration sync were not established. |

The phone-stop fix handles app-channel CLOSE and failed polls by checking that
the owned receiver application ended. Owned CANCELLED media status also ends
normally; Yeet skips a redundant STOP. Transport timeout alone never proves a
normal stop. Foreign app closes and closes while the owned app remains active
are covered by simulated regressions.

To repeat a short forced-conversion check:

```sh
cargo run --locked -- samples/short/test.mp4 \
  --device "Living Room" --mode transcode --no-cache --no-resume \
  --subtitles samples/short/subtitles.vtt --http-port 8010
```

Generate the fixture using [CLI cleanup instructions](../README.md#cli-cleanup-check).
The later [M5 batch](automatic-playback.md#hardware-batch-result) adds integrated
completion/reuse/cancellation evidence across preparation modes. Detailed seek
synchronization and long-duration stability remain unconfirmed; phone controls
became unavailable during the earlier tests, and KDE now offers another test path.
Embedded subtitle extraction and PGS burn-in have since been implemented and
validated separately in [subtitle notes](preferences-and-subtitles.md).
