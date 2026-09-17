# Procast implementation plan

Status: M0 and M1 verified; M2 implemented with automated checks passed and
user-confirmed SRT/WebVTT playback, Ctrl+C shutdown, and SIGTERM cleanup with exit
code 143. The IPv4 discovery issue
is fixed and verified; remaining hardware checks are recorded below. M3 is parked;
M5 compatibility work is next, starting with playback without re-encoding. The
local media inventory is complete; broader casting support is not implemented yet.
Updated: 2026-09-17.
Requirements: [outline.md](outline.md).

## Approach

Build a small working path through the entire system first: discover a Chromecast,
serve one compatible local video with external subtitles, and shut down cleanly.
Validate this on real hardware before expanding format support.

Use Rust for a reusable backend and a thin CLI. Keep the Cast implementation behind
an internal adapter. Choose that dependency based on actual playback, subtitle,
and cancellation behaviour, not its advertised feature list.

The first usable release is **M2** below. **M3 is parked in favour of M5**, first
expanding original-file playback and then stream-copy remuxing. Audio/video
encoding stays deferred until these paths have been evaluated. Milestone numbers
retain their original meaning, but no longer prescribe execution order. Each
small change should leave working behaviour behind.

## Initial scope and assumptions

- One local video, one selected device, and one casting session per CLI process.
- Fedora 44 is the initial development and hardware-test platform.
- No GUI, Qt/KDE dependency, daemon, remote-control API, playlist, or account.
- Start with a known-good MP4 containing compatible H.264 video and AAC audio,
  plus an external UTF-8 SRT or WebVTT file. Validate codec profiles, resolution,
  and frame rate against the actual receiver; the extension alone is insufficient.
- A representative MKV has been supplied and inspected. M1 verified a generated
  direct-play clip with external subtitles on a KPN DIW7022 receiver. Broader
  device and format compatibility still require testing.
- The host must remain awake during playback. Automatic suspend inhibition comes
  after the initial casting implementation.
- The installed development toolchain is rustc/Cargo 1.96.0. Start with the Rust
  2024 edition and pin a tested toolchain when scaffolding. Establish an older
  minimum Rust version only if we decide to support one and test it.

## Architecture and dependencies

Use a Cargo workspace with two crates:

```text
Cargo.toml
Cargo.lock
crates/
  procast-core/       # reusable backend library
  procast-cli/        # binary named procast
docs/
  outline.md
  plan.md
```

Create modules as functionality arrives rather than scaffolding empty layers.

| Backend area | Responsibility |
| --- | --- |
| `discovery` | Discover and resolve devices; select by identity |
| `cast` | Adapt the chosen library to Procast commands and state |
| `media` | Probe files and decide direct play/remux/transcode |
| `serve` | Serve explicitly registered video/subtitle resources |
| `subtitles` | Prepare text tracks and their metadata |
| `process` | Run ffprobe/FFmpeg with progress and cancellation |
| `session` | Coordinate resources, playback, events, and shutdown |

The CLI parses arguments, renders state, and handles terminal input/signals. It
must not implement Cast messages, conversion decisions, or media serving.

Proposed supporting crates:

| Concern | Starting choice | Constraint |
| --- | --- | --- |
| Async I/O and child processes | Tokio | One application runtime; explicit cancellation and task ownership |
| CLI parsing | clap | Keep commands and help small |
| Serialization | serde / serde_json | ffprobe output and structured data |
| Error reporting | thiserror in core | Typed errors; human-readable context in the CLI |
| Diagnostics | tracing / tracing-subscriber | CLI configures logging; no backend terminal printing |
| Discovery | mdns-sd | Evaluate direct mDNS on Fedora; no assumed Avahi daemon dependency |
| HTTP | axum + tower-http ServeFile | Reuse file/range handling; register individual resources, not a directory |
| Temporary artifacts | tempfile | Session-owned cleanup; source files remain untouched |
| Cast transport | oxicast 0.0.3 | Selected after M1 validation; pinned behind the adapter |

Check compatible releases and features when implementing; commit Cargo.lock for
reproducible application builds. Do not pin every crate to the versions observed
in research. No FFmpeg C library bindings are needed.

### Backend interface and concurrency

Expose Procast types such as `Device`, `MediaInfo`, `CastRequest`,
`PlaybackSnapshot`, and `ProcastError`. M2 exposes `session::run(CastRequest, ... )`,
a cancellation token, and a Tokio watch channel of `SessionState`. M3 adds a
command interface. Keep third-party Cast types private.

Use a bounded command channel and a latest-state snapshot for routine playback
updates. Deliver completion and errors reliably; a slow consumer must not stall
heartbeats or leak unbounded progress messages. Do not build a public stable ABI
or a general plugin framework now.

`rust_cast` 0.21.0 exposes synchronous connection/receive operations, and its
`CastDevice` is neither Send nor Sync. If adopted, create and use the connection
entirely inside its owning worker thread. That worker must service commands,
heartbeats, incoming messages, and shutdown with bounded delays. Wrapping an
unbounded blocking receive in an async timeout does not cancel it.

M1 must establish whether the transport can support safe timeouts/cancellation
without losing partially read protocol frames. If a small library patch cannot
make this practical, evaluate oxicast before building further on rust_cast.
Avoid scattering blocking calls through Tokio tasks.

## CLI contract

Initial commands:

```sh
procast devices
procast cast movie.mp4 --device "Living Room" --subtitles movie.srt
procast cast movie.mp4 --device <device-uuid>
procast cast movie.mp4 --host 192.168.1.50
```

- `devices` performs a bounded scan (proposed default: five seconds), lists names,
  stable IDs, models, and addresses, and exits.
- Resolve `--device` as a stable ID or an exact friendly name. Duplicate names
  produce an explanation and matching IDs, never arbitrary selection.
- `--host` bypasses discovery and is mutually exclusive with `--device`.
- Without a selector, cast automatically only when exactly one eligible video
  receiver is found; otherwise show the choices and ask for an explicit selector.
- Filter known audio-only devices. Treat unknown capabilities conservatively.
- `cast` remains in the foreground while serving. Show preparation, connection,
  playback, and failure states without flooding the terminal.
- Return nonzero on failure; distinguish argument errors and interruption.
  Logs/progress go to stderr; command results go to stdout.
- Add advanced `--bind-address` and `--http-port` overrides for network diagnosis.
  Default to a reachable local address and an OS-assigned port. A fixed port can
  support a narrowly scoped firewall rule when needed.

No persistent configuration is required for the first release. Use clear errors
for missing executables, missing files, invalid subtitles, ambiguous devices,
unreachable receivers, media rejection, and unavailable conversion capabilities.

## Milestones

### M0 — Minimal workspace and media inspection

**Complete.** The Cargo workspace, `procast inspect`, optional JSON output,
logging, typed errors, and subprocess lifecycle handling are implemented. See
[README.md](../README.md) for setup, commands, and checks.

Verified with Rust 1.96.0 and FFmpeg/ffprobe 8.1.2:

- Formatting and Clippy passed; 10 ordinary tests passed.
- The explicitly enabled FFmpeg integration test passed using a generated MKV
  containing video, audio, and subtitles.
- A supplied MKV reported 1080p H.264, six-channel E-AC-3, and 26 embedded text
  subtitles in both human-readable and JSON output.
- Missing executables, malformed media, missing files, bounded output, timeout,
  and cancellation were checked. Signal tests verified exit codes and reaping
  with normal host execution; they hung inside the development sandbox.

Implemented scope:

- Create the workspace, basic help, shared error types, and logging.
- Add `procast inspect <file>` as a useful diagnostic for container, stream codecs,
  duration, and audio/subtitle tracks, using ffprobe JSON.
- Manage subprocesses asynchronously with argument arrays and no shell. Keep
  paths as filesystem paths, not shell snippets; handle spaces and Unicode.
- Bound metadata capture and retain a bounded stderr tail for failures. Disable
  interactive child stdin, drain outputs concurrently, and kill/reap children on
  cancellation. A dropped handle alone is not the cleanup strategy.

Acceptance: inspect a real local file, report a missing ffprobe or malformed input
clearly, and interrupt a running probe without leaving a child behind. Run Cargo
formatting, linting, and relevant automated checks.

### M1 — Cast feasibility prototype and dependency decision

**Complete.** The `cast_probe` example discovered the target, launched the Default
Media Receiver, served a generated MP4 and WebVTT, and activated subtitles that
the user confirmed on the TV. A 622-second run passed pause/resume and both seek
directions; subsequent SIGINT/SIGTERM runs stopped playback and closed the server
within 100 ms. Automated tests cover connection failure, cancellation, partial
frames, correlation, ownership, HTTP serving, and cleanup.

Selected oxicast 0.0.3 without a dependency patch; Procast supplies subtitle
messages through its raw request API. See the [decision and evidence](decisions/001-cast-library.md)
and [reproduction commands](../README.md#cast-feasibility-example-m1).

Implemented scope:

Build a small backend example before committing to a full session implementation.

1. Discover `_googlecast._tcp.local.` and connect to an explicitly selected device.
2. Launch the Default Media Receiver and serve a short compatible local clip from
   the host, with a known-good external WebVTT track.
3. Send media/track metadata and activate the subtitle track. Verify subtitles
   visually on the TV; successful LOAD acknowledgement is not sufficient.
4. Maintain playback for at least ten minutes. Exercise pause/resume, seeking,
   status updates, and cancellation while waiting for a device response.
5. Simulate an unreachable receiver and a lost connection. Record bounded failure
   behaviour and the library changes needed.

Inspect the exact released library source for track support, raw-message access,
timeouts, and request correlation. Implement Cast text-track metadata and active
track selection where missing. Keep any necessary dependency patch small, pinned,
and documented; propose it upstream if appropriate. Record Cast-specific TLS
verification behaviour rather than assuming ordinary public-web certificates.

Acceptance: document the chosen library and evidence in a short decision note,
including receiver model, firmware if available, tested files, limitations, and
any patch. A library that cannot support subtitles and responsive shutdown fails
this milestone. Try oxicast if rust_cast needs excessive changes. If neither
works, document the concrete gap and revise the plan before writing a new Cast
stack. Do not proceed on the assumption that either candidate already passes.

### M2 — First usable CLI: local video and external subtitles

**Implemented; core hardware playback confirmed, remaining checks pending.**
The `devices` and `cast` commands,
reusable session coordinator, direct-play validation, external subtitle
preparation, and cleanup are in place. The current direct-play policy accepts
MP4-family H.264 up to 1080p30, level 4.1, 8-bit 4:2:0 with zero or one mono/stereo
AAC-LC track. Unknown required metadata fails conservatively. Broader profiles,
remuxing, and audio conversion remain M5 work.

Verification: 27 ordinary tests and both opt-in FFmpeg tests passed, together
with formatting and Clippy. Tests cover actual HTTP delivery to simulated TLS
receivers, SRT conversion, WebVTT without FFmpeg, transient status, natural
completion, failure, takeover, and cancellation. The supplied MKV is rejected
before discovery. The user subsequently confirmed visible video/subtitles and
correct Ctrl+C shutdown in separate WebVTT and SRT CLI runs. A subsequent
natural-completion failure revealed an ignored FINISHED broadcast; consuming
terminal events alongside polls fixed it. The short WebVTT clip now completes
with exit code 0 and a closed HTTP port. The user also verified SIGTERM during
WebVTT playback: cleanup completed and the exit code was 143. Natural completion
with SRT and explicit port closure after signals remain unverified on this hardware path.
The empty IPv4 lists in discovery were traced to IPv6-only service resolution;
explicit hostname lookups fixed it in eight subsequent LAN scans. Three added
regression tests brought the ordinary suite to 30 passing tests; the terminal-event
regression brings it to 31. See
[validation details](m2-validation.md).

Implemented scope:

- Turn the successful prototype into the core session and the `devices`/`cast`
  commands. Verify media prerequisites before changing playback on the receiver.
- Start the HTTP server before LOAD and keep it alive for the entire session.
- Choose the local address from routing to the selected receiver; never advertise
  loopback or a VPN address merely because it is the first interface.
- Serve opaque session URLs mapped to individual files, with correct MIME types,
  GET/HEAD and byte-range behaviour. No directory listing or arbitrary path access.
- Implement subtitle CORS responses, including any required preflight handling.
- Convert UTF-8 SRT to WebVTT with FFmpeg; serve valid WebVTT directly. Require
  FFmpeg for conversion, not for direct playback with an existing VTT file.
- Treat an explicitly requested but unusable subtitle as an error, rather than
  silently playing without it.
- If HTTP requests never reach the host, distinguish this from a Cast connection
  failure and suggest checking the selected interface and firewall. Do not modify
  firewall rules automatically.

Session progression: preparing → connecting → loading → playing/paused/buffering
→ stopping → completed, cancelled, or failed. Progress follows actual receiver
state. A LOAD success or transient IDLE state is not completion.

On Ctrl+C or SIGTERM, attempt a bounded stop of the owned media session, cancel
work, close serving connections, stop/reap children, remove temporary artifacts,
and join workers. Aim for normal cancellation within three seconds; use a bounded
hard cleanup deadline. Do not stop a new session that another sender has taken
over. Natural playback completion also releases resources. A network failure
must still permit complete local cleanup even if remote STOP cannot be delivered.

Acceptance: cast a full representative compatible video with SRT and VTT in
separate runs; verify visible subtitles and clean shutdown during preparation,
loading, and playback. Repeat start/stop cycles without stale servers or children.
Unsupported files must fail clearly without modifying the source.

### M3 — Playback controls and session robustness

**Parked while M5 compatibility work takes priority.** Existing signal handling,
session ownership, and cleanup remain requirements for the expanded media paths.

- Add a simple optional line-oriented control input to the running `cast` process:
  `pause`, `resume`, `seek 120`, `volume 0.5`, `status`, and `stop`.
- Enable it only for an interactive terminal. Noninteractive runs remain alive
  without reading commands; stdin EOF must not accidentally terminate playback.
- Keep signals responsive while input is pending. Route controls through the same
  backend session handle a future frontend would use.
- Reflect receiver-side changes, including commands from another controller.
  Bound outstanding requests and distinguish buffering from disconnects.
- Initially fail clearly on a lost connection; add bounded reconnection only when
  it can reattach to the same session without blindly restarting the video.

Acceptance: repeated pause/resume and forward/backward seeks preserve timing and
subtitles; volume matches the receiver; terminal input and networking do not block
one another. Session takeover ends Procast's ownership cleanly.

### M4 — Better subtitle and audio selection

- Add embedded text-subtitle extraction with explicit track selection.
- Detect matching external subtitle files; prefer an unambiguous exact match.
  If several language variants match, list them rather than picking silently.
- Define precedence: explicit subtitle option wins; include an explicit subtitles
  off option. Keep automatic embedded-language selection out until preferences
  are defined.
- Report audio tracks in `inspect`; implement explicit audio selection with the
  remux/conversion path in M5 if the direct-play receiver cannot select that track.
- Report image subtitles and unsupported styling honestly. Do not silently claim
  faithful ASS rendering after reducing it to plain text.

Acceptance: selected embedded text tracks and external subtitles display correctly
and remain synchronized after seeking. Tests cover ambiguous matches, missing
tracks, Unicode paths, and explicit overrides.

### M5 — Broader playback, remuxing, and eventual transcoding

**Current priority: avoid re-encoding wherever practical.** The
[local media inventory](media-inventory.md) covers 75 videos and 14 format
combinations, with neutral local symlinks and a per-file CSV. It separates 73
original-file/remux candidates from 2 HDR-signalling review cases. These are
provisional classifications, not playback results or a universal support list.

#### M5a — Original-file compatibility

- Expand the pure media-decision step to distinguish verified support, plausible
  receiver-specific trials, container-only preparation, and unsupported codecs.
  Explain uncertainty instead of equating the M2 guard with hardware capability.
- Prioritize MP4 H.264 with multichannel AAC, MP4 HEVC Main 10 with AAC, and
  MP4 H.264 with AC-3. Then evaluate original MKV delivery experimentally and
  AV1 separately. Use representatives from the inventory rather than launching
  every video; hardware tests remain explicit.
- Carry actual container/MIME information through serving and Cast LOAD instead
  of assuming every resource is MP4. Do not merely remove the existing guard.
- Record receiver model and whether the original file actually produced correct
  picture, colour, audio, subtitle timing, completion, and clean cancellation.
  Distinguish metadata predictions from observed results. Receiver hardware
  specifications alone do not prove Cast application support.
- Keep audio/video encoding disabled during this phase. Diagnose rejection
  before choosing a fallback; a server/network problem does not justify encoding.

Acceptance: representative original files for newly supported combinations play
on the test receiver, with clear handling of unsupported and uncertain cases.
Keep the existing external subtitle and lifecycle behaviour working.

#### M5b — Stream-copy remuxing

For files that need a different container, preserve selected encoded video and
audio streams. Start with MKV to MP4 where the codecs are compatible. Text
subtitle extraction can be shared with M4 when needed; embedded subtitle and
attachment streams must not be blindly copied into MP4. Remuxing is preparation
without re-encoding, not original-file playback.

Acceptance: representative MKVs play after stream-copy remuxing, with unchanged
video/audio codecs, correct timing, bounded cancellation, and source preservation.

#### M5c — Encoding fallback (deferred)

Add audio-only encoding first when video can be preserved, then video encoding
for confirmed incompatible cases. Check available encoders/decoders/filters
against the actual FFmpeg build before starting work. The inventory's unusual
HDR cases need inspection, not automatic tone mapping or metadata removal.

#### Shared preparation strategy

Proposed first conversion strategy: **prepare a complete temporary MP4 before
casting**. This is an implementation proposal, not a previously settled product
requirement. It reuses the tested seekable-file server and keeps arbitrary seeking
predictable. Start with stream-copy remuxing, then audio conversion, then video
conversion. Preserve compatible streams, timestamps, selected tracks, and subtitle
alignment; make output suitable for HTTP playback.

Show progress and cancellation while preparing. Store potentially large outputs
in an appropriate user cache location with free-space checks rather than assuming
`/tmp` is disk-backed. Never overwrite input files. Session-cache reuse and
cross-session eviction policies can follow later.

Acceptance: a compatible MKV remuxes without video re-encoding; an unsupported
audio track converts while preserving compatible video; an incompatible video
converts to a tested receiver profile. Verify duration, A/V sync, seeking, subtitle
timing, cancellation, insufficient-space handling, and missing encoder errors.

Record startup delay, preparation speed, CPU use, and temporary disk use on the
target machine. Use those measurements to decide whether immediate playback via
on-the-fly conversion is worth the added buffering and seek/restart complexity.
Keep HDR conversion, hardware acceleration, and bitmap burn-in outside this step.

### M6 — Fedora release readiness

- Document installation, required runtime capabilities, known receiver coverage,
  troubleshooting, and a narrowly scoped firewall setup for a chosen fixed port.
- Test on a clean Fedora 44 environment; distinguish Fedora's restricted
  ffmpeg-free capabilities from a more complete FFmpeg build.
- Keep `cargo build --release` / local installation usable; add RPM packaging
  once runtime dependencies and codec expectations are established.
- Add a small `doctor` command if recurring setup problems justify it.
- Evaluate automatic suspend inhibition with a platform adapter and guaranteed
  release on all shutdown paths.
- State exact tested support rather than promising all Chromecast generations or
  all Fedora releases. Future platform/frontend work starts from the stable core.

Acceptance: follow the installation instructions on a clean target system and
complete the documented playback/control/subtitle test set without developer-only
dependencies or leftover resources.

## Verification strategy

Add tests alongside the behaviour they protect, rather than chasing a coverage
percentage or exhaustively retesting dependencies.

| Area | Meaningful checks |
| --- | --- |
| Media decisions | Representative ffprobe fixtures; unsupported codec/profile and absent fields |
| HTTP integration | Correct bytes for full and ranged reads, HEAD, invalid ranges, VTT MIME/CORS, unregistered paths |
| Cast adapter | Recorded/synthetic messages for LOAD rejection, status transitions, correlation, timeout, takeover |
| Lifecycle | Fake transport/subprocesses for cancellation, failed cleanup, and no surviving workers/children |
| Real FFmpeg | Tiny generated fixtures for SRT conversion, extraction, remux, and selected conversions |
| Hardware | Actual visible subtitles, seek accuracy, long playback, disconnection, and repeated sessions |

Normal automated checks must not discover or launch applications on real TVs.
Hardware tests are explicit and identify the target device. Keep large/private
media out of the repository; generate small fixtures where practical.

Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
and `cargo test --workspace` once the workspace exists. Separate hardware and
FFmpeg-capability-dependent tests so skipped prerequisites are visible rather
than mistaken for passing coverage. Select a CI provider when a repository host
exists; this workspace now has a local Git repository.

## Immediate next step

Use the [inventory](media-inventory.md) to implement M5a's media decisions and
original-file trials, starting with the MP4 combinations. Proceed to stream-copy
remuxing for container compatibility before implementing encoding. M3 controls
and frontend design are parked.

M2's remaining hardware checks stay tracked: natural completion with SRT,
interrupted loading, and explicit port closure after signals. Include relevant
checks when exercising the expanded playback paths. WebVTT natural completion
and port closure, SIGTERM cleanup with exit 143, and visible subtitles/Ctrl+C
with both SRT and WebVTT have already passed; IPv4 discovery is fixed.

## Technical references

- [Requirements and earlier research](outline.md)
- [rust_cast connection API and threading constraints](https://docs.rs/rust_cast/0.21.0/rust_cast/struct.CastDevice.html)
- [oxicast alternative](https://github.com/denniskribl/oxicast)
- [mdns-sd](https://docs.rs/mdns-sd/latest/mdns_sd/)
- [tower-http file serving](https://docs.rs/tower-http/latest/tower_http/services/struct.ServeFile.html)
- [Tokio process management](https://docs.rs/tokio/latest/tokio/process/struct.Command.html)
- [Cast load request and active tracks](https://developers.google.com/cast/docs/reference/web_receiver/cast.framework.messages.LoadRequestData)
- [Cast subtitle-track metadata](https://developers.google.com/cast/docs/reference/web_receiver/cast.framework.messages.Track)
