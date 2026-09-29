#!/usr/bin/env bash
# Re-export the checked-in app icons after editing the SVG master. Requires Inkscape.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
icon_root="$repo_root/apps/werp-kde/icons"
master="$icon_root/scalable/apps/nl.hnsr.Werp.svg"

for size in 16 22 24 32 48 64 96 128 256 512; do
    destination="$icon_root/${size}x${size}/apps/nl.hnsr.Werp.png"
    mkdir -p -- "$(dirname -- "$destination")"
    inkscape "$master" --export-area-page --export-background-opacity=0 \
        --export-width="$size" --export-height="$size" --export-filename="$destination"
done
