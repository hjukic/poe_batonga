#!/usr/bin/env bash
# Refreshes data/layouts/ggg/ from the installed game: run it after each game patch.
#
#   scripts/update-layouts.sh [game folder]   (default: the Steam install of Path of Exile 2)
#
# Needs poe_data_tools (reads the game files, read-only): $POE_DATA_TOOLS, else on PATH.
# Steps: dump the WorldAreas and Topologies tables, extract the graph files the campaign
# areas name (and their master.tsi files), import them (one JSON per area, index.json, stale files removed).
# Works in a temporary folder under ~/.cache and removes it at the end. Safe to re-run.
set -euo pipefail

game="${1:-$HOME/.local/share/Steam/steamapps/common/Path of Exile 2}"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

tool="${POE_DATA_TOOLS:-$(command -v poe_data_tools || true)}"
if [[ -z "$tool" || ! -x "$tool" ]]; then
  echo "poe_data_tools not found. Install it with:" >&2
  echo "  cargo install --git https://github.com/adamthedash/poe_data_tools" >&2
  echo "or point POE_DATA_TOOLS at the binary." >&2
  exit 1
fi
if [[ ! -d "$game/Bundles2" ]]; then
  echo "no game install at: $game (expected a Bundles2 folder)" >&2
  exit 1
fi

mkdir -p "$HOME/.cache"
work="$(mktemp -d "$HOME/.cache/poe-batonga-layouts.XXXXXX")"
trap 'rm -rf "$work"' EXIT
pdt() { "$tool" --patch 2 --steam "$game" "$@"; }

echo "1/3 tables"
pdt dump-tables --mode json "$work/tables" data/balance/worldareas.datc64 data/balance/topologies.datc64 2>"$work/log"

echo "2/3 graph files"
mapfile -t graphs < <(node "$repo/scripts/import-topologies.mjs" --list "$work/tables")
pdt extract "$work/files" "${graphs[@]}" 2>>"$work/log"
extracted="$(find "$work/files" -type f | wc -l)"
if (( extracted != ${#graphs[@]} )); then
  echo "extracted $extracted of ${#graphs[@]} graph files; log:" >&2
  grep -v 'Extracted file' "$work/log" >&2 || true
  exit 1
fi
# The .dgr graphs name a master.tsi each; its RoomUnitSize gives the grid's scale.
mapfile -t masters < <(node "$repo/scripts/import-topologies.mjs" --masters "$work/tables" "$work/files")
pdt extract "$work/files" "${masters[@]}" 2>>"$work/log"

# The game has no version file; the Steam build id and update time identify the release.
manifest="$(dirname "$(dirname "$game")")/appmanifest_2694490.acf"
build="$(sed -n 's/^\s*"buildid"\s*"\([0-9]*\)".*/\1/p' "$manifest" 2>/dev/null || true)"
updated="$(sed -n 's/^\s*"LastUpdated"\s*"\([0-9]*\)".*/\1/p' "$manifest" 2>/dev/null || true)"

echo "3/3 import (${#graphs[@]} graph files)"
cd "$repo"
node scripts/import-topologies.mjs "$work/tables" "$work/files" \
  ${build:+--game-build "$build"} ${updated:+--game-updated "$updated"} --tool "$("$tool" --version)" | grep -E "^removed|areas, "

echo "done: data/layouts/ggg/ ($(( $(cat data/layouts/ggg/*.json | wc -c) / 1024 )) KiB)"
