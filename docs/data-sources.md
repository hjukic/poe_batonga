# Data sources

This is a private project for two players; licences are not a selection
criterion (owner, 2026-11-12). The machine-readable catalog is
[`data/sources.json`](../data/sources.json).

Release target amended by the owner on 2026-11-12: **PoE 2 1.0, Acts 1–6**,
excluding Interludes. Selected Early Access sources are references pending 1.0
validation. Their Interlude coverage does not establish Act 5 or Act 6 coverage.
See the [navigation-rule audit](navigation-rule-research.md) for source dates,
candidate rules and the remaining 1.0 validation work.

| Data | Source | In the repo |
| --- | --- | --- |
| Campaign route | [domistae/poe2-leveling](https://github.com/domistae/poe2-leveling/tree/f682e2b99c4a5a4f31882f3674bba4dcefc6ab46), starting with `poe2_act1_guide.html` | Not yet; our own Riverbank route is in `data/routes/` |
| Zone IDs/names | Our own observations from the owner's local PoE 2 log | `data/observations/campaign-zones.json` |
| Zone layout graphs (every campaign zone) | The game files, `scripts/update-layouts.sh` | `data/layouts/ggg/` |
| Zone terrain (walls, POIs) | [poe2way](https://www.poe2way.com/atlas) collected terrain, first three zones | `data/layouts/poe2way/` |

## Route reference

Reviewed revision: `f682e2b99c4a5a4f31882f3674bba4dcefc6ab46`. It targets patch
0.5.5 and flags reconciliation still in progress, so review each adapted
objective. Keep mandatory progression, optional rewards and optional detours
distinct; never infer completion from a guide's route order. Import a reviewed
snapshot when defining the route format, not mutable HTML at startup. Exclude
endgame/crafting content.

## Collected zone observations

[`data/observations/campaign-zones.json`](../data/observations/campaign-zones.json)
contains 97 campaign area IDs collected from 3,072,602 local log lines spanning
2025-09-09 to 2026-09-10. It stores observed scene names, levels and generation
record counts; no character names, chat or full log lines. An empty name list
means no nearby scene-name observation was paired.

```sh
node scripts/collect-zone-observations.mjs "/path/to/logs/Client.txt" data/observations/campaign-zones.json
```

Do not hard-code observed area levels: patches/variants may change them. Unknown
IDs should display the ID. See [log findings](log-findings.md).

## poe2way zone terrain

poe2way's atlas and act guide load each area from
`https://www.poe2way.com/api/map-layouts/<areaId>` (same area IDs as `Client.txt`).
[`scripts/fetch-poe2way-terrain.mjs`](../scripts/fetch-poe2way-terrain.mjs) stores
it compactly in `data/layouts/poe2way/` (fetched 2026-11-12, game release IDs in
each file):

```sh
node scripts/fetch-poe2way-terrain.mjs            # G1_1 G1_town G1_2 by default
node scripts/render-terrain.mjs                   # pictures in experiments/layouts/output
```

We start with the first three zones (The Riverbank, Clearfell Encampment,
Clearfell) and add more once navigation works there; other area IDs can be
passed to the script. `render-terrain.mjs` draws each zone north-up and turned
45° with y up (`.game.svg`), as the in-game map shows it. That turn was fitted on
a recorded Clearfell Encampment visit: 90% of its minimap walls lie on poe2way's
outline at 0.78 minimap cells per game unit. The arrow (`src-tauri/src/layout/guide.rs`)
bundles these files; see [map reading](map-reading.md#poe2way-terrain-arrow-2026-11-12).

Per area there is `<areaId>.json` and `<areaId>.svg` (extra variants as
`<areaId>.v1.*`), plus `index.json`:

- `<areaId>.svg`: the walkable floor's outline (outer edge and holes) as one path in
  game-space units, traced on a 3-unit grid. Inside = walkable, outside = wall.
- `terrain.bounds`: `[minX, minY, maxX, maxY]` of the walkable area in the same
  units; `terrain.cells`: its size in grid cells.
- `objects[]`: `{ kind, x, y, label, destination }` in the same coordinates. Kinds:
  `checkpoint`, `transition` (zone exit; `destination.areaId` when known),
  `waypoint`, `boss`, `npc`, `object`, `portal`, `stash`.

Coverage: Clearfell Encampment and Clearfell. poe2way has no terrain for The
Riverbank (`G1_1`); for it we have only our own recordings. Exits without a known
destination are labelled `exit` (the encampment's bottom exit leads to The
Riverbank). NPCs appear several times where they stand at different quest stages.

**Limits.** Each area is one *collected instance* ("collected terrain example");
Campaign zones are generated per instance, so the
walls are an example, not the instance being played. The overall shape and the
order of exits/POIs are likely to be similar, but the exact walls are not; live
pathing needs the player's explored minimap or our recordings to confirm which
parts match. Towns are fixed and should match exactly.

## Game layout graphs (datamined)

`data/layouts/ggg/<areaId>.json` comes from the game files (local Steam install,
read-only) via [poe_data_tools](https://github.com/adamthedash/poe_data_tools) and
[`scripts/import-topologies.mjs`](../scripts/import-topologies.mjs). Refresh it after
every game patch with one command:

```sh
scripts/update-layouts.sh [game folder]   # default: ~/.local/share/Steam/steamapps/common/Path of Exile 2
```

It needs `poe_data_tools` (`$POE_DATA_TOOLS`, else `PATH`; install with
`cargo install --git https://github.com/adamthedash/poe_data_tools`). It dumps the
`WorldAreas` and `Topologies` tables, extracts only the graph files (and their
`master.tsi`) that the campaign areas name, writes one JSON per area plus
`index.json`, removes files of areas that are gone, and runs
`node --test scripts/import-topologies.test.mjs`. It works in a temporary folder under
`~/.cache` and takes about half a minute. The game has no version file: `index.json`
records the Steam build id and update time from `appmanifest_2694490.acf`, the import
date and the tool version, and per area its id, name, act, level, number of layouts and
format.

**Which areas:** ids `G<act>_…` and `P<part>_…` (the interludes, act 6 in the table)
with at least one topology; not `…_WorldMap`, map areas, hideouts, or areas named
`[DNT…]` (unused: `G4_ship`, `G4_6`, `G4_12`, `P2_4`).

**Coverage** (import of 2026-11-12, Steam build 26579160):

| Act | Areas | Layouts | .tgr | .dgr | JSON | Areas whose layouts have no exit |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| 1 | 17 | 213 | 59 | 154 | 522 KiB | G1_12 Freythorn, G1_15 Ogham Manor (sub-graphs) |
| 2 | 20 | 123 | 8 | 115 | 364 KiB | G2_12 Dreadnought (sub-graph) |
| 3 | 20 | 134 | 0 | 134 | 459 KiB | G3_town, G3_1 (sub-graphs), G3_15 |
| 4 | 22 | 82 | 0 | 82 | 229 KiB | G4_7, G4_13, G4_14 |
| 6 (interludes) | 22 | 29 | 2 | 27 | 65 KiB | P1_6, P3_2, P3_5 (sub-graphs) |
| **All** | **101** | **581** | **69** | **512** | **1.6 MiB** | |

The app bundles every file (`build.rs`) but so far only uses `G1_1` and `G1_2`
(`AREAS` in `src-tauri/src/layout/graph.rs`).

### The JSON

`{areaId, name, act, level, connections[], connectionNames[], topologies[]}`, one line.
Each topology: `{id, file, format, size, roomUnitSize (dgr only), nodes[], edges[]}`.

- `format`: `"tgr"` (outdoor zones) or `"dgr"` (dungeons, towns, most indoor zones).
- `size`: the layout's grid: tiles for `.tgr`, room cells for `.dgr`.
- `nodes[]`: `{x, y, tag, exit, start, keys}`. `tag` is the node's name (`town_E`,
  `mud_entrance`, `boss`, `campsite`, `arena_*`, `graph`, often empty). `keys` are
  every quoted string after the tag (`"entrance1"`, `"default"`, `"killzone=5"`,
  `"teleport1disabled-MudBurrowArena"`, `"zone=2"`, `"unique1"`, …), kept so
  objectives can be found later. `exit` is the area the first `entranceN` word (tag or
  key) leads to: the N-th of `WorldAreas.Connections`. `start` is true when the word
  `entrance` (tag or key) is on the node: where the zone starts. A node can be both.
- `edges[]`: `{a, b, via, kind}`: a corridor between nodes `a` and `b`, bends in `via`,
  `kind` the first `.et` (edge type) file's name (`slash`, `woods`, `cavern_rooms`, …).

### What the numbers mean

- **`.tgr`**: `x, y` and `via` are world units, x east, y north; `size` is in tiles of
  23 world units. Checked against recordings (Riverbank, Clearfell, encampment).
- **`.dgr`**: `x, y` are **grid units**, 23 per room cell: a node in cell (c, r) sits at
  (23c + 12, 23r + 12) (12,220 of 12,295 nodes do; the rest are between cells). `size`
  is the grid in cells (`Size: 5 6`). From the room files the cell size is known, not
  measured in play:
  - The graph's `MasterFile` (`master.tsi`) gives `RoomUnitSize`: tiles per room cell
    (1 to 29, mostly 7 or 10), stored as `roomUnitSize`.
  - The rooms (`.arm`) of a 1×1 cell are `RoomUnitSize` tiles wide, a 2×2 room
    `2·RoomUnitSize − 1` (Mud Burrow, unit 9: 9×9 and 17×17; Venom Crypts, unit 7:
    7×7 and 13×13): neighbouring rooms share their border tile, so cells are
    `RoomUnitSize − 1` tiles apart.
  - So one grid unit is about `(roomUnitSize − 1)` world units (a cell
    `(roomUnitSize − 1) × 23`). **A derivation, not checked against a recorded
    dungeon**: no `.dgr` zone has been recorded yet (`shared/recordings/` has only
    G1_1, G1_2, G1_town). The master's `OuterBufferSize` (5 tiles in Mud Burrow) is
    probably a margin around the grid, so the origin may be offset.
- **Edge numbers after the `.et` file** (`.tgr`: `40 50`, `15 90`; `.dgr`: `30 40`,
  `0 100`, `100 100`): pairs with the first ≤ the second, all 0–100: probably a
  min–max percentage (width or chance), **not** a length. Unknown.

### Format notes

- Both formats are the same UTF-16 text: `version`, `Size:`, `MasterFile:`, `Nodes:`,
  `Edges:`, ground-type lines (`""` or a `.gt` path, three or four), then one line per
  node and per edge. `.dgr` files add `EnvironmentOverride:` (some), `Default%: 0 0 0`
  and a lone count line before the nodes. Most are `version 25`; two are 21 or 23
  (shorter node lines). The importer skips everything up to the first line that starts
  with three numbers and refuses a node or edge line of the wrong shape.
- Node line: `x y degree neighbour… "tag" rotation keyCount "key"… numbers…`; the
  rotation is `I`, `FI`, `R90`, `R270`, `(any)`, `(best)`, ….
- **Sub-graphs**: 168 nodes in 15 areas are tagged `graph`; their keys name another
  graph (`"Metadata/…/_CanopyE_N.dgr" "3" "3"`: file and size in cells) or a bare
  name that picks one of several files (`"midlevel_S"` → `_midlevel_s_1.dgr` or
  `_2.dgr`), placed with its top-left cell at the node. The top-level graphs of
  Freythorn, Ogham Manor, Dreadnought, the Ziggurat Encampment, Sandswept Marsh,
  Holten Estate, Kriar Village and Kriar Peaks are only such placeholders, so their
  layouts have no exits yet. Not expanded: next step (extract the named files, place
  them by the node's cell and rotation).
- A node often carries several `entranceN` keys (144 nodes; Ardura Caravan's single
  town node has 26): `exit` holds only the first; `keys` has them all.
- Three exits lead out of the campaign data (Grelwood and Infested Barrens to
  expedition areas, Mastodon Badlands to `Abyss_Intro`); one `entranceN` points past the
  connection list (`G1_5` `entrance2`, `G3_15` `entrance1`) and stays `null`.

First check: the recorded Clearfell visit (seed 2693856399) fits `ClearFell_W_S_N`
best with no extra turn at 0.78 minimap cells per unit, the scale measured on the
encampment: its walked path lies on average 44 units from that graph's corridors,
57 for the runner-up `W_N_E`, 75+ for the rest. The screen-to-world turn is the
one fitted on the encampment (`to_screen` in `guide.rs`).

## Other candidates

- [Exile-UI](https://github.com/Lailloken/Exile-UI) Act-Decoder layouts: its
  [changelog](https://github.com/Lailloken/Exile-UI/blob/main/data/changelog.json)
  warns they are outdated for PoE 2.
- Mobalytics campaign-layout guides (Act 1-3) and the PoE 2 wiki have layout
  pictures; not imported yet.
- [Qutm](https://github.com/Vyary/qutm) layout diagrams (0-1 icon positions, no walls):
  used for the first arrow, dropped 2026-11-12 in favour of poe2way's terrain.
- Datamining the game files: see [ideas](../ideas.md).

Offline tests may use synthetic maps, clearly labelled synthetic; they cannot
establish performance on real gameplay.
