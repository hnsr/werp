# Private frontend protocol, version 1

`yeet-backend` is an automatically launched child process, not a user-managed
daemon. A frontend owns its stdin/stdout pipes. UTF-8 JSON objects are delimited
by a newline; embedded newlines in filenames must be JSON-escaped. Diagnostic
logs go only to stderr. The helper accepts `--http-port PORT` (default 0,
OS-assigned), `--ffprobe PATH`, `--ffmpeg PATH`, and `--no-inhibit-sleep`
for host integration and development. There is no shell interpretation of paths or commands.

## Requests and responses

Request IDs are integers from 0 through 9007199254740991 and must be unique among
pending requests. Responses can arrive out of order. Start with a handshake:

```json
{"id":1,"method":"hello","params":{"version":1}}
{"id":1,"ok":true,"result":{"version":1,"application":"yeet-backend"}}
```

Incompatible versions receive `protocol_version`; no operations are accepted
before a successful handshake. Use the bundled matching helper. Breaking contract
changes require a version bump. Frontends should ignore additional response fields.

| Method | Params | Successful result |
| --- | --- | --- |
| `hello` | `version: 1` | `version`, `application` |
| `inspect` | `file: string` | `media`, `subtitles`, `suggested_subtitles`, `subtitle_warning`, `resume_position`, `resume_warning` |
| `discover` | Omit params | `devices: array` from a five-second IPv4 scan |
| `start` | `file`, `device_id`, `subtitles`, `position`, optional `subtitle_delay_ms` | `session_id: integer` |
| `preview_conversion` | `file`, optional `device_id` | Conversion snapshot with `source`, `planned_target`, `target_description`, `message` |
| `convert` | `file`, optional `device_id` | `operation_id: integer` |
| `cancel_conversion` | `operation_id` | Empty object after conversion cleanup |
| `pause`, `play` | `session_id` | Empty object after receiver acknowledgement |
| `seek` | `session_id`, `position` | Empty object after receiver acknowledgement |
| `stop` | `session_id` | Empty object after session cleanup |
| `shutdown` | Omit params | Empty object after cleanup; helper exits |

`start` requires `file`, `device_id`, `subtitles` and `position`; delay is optional.
`position` is seconds from the beginning; zero
means restart. It must be finite, nonnegative, and below a known media duration.
The device must be in this helper's latest discovery results and must have an
IPv4 address. Known audio-only receivers are rejected. Subtitle selection is one
of `{"kind":"none"}`, `{"kind":"embedded","index":2}`, or
`{"kind":"external","path":"/path/to/video.srt"}`. The embedded index is the
source stream index, not the index in the returned subtitle array.
`subtitle_delay_ms` is a signed 32-bit integer, defaulting to zero. Positive values
show subtitles later and negative values earlier, on the full video timeline
(including resume/seek). It applies to prepared text tracks and image burn-in;
it cannot be changed during an active session.

```json
{"id":4,"method":"start","params":{"file":"/path/to/video.mkv","device_id":"example-device-id","subtitles":{"kind":"embedded","index":2},"subtitle_delay_ms":-500,"position":0}}
{"id":4,"ok":true,"result":{"session_id":1}}
{"id":5,"method":"seek","params":{"session_id":1,"position":45}}
```

`start` accepts ownership of an asynchronous operation; it does not mean playback
has started. Wait for state events. Only one session can be active. Commands for
an expired or different session ID fail rather than affecting another session.
Pause/play/seek are accepted only during playing, paused, or buffering phases.
Seek retains the receiver's pause/play state by leaving `resumeState` unset,
following the [Cast media protocol](https://developers.google.com/cast/docs/media/messages#Seek).

Errors have the form:

```json
{"id":5,"ok":false,"error":{"code":"invalid_session","message":"session is no longer active"}}
```

Error codes are `invalid_request`, `protocol_version`, `busy`, `operation_failed`,
`invalid_device`, `invalid_position`, `invalid_session`, `invalid_operation`, and `not_playing`.
Malformed input can produce a null response ID when no ID can be recovered.
Display the message; do not parse its prose to drive frontend behavior.

## Inspection and discovery data

Inspection is read-only and never starts a session or selects a device or starting position.
It does not create checkpoint files or prepare media. The helper never loads CLI
configuration. `resume_position` is a usable saved position, already adjusted five
seconds backwards, or null; `resume_warning` is null or a read error string.

`suggested_subtitles` has the same shape as the `start` subtitle parameter and
uses the shared subtitle selector with fixed English-then-Dutch preferences.
It recommends a supported embedded track, then an exact-basename SRT, or None.
External paths are canonical to match enumeration. `subtitle_warning` is null
or a selection warning (such as ambiguous sidecars); a warning leaves None
recommended without making inspection fail. This suggestion never prepares or
activates subtitles by itself: the frontend still sends an explicit selection
in `start` and can preserve manual user choices instead.

`media` contains canonical `path`, `container`, nullable `duration_seconds`, and
`streams`. Each stream has an `index`, `kind`, codec/profile metadata, dimensions
or audio information, language/title, and disposition flags. The Rust definitions
are in [`media.rs`](../crates/yeet-core/src/media.rs).

Each subtitle choice has `kind` (embedded/external), nullable `index`, `path`,
`codec`, `language`, and `title`, plus boolean `forced`, `hearing_impaired`,
`supported`, and `burn_in`. Embedded choices come first, followed by exact-basename
SRT, VTT, ASS, and SSA files beside the source or original symlink. Canonical paths
are deduplicated. Show unsupported tracks disabled; add None in the frontend.
An explicitly picked external file need not be in this list.

Each discovered device has `id`, `name`, `model`, `addresses` (IPv4 strings), `port`,
and nullable `capabilities`. Bit 0 means video support; null means unknown. Names
are display labels; use IDs in requests. A frontend may restore its last-used
ID when that receiver is present and eligible; the helper does not store this preference.

## State events and lifecycle

```json
{"event":"state","session_id":1,"state":{"phase":"preparing","position_seconds":null,"duration_seconds":120,"preparation_operation":"Transcoding","preparation_fraction":0.42,"message":"Transcoding: 42%","notices":[]}}
{"event":"ended","session_id":1,"phase":"cancelled","error":null}
```

State phases are `preparing`, `discovering`, `connecting`, `loading`, `playing`,
`paused`, `buffering`, `stopping`, `completed`, `stopped`, `cancelled`, and `failed`.
Position, duration, operation, fraction, and message may be null. Preparation
fractions are in 0..1; null means indeterminate. The operation/message are display
text. Notices retain preparation decisions when intermediate snapshots coalesce.
Only the fraction and phase should control the preparation UI.

Events are snapshots and can skip intermediate updates. After `ended`, ignore
later state messages for that ID. Its error is null for normal completion or
cancellation and a string for failures. Cleanup completes before `ended`. A new
session may then start. Successful prepared cache files remain reusable, while
temporary resources and incomplete work are released.

The helper holds sleep inhibition and saves checkpoints during a session. Closing
stdin, explicit shutdown, or output failure cancels active work and waits for
cleanup, including FFmpeg/probe children. The frontend should remain alive until
helper exit; the Qt implementation uses a bounded emergency termination fallback.

Input lines are limited to 64 KiB including the newline. Oversized or unterminated
input ends the connection with cleanup. There are at most 16 concurrent queries
or control requests and 16 queued stop acknowledgements. Output uses a bounded
32-message queue; progress may coalesce, while critical messages have a bounded
enqueue wait. An unresponsive frontend cannot stall the media session indefinitely.
The Qt client additionally bounds buffered output to 8 MiB and pending requests
to 32. Large libraries are not sent wholesale: inspection covers one file.

## Offline conversion

`preview_conversion` probes the file and reports its source and planned target
without writing prepared files, acquiring a sleep inhibitor, contacting a Cast
receiver or changing resume state. It is a read-only query and can finish out of
order; frontends must discard stale previews when the selected device changes.

`convert` starts preparation using the same automatic selection as casting. Both
methods accept an optional `device_id`. When present, it must identify a video
receiver in this helper's latest discovery results; its advertised model selects
the bundled-plus-user policy. Unknown models use Baseline. No receiver connection
or reachable IPv4 address is needed for conversion. Invalid IDs or audio-only
devices return `invalid_device`. With no ID (or null), use conservative H.264/stereo
AAC MP4 without reading device overrides. An empty string is an invalid ID.

`convert` resolves policy and probes again, rather than trusting a previous
preview. Its acknowledgement precedes events. `start` and `convert` reject
overlapping work with `busy`; IDs share the helper's monotonic sequence. No CLI
preferences, subtitle selection, HTTP server or resume state are involved.
Completed output uses the shared retained cache. An already compatible input
completes without invoking FFmpeg or creating a duplicate.

```json
{"id":9,"method":"preview_conversion","params":{"file":"/path/to/video.mkv","device_id":"example-device-id"}}
{"id":10,"method":"convert","params":{"file":"/path/to/video.mkv","device_id":"example-device-id"}}
{"id":10,"ok":true,"result":{"operation_id":2}}
{"event":"conversion_state","operation_id":2,"state":{"phase":"preparing","source":null,"target":null,"operation":"Copying video and audio into MP4","fraction":0.42,"message":"Copying video and audio into MP4","output":null,"reused":false,"already_compatible":false,"warnings":[],"error":null}}
{"id":11,"method":"cancel_conversion","params":{"operation_id":2}}
```

Snapshots contain `phase` (`inspecting`, `preparing`, `completed`, `cancelled`,
`failed`), nullable `source`/`target` media objects, nullable display-text
`target_description` and `operation`, nullable `fraction` in 0..1, display-text `message`, nullable `output`
path, booleans `reused`/`already_compatible`, `warnings` (strings), and nullable
`error`. `target_description` reports the accepted target formats while inspecting,
including any configured permissions. Nullable `planned_target` contains
`container` and `streams` after selecting a plan, before cache lookup. Its stream
fields use the media schema for format display: copied streams retain source
metadata; encoded streams describe the selected codec/profile/channels, leaving
unmeasured resolution/frame rate null. These are projected format fields, not
probed output or selectable track indices. `target` becomes actual probed output
metadata on success. Render `target` when available, otherwise `planned_target`,
in the same rows. The KDE UI starts with placeholders while inspecting and does
not parse `target_description` prose or hardcode the target codec. Fraction may reset to indeterminate
for cache validation; it reaches 1 only on successful completion.

Progress snapshots can coalesce. A reliable `conversion_ended` event carries
`operation_id` and the complete final `state` after cleanup and inhibitor release.
Ignore subsequent snapshots for that operation. Cancellation acknowledgements
wait for cleanup and precede this event. Cancellation racing completion may
still finish successfully. A stale cancellation receives `invalid_operation`.
At most 16 cancellation acknowledgements may be pending. Shutdown, EOF and
output failure cancel and join conversions as well as playback/query tasks.
Auto-close timing belongs to the frontend, not the core or protocol.

Before accepting `start` or a device-targeted `preview_conversion`/`convert`, the
helper snapshots the bundled model database merged with
`$XDG_CONFIG_HOME/yeet/devices.toml` (normally `~/.config/yeet/devices.toml`).
Invalid overrides produce `operation_failed` without starting the operation. A
missing file retains bundled rules. Edits do not affect an active operation.
CLI preferences are not read. Broad-compatibility conversion/preview, inspect and
discovery do not load device overrides. The core receives a resolved policy for
conversion and a database snapshot for casting, keeping configuration loading at
the frontend boundary. See [override semantics](device-compatibility.md#user-overrides).
