# Yeet implementation plan

Status: M0 and M1 verified; M2 implemented with automated checks passed and
user-confirmed SRT/WebVTT playback, Ctrl+C shutdown, and SIGTERM cleanup with exit
code 143. The IPv4 discovery issue
is fixed and verified; remaining hardware checks are recorded below. M3 terminal controls remain parked; shared controls and the first KDE UI are implemented with local checks;
M5 now implements automatic preparation selection, extended HEVC/AAC remuxing,
MKV audio-only conversion, and persistent adjacent-file reuse with user-cache
fallback. All 58 automated tests and 12 representative local preparation/reuse
checks passed; the user also confirmed the consolidated short-clip TV batch
passed without observed issues. Seeking and long-duration checks are deferred
at the user's request and do not block further work. Earlier explicit-mode
hardware evidence remains recorded separately.
Configuration, automatic subtitle selection/extraction, image-subtitle burn-in,
preferred receivers, and persistent resume are implemented. Automatic English
embedded captions, explicit Dutch selection, restart, interrupted playback resume,
completion clearing the saved position, automatic external SRT loading, and
subtitles off, and embedded PGS burn-in are confirmed on the TV. An optional
automatic Dutch preference check remains. See [preferences and subtitles](preferences-and-subtitles.md).
Updated: 2026-09-19.
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
encoding is now selected automatically when the profile requires it. Explicit
`--mode` and `--profile` options influence selection. Milestone numbers
retain their original meaning, but no longer prescribe execution order. Each
small change should leave working behaviour behind.

## Initial scope and assumptions

- One local video, one selected device, and one casting session per CLI process.
- Fedora 44 is the initial development and hardware-test platform.
- The initial CLI has no Qt dependency. The KDE frontend is separate; see
  [KDE UI and interface](kde-ui.md). No separately managed daemon, playlist, or account.
- Start with a known-good MP4 containing compatible H.264 video and AAC audio,
  plus an external UTF-8 SRT or WebVTT file. Validate codec profiles, resolution,
  and frame rate against the actual receiver; the extension alone is insufficient.
- A representative MKV has been supplied and inspected. M1 verified a generated
  direct-play clip with external subtitles on a KPN DIW7022 receiver. Broader
  device and format compatibility still require testing.
- The host must remain awake during playback. The Linux CLI now holds a
  best-effort sleep inhibitor during preparation and casting, verified with KDE;
  see [sleep inhibition](sleep-inhibition.md).
- The installed development toolchain is rustc/Cargo 1.96.0. Start with the Rust
  2024 edition and pin a tested toolchain when scaffolding. Establish an older
  minimum Rust version only if we decide to support one and test it.

## Architecture and dependencies

Use a Cargo workspace with three crates and a separate native frontend:

```text
Cargo.toml
Cargo.lock
crates/
  yeet-core/       # reusable backend library
  yeet-cli/        # binary named yeet, owns CLI automation policy
  yeet-backend/    # private JSON IPC helper, explicit frontend requests
apps/
  yeet-kde/        # C++ Qt Widgets frontend, built with CMake
docs/
  outline.md
  plan.md
```

Create modules as functionality arrives rather than scaffolding empty layers.

| Backend area | Responsibility |
| --- | --- |
| `discovery` | Discover and resolve devices; select by identity |
| `cast` | Adapt the chosen library to Yeet commands and state |
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

Expose Yeet types such as `Device`, `MediaInfo`, `CastRequest`,
`PlaybackSnapshot`, and `YeetError`. M2 exposes `session::run(CastRequest, ... )`,
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
yeet devices
yeet movie.mp4 --device "Living Room" --subtitles movie.srt
yeet movie.mp4 --device <device-uuid>
yeet movie.mp4 --host 192.168.1.50
```

- `devices` performs a bounded scan (proposed default: five seconds), lists names,
  stable IDs, models, and addresses, and exits.
- Resolve `--device` as a stable ID or an exact friendly name. Duplicate names
  produce an explanation and matching IDs, never arbitrary selection.
- `--host` bypasses discovery and is mutually exclusive with `--device`.
- Without a selector, cast automatically only when exactly one eligible video
  receiver is found; otherwise show the choices and ask for an explicit selector.
- Filter known audio-only devices. Treat unknown capabilities conservatively.
- `yeet FILE` remains in the foreground while serving. Show preparation, connection,
  playback, and failure states without flooding the terminal.
- Return nonzero on failure; distinguish argument errors and interruption.
  Logs/progress go to stderr; command results go to stdout.
- Add advanced `--bind-address` and `--http-port` overrides for network diagnosis.
  Default to a reachable local address and an OS-assigned port. A fixed port can
  support a narrowly scoped firewall rule when needed.

TOML configuration now stores subtitle, device, and resume preferences. Use clear errors
for missing executables, missing files, invalid subtitles, ambiguous devices,
unreachable receivers, media rejection, and unavailable conversion capabilities.

## Milestones

### M0 — Minimal workspace and media inspection

**Complete.** The Cargo workspace, `yeet inspect`, optional JSON output,
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
- Add `yeet inspect <file>` as a useful diagnostic for container, stream codecs,
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

Selected oxicast 0.0.3 without a dependency patch; Yeet supplies subtitle
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
The `yeet devices` command and `yeet FILE` playback action,
reusable session coordinator, direct-play validation, external subtitle
preparation, and cleanup are in place. The original M2 direct-play policy accepted
MP4-family H.264 up to 1080p30, level 4.1, 8-bit 4:2:0 with zero or one mono/stereo
AAC-LC track. Unknown required metadata failed conservatively. M5 below adds
broader profiles, remuxing, audio conversion, and automatic selection.

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

- Turn the successful prototype into the core session and the `yeet devices`/`yeet FILE`
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

- Add a simple optional line-oriented control input to the running `yeet` process:
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
one another. Session takeover ends Yeet's ownership cleanly.

### M4 — Better subtitle and audio selection

**Current subtitle scope accepted as complete.** All subtitle codec types in the
provided sample set are supported. Core subtitle/resume TV checks and PGS burn-in
passed. Further subtitle expansion and optional checks are parked until a real
file exposes a gap; explicit audio-track selection remains separate work.

- Extract embedded text to WebVTT, with absolute stream-index selection.
- Automatically select by ordered English/Dutch preferences, language tags and
  track names, preferring full dialogue and text over forced-only/image tracks.
- Fall back to an exact-basename external SRT. Ambiguous matches require an
  explicit file; unrelated or language-suffixed basenames are not guessed.
- Explicit file/track/off options override automatic settings in TOML.
- Burn embedded PGS/DVD/DVB subtitles into full video conversion; key reusable
  output by selected image track. PGS passed local rendered-frame verification
  and user-confirmed TV playback; DVD/DVB remain unverified.
- ASS/SSA text conversion is supported with a styling-loss warning. Faithful
  complex ASS rendering, external bitmap files, and OCR remain outside this scope.
- Report audio tracks in `inspect`; implement explicit audio selection with the
  remux/conversion path in M5 if the direct-play receiver cannot select that track.

Configuration also supports preferred receivers and enabled-by-default resume.
Resume saves owned receiver positions periodically, restores them in the next
LOAD, and clears state after FINISHED. Tests cover cancellation, failed LOAD,
disconnect, explicit restart, disabled resume, corrupt state, and source changes.
The [TV checklist](preferences-and-subtitles.md#verification-and-tv-checklist)
covers new subtitle/device/resume behavior without phone controls.

### M5 — Automatic playback, broader preparation, and reuse

**Implemented; consolidated short-clip hardware validation passed.** See
[automatic playback and reuse](automatic-playback.md) for design, tests, and the
single guided TV-validation command. The earlier [media inventory](media-inventory.md),
[transcoding results](transcode-validation.md), and [remux results](remux-validation.md)
retain the historical evidence from explicit modes.

#### Selection and CLI

- `yeet FILE` chooses original MP4, stream-copy MP4 preparation, copied video with
  stereo AAC conversion, or full SDR H.264/stereo AAC conversion, in that order.
- A pure backend decision module owns selection; the CLI only maps `--mode` and
  `--profile`. Mode overrides require their requested path or return a clear error.
- Automatic receiver profiles use observed Extended support for KPN DIW7022 and
  Baseline for unknown receivers/explicit hosts. Extended permits bounded HEVC and
  multichannel AAC-LC; Dolby and HE-AAC audio convert. The experimental AC-3 trial
  remains explicitly selectable and is not promoted into normal profiles.
- No blind runtime retry after receiver or network failure. Silent audio cannot
  be inferred from successful protocol status.
- Keep only the current mode/profile/storage options; no legacy aliases. Explain
  the chosen mode, reason, reuse, and fallback storage reliably even when progress
  updates coalesce.

#### Broader preparation

- Remux H.264 or bounded HEVC from MKV/MP4 with compatible AAC-LC tracks under the
  selected profile. Preserve encoded payloads and relative stream timing.
- Convert unsupported MKV/MP4 audio to stereo AAC while preserving supported
  video. Full conversion remains the fallback for unsupported SDR video.
- Preserve external subtitle handling, cancellation, subprocess reaping, progress,
  free-space checks, and output validation. Do not blindly copy embedded subtitles,
  attachments, titles, or chapters into the prepared MP4.
- Known HDR, multiple tracks requiring selection, and incomplete necessary
  metadata remain unsupported. HDR, hardware acceleration, and live encoding are deferred.

#### Persistent reuse

- Default to prepared MP4 plus completion metadata beside the real source,
  following symlinks. Fall back to the user cache for an unwritable source folder.
- `--cache-dir` overrides storage; `--no-cache` requests temporary-only behavior.
- Fingerprint source contents, canonical identity, output recipe/profile, and
  completed output. Re-probe reusable files; reject incomplete/corrupt metadata.
- Publish complete validated files without replacing originals or unrelated files.
  Concurrent preparation may produce separate valid generations. Keep completed
  outputs after session termination; clean partial session work on handled errors.
- No automatic eviction or crash garbage collection yet. Document retained files
  separately from temporary cleanup and expose their paths.

#### Acceptance and batch validation

Automatic tests cover routing, overrides, fingerprint invalidation, corrupt output,
symlink destinations, read-only fallback, cancellation, concurrency, codec payloads,
relative timing, subtitles, and loopback sessions. Tests must not contact TVs.
The user completed `scripts/validate-m5.sh` on the KPN DIW7022 and confirmed
all short representative clips looked and sounded correct. The script reported
all checks passed, covering all preparation paths, cache reuse, subtitles,
natural completion, cancellation, temporary cleanup, and HTTP port closure.
Longer playback and seeking are deferred at the user's request because phone
media controls are unavailable; these checks are not claimed as passed and do
not block subsequent work. The one unexplained early
audio-only startup failure remains historical
rather than being treated as a proven codec defect or proven fix.

### M6 — Fedora release readiness

- Document installation, required runtime capabilities, known receiver coverage,
  troubleshooting, and a narrowly scoped firewall setup for a chosen fixed port.
- Test on a clean Fedora 44 environment; distinguish Fedora's restricted
  ffmpeg-free capabilities from a more complete FFmpeg build.
- Keep `cargo build --release` / local installation usable; add RPM packaging
  once runtime dependencies and codec expectations are established.
- Add a small `doctor` command if recurring setup problems justify it.
- Revisit the initial Linux `systemd-inhibit` adapter with frontend/platform work.
  Acquisition and release are verified with KDE; broader platform integration
  remains deferred. Preserve bounded startup and cleanup on all shutdown paths.
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

Validate the implemented [KDE frontend](kde-ui.md#tv-checklist) on the TV.
CLI automation now uses `[cli.*]` configuration and remains separate from explicit
frontend requests. The C++/Qt Widgets app launches a private Rust helper through
the documented [versioned JSON protocol](backend-protocol.md). Selection,
preparation progress/cancellation, and playback controls are separate UI states.
Local protocol, simulated receiver, and native window checks cover the new path.
The user has installed all required development packages; TV validation remains
user-run.

The project and executable are named `yeet`; playback is the default action
(`yeet FILE`), with `yeet devices` and `yeet inspect FILE` retained as subcommands.
Configuration, state, cache, prepared output names, crates, scripts, and docs use
the new name. Existing local user directories and prepared video/metadata pairs
were renamed directly; no compatibility fallback was added. All 69 tests,
including real FFmpeg fixtures, passed after the rename, along with formatting,
Clippy, and reuse of a renamed prepared video with FFmpeg unavailable.

The current subtitle scope is accepted as complete for the provided sample set.
Further subtitle expansion and optional checks are parked. The short fixture and
checklist in [preferences and subtitles](preferences-and-subtitles.md) remain
available for regression testing when needed.
The earlier M5 hardware batch passed. Seeking and longer playback on the expanded preparation
paths, especially copied HEVC, are deferred until controls are available or the
user resumes those checks. They do not block choosing the next development task.
Keep M3 terminal controls parked; shared playback controls and the KDE frontend
are implemented, with frontend hardware validation next.

## Technical references

- [Requirements and earlier research](outline.md)
- [rust_cast connection API and threading constraints](https://docs.rs/rust_cast/0.21.0/rust_cast/struct.CastDevice.html)
- [oxicast alternative](https://github.com/denniskribl/oxicast)
- [mdns-sd](https://docs.rs/mdns-sd/latest/mdns_sd/)
- [tower-http file serving](https://docs.rs/tower-http/latest/tower_http/services/struct.ServeFile.html)
- [Tokio process management](https://docs.rs/tokio/latest/tokio/process/struct.Command.html)
- [Cast load request and active tracks](https://developers.google.com/cast/docs/reference/web_receiver/cast.framework.messages.LoadRequestData)
- [Cast subtitle-track metadata](https://developers.google.com/cast/docs/reference/web_receiver/cast.framework.messages.Track)
