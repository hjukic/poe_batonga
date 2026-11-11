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
| Zone terrain (walls, POIs) | [poe2way](https://www.poe2way.com/atlas) collected terrain, first three zones | `data/layouts/poe2way/` |
| Layout diagrams | [Vyary/qutm](https://github.com/Vyary/qutm) `layouts.json` | `data/layouts/qutm/` |

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
45° (`.game.svg`) to the in-game map orientation. The turn is checked against
Qutm's Clearfell diagram (encampment exit at the bottom, waypoint and Mud Burrow
top-left, The Grelwood right), not yet against live gameplay.

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

## Qutm layout diagrams

[Qutm](https://github.com/Vyary/qutm) diagrams cover 94 zones with 106 variants and
give positions for the start, exits, waypoints, checkpoints and bosses; see the
[README](../data/layouts/qutm/README.md). Coordinates are authored 0-1 positions,
not to scale.

## Other candidates

- [Exile-UI](https://github.com/Lailloken/Exile-UI) Act-Decoder layouts: its
  [changelog](https://github.com/Lailloken/Exile-UI/blob/main/data/changelog.json)
  warns they are outdated for PoE 2.
- Mobalytics campaign-layout guides (Act 1-3) and the PoE 2 wiki have layout
  pictures; not imported yet.
- Direct game-data extraction is not needed for the zone catalog.

Offline tests may use synthetic maps, clearly labelled synthetic; they cannot
establish performance on real gameplay.
