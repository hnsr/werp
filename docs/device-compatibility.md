# Device compatibility database

Yeet bundles a versioned [TOML database](../crates/yeet-core/data/devices.toml)
with model-specific playback rules and observations. The CLI and GUI share it
for casting and device-targeted conversion. Rules determine format selection;
observations record what worked, failed or remains untested. Recording an
observation does not automatically enable a format.

## Matching and precedence

Matching uses the discovered **model string**, ignoring ASCII case and surrounding
whitespace. Exact aliases are supported: `DIW7022` matches `KPN DIW7022`. Friendly
names, addresses, unique device IDs and partial prefixes are not matched.
Fuzzy matching and individual-device rules remain deferred.

Automatic selection merges bundled rules with optional user overrides. Unknown
models and CLI `--host` use Baseline under automatic selection. Explicit CLI
profiles bypass model rules. GUI convert-only uses the selected device's rules,
or Baseline for **Broad compatibility**. Full transcoding always targets conservative
H.264/stereo AAC.

Rules work within the existing [media conversion](media-conversion.md) constraints;
they cannot enable arbitrary codecs or bypass HDR, metadata or track-selection
guards. Failed playback does not trigger a blind retry with encoding.

## User overrides

Create `$XDG_CONFIG_HOME/yeet/devices.toml` (normally `~/.config/yeet/devices.toml`).
It is separate from CLI preferences in `config.toml` and needs no rebuild.
For example, to disable copied surround audio for a known model:

```toml
schema_version = 1

[[devices]]
id = "KPN DIW7022"
[devices.playback]
allow_aac_surround = false
```

See [the example](devices.example.toml) for extending the database. Merge rules:

- Entries match by canonical model ID, ignoring case/whitespace; use `KPN DIW7022`
  to edit that entry, not its alias.
- Omitted fields inherit bundled values; explicit `false` restricts a permission.
  New models start with Baseline rules and unverified local metadata.
- Arrays (`aliases`, `limitations`, `observations`) replace rather than append;
  empty arrays clear them. Personal entries need no observations, but supplied
  observations must follow the schema. Overrides do not change bundled evidence.

### TOML fields

At the top level, `schema_version = 1` is required. Each `[[devices]]` entry has:

| Field | Type | Purpose |
| --- | --- | --- |
| `id` | String, required | Canonical model identifier used for merging. |
| `aliases` | String array | Additional exact model strings that match this entry. |
| `receiver_app` | String | Receiver application covered by the observations. |
| `firmware` | String | Firmware version, or a note that it is unknown. |
| `scope` | String | Setup covered by the entry and limits of its applicability. |
| `limitations` | String array | Known restrictions or unverified capabilities. |
| `observations` | Array of tables | Recorded outcomes; see [observation fields](#observation-fields). |
| `playback` | Table | Format-selection permissions listed below. |

Only `id` is required in a user override. Other fields inherit existing values;
new models use Baseline and unverified local metadata as described above.

Under `[devices.playback]`:

| Field | Values | Effect | Default for a new model |
| --- | --- | --- | --- |
| `allow_hevc` | Boolean | Allow bounded HEVC Main/Main 10 SDR direct/copy paths. | `false` |
| `allow_aac_surround` | Boolean | Allow copying AAC-LC with up to six channels. | `false` |
| `h264_max_level` | `41` or `42` | Highest H.264 level admitted for direct/copy paths (4.1 or 4.2). | `41` |
| `h264_max_fps` | `30`, `50` or `60` | Highest H.264 frame rate admitted for direct/copy paths. | `30` |

Valid level/fps combinations are **4.1/30** and **4.2/30, 50 or 60**. Set both
numeric fields when adding a Level 4.2 model; fields merge independently.
Enabling a permission still applies the bounds in [media conversion](media-conversion.md).

### Loading and application scope

The device TOML file applies to **both the CLI and GUI**. It is not a CLI-only
preference file.

Missing default files use bundled rules. Invalid TOML, unknown keys, conflicting
IDs/aliases, unsupported limits and files over 64 KiB fail visibly. The file is
read for each automatic CLI cast and device-targeted GUI preview/start/convert;
changes do not affect active work.

The following command-line options belong to **`yeet` (the CLI) only**.
They are not accepted by **`yeet-kde` (the current GUI)**:

| CLI option | Effect |
| --- | --- |
| `--profile auto` | Use bundled model rules plus user overrides (default). |
| `--profile baseline`, `--profile extended`, `--profile experimental` | Use the explicit profile without loading user device overrides. |
| `--host IP` | Bypass discovery/model detection and use Baseline under the automatic profile. |
| `--no-config` | Skip CLI preferences and user device overrides; retain bundled model rules. |
| `--config PATH` | Read CLI preferences from another file; the device-file location stays unchanged. |

The GUI uses model rules automatically for its selected device. Convert-only's
**Broad compatibility** choice uses Baseline and skips device overrides. The GUI
has no equivalent of `--config`, `--no-config` or a casting profile selector.
Inspection and discovery also do not load device overrides. See the
[GUI launch options](gui.md#launch-options) for flags the GUI actually accepts.

## Observation fields

Each `[[devices.observations]]` entry has:

| Field | Type / values | Purpose |
| --- | --- | --- |
| `id` | String | Stable identifier, unique within the model. |
| `status` | `passed`, `failed`, `intermittent`, `untested` | Observed outcome. |
| `path` | String | Playback/preparation path being described. |
| `format` | String | Source/output format and relevant parameters. |
| `result` | Non-empty string | What was observed and what remains uncertain. |
| `evidence` | Optional string array | Supporting repository-relative document links, optionally with heading fragments. |

All observation fields except `evidence` are required. Self-contained observations
need no separate report. See [database maintenance](../DEVELOPMENT.md#device-database-maintenance)
for contribution and validation instructions.
