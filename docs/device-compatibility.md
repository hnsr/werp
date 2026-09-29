# Device compatibility database

Yeet ships an editable, versioned [TOML database](../crates/yeet-core/data/devices.toml)
inside the core binary. It serves CLI/KDE casting and device-targeted conversion,
without a database server or separately installed data file. The core exposes
`devices::database()`, exact model/alias lookup, and the resulting playback policy.

## Matching and precedence

The database identifies **models**, not individual devices. `KPN DIW7022` is the
canonical first entry; `DIW7022` is an alias. Matching uses the discovered model
string, ignoring ASCII case and surrounding whitespace. It never matches a
friendly name, IP address, serial number, or partial model prefix.

With `--profile auto`, the bundled rules are merged with the user's optional
`devices.toml`, then matched against the discovered model. Unknown models and
`--host` use Baseline. Explicit Baseline/Extended/Experimental profiles override
model rules; forced full transcoding still targets conservative H.264/stereo AAC.
Convert-only can use a discovered device's merged model rules, or Baseline through
its Broad compatibility option. It previews the format before an explicit Convert click.

## User overrides

Create `$XDG_CONFIG_HOME/yeet/devices.toml` (normally `~/.config/yeet/devices.toml`).
This is separate from CLI preferences in `config.toml`. No rebuild is needed.
See [the example](devices.example.toml). For example, to restrict one known model:

```toml
schema_version = 1

[[devices]]
id = "KPN DIW7022"
[devices.playback]
allow_aac_surround = false
```

Entries merge by **canonical model ID**, ignoring ASCII case and surrounding
whitespace. Use `KPN DIW7022` to edit that entry, rather than its `DIW7022` alias.
Omitted fields inherit bundled values; explicit `false` values can disable a
permission. Provided arrays (`aliases`, `limitations`, `observations`) replace
the corresponding array, so include existing aliases you want to keep. An empty
array clears the list. Other optional fields are `receiver_app`, `firmware` and
`scope`. A new model starts with Baseline playback, no aliases or observations,
and metadata identifying it as a local, unverified override. Personal entries
need no observation records; any supplied observations must follow the bundled
schema. Overrides never modify the bundled evidence or count as hardware passes.

Playback fields are `allow_hevc`, `allow_aac_surround`, `h264_max_level` and
`h264_max_fps`. Supported level/fps pairs are `(41, 30)` and `(42, 30|50|60)`.
For a new Level 4.2 model set both numeric fields; they are merged independently.
These permissions retain the media guards below; arbitrary new codecs cannot
be enabled by adding unknown keys.

Missing default files use the bundled database. Invalid TOML, unknown keys,
duplicate IDs, conflicting aliases, invalid limits and files over 64 KiB fail
visibly. Validation checks the entire merged database, so a local alias cannot
silently shadow another model. The file is read for every new automatic CLI cast
and device-targeted KDE preview/`convert`/`start`; edits do not affect active work.
`--no-config` ignores CLI preferences and local overrides but retains bundled rules.
`--config PATH` changes only the CLI preferences file; device overrides remain in the XDG
location. Explicit CLI profiles bypass user database loading. Broad-compatibility
conversion/preview and backend inspect/discovery do not read this file.

Exact aliases are already supported. Fuzzy matching, individual-device rules and
stronger identification remain deferred.

The database controls format selection through Yeet's existing media guards.
Missing metadata, HDR, unsupported pixel formats and ambiguous track selection
remain subject to those guards. No failed cast is blindly retried with encoding.

## KPN DIW7022

The entry covers the Google Cast Default Media Receiver on the development unit.
Firmware was not recorded. It separates successful playback, failures,
intermittent behavior, and untested formats; every observation links to the
validation notes. The machine-readable policy is deliberately separate
from these observations, so recording a test does not silently enable a format.

| Area | Evidence / automatic behavior |
| --- | --- |
| H.264 MP4 with stereo AAC-LC | Original-file picture and sound confirmed on several files. |
| H.264 High Level 4.2 / 1080p50 | Original-file playback confirmed; admitted automatically for this model. |
| H.264 59.94/60 fps | Untested on hardware; requires a local `h264_max_fps = 60` model override. |
| HEVC Main/Main 10 SDR | Bounded Level 4.0/1080p30 copying/direct policy; Main 10 original-file and Main/Main 10 remux trials passed. |
| AAC-LC through six channels | Audible playback confirmed; discrete surround output was not established. |
| AC-3 original-file audio | One trial was silent. Automatic playback converts audio to stereo AAC. |
| HE-AAC / E-AC-3 | Audio-converted paths passed; native passthrough remains untested and is not admitted. |
| Matroska/MKV | Successful MP4 remux paths; no claim of native MKV playback. |
| AV1 | Full-conversion path passed; native AV1 remains untested. |
| Text subtitles | External SRT/WebVTT and embedded English/Dutch caption checks passed through WebVTT. |
| PGS image subtitles | Burn-in passed; native PGS-track support is not established. |
| Controls and lifecycle | Completion, cancellation/cleanup, resume, reuse, and some phone-control checks passed; coverage varies by path. |
| Reliability | An initial playback exit later passed on retry. Phone controls later disappeared; a separate longer interruption prompted host sleep inhibition. Root causes remain unresolved. |

The 50 fps observation does not establish 59.94/60 fps support, full-duration
stability, every possible codec combination, or support on every unit/firmware.
The generic Extended profile retains its old 30 fps/Level 4.1 H.264 boundary.
A local model override can permit 60 fps without changing other receivers.

### Original CLI hardware checks

On KPN DIW7022, separate external SRT and WebVTT runs confirmed visible video,
subtitles and correct Ctrl+C shutdown. SIGTERM during WebVTT playback reported
cleanup and exit 143; that transcript did not include a separate port check.
After the unsolicited-completion fix, a 30-second WebVTT run exited successfully
and closed its HTTP port. Later [batch checks](automatic-playback.md#hardware-batch-result)
cover SRT/WebVTT completion and cancellation/port cleanup across preparation paths.
Interrupted loading and signal-port closure were not individually recorded for
all original CLI runs; those remain optional regression checks, not known defects.

### Initial remux hardware check

The user reported no problems with sample-006 (MKV, H.264 High 720×480 at
23.976 fps, stereo AAC-LC) remuxed to MP4 on KPN DIW7022. Requested checks covered
picture, sound, sync and Ctrl+C; the reply was a general success report without
measurements or a cleanup transcript. No subtitles were selected. The later
[batch checks](automatic-playback.md#hardware-batch-result) add short-clip
H.264/HEVC remux, captions, completion, reuse and cancellation coverage.
Expanded-path seeking and long-duration sync remain deferred; original MKV
playback is not established.

### Initial full-conversion hardware checks

On KPN DIW7022, full conversion of sample-004 (H.264/six-channel AAC) produced
working video/audio; phone seek, pause/resume and stop passed. No subtitles were
selected, and channel balance/full-duration playback were not separately checked.
A 30-second forced-conversion fixture subsequently passed with SRT and WebVTT,
natural completion and exit 0.

Separate natural-completion and phone-stop runs left an empty preparation directory
and no HTTP listener, although the first phone stop returned timeout errors/exit 1.
After the [receiver-stop fix](decisions/001-cast-library.md#follow-up-fixes), phone
stop reported `Playback stopped on receiver.` and exit 0 without timeout/cleanup
warnings; filesystem/port checks were not repeated in that transcript.
See the [inventory](media-inventory.md#user-playback-results) for audio-only
conversion and its unresolved initial startup failure, and the later
[batch checks](automatic-playback.md#hardware-batch-result) for broader coverage.
Detailed seek/subtitle synchronization and long-duration stability remain unverified.

## Maintaining the bundled database

Edit `crates/yeet-core/data/devices.toml` and rebuild the app. Add an exact model ID
and explicit aliases, then record bounded playback permissions and observations.
Keep unsuccessful and uncertain results: a successful preparation or protocol
LOAD is not evidence of audible sound or visible picture on hardware.

Each observation has a stable local ID, status (`passed`, `failed`,
`intermittent`, `untested`), playback path, format description, result and
repository-relative evidence links. Describe which checks the user actually
confirmed. The current schema permits the existing bounded HEVC/AAC options and
H.264 Level 4.1/30 fps or Level 4.2 with a 30/50/60 fps ceiling; adding unrelated
codec capabilities requires corresponding core support, not just a data edit.

Model identifiers are suitable for committed data. Keep personal device names,
network addresses, unique receiver IDs and downloaded filenames out of it.
Automated checks validate the schema, unique model names/aliases, supported policy
bounds, evidence links and selection behavior, including override/profile precedence.

Run `cargo test --locked -p yeet-core --lib` after database edits. Hardware tests
remain manual and must name the path and observed result; unit tests do not
upgrade an untested entry to a hardware pass.
