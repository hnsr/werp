# Werp

Cast local videos and subtitles to a Chromecast-compatible receiver using a native
GUI application (currently Qt/KDE, support for GTK, macOS coming later).

Werp supports embedded or external subtitles and tries to avoid or minimize
transcoding/remuxing when possible, depending on source format and target device
capabilities. Newer generation devices allow more video and audio codecs/profiles
(compared to the baseline profile) and a device compatibility DB is maintained to
document capabilities, to allow direct-play for more video formats.

See [media conversion](docs/media-conversion.md) for selection, encoding and reuse.
Conversion is a one-off operation done before casting starts. See also 
[storage and reuse](docs/media-conversion.md#storage-and-reuse).

See the [project direction and roadmap](docs/plan.md) for decisions and open work,
and the [device database](crates/werp-core/data/devices.toml) for recorded support.

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

Werp consists of a platform-independent backend helper written in rust, which native
GUI applications automatically launch and interface with. This allows
the GUI applications to be written in the native language/tooling without having to use
bindings. Werp also comes with a CLI binary for casting from the command-line.

On Linux, Werp uses `systemd-inhibit` to block sleep during casting and conversion,
including preparation, paused playback and cleanup. Merely opening the GUI or
browsing devices does not acquire a lock. Screen dimming/locking remains enabled.

### GUI application

The current frontend uses C++/Qt and KDE Frameworks. GTK and other native
frontends can share the same Rust backend; they are not implemented yet.

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

Known HDR/Dolby Vision, multiple tracks requiring explicit audio/video selection,
and missing required metadata remain errors. On-the-fly transcoding/remuxing, hardware
acceleration, HDR tone mapping and broader platform support are deferred. DVD/VobSub
and DVB image subtitles are not supported; embedded PGS burn-in is supported.
See [subtitles](docs/preferences-and-subtitles.md).

PQ/HLG colour tags are accepted when the video is played directly or copied;
rendering depends on the receiver. Transcoding HDR-tagged sources, including
subtitle burn-in, is experimental: the usual 8-bit H.264 output is produced without
tone mapping or a guarantee of correct HDR rendering. Dolby Vision, multiple tracks
requiring explicit audio/video selection, and missing required metadata remain errors.
Live encoding, hardware acceleration, HDR tone mapping and broader platform support
are deferred. Advanced ASS styling and external bitmap files remain limited.
DVD/VobSub and DVB image subtitles are not supported; embedded PGS burn-in remains
supported. See [subtitles](docs/preferences-and-subtitles.md).

