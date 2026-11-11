# Plan

What has to be made, in order. Tick a step when its "Done when" holds.
The design lives in `docs/architecture.md`, the decisions and their reasons in `docs/decisions.md`.

## Phase 1: Set up

- [x] **1. Create the repository**
  Create the GitHub repo, add your friend as collaborator, and add a README, a licence and the `docs/` folder.
  Done when: both of you can push a branch and open a pull request.

  Status: repository, README, MIT license and `docs/` are present; the second
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
  List PoE 2-specific sources for each kind of data and their licences. The
  candidates below are unverified and must be checked for PoE 2 relevance:
  - route steps: Domistae's route, the wiki's act guides, the Exile Leveling fork
  - zone table: exports of the game's data
  - layout pictures: Exile-UI's images or your own screenshots

  Done when: each kind of data has a chosen source you are allowed to use.

  Status: `docs/data-sources.md` and `data/sources.json` select Domistae's MIT
  original route text, locally observed zone metadata and project-authored layout
  schematics. The route revision, licence and attribution are recorded; 97
  historical campaign zone IDs are collected. Route adaptation, current-patch
  validation and actual layout assets remain later work.

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
