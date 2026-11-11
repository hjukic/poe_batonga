# Qutm layout diagrams (imported)

`layouts.json` is an unmodified copy of
[Vyary/qutm `src/assets/layouts.json`](https://github.com/Vyary/qutm/blob/d18b4e6abee3f87c90342222918c6b08422967c5/src/assets/layouts.json)
at revision `d18b4e6abee3f87c90342222918c6b08422967c5`, imported 2026-11-11.
Upstream last changed this file on 2026-09-14 (Interlude layouts and the
Forbidden Rites content update).

## Contents

94 zones keyed by internal area ID (`G1_1` = The Riverbank, the same IDs as the
`Generating level … area "…"` log line), with 106 layout variants in total. Each
variant is an authored diagram:

- `name`: the variant's name (`"1"`, `"2"`, …).
- `icons[]`: `{ id, label?, x, y }`. `x` and `y` are 0-1 positions on the diagram,
  x to the right and y down. They match the map's on-screen orientation, but they
  are not to scale and are not game coordinates.
- `lines[]`: `{ fromIconId, toIconId }`, the suggested route between icons, as
  indexes into `icons`.
- `image`: empty in this snapshot; no pictures are included.

Icon ids include `myplayer` (where you start), `entrance` (zone transitions; the
`label` names the destination), `waypoint`, `checkpoint`, `boss` (label = boss),
`reward generic`, `quest` and league mechanics (`ritual`, `abyss`, …).

## How we use it

These are points of interest and directions, not wall maps. They are hand-made
diagrams, so a position is approximate and an instance may be laid out in a way no
variant covers (rotated, mirrored or with swapped exits). To place them on the
map we record ourselves (`src-tauri/src/layout.rs`), we need each zone's scale and
which variant is being played. Our recordings supply both.
Qutm's icon images are not imported.
