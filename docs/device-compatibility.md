# Device compatibility database

Yeet ships an editable, versioned [TOML database](../crates/yeet-core/data/devices.toml)
inside the core binary. It is shared by CLI and KDE casting and needs no database
server, network access, or separately installed data file. The core exposes
`devices::database()`, exact model/alias lookup, and the resulting playback policy.

## Matching and precedence

The database identifies **models**, not individual devices. `KPN DIW7022` is the
canonical first entry; `DIW7022` is an alias. Matching uses the discovered model
string, ignoring ASCII case and surrounding whitespace. It never matches a
friendly name, IP address, serial number, or partial model prefix.

With `--profile auto`, a known model supplies the base policy. Shared
`[compatibility]` settings can add permissions. Unknown models and `--host` begin
with Baseline. Explicit Baseline/Extended/Experimental profiles override both
model policy and config; forced full transcoding still targets conservative
H.264/stereo AAC. Convert-only has no target device and continues to use Baseline
plus the shared settings, rather than assuming the KPN device.

The database controls format selection through Yeet's existing media guards.
Missing metadata, HDR, unsupported pixel formats and ambiguous track selection
remain subject to those guards. No failed cast is blindly retried with encoding.

## KPN DIW7022

The entry covers the Google Cast Default Media Receiver on the development unit.
Firmware was not recorded. It separates successful playback, failures,
intermittent behavior, and untested formats; every observation links to the
original validation notes. The machine-readable policy is deliberately separate
from these observations, so recording a test does not silently enable a format.

| Area | Evidence / automatic behavior |
| --- | --- |
| H.264 MP4 with stereo AAC-LC | Original-file picture and sound confirmed on several files. |
| H.264 High Level 4.2 / 1080p50 | Original-file playback confirmed; admitted automatically for this model. |
| H.264 59.94/60 fps | Untested on hardware; requires `allow_h264_high_frame_rate = true`. |
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
Personal high-frame-rate opt-ins can still permit 60 fps independently.

## Maintaining the database

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
bounds, evidence links and selection behavior, including config/profile precedence.

Run `cargo test --locked -p yeet-core --lib` after database edits. Hardware tests
remain manual and must name the path and observed result; unit tests do not
upgrade an untested entry to a hardware pass.
