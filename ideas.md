# Ideas for later

## Datamine zone layouts from the game files

**Done for every campaign zone (2026-11-12):** 101 areas, 584 layout graphs (`.tgr` and
`.dgr`), refreshed by `scripts/update-layouts.sh` after each patch; see
[data sources](docs/data-sources.md#game-layout-graphs-datamined). Still open:

- sub-graphs are expanded (2026-11-12); still guessed: the turn of `(any)`/`(best)`
  placements and the scale of sub-graphs on another grid (Freythorn, Halani Gates);
  stairs and teleports between the parts (matching `uniqueA0` keys) are not edges yet;
- check the `.dgr` scale (one grid unit ≈ `roomUnitSize − 1` world units, derived from
  the room files) on a recorded dungeon, e.g. Mud Burrow;
- the navigator runs in every area with graphs, `.dgr` ones too, but trusts gold only in
  G1_1 and G1_2 (`MEASURED` in `nav.rs`), the two checked against a recorded visit;
- find objectives through node `keys` (`boss`, `unique1`, `teleport…`, `zone=N`).
