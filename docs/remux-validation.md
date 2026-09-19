# Stream-copy remux validation

These notes describe the earlier explicit-mode implementation. Current default
selection, broader profiles, and persistent storage supersede its temporary-only
behavior; see [automatic playback and reuse](automatic-playback.md). Commands
below use the current CLI spelling; the observations retain their original scope.

Implemented on 2026-09-18 with the existing Rust and FFmpeg/ffprobe toolchain.
No additional packages or dependencies were needed. The first hardware playback
trial succeeded; detailed evidence and limits are recorded below.

## Behavior and limits

`yeet FILE --mode remux` prepares a complete MP4 from Matroska/MKV or MP4 before contacting
the receiver. It copies one H.264 stream and zero or one mono/stereo AAC-LC stream
without encoding. The conservative video profile remains 8-bit 4:2:0, up to
1920×1080/30 fps and level 4.1; known HDR and missing required metadata are rejected.
HEVC, surround AAC, Dolby audio, and ambiguous audio selection remain unsupported
in this first version. No automatic fallback or default-policy widening occurs.

Explicit stream maps exclude embedded subtitles and attachments. Source metadata
and chapters are omitted. External SRT/WebVTT uses the existing subtitle path.
Remuxing requires FFmpeg's demuxer/muxer support but no audio/video encoders.

The preparation code reuses progress, bounded subprocess handling, cancellation,
private session storage, and cleanup. `--cache-dir` can choose storage for
remuxing as well. Space estimation uses the source size plus 10% overhead and
64 MiB headroom, with free-space checks during progress. The output has faststart
metadata; probing verifies its conservative playback profile, audio parameters,
and duration before it is served. The original is never overwritten.

Choose one preparation path using `--mode`.

## Automated and local evidence

- All 50 workspace tests passed, including all five opt-in FFmpeg tests.
  Formatting, Clippy with warnings denied, and diff whitespace checks passed.
- A generated MKV includes H.264 with B-frames, mono AAC with a delayed start,
  embedded SubRip, and an attachment. The resulting MP4 contains only video/audio.
  SHA-256 stream hashes confirm that both encoded payloads are unchanged. Relative
  audio/video start timestamps match within 3 ms; mono audio remains mono.
- Output has faststart metadata, source bytes remain unchanged, and the prepared
  file is removed after close. A separate check verifies audio-free input and
  rejects unexpected loss of audio during output validation.
- Mock subprocess tests exercise remuxing without encoder enumeration, failures,
  invalid output, timeouts, and cancellation. CLI SIGINT/SIGTERM checks verify
  exit codes 130/143, child reaping, and partial-file removal.
- Full sessions remux generated MKV, serve the prepared MP4 to a loopback receiver,
  deliver external SRT/WebVTT, and remove session files after completion.
- A local CLI run prepared the actual `sample-006`, then deliberately encountered
  a refused loopback connection. Preparation and cleanup took about 0.87 seconds
  on the development machine. The preparation directory was empty afterward;
  source size and modification time were unchanged. This is not a performance
  guarantee for all files or disks. No TV was contacted.

## Hardware playback result

```sh
cargo run --locked -- \
  samples/library/no-transcode-candidates/sample-006_h264-high-8bit_720x480_23.976fps_aac-lc-2ch.mkv \
  --device "Living Room" --mode remux --profile baseline --no-cache --http-port 8010
```

The user reported no problems running the equivalent command on the existing KPN DIW7022
receiver. The requested checks covered picture, sound, synchronization, and Ctrl+C;
the reply was a general success report without per-check details or a terminal
transcript. This is initial hardware confirmation for this remuxed sample, not
all MKVs or receiver models. Embedded subtitles were not selected.

Natural completion, seeking, external subtitle alignment, and explicit exit-code,
cache-directory, and HTTP-port checks remain useful follow-up hardware validation.
Keep these separate from the automated and local cleanup evidence above.
