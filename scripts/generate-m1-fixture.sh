#!/usr/bin/env bash
set -euo pipefail
output_dir="${1:-samples/m1}"
duration="${2:-720}"
if [[ ! "$duration" =~ ^[1-9][0-9]{0,4}$ ]] || (( duration > 86400 )); then
    echo "Duration must be a whole number of seconds from 1 to 86400" >&2
    exit 1
fi
mkdir -p "$output_dir"
if [[ -e "$output_dir/test.mp4" || -e "$output_dir/subtitles.vtt" || -e "$output_dir/subtitles.srt" ]]; then
    echo "Refusing to replace an existing fixture in $output_dir" >&2
    exit 1
fi
# Silent audio avoids changing the user's volume. The default twelve minutes
# allows a long playback test; pass an output directory and duration for a short one.
ffmpeg -nostdin -v error -n \
    -f lavfi -i 'testsrc2=size=640x360:rate=15' \
    -f lavfi -i 'anullsrc=channel_layout=stereo:sample_rate=48000' \
    -t "$duration" -c:v libx264 -preset ultrafast -crf 30 -pix_fmt yuv420p \
    -profile:v baseline -level:v 3.0 -g 30 -c:a aac -b:a 64k \
    -movflags +faststart "$output_dir/test.mp4"
{
    printf 'WEBVTT\n\n'
    for ((second=0; second<duration; second+=5)); do
        end=$((second+5))
        if (( end > duration )); then end=$duration; fi
        printf '%02d:%02d:%02d.000 --> %02d:%02d:%02d.000\n' \
            "$((second/3600))" "$((second/60%60))" "$((second%60))" \
            "$((end/3600))" "$((end/60%60))" "$((end%60))"
        printf 'YEET subtitles working — cue %03d — %02d:%02d\n\n' \
            "$((second/5+1))" "$((second/60))" "$((second%60))"
    done
} > "$output_dir/subtitles.vtt"
ffmpeg -nostdin -v error -n -i "$output_dir/subtitles.vtt" "$output_dir/subtitles.srt"
echo "Generated ${duration}s fixture: $output_dir/test.mp4, subtitles.vtt and subtitles.srt"
