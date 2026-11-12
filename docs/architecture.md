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
