#!/usr/bin/env bash
# Local-only fixture generation; never discovers or contacts a receiver.
set -euo pipefail
output=${1:-samples/preferences}
seconds=${2:-60}
[[ "$seconds" =~ ^[0-9]+$ ]] && (( seconds >= 20 && seconds <= 300 )) || {
    echo 'Duration must be an integer from 20 to 300 seconds.' >&2; exit 2;
}
mkdir -p -- "$output"
for file in test.mp4 embedded.mkv external.mp4 english.srt dutch.srt forced.srt external.srt dutch.toml; do
    [[ ! -e "$output/$file" ]] || { echo "Refusing to replace existing fixture: $output/$file" >&2; exit 1; }
done
for language in english dutch forced; do
    index=1
    for ((start=0; start<seconds; start+=10)); do
        end=$((start+10)); (( end <= seconds )) || end=$seconds
        printf '%d\n00:%02d:%02d,000 --> 00:%02d:%02d,000\nYEET %s: %d-%d seconds\n\n' \
            "$index" "$((start/60))" "$((start%60))" "$((end/60))" "$((end%60))" \
            "$language" "$start" "$end" >> "$output/$language.srt"
        index=$((index+1))
    done
done
ffmpeg -nostdin -hide_banner -v error -n \
    -f lavfi -i testsrc2=size=640x360:rate=15 \
    -f lavfi -i sine=frequency=440:sample_rate=48000 \
    -t "$seconds" -c:v libx264 -preset veryfast -pix_fmt yuv420p -profile:v baseline -level:v 3.0 \
    -c:a aac -ac 2 -b:a 128k -af volume=0.1 -movflags +faststart "$output/test.mp4"
ffmpeg -nostdin -hide_banner -v error -n \
    -i "$output/test.mp4" -i "$output/forced.srt" -i "$output/english.srt" -i "$output/dutch.srt" \
    -map 0:v -map 0:a -map 1:s -map 2:s -map 3:s -c:v copy -c:a copy -c:s srt \
    -metadata:s:s:0 language=eng -metadata:s:s:0 title=Forced -disposition:s:0 default+forced \
    -metadata:s:s:1 language=eng -metadata:s:s:1 title=English -disposition:s:1 0 \
    -metadata:s:s:2 language=dut -metadata:s:s:2 title=Nederlands -disposition:s:2 0 \
    "$output/embedded.mkv"
cp -- "$output/test.mp4" "$output/external.mp4"
cp -- "$output/english.srt" "$output/external.srt"
cat > "$output/dutch.toml" <<'TOML'
[subtitles]
auto_load = true
languages = ["nl", "en"]

[playback]
resume = false
TOML
printf 'Prepared %s-second fixtures in %s (no TV contacted).\n' "$seconds" "$output"
printf 'Embedded indexes: 2 = forced English, 3 = full English, 4 = Dutch.\n'
