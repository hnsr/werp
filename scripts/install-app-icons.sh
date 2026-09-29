#!/usr/bin/env bash
# Register the artwork by name so KDE can load it for development-build windows.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
icon_root="$repo_root/apps/werp-kde/icons"
destination="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"

for source in "$icon_root"/*/apps/nl.hnsr.Werp.{png,svg}; do
    [[ -f "$source" ]] || continue
    install -Dm644 -- "$source" "$destination/${source#"$icon_root/"}"
done
xdg-icon-resource forceupdate --mode user --theme hicolor
