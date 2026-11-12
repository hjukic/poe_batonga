#!/usr/bin/env bash
# Checks the checklists' npcLine triggers against the installed game: run it after each patch.
#
#   scripts/check-triggers.sh [game folder]   (default: the Steam install of Path of Exile 2)
#
# Needs poe_data_tools (reads the game files, read-only): $POE_DATA_TOOLS, else on PATH.
# Dumps the dialogue tables (NPCTextAudio, NPCs, NPCTalk, NPCTalkDialogueSteps,
# NPCTalkDialogueTextAudio) and runs scripts/check-triggers.mjs on them; exit code 1 when
# a trigger's line changed or is gone. Works in a temporary folder under ~/.cache and
# removes it at the end. Changes nothing in the repository.
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
work="$(mktemp -d "$HOME/.cache/poe-batonga-triggers.XXXXXX")"
trap 'rm -rf "$work"' EXIT

echo "1/2 tables"
tables=()
for t in npctextaudio npcs npctalk npctalkdialoguesteps npctalkdialoguetextaudio; do
  tables+=("data/balance/$t.datc64")
done
"$tool" --patch 2 --steam "$game" dump-tables --mode json "$work/tables" "${tables[@]}" 2>"$work/log" || {
  cat "$work/log" >&2
  exit 1
}

echo "2/2 check"
node "$repo/scripts/check-triggers.mjs" "$work/tables" --routes "$repo/data/routes"
