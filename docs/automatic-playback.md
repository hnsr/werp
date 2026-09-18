# Automatic playback and reusable preparation

Updated: 2026-09-18. Implementation and local verification complete; the
consolidated hardware batch remains pending. Earlier receiver observations are
recorded in [transcode-validation.md](transcode-validation.md) and
[remux-validation.md](remux-validation.md).

## Default behavior and overrides

`procast cast FILE --device "Living Room"` inspects the file and discovers the
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

`--profile auto` chooses Extended for the observed KPN DIW7022 model and Baseline
for unknown models. `--host` bypasses discovery, so it uses Baseline unless a
profile is supplied explicitly. This is an observation-based model mapping, not
receiver capability negotiation or a guarantee for every device of that model.

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

External UTF-8 SRT and WebVTT still use `--subtitles`. Embedded tracks, attachments,
titles, and chapters are omitted during preparation. SRT conversion needs FFmpeg;
an existing WebVTT file does not.

The old `--force-transcode`, `--transcode-audio`, `--remux`, and
`--experimental-direct-play` flags remain hidden migration aliases. They conflict
with an explicit `--mode`. The experimental alias selects Direct/Experimental.
`--transcode-dir` aliases `--cache-dir`; old conversion commands now retain their
completed output unless `--no-cache` is supplied.

## Storage and reuse

By default, preparation places an MP4 and `.mp4.json` completion record beside the
canonical source, following symlinks. A name has this form:

```text
movie.mkv.procast-<recipe-key>-<generation>.mp4
movie.mkv.procast-<recipe-key>-<generation>.mp4.json
```

The readable source-name prefix is truncated when necessary. The key includes
the canonical source path, full source SHA-256 digest, preparation mode, resolved
profile, and recipe version. Recipe version 1 describes the current output
settings; output-affecting changes must bump it. External subtitle changes do not
invalidate the prepared video.

A cache hit verifies the completion record, output size, full output SHA-256, and
fresh ffprobe metadata against the profile. Reuse reads the source and output
from disk, but does not encode or require FFmpeg for media preparation. ffprobe
remains required. Missing, corrupt, or mismatched entries are ignored; new outputs
get new names without replacing them.

If the source folder is unwritable, Procast announces a fallback to
`$XDG_CACHE_HOME/procast` or `$HOME/.cache/procast`. Valid adjacent output can still
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

Formatting and Clippy passed. The complete workspace suite passed **58 tests**,
including all **six opt-in tests using real FFmpeg**, with none skipped:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace -- --include-ignored
```

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

## One guided hardware batch

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

Allow roughly 15–20 minutes for the interactive run. For each clip:

1. Let it finish naturally, then confirm picture, audible sound, audio/video
   timing, and the two subtitle cues.
2. Prepared cases repeat with FFmpeg unavailable and WebVTT captions to prove
   reuse. Confirm the same visible/audible behavior.
3. The last case disables caching and asks for Enter during playback. The script
   sends SIGINT and checks exit 130, temporary-file removal, and HTTP port closure.

The script checks selected modes, completion messages, reuse messages, and port
closure. It stops at the first failure and preserves logs and successful outputs.
If a visual check fails, record the sample ID and what happened; protocol success
alone does not count as a hardware pass. Seeking and long-duration playback are
separate follow-ups, especially for newly supported copied HEVC paths. M3 controls
remain parked, so this batch does not depend on phone controls.
