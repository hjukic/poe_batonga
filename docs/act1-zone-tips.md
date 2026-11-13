# Act 1 zone guides: sources and evidence

The panel shows how to cross the zone you are in: under each open checklist row a short line
on how to get there (in the direction colour), and under the list a schematic of the zone's
usual shape drawn from where you come in (the triangle), with the route, places and clues,
plus a line or two for the whole zone (`data/guides/act1.json`: `hints` by step id and
`general`; `data/guides/act1-drawings.json`: the schematics, drawn by `src/guide-svg.ts`; shown by `src/zone-guides.ts`; setting
"Zone guide"). Each line's sources stay in the file (`src`), not in the panel. The schematics follow the Mobalytics layout
guide's idea: one general pattern per zone, not one picture per layout; the game turns
zones, the pattern stays. The Act 1 routes' own direction notes were folded into the hints
(owner, 2026-11-13: "if i have those 1.2.3 comments why do i need the blue ones under kill
beira"). This file keeps what it all rests on. Started 2026-11-13, after the radar and
layout recognition were parked; Act 1 only for now.

## How hints are written (owner, 2026-11-13)

For a league start, levelling fast: "very concise and straight to the point". A line is
read in a second, mid-run: a few words, the one thing that matters.

- One short line under the row it is about: under "Kill Beira" "Away from town, far
  side.", under "Enter The Grelwood" "Far side, past the bridge." At most 46 characters: what fits on one line of the panel.
- No percentages and no "N of M" counts (owner, 2026-11-13: "we do not need % on the
  text ... its a noise"): how often a pattern holds is kept in the table below, not in
  the hint. Digits only on level lines ("Level 7 first.") and town errands.
- No rewards for what you do anyway ("Kill Beira", not "+10% cold res"). What a place
  gives may be said where it is the point of the line (Areagne's cauldron, the Red Vale's
  weapon racks).
- Side content is an optional row of the checklist; its reason stays on the row (the
  reward on the right), the hint says only where it is.
- No lists, no numbered tips, no tooltips on rows or hints.
- Build-neutral: where to stand, what to dodge, what the place gives; no skill or gem
  advice (the fastest guide plays a grenade Mercenary; its gem moves stay out).
- Fewer, better lines: a boss gets a line only where one move saves a death; most get
  none.

`scripts/guides.test.mjs` checks the mechanical part of these rules, that every key and
condition names a step of the zone's route file, the level targets, and the town errands.

## Kinds and conditions

A hint is `{text, src}` or a list of them under a step id; `general` lines are for the whole
zone, and in town they are the errands. Optional fields:

| Field | Meaning |
|---|---|
| `kind` | `way` (default: how to get there), `boss` (how to fight it; read before the fight, the panel hides during it), `level` (a level target; reads "Level N first."), `tip` (a time saver, what the place gives, a town errand) |
| `below` | only while the character's level is under it (unknown level: shown); every level line has one |
| `from` | only once the character's level is at least this (unknown level: hidden) |
| `after` | only once that step of this zone is done |
| `until` | only until that step of this zone is done |

A level line comes first under its row. Town errands are pinned to a visit by the leaving
steps (zone entries the log ticks), so a visit never shows more than two, at any level:

| Visit (contract 3) | Errands | Gate |
|---|---|---|
| 1, first arrival | boots with movement speed; damage gloves | until the Grim Tangle waypoint |
| 2, from the Red Vale | the same, and a better weapon if sold | after Clearfell, until the Grim Tangle waypoint |
| 3, from the seals | the same two | as above |
| 4, from Freythorn | "Level 10:" the next weapon base; greater flasks from Una if needed | while under level 12 (the line names the level, so it shows under 10 too), until the Hunting Grounds waypoint |
| 4–5 | a cold-resistance ring for Geonor | from level 12, after the Grim Tangle waypoint, until the Ogham Village waypoint (it shows on the trip from Ogham Village also when the one after Freythorn was skipped) |
| 6, after Geonor | none (grand flasks and the next boots come at 16, after Act 1's vendors) | |

Level targets (gtd): 5 leaving the Grim Tangle, 7 before the Mausoleum (Draven), 8 before
Freythorn, 13 before Ogham Manor, 14 before Geonor; 10 is the town's weapon base errand.

## Sources

| Key | Source | Date / version |
|---|---|---|
| gtd | GuyThatDies, *I Spent One Month Making This Campaign Guide* (YouTube `UdIsZHbhXOY`): the fastest route, zone by zone; level targets, the Miller's corner, the Red Vale's weapon racks, the ring left, per-trip errands. Read through the council's benchmark summary. | 2026-04-23, 0.5 |
| maxroll-ls | ZiggyD, *The Comprehensive League Start Leveling Guide* (Maxroll): gear and flask checkpoints by level (medium flasks from Areagne's cauldron, greater at 10 from Una, a cold-resistance ring for the Count). | 0.5.5, updated 2026-09-08 |
| maxroll-secrets | ZiggyD, *Act 1 Campaign Secrets Guide* (Maxroll): where each secret is (sarcophagus, workshop chest, gallows, Candlemass's psalm, Forgotten Riches). | 0.3.0, updated 2025-08-27 |
| domistae | Domistae `poe2-leveling`, `poe2_act1_guide.html` at `f682e2b`: the route files' source; its boss notes (Crowbell's slam, the King in the Mists' curses). | 0.5.5 |
| poevault | poe-vault.com, *Act 1 Boss Guide*: each boss's attacks and phases (Asinia's bone cage, Geonor's frost sigils). | no date or version; read 2026-11-13 |
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

## Not confirmed for 0.5.5 (check in 1.0)

- Boss lines from poevault (Asinia's bone cage, Geonor's frost sigils, the King in the Mists'
  two curses): the page gives no version; the moves are old ones and may have changed.
- Where-lines from maxroll-secrets (0.3.0): the workshop chest, the gallows by a checkpoint,
  Candlemass's psalm, Forgotten Riches' pillar switch.
- The Miller's far corner and the Red Vale's weapon racks: gtd (0.5), read only through the
  benchmark's summary, not the video.
- Left without a way line: the Grim Tangle waypoint grab, the Tomb's Haunted Treasure and the
  Farmlands' crop circle (no source says where they are).

## Next

- Acts 2–6 the same way, after 1.0 (11 December).
