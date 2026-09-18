#!/usr/bin/env bash
# Explicit, interactive hardware validation. Never run as part of cargo test.
set -euo pipefail
shopt -s nullglob
if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "Usage: $0 DEVICE_NAME_OR_ID [HTTP_PORT] | --prepare-only" >&2
    exit 2
fi
device=$1
prepare_only=false
[[ "$device" != --prepare-only ]] || prepare_only=true
port=${2:-8010}
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
cargo build --locked
binary="$root/target/debug/procast"
work=$(mktemp -d "$root/samples/m5-batch.XXXXXX")
echo "Clips, retained outputs, and logs: $work"
pid=
cleanup() {
    if [[ -n "$pid" ]] && jobs -pr | grep -qx "$pid"; then
        kill -INT "$pid" 2>/dev/null || true
        wait "$pid" || true
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Strip titles, chapters, and embedded subtitles; retain original codec payloads.
# No source library file is modified or used as a cache destination in this run.
ids=(009 004 005 006 051 008 010 001 022 041 007)
for id in "${ids[@]}"; do
    sources=("$root"/samples/library/no-transcode-candidates/sample-"$id"_*)
    if [[ ${#sources[@]} -ne 1 || ! -f "${sources[0]}" ]]; then
        echo "Missing or ambiguous neutral sample $id; stopping before casting." >&2
        exit 1
    fi
    source=${sources[0]}
    extension=${source##*.}
    echo "Preparing 30-second sample $id"
    ffmpeg -nostdin -hide_banner -v error -n -i "$source" -t 30 \
        -map 0:v:0 -map '0:a:0?' -c copy -map_metadata -1 -map_chapters -1 \
        -sn -dn "$work/sample-$id.$extension"
done
cat > "$work/subtitles.vtt" <<'EOF'
WEBVTT

00:00.000 --> 00:15.000
PROCAST: first 15 seconds

00:15.000 --> 00:29.000
PROCAST: second half
EOF
cat > "$work/subtitles.srt" <<'EOF'
1
00:00:00,000 --> 00:00:15,000
PROCAST: first 15 seconds

2
00:00:15,000 --> 00:00:29,000
PROCAST: second half
EOF

if $prepare_only; then
    echo "Prepared batch files: $work"
    exit 0
fi

check_port() {
    if [[ -n $(ss -H -ltn "sport = :$port") ]]; then
        echo "HTTP port $port is still in use." >&2; exit 1
    fi
}
check_visual() {
    local answer
    read -r -p "Picture, sound, timing, and captions OK? [y/N] " answer
    [[ "$answer" == y || "$answer" == Y ]] || { echo "Hardware check failed; logs retained in $work" >&2; exit 1; }
}
run_case() {
    local id=$1 expected=$2 subtitle=$3
    shift 3
    local clips=("$work/sample-$id.mp4" "$work/sample-$id.mkv")
    local clip=
    for candidate in "${clips[@]}"; do [[ ! -f "$candidate" ]] || clip=$candidate; done
    [[ -n "$clip" ]]
    local label="$id-$expected" log="$work/$id-$expected.log"
    check_port
    echo "Testing sample $id: expect $expected; allow the short clip to finish."
    "$binary" cast "$clip" --device "$device" --http-port "$port" \
        --subtitles "$work/subtitles.$subtitle" "$@" 2>&1 | tee "$log"
    grep -qF "Selected $expected" "$log"
    grep -qF 'Playback completed.' "$log"
    check_port
    check_visual
    if [[ "$expected" != Direct ]]; then
        echo "Repeating $label with FFmpeg unavailable to prove persistent reuse."
        "$binary" cast "$clip" --device "$device" --http-port "$port" \
            --subtitles "$work/subtitles.vtt" --ffmpeg "$work/ffmpeg-intentionally-missing" "$@" \
            2>&1 | tee "$work/$label-reuse.log"
        grep -qF 'Reusing prepared file:' "$work/$label-reuse.log"
        grep -qF 'Playback completed.' "$work/$label-reuse.log"
        check_port
        check_visual
    fi
}

# Auto profile is expected to select Extended on the development receiver.
run_case 009 Direct vtt
run_case 004 Direct srt
run_case 005 Direct vtt
run_case 006 Remux srt
run_case 051 Remux vtt
run_case 008 Remux vtt
run_case 010 Audio srt
run_case 001 Audio vtt
run_case 022 Audio vtt
run_case 041 Audio vtt
run_case 007 Transcode vtt
# Baseline override must encode HEVC, regardless of the discovered model.
run_case 005 Transcode vtt --profile baseline

# Cancellation with caching disabled must remove its own temporary preparation.
check_port
mkdir "$work/temporary"
"$binary" cast "$work/sample-041.mp4" --device "$device" --http-port "$port" \
    --no-cache --cache-dir "$work/temporary" > "$work/cancel.log" 2>&1 &
pid=$!
started=$SECONDS
while ! grep -q '^Playing' "$work/cancel.log"; do
    if ! kill -0 "$pid" 2>/dev/null || (( SECONDS - started > 120 )); then
        cat "$work/cancel.log"; echo "Cancellation test never reached playback." >&2; exit 1
    fi
    sleep 0.2
done
read -r -p "Playback started. Press Enter now to test cancellation (before the 30-second clip ends). " _
jobs -pr | grep -qx "$pid" || { echo "Clip already ended; rerun the cancellation check." >&2; exit 1; }
kill -INT "$pid"
set +e
wait "$pid"
status=$?
set -e
pid=
cat "$work/cancel.log"
[[ $status -eq 130 ]]
grep -qF 'Cancelled; cleanup completed.' "$work/cancel.log"
remaining=("$work/temporary/"*)
[[ ${#remaining[@]} -eq 0 ]]
check_port
echo "All batch checks passed. Retained outputs and logs: $work"
echo "Seeking and long-duration playback are separate checks; this batch covers startup, captions, completion, reuse, and cancellation."
