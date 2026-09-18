# Forced-transcoding validation

Implemented on 2026-09-17 with Rust 1.96.0 and FFmpeg/ffprobe 8.1.2 on Fedora 44.
No additional system packages were needed. This is an explicit escape hatch;
automatic conversion decisions remain future work. Explicit stream-copy remuxing
was added later; see [remux validation](remux-validation.md).

## Behaviour

`cast --force-transcode` always encodes the selected video and any audio before
receiver discovery/connection. The first version accepts one video and at most
one audio track, with known positive duration and frame rate. It rejects known
PQ/HLG and Dolby Vision metadata; it does not implement HDR tone mapping.
Subtitles use the existing explicit external SRT/WebVTT route.

Output is MP4 with CPU-encoded H.264 High/level 4.1, 8-bit 4:2:0, at most
1920×1080/30 fps, with stereo AAC at 48 kHz when audio exists. Smaller inputs
are not deliberately enlarged; the scale filter retains display aspect ratio.
The current settings are libx264 veryfast/CRF 20, 8 Mbps maximum video rate,
16 Mbit encoder buffer, and AAC 192 kbps. Silent inputs stay silent. Only the
selected video/audio streams are mapped, so attachments, embedded subtitles,
and source titles/chapters are not copied into the output.

FFmpeg progress is consumed incrementally with a bounded line buffer and bounded
stderr diagnostics. Source audio/video timestamp offsets share an input timeline;
output duration and the conservative playback profile are checked afterward.
Progress reaches 100% only after output validation. This is metadata validation,
not a complete verification of perceptual quality or synchronization.

Preparation uses a private session directory under the user's cache, or an
explicit `--transcode-dir`. An upper-rate estimate with overhead and 64 MiB
headroom is checked before encoding; progress updates check remaining headroom.
This does not reserve space against other writers. The default encoding deadline
is 24 hours, separate from the probe timeout. Handled interruption, conversion
failure, invalid output, and session termination remove temporary media. A crash
or SIGKILL can leave files; there is no cross-session cache reuse or eviction yet.

## Automated evidence

- The complete workspace suite passed **43 tests**, including all three tests
  normally gated behind real FFmpeg availability. No tests were skipped in that run.
- Formatting and Clippy with warnings denied passed.
- The default direct-play guard remains active without either explicit option.
  Experimental direct play and forced transcoding cannot be combined.
- Encoder availability, known HDR/Dolby Vision, ambiguous audio selection,
  insufficient space, subprocess errors, output validation failures, cancellation,
  deadlines, and bounded progress parsing are checked.
- Real FFmpeg tests convert generated FFV1/PCM MKV with six-channel audio into
  H.264/stereo AAC MP4. Another generated input exercises downscaling from
  1922×1082 and frame-rate reduction from 60 fps with no audio track.
- Generated output satisfies the conservative media guard, retains expected
  duration, and has its MP4 metadata before media data. Source bytes are unchanged.
- Complete sessions against a loopback TLS receiver fetch the prepared MP4 and
  external SRT-converted/WebVTT subtitles. Compatible source MP4s produce different
  output bytes when forced, proving that the option does not silently stream-copy.
  Prepared media is removed after completion.
- CLI SIGINT and SIGTERM tests interrupt the encoding stage, verify exit codes
  130/143, reap the child process, and remove partial output. Existing session and
  process lifecycle regressions also pass.

A separate real CLI smoke run prepared the existing 30-second generated fixture,
reported 100%, and then deliberately encountered a refused connection on a closed
loopback port. Cleanup left the preparation directory empty and source size/mtime
unchanged. The whole run took approximately **0.63 seconds** on the development
machine. This low-resolution synthetic fixture is not a performance estimate for
full-length 1080p movies. The run did not contact a real receiver.

Socket and signal tests ran outside the development sandbox, whose restrictions
prevent representative socket/signal behaviour. The tests target loopback only.

Reproduce the automated checks:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace -- --include-ignored
```

## Audio-only preparation

Added on 2026-09-18: `--transcode-audio` preserves conservative-profile H.264
video from MP4 and converts its single audio track to stereo AAC at 48 kHz,
192 kbps. It requires AAC encoding but no video encoder. Initial input limits
exclude HEVC, MKV, known HDR, missing audio, and multiple audio tracks. This is
an explicit option; direct play does not automatically retry with conversion.

Preparation uses the existing subprocess, progress, cancellation, private-cache,
output-validation, and cleanup flow. Copied video has no encoder bitrate cap, so
space estimation uses the source file size plus new AAC audio and muxing overhead.
The complete prepared MP4 has faststart metadata and is served only after probing.

Validation includes a generated H.264/AC-3 MP4 with an audible tone, B-frames,
and audio starting later than video. The check compares SHA-256 hashes of the
copied encoded video payload, decodes AAC to verify non-silent output, and compares
relative audio/video start timestamps within 50 ms. It also checks source
preservation and faststart metadata. This does not establish perceptual sync on
hardware or long-duration drift.

Tests also cover policy rejection, AAC-only encoder availability, failure and
invalid-output cleanup, timeouts, SIGINT/SIGTERM, and full loopback sessions with
external SRT and WebVTT. The hardware retry for `sample-041` briefly produced
sound, then returned to the Google TV home screen; picture was not observed.
The CLI waited in Buffering and timed out. A subsequent direct cast of the saved
converted output was reported to have no issues. Its verbose log shows HTTP 206
responses, PLAYING positions through about 298.6 seconds, and pause/resume, without
warnings or errors. The inspected log ends in PAUSED; it does not verify natural
completion or cleanup. That retry did not rerun the integrated preparation step.
The user reports similar intermittent startup problems on this device; the
original failure's cause remains unresolved.

The user subsequently reran the complete `--transcode-audio` workflow for
`sample-041` with a dedicated preparation directory and interrupted playback
with Ctrl+C. Sound was good and no issues were reported. The CLI reached Playing,
then printed `Cancelled; cleanup completed.` and returned exit code 130.
`ls -A` showed no remaining files in the preparation directory, and `ss` showed
no listener on the chosen HTTP port. This confirms integrated preparation,
playback, and local cleanup after SIGINT for audio-only conversion. Natural
completion and receiver-initiated stopping for this mode remain unverified on
hardware; phone controls were unavailable, so that stop check was deferred.

A diagnostic recreation using the same FFmpeg arguments is saved locally under
ignored `samples/audio-debug/sample-041-aac.mp4` (about 1.28 GB). SHA-256 stream
hashes confirm that the entire encoded video payload matches the original.
The output has stereo AAC-LC at 48 kHz, matching duration, and decodes the first
60 seconds without errors. This is not proof of Cast compatibility. It is a
reusable diagnostic input, retained outside the normal session cache and not
automatically removed after casting. No source file was changed.

A lifecycle gap was identified during investigation: stale BUFFERING or empty
media replies did not trigger an application-presence check. These replies now
query the receiver's application list before continuing to wait. A confirmed
application exit is reported as receiver-stopped; it does not distinguish a
user stop from a crash. Failure to query receiver status remains an error.
Regression cases verify BUFFERING/empty replies after app exit and transient
BUFFERING while the owned app remains running. Verbose logs include media
state/position and HTTP Range, Content-Range, and Content-Length. A stall after
playback now has a distinct timeout message. These changes improve diagnostics;
the original media failure was not reproduced in the successful retry, and
these changes are not proven to have resolved its cause.

The complete workspace run on 2026-09-18 passed all 47 tests, including all four
normally opt-in FFmpeg tests; none were skipped. Formatting and Clippy with
warnings denied also passed. No real receiver was contacted by these checks.

## Hardware feedback and remaining checks

The user reported that the suggested forced-transcoding test with the short
generated fixture worked on the TV. This is initial hardware confirmation of
the preparation/playback path. Subsequent focused checks are recorded below.

The suggested command was:

```sh
cargo run --locked -- cast samples/short/test.mp4 \
  --device "Living Room" --force-transcode \
  --subtitles samples/short/subtitles.vtt --http-port 8010
```

On 2026-09-18, the user additionally tested `sample-004` with
`--force-transcode` on the existing KPN DIW7022 test receiver and confirmed:

- Visible video and audible audio after the complete file was prepared.
- Seeking, pause, resume, and stop using the receiver controls on a phone.
- No subtitles; the supplied command did not select an external subtitle file.

This sample contains H.264 video and six-channel AAC-LC, so the run establishes
audible output from the forced stereo conversion. Detailed channel balance and
A/V synchronization were not separately assessed. Phone controls demonstrate
receiver-side control of the prepared file; M3 CLI controls remain parked.
This first real-file run did not separately report Procast's exit status or cache
cleanup after the phone-issued stop, and does not establish natural completion.

On 2026-09-18, the user reported success after following the separate SRT and
WebVTT checks with the 30-second fixture and forced transcoding. The requested
checks covered visible captions changing with their elapsed-time labels, natural
completion, `Playback completed.`, and exit code 0. This confirms the short-fixture
subtitle/completion workflow for both formats. It did not include filesystem or
serving-port checks.

Subsequent explicit cleanup checks passed after both natural completion and
phone-issued stop: the dedicated preparation directory was empty and no listener
remained on the serving port. Phone stop nevertheless produced a media-request
timeout, followed by a redundant remote-STOP timeout, and exit code 1. These
errors did not prevent local cleanup.

The adapter previously ignored app-channel CLOSE events. It now checks that the
owned receiver application has actually ended before treating such a close as
a normal stop. A failed media poll also consults receiver status; a timeout alone
is never taken as proof of a stop. An owned CANCELLED media status is handled
normally too. Confirmed receiver stops yield `Playback stopped on receiver.` and
exit code 0, and skip another STOP. Network failures and closes while the owned
app is still running remain errors. A simulated-receiver regression covers these
paths, foreign app closes, and HTTP shutdown.

The user repeated the short forced-transcoding/WebVTT phone-stop test on
2026-09-18. The CLI transitioned from Playing to Stopping, printed
`Playback stopped on receiver.`, and returned exit code 0 without timeout or
cleanup warnings. This confirms that the new handling resolves the observed
hardware failure. Filesystem/port checks were not repeated in that transcript;
both passed in the preceding phone-stop run.

Remaining hardware checks: detailed A/V synchronization including after seeking,
and explicit signal-path cleanup for
the forced-transcoding workflow.
Embedded subtitle extraction remains deferred; external subtitles are supported
when explicitly selected with `--subtitles`.

## References

- [FFmpeg CLI and progress output](https://ffmpeg.org/ffmpeg.html)
- [FFmpeg video filters](https://ffmpeg.org/ffmpeg-filters.html)
- [Google Cast supported media](https://developers.google.com/cast/docs/media)
