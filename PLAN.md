# Plan

What has to be made, in order. Tick a step when its "Done when" holds.
The design lives in `docs/architecture.md`, the decisions and their reasons in `docs/decisions.md`.

## Roadmap (owner, 2026-11-12)

The owner's call after the first live session: "if we are unable to figure out layout first,
the arrow is just not right feature for now". The arrow is parked. Two phases come first:
the radar (reading the map), then layout recognition, shown under the checklist. The
checklist itself keeps improving beside them.

**Where it stands (measured 2026-11-12).** Map icons on the owner's 24 captures: every
real exit, checkpoint and waypoint found, nothing false (since v0.1.59). Exit names read on
the big map and the minimap when the label is fully on screen. One real boss icon (the
Miller's). Tracking: an app restart or a lost frame run starts a new part with its own
coordinates; drift is not measured. Scale: 0.78 cells per world unit assumed; the
Riverbank's walk was 1.14–1.18 times the graphs', the Clearfell Encampment's outline fit
at 0.80. Layout fit: the one instance with a known layout fit its graph within 132 units;
the owner's Clearfell of 2026-11-12 fits no layout as drawn, but several within 50–70
cells once the graphs may be turned or mirrored: whether the game turns layouts is open.

### Phase 1: the radar

The radar turns screenshots of the minimap and the big map into one map of the zone
visit: the player's track, the walls, and every map icon with its kind, name and place,
in one frame, with a known scale. Everything in phase 2 rests on it, so each item has a
measured "done when".

- [ ] **R1. Truth set and scoreboard.** A capture mode that keeps, while the owner plays,
  frames the radar read and what it read; a format to mark what is really there; a test
  that scores the radar on the set (per icon kind: found and false; names right; place
  error). Done when: at least 200 marked frames from 8 or more Act 1 zones, both views,
  and the score runs as a test.
- [ ] **R2. One map per visit.** The player's place carries across big map and minimap
  switches, tracking losses and app restarts (a restart continues the saved map).
  Done when: a static icon seen again lands within 10 cells of where it was first seen,
  on recorded visits, and a restart mid-zone keeps the map.
- [ ] **R3. Every icon, named.** All zone transitions (exit icons and place icons such as
  "Besieged Encampment"), waypoint, checkpoints, boss places, NPCs, quest objects; the
  label of every labelled icon, also when partly off screen; both views.
  Done when: on the truth set exits and their names are found 98% of the time and never
  invented (99% of named icons right), other kinds 95% / 98%.
- [ ] **R4. Clean walls.** Scene pixels (water, foam, fire, the character) stay out; real
  map lines stay in. Done when: false wall cells under 2% and real ones kept 95% on the
  truth set.
- [ ] **R5. Exits where they are.** Each exit recorded at its icon, not where the player
  appeared; the arrival known for exits, waypoints, portals, deaths and new characters.
  Done when: every exit of a recorded visit has one place, and arrivals are right on the
  truth set.
- [ ] **R6. Scale and turn measured.** Cells per world unit and how the map is turned,
  measured on visits whose exits were all seen, cross-checked with the walked distance.
  Done when: the scale is known within 3% and the method is in `docs/map-reading.md`.
- [ ] **R7. Fast and robust.** About 8 frames a second at no more than 15% of a core;
  correct with panels over the map, the inventory open, loading screens, and at other
  resolutions (1920x1080, 2560x1440). Done when: measured on the truth set and on the
  co-owner's Windows machine.
- [ ] **R8. The radar under the checklist.** The panel shows what the radar knows: walls,
  icons with their names, the player. The owner's check while playing, and the base the
  layout view of phase 2 draws on.

### Phase 2: layout recognition

- [ ] **L1. Truth.** Every visit whose exits were all seen and named gets its layout (and
  any turn) as truth; a table of such visits per zone.
- [ ] **L2. Matching.** Exits (named and not), checkpoints, the waypoint, boss places and
  the shape of the walls against each layout's graph, with turns and mirrors if R6/L1
  show the game uses them, and how far exits sit from their graph nodes (learnt from L1).
- [ ] **L3. Honest answer.** A layout with how sure it is, or "not known"; never sure and
  wrong.
- [ ] **L4. The layout under the checklist.** The recognised layout drawn (corridors, exits
  by name, boss and other places), the player on it, the checklist's next places marked;
  the candidates while it is not known.
- [ ] **L5. Score.** Done when, on real visits to Act 1's outdoor zones: the right layout
  in 80% of visits by the time two exits are named, and sure-and-wrong in under 1%.

### Phase 3 (parked): the arrow

The compass needle, on top of a recognised layout. Off by default (setting "Compass
(parked)"); the navigator keeps running and writing its flight recorder.

## Earlier plan (2026-11-11)

Release target (owner clarification, 2026-11-12): **PoE 2 1.0, Acts 1–6**.
Do not implement Interlude content. Act 1 remains the first validation milestone;
then extend the checklist, rule catalog and tested navigation through Act 6.
Revalidate existing Act 1–4 rules for 1.0. Act 5–6 objectives and layout rules
remain pending verified 1.0 sources/gameplay, not inferred from Interlude data.

## Phase 1: Set up

- [x] **1. Create the repository**
  Create the GitHub repo, add your friend as collaborator, and add a README and the `docs/` folder.
  Done when: both of you can push a branch and open a pull request.

  Status: repository, README and `docs/` are present; the second
  contributor has collaborator access (confirmed by the owner 2026-11-11).

- [x] **2. Agree the scope**
  Write down the PoE 2 campaign overlay scope: Windows and Linux, automatic
  objective tracking, mandatory and selected optional quest items, a map panel
  and automatic layout-based directional guidance. GGG compliance is mandatory;
  automatic recognition and the arrow depend on compliance and feasibility.
  Done when: you both agree on one page of text.

  Status: `docs/scope.md` agreed by both contributors (2026-11-11). Automatic
  recognition and the arrow remain conditional on step 7.

- [x] **3. Draft the architecture**
  Write `docs/architecture.md` with the parts of the app, how data flows between them, and the open questions. Start `docs/decisions.md` with the decisions from step 2.
  Done when: the open questions in it match steps 4 to 7.

  Status: `docs/architecture.md` describes the proposed components, evidence/data
  flow and experiments mapped to steps 4-7. `docs/decisions.md` records scope
  requirements and separates technical proposals from validated decisions.

## Phase 2: Find out

- [ ] **4. Test the log**
  Play through a few PoE 2 zones and establish what `Client.txt` actually records
  on zone entry and objective completion. Check zone, level and seed rather than
  assuming they exist; record whether character and quest/item evidence exists.
  Done when: the architecture file lists exactly what the log provides.

  Research: `docs/qutm-review.md` identifies candidate zone, level-up and
  dialogue/reward patterns from Qutm source. `docs/log-findings.md` records a
  full-file search of local historical logs: zone/seed/hash and reward evidence
  exists; generic quest-item pickup, player position and map geometry were not
  established. Labelled gameplay checks and current-patch validation remain.

- [ ] **5. Test the overlay window**
  Build a tiny window that stays on top of the game, once in Tauri and, if that fails, in Electron. Try it on Hyprland and on your friend's system.
  Done when: you know which toolkit shows a window over the game on both systems.

  Status: initial Tauri prototype includes a transparent topmost overlay,
  click-through/interaction toggling, global shortcuts and a recovery controls
  window. See `docs/overlay-test.md` for setup, verification and outstanding
  Windows/Hyprland game tests. Windows release build, startup and shortcut-state
  checks pass; actual mouse routing and game stacking remain unverified.
  This step is not yet complete.

- [x] **6. Collect the data sources**
  List PoE 2-specific sources for each kind of data. The
  candidates below are unverified and must be checked for PoE 2 relevance:
  - route steps: Domistae's route, the wiki's act guides, the Exile Leveling fork
  - zone table: exports of the game's data
  - layout pictures: Exile-UI's images or your own screenshots

  Done when: each kind of data has a chosen source.

  Status: `docs/data-sources.md` and `data/sources.json` select Domistae's route
  text, locally observed zone metadata and poe2way's
  collected terrain (walkable outlines and points of interest for 51 Act 1-3
  areas). 97 historical campaign zone IDs are collected. Route adaptation, current-patch
  validation and actual layout assets remain later work.
  Source audits: `docs/layout-shape-research.md` and
  `docs/navigation-rule-research.md`. Earlier Interlude research is historical
  and does not count toward the six-act 1.0 release.

- [ ] **7. Test map reading**
  Establish a permitted basis for automatic local screen analysis, layout-based
  guidance and progress recognition; seek written GGG clarification where unclear.
  Test visible map geometry, player-position detection, objective evidence and
  layout matching on offline samples first. Validate automatic capture on Windows
  and Linux before any permitted live-game test. No manual screenshot delivery
  or position entry is required in the target workflow.
  Done when: compliance evidence and technical results are documented, including
  unknown states and ambiguous matches. If automation or the arrow cannot meet
  these constraints, agree a reduced checklist/map-panel scope with both contributors.

  Status: `docs/map-reading.md` records an offline PNG baseline, 15 passing
  checks and diagnostics on two existing local screenshots. Synthetic cases work;
  real minimap extraction/player detection do not yet meet requirements. A Windows
  windowed/borderless GDI capture adapter is implemented alongside Hyprland capture;
  Map reading starts automatically as requested by the owner; Windows gameplay
  verification is still pending. Policy permits informed log reading; visual
  analysis/navigation clarification and Windows/Linux capture tests remain open.

- [ ] **8. Choose the language**
  Decide from the result of step 5. Recommendation: Tauri 2 (Rust core, web UI), with Electron as the fallback.
  Done when: the choice and its reason are in `docs/decisions.md`.

## Phase 3: Build

- [ ] **9. Define the data files**
  Design the file formats for the route (acts, zones, steps), the zone table and the layout variants, as plain files in the repo.
  Done when: Act 1 is written in the format and reads well by eye.

- [ ] **10. Milestone 1: the checklist**
  The app uses verified permitted inputs, shows current-zone steps and remembers
  progress per character. It automatically checks objectives only on reliable
  completion evidence; zone changes alone never imply item or quest completion.
  Done when: both of you can install it and play Act 1 with it.

- [ ] **11. Milestone 2: the map panel**
  The app shows layout variants for the current zone. Automatic matching is the
  target; manual selection is a recovery/fallback option under the agreed scope.
  Done when: it works for every Act 1 zone.

- [ ] **12. Milestone 3: the arrow**
  Subject to step 7, the app automatically captures the visible map locally,
  estimates player position and layout match, and points towards mandatory quest
  items/objectives, selected optional objectives and exits. Do not assume the map
  is always player-centred. Suppress the arrow on uncertain or stale matches.
  Done when: guidance works reliably in tested Act 1 zones on Windows and Linux
  without manual position input or screenshot uploads, within the documented
  GGG constraints; record measured coverage and accuracy.

## Phase 4: Keep it working

- [ ] **13. Patch-day routine**
  Write down the steps for a new league: compare the game files with the previous patch, mark changed zones as unknown, update the data files.
  Done when: the routine is in the repo and has been tried on one patch.
