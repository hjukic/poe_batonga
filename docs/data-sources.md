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

| Act | Areas | Layouts | .tgr | .dgr | Expanded | JSON | Areas whose layouts have no exit |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 1 | 17 | 213 | 59 | 154 | 40 | 627 KiB | G1_15 Ogham Manor (its one connection is the way back, marked as start) |
| 2 | 20 | 126 | 8 | 118 | 32 | 394 KiB | |
| 3 | 20 | 134 | 0 | 134 | 22 | 497 KiB | G3_town (exits inside the fixed town rooms), G3_15 (no connections) |
| 4 | 22 | 82 | 0 | 82 | 0 | 229 KiB | |
| 6 (interludes) | 22 | 29 | 2 | 27 | 3 | 76 KiB | |
| **All** | **101** | **584** | **69** | **515** | **97** | **1.8 MiB** | |

The app bundles every file (`build.rs`) but so far only uses `G1_1` and `G1_2`
.

### The JSON

`{areaId, name, act, level, connections[], connectionNames[], topologies[]}`, one line.
Each topology: `{id, file, format, size, roomUnitSize (dgr only), nodes[], edges[]}`, and
on layouts with sub-graphs `expanded: true`, `parts`, `guessedTurns`, maybe `alternatives`
(see Sub-graphs below).

- `format`: `"tgr"` (outdoor zones) or `"dgr"` (dungeons, towns, most indoor zones).
- `size`: the layout's grid: tiles for `.tgr`, room cells for `.dgr`.
- `nodes[]`: `{x, y, tag, exit, start, keys}`. `tag` is the node's name (`town_E`,
  `mud_entrance`, `boss`, `campsite`, `arena_*`, `graph`, often empty). `keys` are
  every quoted string after the tag (`"entrance1"`, `"default"`, `"killzone=5"`,
  `"teleport1disabled-MudBurrowArena"`, `"zone=2"`, `"unique1"`, …), kept so
  objectives can be found later. `exit` is the area the first `entranceN` word (tag or
  key) that names a connection leads to: the N-th of `WorldAreas.Connections` (Red Vale
  has rooms tagged `entrance2` with the key `entrance1` and only one connection). `start` is true when the word
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
  - Many masters also set `RoomOverlap` (0 or 1, or not at all; Mud Burrow has none
    although its rooms overlap). Sandswept Marsh's start part (unit 10, overlap 1,
    buffer 1: 7×6 cells) fills its 22×19 box of 3-tile cells exactly as
    `cells·(unit − 1) + 1 + 2·buffer` tiles, which supports the shared border tile;
    whether overlap 0 means no shared tile is not checked.
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
- **Sub-graphs**: 168 nodes in 15 areas are tagged `graph`: placeholders for another graph.
  Their keys are a file (`"Metadata/…/_CanopyE_N.dgr"`) or a group name (`"midlevel_S"`),
  then the box's width and height in the parent's cells. Group names are looked up in the
  `.fgp` files that the graph's `master.tsi` names (`FileGroups "filegroups.fgp"`): a quoted
  name, then its files, relative to the master's folder or absolute. All placeholders have no
  corridors of their own (degree 0): the parts are joined by stairs and teleports (keys such as
  `uniqueA0` on `StairsDown1` and `StairsUp1`), not by edges. The importer expands them:
  - The placeholder's cell is the box's corner with the lowest x and y; the sub-graph's nodes
    and edges are moved there and the placeholder is dropped (edges renumbered). Nested
    placeholders are expanded too.
  - Size: most sub-graphs have the box's size and the parent's `RoomUnitSize` (scale 1). In
    Freythorn (3×3 box, 10×10 sub-grid), the Halani Gates canyons and Sandswept Marsh the
    sub-graph is on another grid; it is scaled to fit the box, keeping its shape (`scale` in
    `parts`). For Sandswept Marsh this matches the tiles exactly (its end part is a `.tgr` of
    36×57 tiles in a 12×19 box of 3-tile cells); for the others it is an estimate.
  - Turns: `I` is exact. `R90`/`R270` (two canyons in the Halani Gates) are turned with a
    guessed direction; `(any)` and `(best)` (the game picks) are placed unturned. Each such
    placement counts in `guessedTurns`.
  - A group with several files: every choice is a layout of its own (`id#1`, `id#1-2`, … the
    choices in order) while the area stays at 60 layouts or fewer (Dreadnought: 1 → 4).
    Above that each placeholder keeps its first file and lists the group in
    `alternatives` (`{name, files, at}`): Ogham Manor (16 layouts × 2 basements × 2 middle
    floors would be 64).
  - `parts`: `{file, at, turn, scale}` per placed sub-graph.
  - Result: Freythorn, Dreadnought, Sandswept Marsh, Holten Estate, Kriar Village and Kriar
    Peaks now have exits. Ogham Manor's floors are placed but its only connection (the
    Ramparts) is the start. The Ziggurat Encampment is two one-node town rooms whose exits are
    inside the fixed rooms (like Clearfell Encampment).
- **Areas left without exits** (the test names them): Ogham Manor (above); the Ziggurat
  Encampment (above); the Library of Kamasa (`G3_15`): its `Connections` is empty, no area
  connects to it, so its `entrance1` names nothing.
- **Act 4's islands** (Shrike Island, Plunder's Point, Brinerot Cove) have no `entranceN`:
  the way back to Kingsmarch is the boat at the landing, keyed `teleport1` (Ngakanu's
  landing carries both `entrance1` and `teleport1`). When a layout has no exit and the
  area one connection, nodes keyed `teleport1` lead to it. Other `teleportN` keys are
  teleports inside a zone (arenas, stairs, interiors) and stay without `exit`.
- A node often carries several `entranceN` keys (144 nodes; Ardura Caravan's single
  town node has 26): `exit` holds only the first; `keys` has them all.
- Three exits lead out of the campaign data (Grelwood and Infested Barrens to
  expedition areas, Mastodon Badlands to `Abyss_Intro`); one `entranceN` points past the
  connection list (`G3_15` `entrance1`) and stays `null`.

First check: the recorded Clearfell visit (seed 2693856399) fits `ClearFell_W_S_N`
best with no extra turn at 0.78 minimap cells per unit, the scale measured on the
encampment: its walked path lies on average 44 units from that graph's corridors,
57 for the runner-up `W_N_E`, 75+ for the rest. The screen-to-world turn is the
one fitted on the encampment (`to_screen` in `guide.rs`).

## Game font

`data/fonts/fontin-regular.ttf`: Fontin Regular from the game's own data
(`art/2dart/fonts/fontin-regular.ttf`, extracted with poe_data_tools, `scripts/fetch-fonts.sh`),
compiled in to read the big map's exit labels. Private project; not redistributed.

## Game dialogue tables (datamined)

The `npcLine` triggers in `data/routes/` come from the game's dialogue tables, dumped with
poe_data_tools (`dump-tables --mode json`, Steam build of 2026-11-12):

| Table | What it gives |
| --- | --- |
| `NPCTextAudio` | Every spoken line: `Id`, `Text`, `NPCs` (rows of `NPCs`: who says it). World speech (boss barks, shouts, cutscenes) and the lines of most panel talks. |
| `NPCTalkDialogueTextAudio` | The lines of newer multi-step panel talks (Act 4, interludes); the speaker is in `NPC1`. 41 ids also appear in `NPCTextAudio` with the same text (endgame and interlude talks, `AsalaA2EndOfAct`, `HoodedOneNewTravelEast`); the checker reads those from `NPCTextAudio`. |
| `NPCs` | `Name` and `ShortName` per NPC row (`Metadata/NPC/…`, `Metadata/Monsters/…`). |
| `NPCTalk` | The options of an NPC's panel: `DialogueOption`, the lines (`NPCTextAudioKeys` or `TalkDialogue`), the quest flags it needs (`unknown_1`), hides on (`unknown_2`) and sets (`QuestFlags1`). |
| `NPCTalkDialogue`, `NPCTalkDialogueSteps` | A panel talk's lines in order; a step names its lines in `NPCTextAudio` or in `NPCTalkDialogueTextAudio`. |

Measured, from the owner's log (`Client.txt`, 2026-11-11/12): every NPC line seen (the Wounded
Man, Mortimer, The Bloated Miller, Renly, Una; 36 lines) is an `NPCTextAudio` row with the same
text, and the speaker is that row's NPC `Name` (for those five `Name` equals `ShortName`).
Derived from the tables: `Name` is the field to match, because `ShortName` is empty for 81 NPCs
that speak; that the log shows the full `Name` where they differ ("Lachlann the Lost", not
"Lachlann") is assumed, not yet seen. Texts carry markup (`<i>{word}`, `<italic>{…}`,
`<b>{…}`, `<continue>`, `<br/>`, `\r\n`), no `[Tag|shown]` links; the tracker and the checker
compare both sides in a shown form with it removed. Not known: whether panel text is logged.

Bark or panel: a line named by an `NPCTalk` option or a dialogue's steps (either column), and
every `NPCTalkDialogueTextAudio` line, is panel text; any other `NPCTextAudio` line is world
speech.

After a patch: `scripts/check-triggers.sh` dumps those tables from the install and runs
`scripts/check-triggers.mjs`, which reports lines that are gone, changed text, speakers that
no longer match and variants a trigger lacks (exit code 1 on the first three), and lines said
another way (the text in another row of the same speaker, the row also spoken by another NPC,
`NPCTalk` options that differ in NPC, flags or zone). See
[triggers](triggers.md).

## Map icons (datamined)

`data/icons/` holds the map icon sprites the reader compares the screen with (`bigmap.rs`,
[map reading](map-reading.md#icons-by-their-sprite)). From the game files, read-only:

```sh
scripts/update-icons.sh [game folder]   # needs poe_data_tools, ImageMagick (magick) and node
```

It dumps the `MinimapIcons` and `WorldAreas` tables and the sheet `art/2dart/minimap/player.dds`
(896x4928 px, RGBA), and `scripts/import-icons.mjs` cuts the icons. The sheet is a grid of 64x64
cells, 14 a row: row `i` of `MinimapIcons` is cell (`i % 14`, `i / 14`) (checked by eye: 18 is the
orange door, 9 the waypoint ring, 804/805 the orange and grey skulls). Check with
`node --test scripts/import-icons.test.mjs`.

- `sprites.rgba`: 32x32 RGBA per icon, 4096 bytes each, in `index.json` order (432 KiB for 108
  icons). Each 64x64 cell is reduced by a box filter: the sheet's first mip level, about what the
  game draws (the exit icon is 0.48 of its sprite on the big map at 2160 px height).
- `index.json`: the game build and tool; per icon `id` (the table's), `row`, `kind` (what the
  reader calls it: `exit`, `checkpoint`, `waypoint`, `portal`, `boss`, `boss-dead`, `rare`,
  `dead` (both grey skulls: one picture), `npc`, `quest`), the table's `minimapRadius` / `largemapRadius`, the opaque box at
  64 px (`box`: x, y, w, h); boss pairs also `pair`, `area`, `areaName`, `boss`, `how`, and the
  place name the game gives the live icon (`place`, the Active row's `Name`).
- **Which icons:** `Entrance`, `EntrancePortal` (exits), `Portal`, `Waypoint`, `Checkpoint`,
  `CheckpointNotActive`, `BossCheckpointActive`, `UniqueMonsterAlive`/`Dead` (orange and grey
  skull), `RareMonsterAlive`/`Dead` (yellow and grey skull), `NPC`, `QuestObject`, `QuestItem`, and
  47 campaign boss pairs (`<name>Active`/`<name>Inactive`, e.g. `MillerActive`,
  `GrimTangleBossActive`, `G4_7_BossActive`). League icons (Breach, Delirium, Incursion, …) and
  landmarks are left out.
- **Which area a boss pair belongs to** (`how`): `id` (the id holds the area id, `G4_7_Boss`),
  `name` (the area's or boss's name, `GrimTangleBoss`, `IgnagdukBoss`), `monster` (the area's boss
  in `WorldAreas.Bosses_MonsterVarietiesKeys` carries the id's word: `MudBurrower` →
  `MudBurrowerHeadBoss`; the script checks it), `place` (the place name is the checklist target's
  label: `Primal Arena`, `Shark Pit`, `Crabshell Cavern`), `assumed` (read by hand: `Crone` ↔
  `UrchinHagBoss` in Clearfell, `Count` ↔ `BaronBossHumanForm`). 45 of 47 pairs have an area;
  `BeetleBoss` ("The Galleria") and `GoldcrushBoss` ("The Aureaduct") have none.
- `boss` is the checklist's name for the boss (the text of a step's boss part; the script checks it
  is one of the area's) and is set only where the tie names that boss (24 pairs); never for
  `assumed` ties. A pair whose boss is no checklist boss (`CarnivorousPlant` in The Grelwood,
  `SnakePitBoss` in The Venom Crypts) has an area and no `boss`.

Nothing here says that a campaign boss shows its icon, when, or that it turns grey when killed:
that is the table's word, not seen on a real screen yet.

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
