# Ideas for later

## Datamine zone layouts from the game files

**Done for every campaign zone (2026-11-12):** 101 areas, 581 layout graphs (`.tgr` and
`.dgr`), refreshed by `scripts/update-layouts.sh` after each patch; see
[data sources](docs/data-sources.md#game-layout-graphs-datamined). Still open:

- expand sub-graphs (`graph` nodes): eight areas' layouts are only placeholders for them
  (Freythorn, Ogham Manor, Dreadnought, …), so they have no exits yet;
- check the `.dgr` scale (one grid unit ≈ `roomUnitSize − 1` world units, derived from
  the room files) on a recorded dungeon, e.g. Mud Burrow;
- use the `.dgr` areas in the app (`AREAS` in `graph.rs` lists only G1_1 and G1_2);
- find objectives through node `keys` (`boss`, `unique1`, `teleport…`, `zone=N`).

Notes below are the original plan.

Campaign zones are not stored as finished maps: the game builds each instance
from parts, picked with a random seed. The parts are in the game's bundles
(local install: `~/.local/share/Steam/steamapps/common/Path of Exile 2`):

- `.arm` rooms, `.tgt` tile groups, `.rs` room sets, `.gt` ground types (all
  UCS-2 plain text, per poe_data_tools' format notes)
- `.datc64` data tables that link area IDs (`G1_2`, …) to those parts

Datamining gives the building blocks and probably each zone's layout templates
(variants), not the exact walls of the instance being played. poe2way labels
some terrain "possible layout variants from the game files", so they seem to do
this already.

What it could give us:

- each zone's layout variants, to recognise which one is being played;
- real walkability per tile, instead of one recorded instance;
- fresh data after every patch by extracting again.

Tools:

- [poe_data_tools](https://github.com/adamthedash/poe_data_tools): CLI with PoE 2
  support (`--patch 2`); `list`, `extract`, `dump-tables` (datc64 with community
  schemas), `translate`.
- [LibGGPK3](https://poedb.tw/tw/GGPK) (Windows GUI) and the dat schema via poedb.
- [ggpk](https://docs.rs/crate/ggpk/latest) and [poe_bundle](https://lib.rs/crates/poe_bundle)
  Rust crates, if we ever read the files from the app itself.

First step (about an hour, offline, read-only): extract Clearfell's (`G1_2`)
area files with poe_data_tools and check whether its variant templates and
walkability are readable.

Patch release notes do not contain map definitions.
