#!/usr/bin/env bash
# Refreshes data/icons/ (the map icon sprites the reader matches) from the installed game:
# run it after each game patch.
#
#   scripts/update-icons.sh [game folder]   (default: the Steam install of Path of Exile 2)
#
# Needs poe_data_tools (reads the game files, read-only; $POE_DATA_TOOLS, else on PATH),
# ImageMagick (`magick`) and node. Steps: dump the MinimapIcons and WorldAreas tables and
# the icon sheet art/2dart/minimap/player.dds (as PNG), cut the icons the app needs
# (scripts/import-icons.mjs). Works in a temporary folder under ~/.cache, removed at the end.
set -euo pipefail

game="${1:-$HOME/.local/share/Steam/steamapps/common/Path of Exile 2}"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

tool="${POE_DATA_TOOLS:-$(command -v poe_data_tools || true)}"
if [[ -z "$tool" || ! -x "$tool" ]]; then
  echo "poe_data_tools not found (cargo install --git https://github.com/adamthedash/poe_data_tools," >&2
  echo "or point POE_DATA_TOOLS at the binary)." >&2
  exit 1
fi
command -v magick >/dev/null || { echo "ImageMagick (magick) not found" >&2; exit 1; }
if [[ ! -d "$game/Bundles2" ]]; then
  echo "no game install at: $game (expected a Bundles2 folder)" >&2
  exit 1
fi

mkdir -p "$HOME/.cache"
work="$(mktemp -d "$HOME/.cache/poe-batonga-icons.XXXXXX")"
trap 'rm -rf "$work"' EXIT
pdt() { "$tool" --patch 2 --steam "$game" "$@"; }

echo "1/3 tables"
pdt dump-tables --mode json "$work/tables" data/balance/minimapicons.datc64 data/balance/worldareas.datc64 2>"$work/log"

echo "2/3 icon sheet"
pdt dump-art "$work/art" art/2dart/minimap/player.dds 2>>"$work/log"
sheet="$work/art/art/2dart/minimap/player.png"
[[ -f "$sheet" ]] || { cat "$work/log" >&2; echo "no $sheet" >&2; exit 1; }

manifest="$(dirname "$(dirname "$game")")/appmanifest_2694490.acf"
build="$(sed -n 's/^\s*"buildid"\s*"\([0-9]*\)".*/\1/p' "$manifest" 2>/dev/null || true)"
updated="$(sed -n 's/^\s*"LastUpdated"\s*"\([0-9]*\)".*/\1/p' "$manifest" 2>/dev/null || true)"

echo "3/3 import"
node "$repo/scripts/import-icons.mjs" "$work/tables" "$sheet" \
  ${build:+--game-build "$build"} ${updated:+--game-updated "$updated"} --tool "$("$tool" --version)"
echo "done: data/icons/"
