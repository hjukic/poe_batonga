# Overlay window experiment (Phase 2, step 5)

This prototype tests native window behavior only. It does not read logs, capture
the screen, scan inventory or send input to the game. The sample checkbox is an
input target, not character progress.

## Run

Install [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/): on Windows,
Rust's MSVC toolchain, Visual Studio C++ Build Tools/Windows SDK and WebView2;
on Linux, Rust and the distro-specific WebKitGTK/GTK development packages.
Node 20.18+ works with this prototype's Vite 6 toolchain.

```sh
npm ci
npm run desktop
```

`npm run build` checks TypeScript and builds the UI only. It does not validate
native window behavior. `npm run desktop:build` produces an unbundled native
executable; Windows output is `src-tauri/target/release/poe-batonga.exe`.
`npm run dev` offers a labelled browser preview with native controls disabled.

## Controls

Two windows are created: a regular controls window (initially minimized to the
taskbar) and a transparent, undecorated overlay, initially visible in click-through
mode. The overlay requests topmost
stacking; the compositor still determines actual behavior.

- **Ctrl+Alt+I:** toggle interaction. Click the sample checkbox or drag the panel
  header when enabled. Lock again and click the window underneath to resume play.
  Interaction also reveals an opacity slider (default 25%, saved locally).
- **Ctrl+Alt+O:** show/hide. Showing or hiding resets interaction to click-through.
- **Recenter & lock:** recover an off-screen panel (where compositor positioning
  is supported).
- **Quit**, or close the controls window: exit the whole application.

The controls window stays accessible through the taskbar/Alt+Tab. Shortcut
registration failures appear there and do not prevent button use. Registration
success does not prove the key works with the game focused: verify that manually.
On Hyprland/Wayland, record shortcut, placement or stacking failures rather than
assuming XWayland or Electron resolves them. Never run both display-server tests
at once with the same shortcuts.

The visual reference is [Qutm's published overlay screenshot](https://qutm.vercel.app/):
a quiet translucent zone header, plain route rows, cyan italic directions and
an orange boss name. The Riverbank route shown here is explicitly marked Preview;
it does not represent the active zone or automatically tracked progress.

## Manual verification

Start on the desktop with another clickable application underneath. Then repeat
over PoE 2, starting with windowed/borderless mode. Test exclusive fullscreen
separately and record it as unsupported if necessary.

1. Verify panel corners/outside background are transparent and the panel stays
   above the target window when that window is focused.
2. In click-through mode, clicks on the panel reach the underlying application;
   the checkbox does not change and the overlay does not take keyboard focus.
3. With the target focused, toggle interaction once. Verify one key press toggles
   once, checkbox clicks register, and dragging the header moves the panel.
4. Lock again; verify input passes through. Hide/show and repeat the check.
5. Recover using controls with the overlay hidden, moved away or shortcuts
   unavailable. Close controls and confirm both windows exit.
6. Repeat with display scaling, another monitor and after moving the game.
   This prototype does not automatically attach to or follow the game window;
   move it manually in interaction mode.

## Results

Record toolkit version, OS/compositor, display protocol, resolution/scaling,
game display mode, transparency, topmost, mouse pass-through, keyboard focus,
shortcuts, recovery and any errors. Mark untested rows explicitly.

| Environment | Result |
| --- | --- |
| Windows UI compile | TypeScript and Vite production build pass |
| Windows native build | Pass: release executable built with Rust 1.99.1 and Tauri 2.12.5 on Windows 10 build 19045 |
| Windows startup and shortcut state | Pass: both windows open; Ctrl+Alt+I switches interaction; Ctrl+Alt+O hides/shows and resets to click-through; both window UIs reflect state changes |
| Qutm-style restyle | Release build and launch pass; UI inspection confirms zone/route-only play mode and opacity/test controls appearing only during interaction. Screenshot capture still times out, so pixel-level visual comparison is unverified. |
| Windows mouse pass-through, drag, focus and visual stacking | Not verified: window screenshot capture timed out and coordinate input geometry was unavailable in the UI test tool |
| Windows PoE 2 borderless/fullscreen | Not tested |
| Linux X11 | Not tested |
| Linux Hyprland/Wayland | Not tested |

Step 5 stays open until the prototype has been tested over the actual game on
both contributors' systems. Try Electron only after documenting a Tauri failure;
toolkit changes should be driven by the observed limitation.

On this machine, Rust stable, npm dependencies and Visual Studio C++ Build
Tools/SDK are installed; WebView2 is available. Open a new terminal after installing
Rust if `cargo` is not on PATH in an existing terminal. The release executable
was launched successfully and left visible in click-through mode. The test tool
confirmed the checkbox becomes enabled/disabled with interaction state, but this
does not establish actual mouse routing or visual transparency. No game/compositor
stacking conclusion is drawn from the limited UI check. No Electron fallback has
been attempted.

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

If the game lets the big map be zoomed (mouse wheel or a setting), zoom it in and out
once with the big map open and note the minimap state: "map lines … px wide" means
the zoom was caught; "Reading big map" at another zoom means it was not, and the
trail should be checked for a jump or a new part.

The big map's line colour also fits some floors seen through the map. Repeat step 2
in scenes unlike Clearfell and note any "too much of the view looks like map lines"
or a trail that wanders while standing still: a cold or icy zone, a dungeon with
blue-grey stone floors, ground lit by water or frost, a fight with frost or lightning
effects, and a town.
