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
| Windows PoE 2 DX12, borderless, RX 7900 XTX (owner, 2026-11-12; v0.1.87) | Visible overlay stutters with VSync; hiding it removes the stutter. A 120 FPS cap did not resolve it. VSync Off resolves it with the overlay visible. See the investigation below. Exclusive fullscreen not tested. |
| Linux X11 | Not tested |
| Linux Hyprland/Wayland | The owner's daily setup since 2026-11-12 (layer surface over the game); see the [build log](build-log.md) |

## Character profiles and idle panel (v0.1.88)

Working text: `Leveling batonge` outside a live zone, `quests completed` after
every selected objective in the zone's visits is done. Final wording and the
VSync issue are separate items in the [scope TODO](scope.md#scope-todo-choose-later).

The tracker confirms game presence from OS process metadata, alongside zone and
login/reconnect events from Client.txt. Switching desktop focus alone does not
erase the zone. The log does not reliably give the local character's name on
login; choose a profile under Settings → Character after entering the zone.
New logged logins/reconnects clear that selection. Character names are stored
locally; no account login or API credentials are needed.

Pre-profile ticks are preserved separately. To associate them with a character,
enter a new profile name and check “Use previous saved progress”. Without that
choice, a new character starts without those old ticks. Reset progress clears
the selected profile only.

Automated checks cover separate profiles across switches/restart, new-login
isolation, explicit legacy import, manual corrections, a shared log cursor,
idle/unknown-zone rendering data, optional objectives and zone-wide completion.
Run `node --test scripts/checklist.test.mjs`, and the Rust `characters::tests`
and `tracker::tests` filters.

Local validation (2026-11-13): frontend checks/build and the Windows release
build pass; 4 checklist, 4 character and 30 tracker tests pass. The upgraded
process starts and responds; its saved data contains the preserved legacy
progress. Visual inspection was attempted, but the computer-use helper's
window capture timed out, so no screenshot-based or live-game acceptance is
claimed for this change.

Gameplay acceptance still needs:

1. Start the overlay with the game closed: working app title, no stale checklist
   or map. Start the game and remain at login: the same idle panel.
2. Enter a zone, choose character A, tick a task: it is struck through. Leave
   and return: A's tick remains. Finish the selected objectives: completion text
   replaces the list. Turning optional content on restores unfinished optional tasks.
3. Choose B: A's ticks do not appear. Return to A: its ticks return. Reset B:
   A's progress stays intact.
4. Restart the overlay during the same game login: the chosen profile returns.
   Log out/reconnect: choose the character again. Quit the game: idle panel.
5. Visit an area without a route: the last campaign checklist is not reused.

Only recognized login boundaries can invalidate a character selection. If a new
client version returns to character selection without writing one, that path
needs a new observed boundary; do not infer identity from another player's
level-up/chat, a zone seed or the last saved zone.

## Automatic character recognition (v0.1.89)

The earlier manual-flow checks above describe v0.1.88. Windows now recognizes
creation/selection screen names and selects a profile after activation and zone
entry. Application restart clears the live binding while preserving histories.
The name hidden by a league tooltip is completed only from unique same-league
history. Details and known limits: [character identity](character-identity.md).

Automated validation: 19 recognizer tests, 4 checklist tests, 5 identity-state
tests, 6 profile tests and 30 tracker tests. The native Windows OCR path also
reads all four supplied references correctly. Frontend checks/build pass.
The Windows release build passes and v0.1.89 starts successfully. Saved progress
was backed up locally before replacing the running version.

Live acceptance sequence, with the overlay running first:

1. Create a character, edit its final name, then Continue into the first zone.
   Verify Settings shows that name without a manual choice. Cancelled/rejected
   creation must not create a quest profile.
2. Tick a task, leave the zone and return: the tick remains.
3. Return through Escape to selection; select a second character and Play.
   Its progress must remain separate. Repeat with an existing highlighted middle
   row and a previously read name partly hidden by its league tooltip.
4. Return to the first character: its ticks return. Finish a zone's objectives:
   `quests completed` replaces the checklist.
5. Repeat with fast Play/Continue and Enter. If a frame is missed or ambiguous,
   identity must remain unresolved. C is not an implemented recovery recognizer;
   use Settings → Character for this build.
6. Quit the game: idle panel. Restart the overlay mid-game: no stale identity;
   use fresh selection or manual recovery. Also verify focus changes alone keep
   the character bound.

Initial v0.1.89 live creation and Continue failed; v0.1.90 diagnostics isolated a
lost activation and caret-related OCR noise. v0.1.91 recognizes the game window
under a click, preserves names across screenshot/focus shortcuts, masks tall
input carets before OCR and displays identity status in menus. A repeated live
creation-to-Riverbank flow now binds and persists the correct new profile.
Existing-character switching/progress isolation and other transition/focus paths
still need live acceptance. No game files or game memory are modified.

## Windows overlay and VSync: deferred investigation

The owner tested v0.1.87 on 2026-11-12. PoE 2 was using DirectX 12,
windowed fullscreen at 2560x1440 on an RX 7900 XTX. The OS reported a 165 Hz
display on that adapter and a roughly 60 Hz display on the integrated AMD GPU.

Observed by the owner:

- Hiding the overlay with Ctrl+Space removes the stutter. The map reader still
  runs while hidden (`minimap::spawn` does not consult overlay visibility).
- Setting the game's FPS cap to 120 did not resolve it with the overlay visible.
  The later screenshot shows 171 FPS, so whether the cap was applied and enforced
  throughout that test is not established.
- Setting the game's VSync to Off resolves the problem with the overlay visible.
- Exiled Exchange 2 has a similar symptom on this machine; PoE Lurker does not.
- Disabling the second display was not tested. FreeSync's state was not established.

This points to an interaction between the visible overlay and the game's VSync /
Windows presentation path. It does not establish which DWM, WebView2 or driver
operation causes it. No game-frame timing or presentation-mode trace was captured.
VSync and FreeSync are separate settings: this test does not implicate FreeSync.

**Current workaround:** leave the game's VSync Off if that remains smooth. This can
allow tearing. Do not change the game's settings automatically. This issue is a
later Windows compatibility task, not a blocker for the current Act 1 layout work.

### Smallest experiment to try later

Use a separate Windows test configuration with `noRedirectionBitmap: true` on
the overlay and compass windows; keep the normal configuration as the baseline.
No dependency upgrade is needed: Cargo.lock already has Tauri 2.12.5, runtime-wry
2.12.3 and Tao 0.37.2, whose local sources expose and implement this option.

What the source inspection established:

- The current setting defaults to false. With a transparent window, Tao calls
  `DwmEnableBlurBehindWindow` with an empty region.
- Enabling it sets `WS_EX_NOREDIRECTIONBITMAP` and skips that transparency path.
- Our Windows hover watcher changes click-through only when the cursor crosses
  an input rectangle. Tao implements this by toggling `WS_EX_TRANSPARENT` and
  `WS_EX_LAYERED`; compatibility with the alternate path must be checked.

The [Tauri change](https://github.com/tauri-apps/tauri/pull/15410) documents the
option as a fix for a white flash, not game stuttering. It does not guarantee
Independent Flip, VRR or elimination of VSync stalls. A related
[upstream transparency report](https://github.com/tauri-apps/tauri/issues/15947)
also discusses the click-through styles, but its proposed cause is unconfirmed
and it is not evidence that this option fixes our issue.

Compare baseline and candidate in the same scene, with the same game settings:

1. Overlay hidden and visible, with VSync enabled; repeat with VSync Off.
2. Record game frame times and presentation mode with
   [PresentMon](https://github.com/GameTechDev/PresentMon), without an additional
   visible metrics overlay. Compare frame-time spikes as well as average FPS.
3. Verify transparency, checklist clicks, click-through outside the rows,
   dragging, settings, Ctrl+Space, Alt+Tab and the compass if enabled.

Keep the change only if it reduces the reported stutter without interaction or
display regressions. Otherwise stop this experiment and leave the broader
renderer/driver investigation for later. Test the second display only if the
first comparison is inconclusive.

Do not adopt `--disable-gpu` as a default fix: Microsoft's
[WebView2 performance guidance](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/performance)
recommends retaining GPU acceleration except for troubleshooting. Replacing the
renderer with WPF/DirectComposition or changing the capture backend would be a
larger task and is not justified by these observations alone. Microsoft's
[flip-model explanation](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/for-best-performance--use-dxgi-flip-model)
describes how overlaid desktop content can change composition, but a trace is
needed to establish the actual presentation mode on this machine.

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
