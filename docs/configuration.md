# Configuration

CLI and GUI preferences share one `config.toml`. The Rust core owns the schema,
defaults and validation; the CLI uses it directly, and native GUIs receive their
settings through the backend protocol. Omitted settings use built-in defaults.
Werp does not write this file during normal use.

Create `$XDG_CONFIG_HOME/werp/config.toml`, or `~/.config/werp/config.toml` when
`XDG_CONFIG_HOME` is unset or not absolute. Start with the commented
[example configuration](config.example.toml). A missing default file is fine.
Unknown keys are ignored with a warning; malformed TOML and invalid values for
known settings are errors. The CLI prints warnings to stderr, keeping JSON output
on stdout usable. The backend returns preference warnings to the GUI.

## Settings reference

| Key | Default | Meaning |
| --- | --- | --- |
| `http_port` | `0` | Shared casting HTTP port, from 0 to 65535. Zero lets the OS choose; use a fixed port such as 8010 for an existing firewall rule. |
| `cli.subtitles.auto_load` | `true` | Automatically select CLI subtitles. |
| `cli.subtitles.languages` | `["en", "nl"]` | Ordered CLI language preferences. English and Dutch aliases are supported; see [subtitle selection](cli.md#subtitle-selection-and-rendering). |
| `cli.devices.preferred` | `[]` | Ordered exact friendly names or stable IDs for CLI device selection. Entries must not be blank. |
| `cli.playback.auto_resume` | `true` | Start CLI playback at a saved position when available. Disabling automatic resume still records progress. |
| `gui.conversion.auto_close` | `true` | Close the GUI converter five seconds after success, including reused or already-compatible output. Errors and cancellation stay open. |

`cli.*` affects only the CLI. The GUI keeps its own subtitle choices and explicit
resume action. `gui.*` affects native GUIs only. The HTTP port has no effect on
inspection or convert-only operations. Werp never changes firewall rules.

## CLI overrides

Explicit CLI options take precedence over file preferences. `--config PATH`
selects another configuration file; a missing explicit file is an error.
`--no-config` uses built-in preferences and ignores local device overrides while
retaining bundled model rules. `--config` and `--no-config` are mutually exclusive.
`--http-port 0` overrides a configured fixed port with OS assignment.
See the [CLI guide](cli.md) for subtitle, device and resume overrides.
The GUI has no `--config` or `--no-config` launch option.

## Other files

- Device capabilities remain in [`devices.toml`](device-compatibility.md#user-overrides)
  in the default configuration directory. `--config PATH` does not relocate it.
- The last GUI device is saved in `$XDG_STATE_HOME/werp/gui.toml`, falling back to
  `~/.local/state/werp/gui.toml` when the variable is unset or not absolute.
- Playback checkpoints live under the same state root in `werp/resume`; see
  [playback positions](cli.md#playback-positions).

The former configuration-directory `gui.toml` is no longer read. There is no
automatic migration or legacy fallback.
