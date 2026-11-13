# Ideas for later

## Windows overlay stutter with VSync

Deferred after the owner's v0.1.87 test (2026-11-12): a visible overlay causes
stutter; hiding it or setting the game's VSync Off resolves it. A simple,
confirmed application-side fix has not been established. Keep VSync Off as the
current workaround; no automatic changes to game or driver settings.

First later experiment: an isolated Windows `noRedirectionBitmap` A/B build,
with game frame-time measurements and click-through checks. The option already
exists in our dependencies, but is not a documented VSync fix. Stop if it does
not help; a renderer rewrite or driver investigation is a separate, larger task.
Evidence, sources and acceptance checks: [Windows VSync investigation](docs/overlay-test.md#windows-overlay-and-vsync-deferred-investigation).

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
