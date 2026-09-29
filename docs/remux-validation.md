# Stream-copy remux validation

Historical evidence from 2026-09-18. The initial implementation admitted only
H.264 with mono/stereo AAC and deleted temporary outputs after playback. Current
model-aware HEVC/surround support, automatic selection and retention are described
in [automatic playback](automatic-playback.md).

## Decision and implementation

Remuxing copies supported streams into a complete faststart MP4 without encoding,
avoiding quality loss and video-encoder cost when only the container is unsuitable.
Explicit maps exclude unselected tracks, attachments and source metadata/chapters.
Subtitles are prepared separately. FFmpeg demuxer/muxer support is required;
audio/video encoders are not. The source is never overwritten.

Preparation reuses bounded progress, cancellation, free-space checks, output
validation and cleanup. Probing checks the resolved profile, audio and duration
before the file is served. See the playback guide for current storage rules.

## Automated and local evidence

- A generated MKV with H.264 B-frames, delayed mono AAC, SubRip and an attachment
  produced only video/audio in MP4. Both encoded payload hashes were unchanged,
  relative start times matched within 3 ms, and mono audio stayed mono.
- Faststart, source preservation, audio-free inputs and rejection of lost audio
  were checked. Mock/CLI tests covered missing encoders (not required), invalid
  output, timeouts, cancellation, exit 130/143 and child/partial-file cleanup.
- Loopback sessions delivered remuxed video and external SRT/WebVTT. A local
  sample-006 preparation/failed-loopback check took about 0.87 seconds and cleaned
  temporary output. This was not a TV test or general performance estimate.

## Hardware playback result

The user reported no problems with the equivalent of:

```sh
cargo run --locked -- \
  samples/library/no-transcode-candidates/sample-006_h264-high-8bit_720x480_23.976fps_aac-lc-2ch.mkv \
  --device "Living Room" --mode remux --profile baseline --no-cache --no-subtitles --no-resume --http-port 8010
```

The requested checks covered picture, sound, sync and Ctrl+C on KPN DIW7022.
The reply was a general success report, without measurements or a cleanup
transcript. No subtitles were selected. This establishes one successful sample,
not every MKV/model. The later [M5 batch](automatic-playback.md#hardware-batch-result)
adds short-clip H.264/HEVC remux, captions, completion, reuse and cancellation
coverage. Expanded-path seeking and long-duration sync remain deferred.
