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
models and CLI `--host` use Baseline. Explicit `--profile baseline|extended|experimental`
bypasses model rules. GUI convert-only uses the selected device's rules, or
Baseline for **Broad compatibility**. Full transcoding always targets conservative
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

Playback fields are `allow_hevc`, `allow_aac_surround`, `h264_max_level` and
`h264_max_fps`. Valid level/fps pairs are `(41, 30)` and `(42, 30|50|60)`; set both
numeric fields when adding a Level 4.2 model. Optional descriptive fields include
`receiver_app`, `firmware` and `scope`.

Missing default files use bundled rules. Invalid TOML, unknown keys, conflicting
IDs/aliases, unsupported limits and files over 64 KiB fail visibly. The file is
read for each automatic CLI cast and device-targeted GUI preview/start/convert;
changes do not affect active work.

`--no-config` skips local overrides and CLI preferences, retaining bundled rules.
`--config PATH` changes only CLI preferences, not the device-file location.
Explicit CLI profiles, Broad-compatibility conversion, inspection and discovery
do not load user device overrides.

## KPN DIW7022

The bundled policy allows H.264 through Level 4.2/1080p50, bounded HEVC Main/Main 10
SDR through Level 4.0/1080p30, and AAC-LC through six channels. AC-3, E-AC-3 and
HE-AAC take the audio-conversion path; compatible MKV streams are remuxed to MP4.

The database records successful direct, remux, audio-only and full-conversion
playback, text captions, PGS burn-in, resume, reuse and lifecycle/control checks.
These observations concern one Default Media Receiver setup, with firmware
unrecorded. They do not establish discrete surround output, 59.94/60 fps,
long-duration stability or seeking across every path. Intermittent startup exits,
missing phone controls and a longer interruption remain recorded with unresolved
causes. See the database entries for formats, results and per-observation limits.

## Maintaining the bundled database

Edit the TOML database and rebuild. Add canonical model IDs, explicit aliases and
bounded permissions. Each observation has a stable ID, status (`passed`, `failed`,
`intermittent`, `untested`), path, format, result and repository-relative evidence
links. Keep results scoped to what was actually observed; local conversion or
protocol success alone does not establish visible picture or audible sound.
New codec capabilities require core support as well as a database edit.

Keep personal device names, IDs, addresses and downloaded filenames out of the
database; public model identifiers and neutral sample aliases are suitable.
Run `cargo test --locked -p yeet-core --lib devices::tests` to validate the schema,
matching, override behavior and evidence links. Tests do not upgrade untested
formats to hardware passes.
