# Werp

Native GUI application (currently only Qt/KDE) for casting videos to a Chromecast-compatible
receiver.

## Features

- Native application written in Rust
- Minimize or entirely avoid remuxing or transcoding when possible (uses ffmpeg)
- Device capability DB + user-configuration to allow more video to be streamed directly
- Embedded or external subtitles (SubRip)
- Control playback through standard media keys/controls (MPRIS)
- Inhibit sleep/suspend while playing

See [media conversion](docs/media-conversion.md) for media conversion details. Conversion is a one-off operation
done before casting starts. See also [storage and reuse](docs/media-conversion.md#storage-and-reuse).

See the [project direction and roadmap](docs/plan.md) for decisions and open work, and the [device database](crates/werp-core/data/devices.toml)
for recorded device capability support (currently a work in progress)

## Requirements

`ffprobe` must be on PATH. `ffmpeg` is required for subtitle conversion/extraction,
remuxing and encoding; full video transcoding needs libx264/AAC and input decoders.

Distribution packaging is still pending. See [DEVELOPMENT.md](DEVELOPMENT.md)
for dependencies, building from source, local installation and testing.

Werp chooses  an OS-assigned HTTP port. For an existing firewall rule, use
`--http-port 8010` (CLI or GUI); `--bind-address` is a CLI diagnostic override.
Check firewall, VPN routes, mDNS and Wi-Fi isolation if discovery or downloads
fail. Werp never edits firewall rules.

Only registered media/subtitle resources are served, with ranges and subtitle CORS.
Use a trusted LAN: Cast TLS does not authenticate receiver identity, and media
uses HTTP. 

## Design

Werp consists of a platform-independent backend helper written in Rust, which native
GUI applications automatically launch and interface with. This allows
the GUI applications to be written in the native language/tooling without having to use
bindings. Werp also comes with a CLI binary for casting from the command-line, which
is mainly intended for debugging/diagnostic purposes.

On Linux, Werp uses `systemd-inhibit` to block sleep during casting and conversion,
including preparation, paused playback and cleanup. Merely opening the GUI or
browsing devices does not acquire a lock. Screen dimming/locking remains enabled.

### GUI application

The GUI application uses C++/Qt and KDE Frameworks for a native experience and is a thin
layer on top of the shared backend, making it easy to add Gnome/GTK and macOS versions
later.
```sh
werp-kde /path/to/video.mkv
werp-kde --convert-only /path/to/video.mkv
```
The GUI app allows choosing target device, subtitle and other options before
starting the chromecast session.

The GUI app can also run in convert-only mode, allowing conversion ahead of time.

See the [GUI guide](docs/gui.md) for interaction, launch options and settings.

### CLI

```sh
werp devices
werp inspect /path/to/video.mkv --json
werp /path/to/video.mkv --device "Living Room"
werp /path/to/video.mkv --device "Living Room" --subtitles /path/to/captions.srt
werp --help
```

`werp FILE` stays in the foreground while serving. It replaces playback on the
selected receiver. Ctrl+C/SIGTERM cleans up owned playback, child processes,
HTTP serving and temporary files; successful reusable outputs remain. Another
sender's takeover ends Werp's ownership without stopping that sender's media.
Interactive terminal playback controls remain parked; the GUI has controls.

`--device` accepts an exact friendly name or ID; duplicate names require an ID.
Without it, CLI preferences apply, then the sole eligible video receiver is chosen.
`--host IP` bypasses IPv4 discovery and model detection. Use `--help` for diagnostic
options, or a path such as `./devices` for filenames that match subcommands.

| Options | Purpose |
| --- | --- |
| `--mode auto/direct/remux/audio/transcode` | Choose automatically (default), or require a preparation path |
| `--profile auto/baseline/extended/experimental` | Model rules (default), conservative support, bounded HEVC/AAC surround, or explicit AC-3 trials |
| `--subtitles FILE`, `--subtitle-track INDEX`, `--no-subtitles` | Override automatic subtitle selection |
| `--subtitle-delay-ms N` | Signed delay: positive is later, negative earlier |
| `--restart`, `--resume`, `--no-resume` | Control starting position/checkpoint behavior |
| `--cache-dir PATH`, `--no-cache` | Override storage or disable output reuse/retention |
| `--config PATH`, `--no-config` | Override CLI preferences, or ignore preferences and local model overrides |

CLI preferences live in `~/.config/werp/config.toml`: automatic subtitles
(English then Dutch), preferred devices and automatic resume. The GUI does not
apply these CLI settings. Device rules live separately in `devices.toml`; GUI
preferences for the KDE frontend use `kde-ui.ini`. XDG locations are supported. See
[configuration and resume](docs/preferences-and-subtitles.md) and
[device overrides](docs/device-compatibility.md#user-overrides).


## Limits and networking

PQ/HLG colour tags alone do not prevent direct playback or video copying. Werp
preserves the video and its colour signalling on those paths; the receiver may
still reject the format or render it incorrectly. Transcoding HDR-tagged sources,
including subtitle burn-in, is experimental: Werp produces its usual 8-bit H.264
output without tone mapping or a guarantee of correct rendering. Dolby Vision,
multiple tracks requiring explicit audio/video selection, and missing required
metadata remain errors.
Live encoding, hardware acceleration, HDR tone mapping and broader platform support
are deferred. Advanced ASS styling and external bitmap files remain limited.
DVD/VobSub and DVB image subtitles are not supported; embedded PGS burn-in remains
supported. See [subtitles](docs/preferences-and-subtitles.md).
