# Overlay test (Phase 2, step 5)

How to run the overlay and check it over the game. It never sends input to the game.

## Run

Install [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/): on Windows,
Rust's MSVC toolchain, Visual Studio C++ Build Tools/Windows SDK and WebView2;
on Linux, Rust, the WebKitGTK/GTK development packages and `gtk-layer-shell`.

```sh
npm ci
npm run desktop
```

`npm run build` checks TypeScript and builds the page only. `npm run desktop:build`
produces an unbundled native executable; Windows output is
`src-tauri/target/release/poe-batonga.exe`. `npm run dev` shows the page in a browser with a
demo checklist and layout.

## Controls

Two windows: the panel (zone, checklist, recognised layout) and the compass around the
character (parked: shown only with "Compass (parked)" on). **Ctrl+Space** shows and hides
both (settings: another shortcut). The lock button: locked, clicks pass through to the game
except on the lock button and the checklist's rows; unlocked, the top bar drags the panel
(on a layer surface its place is kept for the next start) and the settings, restart and quit buttons work. On Wayland the panel is
a layer surface, so it stays above a fullscreen game.

## Manual checks

Over PoE 2 in windowed or borderless mode (exclusive fullscreen separately):

1. The panel's corners are transparent and it stays above the game while the game has focus.
2. Locked: clicks beside the rows reach the game; a click on a row ticks or unticks it; the
   overlay never takes the keyboard.
3. Unlocked: dragging the top bar moves the panel (Hyprland: after a restart it is where it
   was left).
4. The shortcut works while the game has focus. Hyprland: with "Hide when game is off
   screen" the overlay hides when the game's workspace is not shown and comes back with it.

## Results

| Environment | Result |
| --- | --- |
| Windows 10, the two-window prototype (2026-11-11) | Release build and launch pass; shortcuts and interaction state pass. Mouse pass-through, drag, focus and stacking over the game not verified. |
| Windows PoE 2 borderless/fullscreen | Not tested |
| Linux X11 | Not tested |
| Linux Hyprland/Wayland | The owner's daily setup since 2026-11-12 (layer surface over the game); see the [build log](build-log.md) |

## Big map reading

The reader switches between the corner minimap and the big map (Tab) by itself
([map reading, "Big map"](map-reading.md#big-map)). Check it on the live game, the
app running, in a zone with walls nearby:

1. Big map closed: walk for about 10 seconds. The minimap state reads "Reading
   minimap" and the layout overview grows.
2. Open the big map and walk on for about 20 seconds, along a corridor and round a
   corner. The state reads "Reading big map"; the overview keeps growing from where
   it was, with the same part number and no jump in the trail.
3. Close the big map and walk back. "Reading minimap" again, still the same part,
   and the trail runs back over itself.
4. Change zone, then render the recording (`node scripts/render-layouts.mjs`): one
   part for the walk, walls without vertical bars at the right edge.

If the trail jumps or a new part starts at a switch, note the zone, the direction
and whether the big map was zoomed.

Icons: with the big map open, `cargo test live_timing -- --ignored --nocapture` (in
`src-tauri`, the app may keep running) prints every icon each whole-window frame found,
in cells from the player; compare with the exits, checkpoints and waypoints on screen.

If the game lets the big map be zoomed (mouse wheel or a setting), zoom it in and out
once with the big map open and note the minimap state: "map lines … px wide" means
the zoom was caught; "Reading big map" at another zoom means it was not, and the
trail should be checked for a jump or a new part.

The big map's line colour also fits some floors seen through the map. Repeat step 2
in scenes unlike Clearfell and note any "too much of the view looks like map lines"
or a trail that wanders while standing still: a cold or icy zone, a dungeon with
blue-grey stone floors, ground lit by water or frost, a fight with frost or lightning
effects, and a town.
