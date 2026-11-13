# Architecture

What the app is made of, as built (v0.1.74). The first proposal (2026-11-11) planned more
parts than were needed (SQLite, portal capture, a vision module); this page describes what
is there. Requirements: [scope](scope.md); decisions: [decision log](decisions.md).

The app is a Tauri 2 overlay: a Rust core in `src-tauri/`, two small TypeScript pages in
`src/`. It reads the game's log (`Client.txt`), screenshots of the player's own screen and
data files compiled in from `data/`. It never reads game memory and never sends input.

## Rust (`src-tauri/src/`)

| File | What it does |
| --- | --- |
| `main.rs` | The windows (the panel, the compass), settings, the show/hide shortcut, the lock, hiding while the game is off screen or during a boss fight, the page's commands |
| `layer_shell.rs` | Wayland (Hyprland): the overlay as a layer surface above a fullscreen game, clicks passing through outside the buttons and rows |
| `restart.rs` | The restart button: pull, rebuild, run the new build |
| `bundle.rs`, `build.rs` | The checklists (`data/routes/`) and layout graphs (`data/layouts/ggg/`) compiled in |
| `tracker.rs` | Tails `Client.txt`; zone entries and NPC lines tick steps (`trigger`), earlier steps are implied, a click ticks or unticks; per-visit lists; saves `progress.json` |
| `minimap.rs` | The capture loop (about 8 frames a second): the corner minimap, or the big map while it is open; `minimap_windows.rs` on Windows |
| `bigmap.rs` | Reads the big map: wall lines by colour, icons by the game's sprites (`data/icons/`); `labels.rs` reads exit names with the game's font (`data/fonts/`) |
| `layout.rs` | The recorder: stitches frames into one map per zone visit, keeps named exits, the arrival exit, boss kills (`dead_bosses`), and feeds the navigator |
| `layout/graph.rs` | The zone's layouts from the game's layout graphs |
| `layout/nav.rs` | The navigator: a belief over (layout, position), the picture of the recognised layout, the compass needle |
| `layout/guide.rs` | Targets, the needle the page gets, the turn between world and map |
| `layout/obs.rs` | The flight recorder (`.obs.jsonl`) and its replay (tests only) |
| `layout/nav_sim.rs` | Simulated walks that check the navigator (tests only) |

## Visits (`tracker.rs`, `Route::shown`)

A zone's list covers every pass through it; a visit ends with a step that enters another
zone, unless the step is optional or a detour that returns (`"returns": true`: a waypoint
grab, the Cemetery's Mausoleum and Tomb). The visit on show depends on what is done, not on
the order it was done in (2026-11-13):

1. The high-water mark is the visit after the last finished one (its leaving step done, or
   no longer listed). Skipped detours and older open visits do not hold the list back.
2. A later visit is due once the player has arrived for it: its leaving step's `after` is
   done (the town's visits wait for the zone the route comes from), or, without one, it is
   the visit at the mark. The last due visit from the mark on is shown.
3. Nothing due (a trip the route does not plan): the visit before the mark while it has
   open required rows, else nothing (`visit` is then one past the last visit). When the
   player was not seen leaving that visit (its leaving row only inferred: a start in the
   middle of an act, or "came back from X"), the visit at the mark instead.
4. Every visit finished: the last one.
5. The open required rows of an earlier visit the player never finished (its leaving row
   not done) are listed with the visit on show, its leaving row too unless that waits for
   an `after` not yet done: the Farmlands before Freythorn leaves Crowbell, the ritual and
   "Enter Freythorn" listed; a skipped town trip's Finn's charm comes with the next one.
6. A boss with a reward left alive in a visit that was finished comes with the visit on
   show too, until it is killed or ticked by hand: Crowbell's points back in the Hunting
   Grounds, Areagne's gem with the seals. The app knows whether a boss is dead; a pickup it
   cannot see does not come along (done but never clicked, it would show on every return).
7. A step with `"until": "<step>"` is not listed at all once that step is done (the Grim
   Tangle's waypoint grab, only while the roots still block the way).

What ticks a row (`apply`): a zone entry ticks rows of the list of the zone the player left
only, and of one list only the last row whose `after` holds (`docs/triggers.md`). Coming
into Z from X also takes Z's open row into X as done ("Came back from X", implied).
A later logged row implies the earlier rows it proves (talks), not what a player may skip
and still need (waypoints, rewards out in a zone, zone entries); a town's hand-in (Renly's
gem, the lute's points) is taken with the visit it belongs to when that visit is left.

Tests: `tracker.rs`, the Act 1 replays (S1–S8) and `no_log_event_ticks_two_steps_of_one_list`.

## Pages (`src/`)

- `main.ts`: the panel: the zone, the checklist, the recognised layout under it, settings,
  the optional map preview.
- `compass.ts`: the compass window around the character (parked: off unless switched on).

## Data flow

```
Client.txt ──> tracker ──> checklist (page)
                  │ targets, bosses, ticks
                  v
screen ──> minimap / bigmap ──> recorder ──> navigator ──> layout picture, needle (pages)
                                   │
                                   └──> layouts/<area>/<seed>.json, .nav.json, .obs.jsonl
```

## What it writes

In the app's data folder (on Linux `~/.local/share/com.poebatonga.overlaytest/`):
`settings.json`, `progress.json` and `step-observations.jsonl` (the tracker), `layouts/`
(recordings, navigator memory, flight recordings, `layout-history.jsonl`) and `samples/`
(a few captures as PPM, for checking the reader).
