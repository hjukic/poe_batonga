# Plan

What has to be made, in order. Tick a step when its "Done when" holds.
The design lives in `docs/architecture.md`, the decisions and their reasons in `docs/decisions.md`.

## Phase 1: Set up

- [ ] **1. Create the repository**
  Create the GitHub repo, add your friend as collaborator, and add a README, a licence and the `docs/` folder.
  Done when: both of you can push a branch and open a pull request.

- [ ] **2. Agree the scope**
  Write down what the app does and doesn't do: campaign only, a checklist, a map panel, the arrow as a stretch goal, no memory reading, and which systems it must run on.
  Done when: you both agree on one page of text.

- [ ] **3. Draft the architecture**
  Write `docs/architecture.md` with the parts of the app, how data flows between them, and the open questions. Start `docs/decisions.md` with the decisions from step 2.
  Done when: the open questions in it match steps 4 to 7.

## Phase 2: Find out

- [ ] **4. Test the log**
  Play through a few zones and look at what `Client.txt` records on each zone entry (zone, level, seed).
  Done when: the architecture file lists exactly what the log provides.

- [ ] **5. Test the overlay window**
  Build a tiny window that stays on top of the game, once in Tauri and, if that fails, in Electron. Try it on Hyprland and on your friend's system.
  Done when: you know which toolkit shows a window over the game on both systems.

- [ ] **6. Collect the data sources**
  List where each kind of data can come from and under which licence:
  - route steps: Domistae's route, the wiki's act guides, the Exile Leveling fork
  - zone table: exports of the game's data
  - layout pictures: Exile-UI's images or your own screenshots

  Done when: each kind of data has a chosen source you are allowed to use.

- [ ] **7. Test map reading**
  Take screenshots with the map overlay open and try to extract the map lines from them.
  Done when: you know whether the lines come out cleanly enough to match against a layout. If not, the arrow is dropped and the map panel stays manual.

- [ ] **8. Choose the language**
  Decide from the result of step 5. Recommendation: Tauri 2 (Rust core, web UI), with Electron as the fallback.
  Done when: the choice and its reason are in `docs/decisions.md`.

## Phase 3: Build

- [ ] **9. Define the data files**
  Design the file formats for the route (acts, zones, steps), the zone table and the layout variants, as plain files in the repo.
  Done when: Act 1 is written in the format and reads well by eye.

- [ ] **10. Milestone 1: the checklist**
  The app watches the log, shows the steps for the current zone, advances when you change zone, and remembers progress per character.
  Done when: both of you can install it and play Act 1 with it.

- [ ] **11. Milestone 2: the map panel**
  The app shows the layout variants for the current zone, and a hotkey picks the one you recognise.
  Done when: it works for every Act 1 zone.

- [ ] **12. Milestone 3: the arrow**
  The app captures the map, matches it to a variant and points to the exit. The map is centred on you, so the direction follows from the match. It starts as a straight line to the exit.
  Done when: the arrow points the right way in most Act 1 zones.

## Phase 4: Keep it working

- [ ] **13. Patch-day routine**
  Write down the steps for a new league: compare the game files with the previous patch, mark changed zones as unknown, update the data files.
  Done when: the routine is in the repo and has been tried on one patch.
