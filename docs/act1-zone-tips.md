# Act 1 zone guides: sources and evidence

The panel shows, under the checklist, how to cross the zone you are in: a schematic of the
zone's usual shape drawn from where you come in (the triangle), the route to follow, the
places and clues on it, and two to four numbered steps (`data/guides/act1.json`, drawn by
`src/guide-svg.ts`, shown by `src/zone-guides.ts`; setting "Zone guide"). Each step names its
sources (hover it). The schematics follow the Mobalytics layout guide's idea: one general
pattern per zone, not one picture per layout; the game turns zones, the pattern stays. This
file keeps what they rest on. Started 2026-11-13, after the radar
and layout recognition were parked; Act 1 only for now.

## Sources

| Key | Source | Date / version |
|---|---|---|
| codex | Mobalytics, *Campaign Layout Guide: Act 1* (Lolcohol; Campaign Codex: Haddoq, Guitaraholic, CrimsonCasts, Angormus). Generalised patterns per zone with confidence levels; 30–40 instances per zone observed. | updated 2026-05-23; warns that 0.5 reworked some zones |
| poe2way | poe2way.com act guide, Act 1: per-chapter cards; environmental clues tagged 0.5.0. | ~2026-05 |
| maxroll | Maxroll, *Path of Exile 2 Campaign Guide*: zone order, waypoints, what is optional. | 2026-06-02 |
| aoeah | AOEAH, *PoE 2 0.5 Campaign Speedrun & Fast Leveling Route Guide*: general habits (wall hugging). | 2026-05-22 |
| data | The game's layout graphs from the owner's install (`data/layouts/ggg`): every layout a zone can have, with its exits and tagged places. | counted 2026-11-13 |

The community guides were written for Early Access 0.4/0.5; the data is from the game as
installed. Where they disagree, the tips follow the data.

## How the data was counted

For every layout of a zone: the node of the exit the player comes in by (campaign order),
the distance from it to every other exit and tagged place, as a share of the farthest
node's distance. "Far side" is beyond 0.6 of it. Distances are on the map as drawn (the
45° turn, up-down at 0.54), so they come out about the same however the zone is turned. Absolute
directions (north, east) are not used for dungeon zones: their layouts may lie turned round
(Ogham Farmlands, 2026-11-13).

`node scripts/zone-layout-stats.mjs data/layouts/ggg` prints it. Results:

| Zone | Layouts | What | Far side | Median distance |
|---|---|---|---|---|
| Clearfell | 33 | The Grelwood | 88% | 0.85 |
| | | Beira (boss nodes) | 67% | 0.66 |
| | | Campsite | 64% | 0.64 |
| | | Mud Burrow | 61% | 0.77 |
| Mud Burrow | 4 | The Devourer | 100% | 1.00 |
| The Grelwood | 12 | The Red Vale | 100% | 0.84 |
| | | The Grim Tangle | 8% | 0.48 |
| | | Lost Catacombs | 0% | 0.25 |
| | | Areagne's hut | 17% | 0.46 |
| | | Brambleghast | 50% | 0.62 |
| The Grim Tangle | 8 | Cemetery of the Eternals | 100% | 0.98 |
| | | The Rotten Druid | 75% | 1.00 |
| Cemetery of the Eternals | 12 | Lachlann's arena | 17% | 0.41 |
| | | Tomb of the Consort (places) | 36% | 0.48 |
| | | Mausoleum of the Praetor (places) | 28% | 0.49 |
| Hunting Grounds | 24 | Freythorn | 83% | 0.78 |
| | | Ogham Farmlands | 67% | 0.83 |
| | | The Crowbell | 58% | 0.60 |
| Ogham Farmlands | 32 | Ogham Village | 88% | 0.87 |
| | | Una's lute (quest) | 0% | 0.27 |
| Ogham Village | 12 | The Manor Ramparts (through the arena) | 75% | 0.70 |
| The Manor Ramparts | 6 | Ogham Manor (places) | 71% | 0.85 |
| Ogham Manor | 16 | Count Geonor | 100% | 1.00 |

## Community claims the data does not back

- **Clearfell: "the boss is always in the north"** (codex). On the map as drawn Beira lies
  east or north-east of the town exit in 31 of 48 boss nodes, and on the far side in two of
  three. The tip says "usually on the far side".
- **Grelwood: "the Red Vale is in the same quadrant as the Brambleghast 87% of the time"**
  (codex): not checked (the boss is on the far side in only half the layouts).
- **Mausoleum: Draven on the line through the wheel tile (75%)** (codex, CrimsonCasts): the
  graphs tag the wheel tile (`mausoleum_centre`, always within the nearer two thirds) but not
  Draven's arena; kept as the codex says it.

## Tried and dropped: every layout over each other

v0.1.106 drew every layout of the zone laid over each other from the entrance. The owner:
"that drawing does not help, i was thinking something like those mobalytics, where i get an
idea of the layout and know what to follow or do in each zone". Replaced by the schematics
in v0.1.107. What the overlay showed went into them: the Brambleghast next to the Red Vale
exit, Una's lute in front of the Farmlands entrance, Lachlann's arena behind you as you come
into the Cemetery.

## Next

- Acts 2–6 the same way.
