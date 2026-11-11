# Plan

What has to be made, in order. Tick a step when its "Done when" holds.
The design lives in `docs/architecture.md`, the decisions and their reasons in `docs/decisions.md`.

## Phase 1: Set up

- [ ] **1. Create the repository**
  Create the GitHub repo, add your friend as collaborator, and add a README, a licence and the `docs/` folder.
  Done when: both of you can push a branch and open a pull request.

  Status: repository, README, MIT license and `docs/` are present. Collaborator
  invitation and both contributors' branch/PR checks are still pending; see
  `docs/README.md`.

- [ ] **2. Agree the scope**
  Write down the PoE 2 campaign overlay scope: Windows and Linux, automatic
  objective tracking, mandatory and selected optional quest items, a map panel
  and automatic layout-based directional guidance. GGG compliance is mandatory;
  automatic recognition and the arrow depend on compliance and feasibility.
  Done when: you both agree on one page of text.

  Status: owner requirements recorded in `docs/scope.md`; agreement by the
  second contributor is pending. No automatic recognition feature is approved yet.

- [ ] **3. Draft the architecture**
  Write `docs/architecture.md` with the parts of the app, how data flows between them, and the open questions. Start `docs/decisions.md` with the decisions from step 2.
  Done when: the open questions in it match steps 4 to 7.

## Phase 2: Find out

- [ ] **4. Test the log**
  Play through a few PoE 2 zones and establish what `Client.txt` actually records
  on zone entry and objective completion. Check zone, level and seed rather than
  assuming they exist; record whether character and quest/item evidence exists.
  Done when: the architecture file lists exactly what the log provides.

- [ ] **5. Test the overlay window**
  Build a tiny window that stays on top of the game, once in Tauri and, if that fails, in Electron. Try it on Hyprland and on your friend's system.
  Done when: you know which toolkit shows a window over the game on both systems.

- [ ] **6. Collect the data sources**
  List PoE 2-specific sources for each kind of data and their licences. The
  candidates below are unverified and must be checked for PoE 2 relevance:
  - route steps: Domistae's route, the wiki's act guides, the Exile Leveling fork
  - zone table: exports of the game's data
  - layout pictures: Exile-UI's images or your own screenshots

  Done when: each kind of data has a chosen source you are allowed to use.

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
