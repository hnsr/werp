# Werp

Native desktop application (currently Qt/KDE, macOS and GTK/Gnome are next) for casting
videos to a Chromecast-compatible receiver.

## Features

- Native application written in Rust
- Minimize or entirely avoid remuxing or transcoding when possible (uses ffmpeg)
- Device capability DB (WIP) + user-configuration to allow more videos to be streamed directly
- Embedded or external subtitles (SRT, WebVTT, ASS, and SSA in UTF-8 or BOM-marked UTF-16)
- Control playback through standard media keys/controls (MPRIS)
- Inhibit sleep/suspend while playing
- Resume from last position

See [media conversion](docs/media-conversion.md) for media conversion details. Conversion
(if required) is a one-off operation done automatically before casting starts, or it can
be done ahead of time in a separate convert-only mode. See also
[storage and reuse](docs/media-conversion.md#storage-and-reuse).

See the [project direction and roadmap](docs/plan.md) for decisions and open work, and the [device database](crates/werp-core/data/devices.toml)
for recorded device capability support (currently a work in progress)

## Screenshots
<img width="385" height="299" alt="select" src="https://github.com/user-attachments/assets/dbbd2a52-b511-481c-9502-4bd512a1c551" /><br>
<br>
<img width="385" height="299" alt="playing" src="https://github.com/user-attachments/assets/09ddd668-ef34-42d5-b043-65accf92f472" /><br>
<br>
<img width="370" height="313" alt="convert" src="https://github.com/user-attachments/assets/3cdb80d9-623e-428c-9169-c43876d1552a" />



## Requirements

`ffprobe` must be on PATH. `ffmpeg` is required for subtitle conversion/extraction,
remuxing and encoding; full video transcoding needs libx264/AAC and input decoders.

Distribution packaging is still pending. See [DEVELOPMENT.md](DEVELOPMENT.md)
for dependencies, building from source, local installation and testing.

Werp chooses an OS-assigned HTTP port by default. For an existing firewall rule,
set the global `http_port = 8010` in
[`config.toml`](docs/configuration.md) for GUI and CLI
casting. The CLI also accepts `--http-port` and `--bind-address` diagnostic overrides.
Check firewall, VPN routes, mDNS and Wi-Fi isolation if discovery or downloads
fail. Werp never edits firewall rules.

Only registered media/subtitle resources are served, with ranges and subtitle CORS.
Use a trusted LAN: Cast TLS does not authenticate receiver identity, and media
uses HTTP. 

## Design

Werp consists of a platform-independent backend helper written in Rust, which
handles chromecast communication and converting media using ffmpeg.

The GUI application(s) can stay small and fully native and lets the backend helper
do all the work. Werp also comes with a CLI binary for casting from the command-line, which
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

See the [CLI guide](docs/cli.md) for commands, options, subtitle selection and resume.

## Limitations

- HDR handling:
  - PQ/HLG colour tags alone do not prevent direct playback or video copying.
    Werp preserves the video and its colour signalling on those paths; the receiver may
    still reject the format or render it incorrectly.
  - Transcoding HDR-tagged sources, including subtitle burn-in, is experimental: Werp produces
    its usual 8-bit H.264 output without tone mapping or a guarantee of correct rendering.
  - Dolby Vision is not supported yet
- Multiple tracks requiring explicit audio/video selection are not yet implemented and will trigger an error
- Live remuxing/transcoding is not yet implemented
- Hardware accellerated transcoding is not yet implemented/verified
- Subtitle handling:
  - Advanced ASS styling and external bitmap files remain limited.
  - DVD/VobSub and DVB image subtitles are not supported (embedded PGS burn-in is supported)
  - See [subtitles](docs/cli.md).
