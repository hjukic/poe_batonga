#!/usr/bin/env bash
# Fast-forwards main to a branch and pushes, but only while the player is in a town (or the
# game is closed): the overlay runs as `tauri dev` on this checkout and restarts on every
# change, and a restart inside a zone splits its recording (review, 2026-11-12).
#
#   scripts/land-in-town.sh <branch>
set -euo pipefail
branch=${1:?usage: scripts/land-in-town.sh <branch>}
log="${POE2_CLIENT_LOG:-$HOME/.local/share/Steam/steamapps/common/Path of Exile 2/logs/Client.txt}"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

in_town_or_closed() {
  hyprctl clients -j 2>/dev/null | grep -q '"steam_app_2694490"' || return 0
  local last
  last=$(grep -a -o 'Generating level [0-9]* area "[^"]*"' "$log" | tail -n 1)
  [[ ${last,,} == *town\"* ]]
}

until in_town_or_closed; do sleep 5; done
cd "$repo"
git merge -q --ff-only "$branch"
git -c credential.helper='!gh auth git-credential' push -q origin main
git log --oneline -1
