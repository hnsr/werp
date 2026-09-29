# Yeet: project direction and roadmap

Reviewed 2026-09-29. This is the single project plan, incorporating the former
outline. Feature guides describe current behavior; validation reports preserve
what was actually observed. Milestone numbers are historical labels, not an
execution order.

## Purpose and scope

Make casting a local video and subtitles to a Chromecast simple and reliable.
Choose the least preparation needed for the receiver, preserve the original,
and reuse completed conversions. Fedora KDE is the initial platform.

The application now has a Rust CLI/backend and a native KDE player/converter.
Each CLI process or GUI helper owns at most one playback or conversion operation.
There is no account, playlist, or separately managed service. Other frontends can
reuse the backend protocol, but are not current support commitments.

## Current state

| Area | Status |
| --- | --- |
| Inspection and discovery | ffprobe metadata, bounded IPv4 mDNS discovery, explicit selectors and CLI preferences implemented. |
| Casting and lifecycle | Local HTTP range serving, subtitles, ownership-aware stop, cancellation and cleanup implemented. |
| Media preparation | Automatic direct play → MP4 remux → audio-only conversion → full SDR conversion; adjacent-file cache with user-cache fallback. |
| Subtitles | External/embedded text, preferred-language selection, signed delay, and embedded bitmap burn-in. Current sample-set scope accepted. |
| Resume and power | Persistent checkpoints and Linux sleep inhibition implemented; GUI resume is explicit. |
| KDE player | File opening/Dolphin integration, drag-and-drop, device/subtitle selection, preparation progress, pause/play/seek/stop and keyboard shortcuts. |
| Convert-only | Device selection, read-only format preview, explicit Convert, progress/cancel, reusable output and configurable auto-close. |
| Device database | Bundled model rules/evidence, exact aliases and user `devices.toml` overrides shared by casting and device-targeted conversion. |

The short-clip preparation matrix and core subtitle/resume workflows passed on
KPN DIW7022. The user confirmed initial KDE playback and subsequent UI changes;
the complete KDE hardware checklist is not recorded as passed. See the
[device evidence](device-compatibility.md#kpn-diw7022) for narrower claims and
untested cases. An automated or protocol pass alone is not a visible/audible TV pass.

## Decisions and rationale

| Decision | Why |
| --- | --- |
| Rust backend and thin CLI | Rust was preferred over Go once research found no decisive Cast-ecosystem advantage for Go. Keep playback logic reusable and independent of a frontend. |
| C++ / Qt 6 Widgets / KDE Frameworks 6 frontend | Native KDE integration without depending on incomplete or unstable language bindings. This supersedes the initial all-C++ application idea. |
| Private Rust child process, versioned JSON over stdin/stdout | One app launch for the user; native frontends need no Rust ABI/bindings, sockets or separately managed daemon. The helper owns operations and cleans up on EOF. |
| FFmpeg/ffprobe subprocesses | Use installed codec capabilities without linking to FFmpeg's C libraries. This avoids binary-library coupling; executable features and process lifecycle still require validation. |
| oxicast behind a Yeet adapter | Its persistent asynchronous reader and raw requests supported cancellation and subtitles with less transport surgery than rust_cast. [Full decision](decisions/001-cast-library.md). |
| Complete MP4 preparation before playback | A complete seekable file works with HTTP ranges and allows validated reuse. Live encoding adds buffering/seek/lifecycle complexity and remains deferred. |
| Model rules backed by observations | Container extensions and advertised hardware features do not establish Cast playback support. Exact model/alias matches are predictable; unknown models use Baseline. |
| Local model overrides instead of global codec opt-ins | A permission should affect the intended model, not every receiver. Overrides may restrict or extend bundled rules without rebuilding. |
| Retain prepared files beside the canonical source | Avoid repeating expensive work and make outputs discoverable. Use full fingerprints and output validation for correctness; fall back to the user cache when unwritable. |
| Separate CLI and GUI policy | CLI users can automate devices/subtitles/resume. GUI users review choices before starting, with independent convenience defaults. |
| No blind playback retry | A network failure or silent audio does not prove that video encoding is needed. Preserve diagnostic errors instead of silently restarting. |
| Small Linux sleep-inhibitor adapter | `systemd-inhibit` works with the tested KDE setup without desktop bindings. It is best-effort; other platform integrations remain open. |

## Architecture and constraints

```text
crates/yeet-core/       probing, discovery, Cast, media decisions, HTTP,
                       subtitles, conversion/cache, resume and power
crates/yeet-cli/        yeet FILE / devices / inspect; CLI policy and signals
crates/yeet-backend/    explicit requests over private versioned JSON IPC
apps/yeet-kde/          native C++/Qt player and convert-only windows
```

The core exposes Yeet types; third-party Cast types stay behind the adapter.
Frontends render structured state and issue commands rather than interpreting
FFmpeg output or CLI prose. Commands/queues are bounded; routine snapshots can
coalesce, while terminal outcomes are delivered after cleanup.

Maintain these constraints:

- Do not modify original files. Publish completed output without replacing
  unrelated files; remove partial work on handled cancellation/errors.
- Invoke tools with argument arrays, drain bounded diagnostics, and reap children.
  Cancellation means cancel **and await** the operation, not just drop its future.
- Serve only registered media/subtitle URLs, with byte ranges and subtitle CORS.
  Choose the local address using the receiver route; never change firewall rules.
- Stop only Yeet's owned media. A takeover ends ownership; a network failure must
  still release local resources even if remote STOP fails.
- Use a trusted LAN: Cast TLS currently does not authenticate receiver identity,
  and media is served over HTTP. See the transport decision for limits.
- Reject known HDR, ambiguous audio/video tracks and missing required metadata
  rather than guessing a destructive conversion. No HDR tone mapping yet.
- Keep personal device names, IDs, addresses, source filenames and symlink targets
  out of commits. Public model identifiers and neutral sample IDs are suitable.

Rust 2024 / Rust 1.96.0 are pinned; `Cargo.lock` records dependencies. Fedora 44
with FFmpeg/ffprobe 8.1.2 is the recorded development baseline, not a promise that
all codec builds or Fedora versions work. The CLI has no Qt or FFmpeg-header
dependency. There is no application Python dependency; native UI tests use a mock
Python helper.

## Milestones and remaining work

| Milestone | Current status | Remaining scope |
| --- | --- | --- |
| M0 — workspace and inspection | Complete | None in the original scope. |
| M1 — Cast feasibility | Complete | Broader receivers remain separate validation. |
| M2 — usable CLI and external subtitles | Complete | Optional loading-cancellation/signal-port regression checks; [recorded evidence](device-compatibility.md#original-cli-hardware-checks) distinguishes remaining validation gaps. |
| M3 — controls and robustness | Shared controls and KDE implemented; terminal controls parked | Interactive CLI input, volume control, and any bounded reconnection design. |
| M4 — subtitle/audio selection | Current subtitle scope accepted | Explicit audio-track selection; further subtitle expansion only when a real file exposes a gap. |
| M5 — preparation and reuse | Implemented; short-clip hardware matrix passed | Long-duration and expanded-path seek checks; optional cache maintenance. |
| M6 — Fedora release readiness | Open; development/local installation works | Clean-system validation, distribution packaging and supported runtime/dependency policy. |

### Open functional and release work

- **Audio-track selection:** choose a source audio stream/language through the
  core and frontends, preserve it through preparation, and include the choice in
  cache identity. Acceptance: a multi-audio file plays the chosen track; another
  choice cannot reuse the wrong prepared audio. Such files currently fail.
- **Release readiness:** exercise CLI and KDE installation on clean Fedora,
  establish codec requirements (`ffmpeg-free` may be insufficient), add RPM
  packaging, and verify desktop entries/helper lookup outside the checkout.
  Finish actionable network/firewall troubleshooting. A `doctor` command is
  conditional on recurring setup problems, not a required feature yet.
- **Validation:** complete the [KDE TV checklist](kde-ui.md#tv-checklist), longer
  playback and seek/subtitle-sync checks across copied HEVC and converted paths.
  Investigate any repeat of the intermittent startup exit/long-play interruption;
  neither has a proven root cause. Extend the device database only with scoped
  evidence. H.264 59.94/60 fps and DVD/DVB subtitle samples remain unverified.

These items have no newly assigned execution order. Long-duration/seek checks
were deferred when phone controls disappeared; KDE controls now provide another
way to perform them, but that does not count as validation.

### Deliberately parked or later scope

- M3 terminal `pause`, `resume`, `seek`, `volume`, `status`, `stop`: keep input
  optional/interactive, signals responsive and noninteractive stdin EOF harmless.
- Reconnection only if it can safely reattach to the same owned session; never
  blindly relaunch the video.
- KDE settings UI, MPRIS/media keys, single-instance behavior and subtitle
  switching during playback. Window-close cleanup and Dolphin integration exist.
- Stronger model identification/individual-device rules; exact aliases already
  work and matching changes were explicitly deferred.
- Cache eviction, duplicate-generation cleanup and crash-leftover collection.
- Live transcoding, hardware acceleration, HDR/tone mapping, faithful complex
  ASS rendering, external bitmap subtitles/OCR, other frontends/platforms and
  broader power integration. A persistent service is outside current scope.

## Verification and documentation

Use focused tests for media selection, subprocesses, HTTP ranges/CORS, Cast
ownership, IPC, resume/cache identity and UI lifecycle. Opt-in FFmpeg tests use
small generated fixtures. Automated tests use loopback receivers and must not
contact TVs. Hardware runs are explicit and record what the user saw/heard,
separately from protocol/log results. Do not maintain rolling test counts in docs.

[README checks](../README.md#checks) are the entry point for commands. Add CI when
repository hosting and supported build environments are settled.

| Reference | Owns |
| --- | --- |
| [Automatic playback](automatic-playback.md) | Profiles, preparation, storage/downmix and M5 evidence |
| [Preferences and subtitles](preferences-and-subtitles.md) | CLI config, subtitle selection/delay and resume |
| [Device database](device-compatibility.md) | Model matching, overrides and receiver evidence |
| [KDE guide](kde-ui.md) | GUI behavior, build/install and manual checks |
| [Backend protocol](backend-protocol.md) | Exact private frontend contract |
| [Sleep inhibition](sleep-inhibition.md) | Linux adapter and validation limits |
| [Media inventory](media-inventory.md) | Historical 75-file scan and early trials |
| [Transcode report](transcode-validation.md) | Historical observations and bug diagnoses, not current usage specifications |
