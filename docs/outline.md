# Procast outline

Status: agreed direction and input for detailed planning.
Last updated: 2026-09-17.

## Purpose

Procast casts downloaded local videos and subtitles to Chromecast devices on the
local network. It should make this reliable and simple, preparing media only
when the selected device cannot play it directly.

Start with a **Rust backend library and a thin CLI**. Prove the essential casting
workflow on real hardware before building a graphical interface or expanding
media compatibility. A future native KDE application remains desirable, and the
backend should also be reusable by a GNOME/GTK frontend or other platforms.

## Settled decisions

- Use **Rust** for the backend and initial CLI. The earlier C++ application
  direction is superseded now that the GUI is deferred.
- Keep backend logic independent of presentation, GUI toolkits, and GUI event
  loops. The CLI is the first consumer of the backend, not its implementation.
- Use **FFmpeg and ffprobe as subprocesses**, rather than linking directly to
  FFmpeg libraries. No Python runtime or Python casting helper is intended.
- Focus initial development and testing on **Fedora Linux**, with the current
  Fedora 44 machine as the development baseline. Other platforms are future
  possibilities, not initial support commitments.
- Keep the first milestone small: compatible video, external subtitles, and
  reliable session startup and shutdown. Add more media handling incrementally.
- Preserve the option of platform-specific frontends without designing their
  language bindings, plugin system, or IPC protocol prematurely.
- Use generic receiver names and sample filenames in documentation, committed
  files, and commit messages. Keep personal device names and user-provided media
  filenames out of Git history.

Rust was preferred over Go because the Chromecast ecosystem comparison did not
establish a decisive advantage for Go for this project's requirements. M1 later
selected oxicast 0.0.3 after testing on the user's KPN DIW7022 receiver.

## Initial CLI experience

Implemented M2 commands (visible SRT/WebVTT playback and Ctrl+C verified on hardware):

```sh
procast devices
procast cast movie.mp4 --device "Living Room" --subtitles movie.srt
```

The first version implements:

1. Discover Cast devices on the local network and allow explicit selection.
2. Inspect and serve a compatible local video.
3. Accept external WebVTT or convert external SRT to WebVTT, then activate the
   subtitle track on the receiver.
4. Keep running while serving media, showing useful status and actionable errors.
5. Stop playback and clean up the session, child processes, and temporary
   resources when interrupted with Ctrl+C.

Unsupported media should produce a clear explanation until conversion support
is implemented. Do not require users to construct FFmpeg commands.

M2 uses a conservative MP4-family/H.264/AAC direct-play policy and explicit UTF-8
SRT/WebVTT selection. The backend session owns preparation, HTTP delivery, Cast
state, and cleanup; frontends observe a latest-state channel and cancel via a
token. See the [plan](plan.md) and [M2 validation](m2-validation.md) for exact
scope and remaining hardware checks.

Pause, resume, seek, volume, and richer status reporting are the next control
capabilities. Whether these use an interactive CLI, separate commands, or another
mechanism remains open; do not assume a persistent background daemon.

## Backend boundary

The reusable backend owns:

- Device discovery, Cast connections, receiver launch, and session management.
- Media inspection, compatibility decisions, and FFmpeg process management.
- HTTP media delivery and subtitle preparation.
- Playback commands, structured state updates, errors, and cancellation.

The CLI owns argument parsing, terminal presentation, and mapping user actions
onto backend operations. Future frontends own windows, controls, file pickers,
notifications, file associations, and desktop-specific integration.

Expose a small command/event interface using Procast's own types. Keep the chosen
Cast dependency behind an internal adapter so its types and assumptions do not
spread through the application. Keep terminal output out of backend logic.

Backend operations must allow responsive cancellation and defined shutdown.
Platform-dependent facilities, such as discovery and suspend inhibition, should
be isolated where needed. The exact crate layout, async runtime, and concurrency
model belong in the detailed plan.

## Chromecast implementation: candidates and uncertainty

Google's official sender SDKs target Android, iOS, and the web. A native Rust
application needs a third-party Cast implementation or its own protocol support.

The initial research identified:

| Candidate | Relevant findings | Planning implication |
| --- | --- | --- |
| `rust_cast` | Lower-level Rust Cast library with receiver and media control; release history since 2016, with 0.21.0 published in December 2025 | First candidate to evaluate; discovery, local serving, and session orchestration require application integration |
| `oxicast` | Async/Tokio client advertising discovery, heartbeats, reconnection, and optional local-file HTTP serving | Alternative worth evaluating, but its short project history warrants careful validation |
| `go-chromecast` | Existing Go CLI with discovery, local-file serving, controls, and FFmpeg integration | Useful reference and comparison tool; not the selected implementation language |

The initial findings came from documentation and API inspection. M1 subsequently
selected oxicast 0.0.3 and verified video, external subtitles, playback controls,
and cleanup on the KPN DIW7022. Its typed media API lacks subtitle-track fields;
Procast supplies these through the library's raw-message API. No dependency patch
was needed. See the [M1 decision](decisions/001-cast-library.md) for evidence and
limitations, including the trusted-LAN TLS model.

The Cast layer must handle TLS, message framing/serialization, heartbeats,
request/response handling, timeouts, receiver state, and disconnects, whether
through a library or Procast code. Google's Default Media Receiver is the initial
receiver candidate; a custom TV application is not currently required.

## Media delivery and compatibility

### Local HTTP serving and discovery

The Chromecast retrieves video and subtitle resources from an HTTP server on the
computer. The computer must stay awake and reachable throughout playback.

- Support byte-range requests for directly served files and the CORS behaviour
  required for subtitle resources.
- Expose only selected media and generated session resources.
- Discover devices through mDNS and choose a LAN address the receiver can reach.
- Provide useful diagnostics for firewalls, VPN interfaces, and Wi-Fi client
  isolation. Decide discovery dependencies and network configuration during
  implementation planning.
- Keep serving and connection lifetimes coordinated with the casting session.

### Inspection and FFmpeg integration

Use ffprobe for container, codec, audio-track, and subtitle-track information.
Prefer its JSON output and FFmpeg's structured progress output over parsing
human-readable logs. Invoke executables with argument arrays, without a shell.

FFmpeg guarantees backward API/ABI compatibility within each library's major
version, but major library upgrades can break it. Subprocess integration avoids
that binary dependency; command-line compatibility and feature availability still
need validation.

Fedora's ffmpeg-free has restricted codec coverage. Check actual decoder,
encoder, and filter availability instead of assuming an installed executable is
sufficient. The development machine had FFmpeg 8.1.2 installed when inspected;
this is an observation, not a required exact version.

Expand media handling in this order:

1. Direct playback when the device supports the media.
2. Remuxing when changing the container is sufficient.
3. Transcoding only incompatible streams where practical.

Compatibility depends on the receiver model, codecs, profiles, resolution, and
frame rate, not merely the file extension. On-the-fly conversion versus
preprocessing/caching remains open. Seeking, buffering, temporary storage, and
hardware performance must inform that decision. Hardware acceleration is later
scope.

### Subtitles

Subtitles are a core requirement. Chromecast supports separate text tracks such
as WebVTT; missing support in an individual player is not a protocol limitation.

- Start with explicit external SRT/WebVTT files.
- Add embedded text-track extraction and matching sidecar-file discovery later.
- Text extraction/conversion need not re-encode the video.
- Preserve subtitle timing across playback and seeking.
- Defer elaborate ASS styling and bitmap subtitles. Preserving their appearance
  may require burning them into the video and therefore video re-encoding.

## Deferred work

The initial CLI does not require Qt, KDE Frameworks, CMake, or GUI language
bindings. Use Cargo for the Rust project; packaging details remain open.

Defer graphical interfaces, Dolphin integration, default file associations,
MPRIS integration, GUI single-instance behaviour, and window-close semantics.
A future KDE frontend could use C++/Qt/Kirigami; a GNOME frontend could use GTK.
Neither toolkit nor an FFI/IPC boundary is selected for future frontends yet.

Full cross-platform support, advanced subtitle rendering, hardware acceleration,
and a persistent service are outside the first milestone. Automatic suspend
inhibition should be considered after the basic casting path works.

## Validation and detailed-planning questions

M1 proved discovery, a real receiver connection, local compatible video playback,
visible external subtitles, and clean interruption on the user's hardware. Verify
pause, resume, seek, and subtitle synchronization as playback controls are added.
Use automated checks for backend behaviour alongside real-device testing.

Resolve next:

1. Which additional receivers and real-world media files should expand the
   existing KPN DIW7022 / prepared MP4 and WebVTT baseline?
2. Do M2 natural completion with SRT and SIGTERM pass on hardware? Natural
   completion with WebVTT and port closure passed after a terminal-event fix.
   Visible subtitles and Ctrl+C have passed with both SRT and VTT. The IPv6-first discovery issue has
   been fixed with explicit hostname resolution and verified on the test LAN.
3. How should M3 expose playback commands while preserving backend independence?
4. What conversion and caching strategy preserves seeking and subtitle timing?
5. What Fedora versions, Rust toolchain, FFmpeg capabilities, and packaging should
   we support, and how should firewall configuration be handled?

## References

- [rust_cast repository](https://github.com/azasypkin/rust-cast)
- [rust_cast documentation and releases](https://docs.rs/crate/rust_cast/latest)
- [oxicast repository](https://github.com/denniskribl/oxicast)
- [oxicast media API](https://docs.rs/oxicast/latest/oxicast/types/struct.MediaInfo.html)
- [go-chromecast repository](https://github.com/vishen/go-chromecast)
- [go-chromecast media API](https://pkg.go.dev/github.com/vishen/go-chromecast/cast#MediaItem)
- [Google Cast SDK platforms](https://developers.google.com/cast/docs/reference)
- [Google Cast media and subtitle support](https://developers.google.com/cast/docs/media)
- [Google Cast receiver options](https://developers.google.com/cast/docs/web_receiver)
- [FFmpeg API/ABI compatibility](https://www.ffmpeg.org/doxygen/trunk/index.html)
- [ffprobe documentation](https://ffmpeg.org/ffprobe.html)
- [Fedora ffmpeg-free package](https://packages.fedoraproject.org/pkgs/ffmpeg/ffmpeg-free/)
