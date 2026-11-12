#!/bin/sh
# Copies the game's label font (Fontin Regular) into data/fonts/, where the big-map label
# reader compiles it in (src-tauri/src/labels.rs). Run after a game patch if labels stop
# being read.
#   scripts/fetch-fonts.sh <poe_data_tools> ["<Path of Exile 2 folder>"]
set -eu
tool=${1:?usage: scripts/fetch-fonts.sh <poe_data_tools> [game folder]}
game=${2:-"$HOME/.local/share/Steam/steamapps/common/Path of Exile 2"}
out="$(dirname "$0")/../data/fonts"
mkdir -p "$out"
"$tool" --patch 2 --steam "$game" cat art/2dart/fonts/fontin-regular.ttf > "$out/fontin-regular.ttf.part"
mv "$out/fontin-regular.ttf.part" "$out/fontin-regular.ttf"
echo "data/fonts/fontin-regular.ttf: $(wc -c < "$out/fontin-regular.ttf") bytes"
