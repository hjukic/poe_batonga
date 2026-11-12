# Plan

What has to be made, in order. Tick a step when its "Done when" holds.
The design lives in `docs/architecture.md`, the decisions and their reasons in `docs/decisions.md`.

## Roadmap (owner, 2026-11-12)

The arrow is parked: "if we are unable to figure out layout first, the arrow is just not right
feature for now". Keep it simple: "figure out the layout without too much effort, features,
code and overcomplications".

The idea: the big map shows every exit the player has passed near, with its name. Each layout
in the game's graphs puts the zone's exits at fixed places. Two named exits on the map and
their distance and direction pick the layout.

- [ ] **Phase 1: the radar reads exits.** Exit icons and their names on the big map and the
  minimap, also near the screen's edges. Checked on the owner's real captures (a short list of
  which exits each capture shows, and a test).
  Built (v0.1.69–v0.1.75): names on the minimap, names cut by the screen's edge, exits between
  the orbs, one map per zone visit (a return goes on with it), no doubled walls, exits named
  minutes apart kept on the visit's map. Left: check it in play.
- [ ] **Phase 2: the layout.** From the named exits seen in a visit, the layout that fits, or
  "not known" when none fits clearly. Shown under the checklist: its name and a small drawing of
  its exits and paths, with the player on it. Open question to settle with the first real
  visits: whether the game turns or mirrors layouts.
  Built (v0.1.66–v0.1.73): the arrival exit and every named exit of the visit as pairs, "not
  known" when no layout fits, the drawing under the checklist. Left: the turn/mirror question.

**Next, in order:**

1. A play session through Act 1 with the big map open, walking past each zone's exits.
2. From its recordings: did each visit's layout come out right, and do the named exits fit a
   layout as drawn or only turned/mirrored? If turned, let the match try the turns.
3. Only if two layouts still tie: the walls' shape as a tie-breaker.
4. Small leftovers from the code review: `progress.json` written twice a second, unused fields
   in recordings, the Miller's place hard-coded in `graph.rs`.
5. Later: checklists for Acts 5–6 when 1.0 is out; the patch-day routine (refresh layouts,
   icons and triggers with `npm run layouts:update`, `icons:update`, `triggers:check`).

- Parked: the arrow (setting "Compass (parked)").

## Earlier plan (2026-11-11), in short

Release target (owner, 2026-11-12): **PoE 2 1.0, Acts 1–6**, no Interludes; Act 1 first.
The thirteen steps of the first plan, and where they stand:

- Done: the repository (1), the agreed scope (2, `docs/scope.md`), the first architecture
  (3, now `docs/architecture.md`), the data sources (6, `docs/data-sources.md`).
- Partly: the log (4: what it records is in `docs/log-findings.md` and `docs/triggers.md`;
  dialogue-panel text is not logged), the overlay window (5: Tauri, a layer surface on
  Hyprland; Windows mouse routing over the game not verified, `docs/overlay-test.md`), map
  reading (7: live minimap and big-map reading, `docs/map-reading.md`; GGG's view on visual
  analysis is not asked), the checklist (10: 97 zones, ticks from the log, by inference or a
  click), the map panel (11: the recognised layout under the checklist).
- Settled by building: the language (8: Tauri 2, Rust and TypeScript), the data files (9:
  `data/routes/`, `data/layouts/ggg/`).
- Parked: the arrow (12, the roadmap above).
- Open: the patch-day routine (13; `npm run layouts:update`, `icons:update` and
  `triggers:check` exist, the routine is not written down).
