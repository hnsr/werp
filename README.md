# Werp

Cast local videos and subtitles to a Chromecast from a Rust CLI or native GUI.
Werp chooses direct play, MP4 remuxing, audio-only conversion or full SDR conversion
and reuses validated prepared files. Conversion finishes before playback starts.
For MKV files with compatible video/audio, remuxing copies the streams into MP4
without re-encoding or quality loss. H.264/AAC and device-permitted HEVC/surround
are supported; subtitles are handled separately. See [media conversion](docs/media-conversion.md)
for selection, encoding and reuse.

The GUI player provides device/subtitle selection, preparation progress, resume,
and pause/play/seek/stop. The current KDE frontend adds Dolphin **Open With**
integration. Convert-only previews output for a selected device without starting playback. Recorded hardware coverage
centres on **KPN DIW7022**; other models use conservative defaults unless configured.

See the [project direction and roadmap](docs/plan.md) for decisions and open work,
and the [device database](crates/werp-core/data/devices.toml) for recorded support.

## Requirements

`ffprobe` must be on PATH. `ffmpeg` is required for subtitle conversion/extraction,
remuxing and encoding on a cache miss; full encoding needs libx264/AAC and input
decoders. External WebVTT does not need conversion. Linux sleep prevention uses
`systemd-inhibit` on a best-effort basis.

Distribution packaging is still pending. See [DEVELOPMENT.md](DEVELOPMENT.md)
for dependencies, building from source, local installation and testing.

## GUI application

The current frontend uses C++/Qt and KDE Frameworks. GTK and other native
frontends can share the same Rust backend; they are not implemented yet.

```sh
werp-kde /path/to/video.mkv
werp-kde --convert-only /path/to/video.mkv
```

The app launches its Rust helper automatically. The player preselects the last-used
device and suggests English, then Dutch subtitles. Review choices and press
**Werp** or **Werp from last position**. Space toggles pause/play; Ctrl+Q closes
with cleanup. Opening/dropping a file does not start playback.

Convert-only lets you choose a discovered device (using its model rules and local
overrides) or **Broad compatibility** for conservative H.264/stereo AAC output.
Review the source/target preview and press **Convert**. Successful output is
retained; auto-close defaults to five seconds and is configurable in the GUI INI.

See the [GUI guide](docs/gui.md) for interaction, launch options and settings.

## CLI

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

Prepared MP4s and completion sidecars live beside the canonical source, falling
back to the user cache if unwritable. Originals are never changed. Reuse validates
source/output fingerprints and the recipe; this costs disk reads but avoids
encoding. There is no automatic eviction. See [storage and reuse](docs/media-conversion.md#storage-and-reuse).

## Limits and networking

Known HDR/Dolby Vision, multiple tracks requiring explicit audio/video selection,
and missing required metadata remain errors. Live encoding, hardware acceleration,
HDR tone mapping and broader platform support are deferred. Subtitle format scope
is accepted for the sample set; advanced ASS styling and external bitmap files
remain limited. See [subtitles](docs/preferences-and-subtitles.md).

The host must stay awake and be reachable from the receiver. Werp chooses the
local address from the receiver route and an OS-assigned HTTP
port. For an existing firewall rule, use `--http-port 8010` (CLI or GUI);
`--bind-address` is a CLI diagnostic override. Check firewall, VPN routes, mDNS and
Wi-Fi isolation if discovery or downloads fail. Werp never edits firewall rules.

Only registered media/subtitle resources are served, with ranges and subtitle CORS.
Use a trusted LAN: Cast TLS does not authenticate receiver identity, and media
uses HTTP. Lost connections fail rather than automatically restarting playback.
See [transport rationale](docs/decisions/001-cast-library.md).

CLI results go to stdout; diagnostics go to stderr. Use `--verbose` or `RUST_LOG`
for debug output. Exit codes: 0 success, 1 operation failure, 2 argument error,
130 Ctrl+C and 143 SIGTERM on Linux. Inspection JSON is Werp metadata, not raw
ffprobe output or a stable public API.

## Sleep prevention

On Linux, Werp uses `systemd-inhibit` to block sleep during casting and conversion,
including preparation, paused playback and cleanup. Merely opening the GUI or
browsing devices does not acquire a lock. Screen dimming/locking remains enabled.
CLI/helper `--no-inhibit-sleep` opts out. Acquisition failures warn and let work
continue; forced sleep or power-policy overrides may still interrupt playback.
This small adapter avoids desktop bindings; broader platform support is deferred.

Locally verified on Fedora KDE: the lock appeared in logind and KDE's active
inhibitions, and disappeared after SIGINT cleanup. Automated tests cover helper
failure, cancellation, release and signal cleanup. Actual suspend and long-video
validation remain outstanding; standby was only a suspected cause of an earlier
interruption. To inspect an active lock, use the KDE power applet or
`systemd-inhibit --list --no-pager`; Werp's entry should disappear after cleanup.
