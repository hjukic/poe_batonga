# Offline map-reading experiment (Phase 2, step 7)

Status on 2026-11-11: an offline baseline is implemented and tested. It is not
ready for real-game navigation. Step 7 remains open.

**Live minimap reading (owner decision, 2026-11-11).** The owner chose to enable
continuous live reading of the minimap without waiting for GGG's answer, on the
condition that no game files or memory are accessed. `src-tauri/src/minimap.rs`
takes a screenshot of only the top-right minimap corner of the game's monitor about three
times a second (Linux/Hyprland via `hyprctl` and `grim`; Windows via the visible
client-area GDI adapter described in [Windows support](windows-minimap.md)). On
Windows it pauses when the game is not foreground or the minimap is covered.
On Hyprland the game must be on the visible workspace. It finds explored walls by their
lavender line colour (about RGB 91,91,121) and the player by the orange cross,
and shows a live preview in the overlay. Reading starts automatically with the
application; the owner explicitly removed the on/off setting. Raw frames stay
in memory; explored geometry can be saved by the layout recorder below. Nothing
is uploaded. It sends no input to the game. The layout matcher may display an
estimated arrow, whose gameplay accuracy on Windows remains unverified. Best results with the game's map
landscape transparency turned off. The policy notes below still describe the
unresolved question with GGG.

**Layout recorder (2026-11-11).** `src-tauri/src/layout.rs` builds our own layout
data while the owner plays. This follows the data-sources rule: project-original
data from our own explored-map observations, with no third-party images and no
game files. The minimap is centred on the player, so each frame's walls are
matched against the map built so far (an exact overlap scores above a one-cell
near miss) to find how far the player moved. The frames are stitched into a
map of the zone visit. Points of interest come from the live log: `arrived` (from
the previous zone), `left` (towards the next zone, which may also be a waypoint or
portal) and `npc` (where a named speaker, NPC or boss first spoke). Recordings
are saved per instance at `<app data>/layouts/<area>/<seed>.json`. A
re-entry, a teleport or lost tracking starts a new part with its own coordinates.
`node scripts/render-layouts.mjs` draws them as SVG. In a 60-second live test,
all 182 frames were placed with none lost, and the walls lined up cleanly.
Open questions for the arrow: whether instances of a zone share a layout family
(compare several Riverbank seeds), detecting minimap icons (waypoints,
checkpoints, transitions), and a fixed minimap zoom.

## Policy findings

GGG's [official developer policy](https://www.pathofexile.com/developer/docs/index#policy)
explicitly permits log reading when the user knows how the data is used. This
establishes a basis for the project's informed, read-only local log workflow.
The same policy distinguishes independent tools from prohibited game interaction
and constrains input macros. Our scope sends no game inputs. These provisions do
not specifically establish permission for continuous visual layout inference or
navigation. The owner chose live reading despite this unresolved question; that
choice is not GGG approval under the [Terms of Use](https://www.pathofexile.com/legal/terms-of-use-and-privacy-policy).

[A clarification draft](ggg-clarification-draft.md) is prepared but has not been sent.
This review does not claim GGG has approved the app.

### Pre-game review (2026-11-11)

Rechecked the official policy, Terms of Use and the staff reply in
[About translations](https://www.pathofexile.com/forum/view-thread/3590784/filter-account-type/staff).
The staff reply declines tool-specific assurances and advises against tools
providing an in-game advantage, interacting with game files or automating play.
It does not resolve permission for our planned navigation feature.

At the earlier pre-game review, before log/capture integration, source inspection found separate desktop windows and
shortcuts controlling those windows, with no game-memory access, injection,
game-input automation, log reader or capture implementation. The Node experiment
only analyzes existing local image files and is not launched by the native app.
These findings are not a guarantee against account action or GGG approval.

Conservative next test: the user plays manually and records ordinary gameplay
or takes manual screenshots using existing recording facilities, with our app
and analysis stopped. Analyze saved files after closing the game. This keeps
research separate from live assistance; it does not approve a future live
screen-reading/navigation feature. Do not let the computer-use agent move the
character or operate gameplay inputs during that test. No game launch was
confirmed in this review, and launching remains paused.

## Implemented baseline

`experiments/map-reading/` contains Node-only offline research code, separate
from the Tauri runtime. It reads local PNG crops, segments bright low-chroma
pixels, finds connected components in a configurable marker-color mask and
compares observed geometry with known template masks. A distance field tolerates
small pixel differences. Observed geometry supports a match; unexplored parts
of a template do not constitute evidence.

The baseline supports integer translation only (default +/-16 crop pixels),
fixed scale/orientation and map crops up to 256 x 256. It does not automatically
locate a minimap, solve zoom/rotation, distinguish text from map walls or identify
the real PoE 2 player icon. Its default green marker profile is synthetic test
configuration, not a calibrated game profile. Crop calibration is a research
input, not the intended product workflow.

Output includes extracted masks, marker candidates, competing matches, agreement
and an explicit unknown/candidate decision. Agreement is a heuristic, not a
probability. Competing positions within one layout can cause abstention as well
as competing layout IDs. Every result has `liveNavigationEnabled: false`; there
is no exit prediction or game-facing direction output.

## Reproduce

```sh
npm ci
npm run vision:test
npm run vision:experiment
```

The generated `experiments/map-reading/output/report.html` shows frames beside
diagnostics. `results.json` holds numeric results. Generated files and local
screenshots stay ignored by Git. Run arbitrary local PNG samples with:

```sh
npm run vision:experiment -- path/to/manifest.json path/to/local-output
```

See `experiments/map-reading/example-manifest.json`. Paths in the manifest are
relative to that manifest. The example image files are placeholders, not bundled
game assets. Use the default ignored output directory for gameplay-derived files.

## Results and limits

| Evidence | Result | Interpretation |
| --- | --- | --- |
| 12 synthetic development frames | 3 candidates, 9 unknowns as expected | Mechanics work on the deliberately constructed examples; not a gameplay benchmark |
| 15 automated checks | All pass, including duplicate templates, repeated positions in one template and invalid crops | Regression coverage for ambiguity and rejection behavior |
| Two existing historical PoE 2 Steam screenshots | Both unknown with the uncalibrated profile | Current extraction/marker detection is unsuitable for navigation |

The real images were a town UI screenshot and a party-combat screenshot, both
2560 x 1440. For the offline diagnostic only, the visible top-right minimap at
`[2190, 0, 365, 370]` was cropped and resized to 256 x 256. Originals were read
locally and left unchanged; derived images/reports remain in ignored output.
The local real-image report is `output/real-report/report.html`.

The town crop produced 95 bright map-mask pixels and no marker. The party crop
produced 204 pixels and six candidate blobs. Visual inspection of the mask shows
mostly labels rather than connected map lines; green party labels contaminate
the marker filter. No real-layout templates or ground-truth player positions were
available, so neither layout accuracy nor position error can be measured here.
Both screenshots are historical, not current-patch validation.

Synthetic extraction/matching took roughly 3-27 ms per 128-pixel frame in this
single local run. That excludes capture, real image preprocessing and runtime/UI
overhead; it is not a frame-rate or performance guarantee. There is no independent
held-out real dataset yet.

## Next experiment and acceptance plan

1. Build labelled explored-map crops, including the real player icon and party,
   waypoint, quest and text distractors. Prefer testing the expanded map alongside
   minimap crops, rather than tuning on these two unlabelled images alone.
2. Replace the bright-pixel filter with a real map-line/text separation approach;
   compare color/edge/shape features before deciding whether a model is needed.
   Use actual icon appearance and temporal consistency for player identity.
3. Separate calibration/training examples from independent runs/instances used for
   evaluation. Include hidden maps, fog, repetitive corridors, new variants,
   scaling, pan, resolutions, language changes and overlapping party icons.
4. Pre-register initial targets before evaluating that held-out set: map-mask F1
   >=0.85; player error <=8 native pixels on >=95% of clearly visible player
   frames; accepted layout accuracy >=95% with >=60% coverage of eligible frames;
   false acceptance <=1% on negative/ambiguous frames. These are proposed targets,
   not achieved measurements; review dataset size and labels before interpreting.
5. Validate permitted OS capture on Windows and Linux separately after the policy
   question is resolved. Test stale frames, denied capture, map closure and zone
   transitions before connecting any result to navigation.

Decision: retain the arrow as an unresolved conditional feature. The evidence
rejects this simple extraction baseline for gameplay; it does not prove all local
computer-vision approaches are infeasible. Do not advance step 7 to complete or
claim Linux capture support based on these results.

## Live arrow correction (2026-11-12, Qutm diagrams; replaced below)

The live Qutm guide previously favoured the diagram's original orientation over
rotations/mirrors by a fixed prior. With no observation it could report a 65%
lead as if it were useful confidence. It also treated a boss speaking, or walking
near a guessed icon, as completion; this sent the player toward Clearfell while
Miller's fight was starting. The screenshot supplied during testing demonstrates
the premature target change, not a measured angular error.

Bearings now use `atan2(target.y - player.y, target.x - player.x)` in the same
screen axes as the minimap: right 0°, down 90°, left 180°, up -90°. No character
facing or extra isometric rotation is applied. Both the SVG and the map ray use
that bearing. Rotation animation is removed so crossing ±180° cannot spin the
arrow the long way around.

All diagram orientations start with equal weight. Observed encounter landmarks
calibrate continuous rotation and scale, retaining both possible handednesses.
Competing directions suppress the arrow and show “Direction not confirmed”;
the score lead is no longer displayed as a probability. Wall-box fitting is still
only a schematic heuristic, not a validated match to real map geometry.

Boss dialogue pins the **player's location at that moment**, not the boss. It
keeps the boss objective active. Within 32 minimap cells of that encounter point,
the UI shows “Encounter area · defeat boss” with no invented enemy bearing. Farther
away it can point back to the recorded encounter area in screen coordinates.
Neither dialogue nor proximity completes an objective. The existing manual
completion/untick/reset controls now also update navigation through an explicit
route `navigationTarget` mapping. Unlock the overlay to click an objective;
locked mode retains click-through behavior. A verified kill event/visual boss detector is
not implemented; until then Miller cannot automatically switch to the exit just
because combat started. Leaving the zone changes the recording normally.

Regression checks cover the supplied fight-start scenario, cardinal/diagonal
bearings, walking past an encounter, arbitrary rotations, unresolved mirrors,
completion/untick/reset snapshots and suppression during tracking loss. A live
gameplay run remains necessary to measure tracking and navigation accuracy.

## poe2way terrain arrow (2026-11-12)

The arrow no longer uses Qutm's diagrams. It uses poe2way's collected terrain
(`data/layouts/poe2way/`) for Clearfell Encampment and Clearfell; The Riverbank has
no terrain there and shows no arrow.

- **Target:** the onward exit: the lowest-numbered destination that is not the zone
  just left, a town or ticked as done (Clearfell from the encampment: Mud Burrow,
  then The Grelwood). Several exits to the same zone are all goals.
- **Placement:** the visit starts at the exit to the zone it came from (else an
  exit with unknown destination, or a waypoint). Every placement (arrival exit x
  8 turns in 45° steps x mirror x scale 0.55-1.3 minimap cells per game unit) is scored by how many recorded walls lie within two
  6-unit cells of the outline, above the share chance would give for the area they
  cover. Squeezing walls into a narrow corridor therefore scores nothing.
- **Arrow:** "estimated" with at least 150 recorded walls, a best score of 0.5 and a
  30% lead over the best placement pointing more than 45° elsewhere. Before that
  (v0.1.11) a **rough** arrow shows right away: poe2way's layout as is (no turn,
  measured scale or 0.8), entered at the arrival exit; with an ambiguous but good
  match (score 0.5, no lead) the best placement instead. It points 12 cells
  (72 units) along the shortest walkable path, so it bends around walls; the
  distance is the path length. The arrow is only the compass around the character;
  the panel shows the target and how sure the direction is, without an arrow.
- **Scale:** a finished visit that clearly matches stores minimap cells per game
  unit in `terrain-scales.json`; later visits use it (or the median of all areas).

Tests use walls generated from the outline itself (unturned and mirrored), a grid
that matches nothing, and a U-shaped corridor. Real minimap walls are noisier;
the thresholds need checking on recorded Clearfell visits.

### First real checks (2026-11-12)

- **Clearfell Encampment** (fixed layout), 2,744 recorded walls arriving from The
  Riverbank: 90% on the outline at 0.78 cells per unit with no extra turn, once the
  base turn was corrected to `((x - y), -(x + y)) / √2` (world y points up; the
  first guess lacked the flip). This fixes the scale range.
- **Clearfell** instance seed 2693856399, ~2,800 walls near the encampment exit:
  best 40%, no clear lead, so no arrow. With the wide scale range (0.2-5) it had
  "matched" at 3.2 by squeezing all walls into the narrow tail by the exit; the
  narrower range rules that out. Either this instance is another layout than
  poe2way's or too little was explored; more Clearfell visits will tell.

## Arrow on the game's layout graphs (2026-11-12)

The Riverbank and Clearfell now use the datamined layout graphs
(`data/layouts/ggg/`, `src-tauri/src/layout/graph.rs`); Clearfell Encampment keeps
poe2way's map.

- **Placement:** the path walked since arrival (minimap cells, turned to world units
  with `from_screen`) is laid on every layout of the zone, starting at the node of
  the exit the player came through (else the zone's start node), with offsets up to
  80 units and the scale 0.78 ±8%. Score: mean distance of the path from the
  layout's corridors (distance field, 8-unit cells, capped at 150).
- **Target:** the onward exit (lowest-numbered zone not just come from or ticked;
  a town only when nothing else is left). In The Riverbank the exit is the Miller's
  arena, so the target is "The Bloated Miller" until that step is ticked.
- **Needle:** the route follows the corridors to the target and the needle points
  120 units along it. "estimated" when the best layout's mean is at most 60 units
  and it leads the best layout pointing more than 45° elsewhere by 20%; "rough"
  (fainter) when the best mean is at most 80; before 150 units have been walked,
  "rough" only if every layout points the same way.
- **First real checks:** the Clearfell visit (seed 2693856399) fits
  `ClearFell_W_S_N` (mean 42 units; runner-up `W_N_E` 50): rough needle toward Mud
  Burrow. In Riverbank instance 2090944099 the arrow at the spot where the Miller's
  fight began says the arena is 149 minimap cells away.

### Needle always visible (2026-11-12, owner request)

While there is a target the compass always shows its needle: gold when the layout
is confirmed ("estimated"), fainter gold when it is likely ("rough") and gray when
it is not recognised ("unconfirmed": the best guess, before a walk the direction
most layouts agree on), when there is no direction at all (the needle keeps its last
direction) or when the minimap is not tracking the player.
The needle no longer disappears within a guided zone: when no arrow can be worked out
for a moment (minimap tracking lost, a recorded piece with too little to go on) the
last one of the visit stays, gray. With nothing onward (back in the encampment from
Clearfell, back in The Riverbank from the encampment) the target is the zone just
left. It is hidden only in zones without route data and while the overlay is hidden.


## Big map

The owner plays with the big map (Tab) open most of the time. It hides the corner
minimap, so until 2026-11-12 tracking stopped for most of a zone: the Clearfell
visit (seed 2693856399) is 17 unrelated parts and the walk from Mud Burrow to the
Grelwood exit was not recorded at all. The reader now reads the big map whenever the
minimap is not showing, and hands the recorder frames in the minimap's units, so one
recording and one movement track continue across both views.

### Measurements (2026-11-12, Clearfell, 3840x2160, game idle near the Grelwood exit)

- **Line colour.** The big map draws its lines in the minimap's lavender, and the
  minimap's colour test finds them unchanged. Line pixels (groups of 60 px or more,
  50,254 px): mean RGB 95,96,128; 10th–90th percentile R 72–106, B 101–145; B−R
  median 35 (10th percentile 24). Lines are about 3 px wide. Lit ground behind the
  translucent map adds specks that pass the same test (3,305 px: mean 76,81,100, B−R
  median 23), almost all tiny: of 856 groups, 569 are 1–3 px. In 2 px cells, 1200x1036
  px around the player hold 226 groups of which 212 are under 15 cells. Dropping
  groups under 12 cells keeps every line cell and removes all specks of a plain-scene
  crop (207 lavender px, 0 cells left). Water edges (blue, about 24,72,136), labels
  and icons do not pass the test.
- **Player.** The big map is centred on the player: the orange marker (11x11 px, 59
  px) sits at 1919, 1059 px of the 3840x2160 game window, 21 px above the window
  centre, whatever the scene. (The character sprite stands higher; the map marker
  is what counts.)
- **Scale.** The big map's walls laid on recorded minimap walls of the same visit
  (parts 13–15, 2 px cells of the 600x518 capture), scale searched from 0.6 to 2.0
  in steps of 0.05, then 0.97–1.03 in steps of 0.005: best at **1.00** for every part,
  0.995 and 1.005 within 0.3% of it, 0.95 and 1.05 at about half. Big map and
  minimap draw at the same pixel scale, same orientation, no perspective: the lines
  overlap along the whole view (mean residual offset −0.1, +0.1 cell). The spacing of
  the Grelwood exit and the checkpoint next to it is 78x35 px in both views. Of a
  part's cells 58–60% land within a cell of big-map walls; the rest are recording
  artefacts (next point) and stitching drift. The test fixture (380x300 px around
  the player) gives 88% of recorded walls within a cell at scale 1.00, 31% at 0.94
  and 39% at 1.06.
- **Vertical bars in recordings.** The game window floats 10 logical px left of
  DP-1 (at 1910,1, size 1920x1080), so the monitor's last 20 px show a lavender
  window border, and that strip lay inside the minimap crop: it was recorded as
  walls. With the big map open the corner held only that strip (1,743 lavender px)
  and the recorder "tracked" it, which smeared it into the bars seen in parts 1–16.
  The capture areas now follow the game window and are clipped to it.
- **Minimap player.** About 3556, 284 px of the window (read from a quarter-size
  screenshot, so ±4 px): cell 158, 142 of the 300x259 minimap frame, below the
  frame centre (150, 129.5).
- **Time per frame** with the big map open (two grim captures, reading, recorder;
  20 live frames): debug build 107 ms (captures 40 + 41, reading 19, recorder 8),
  release 85 ms (captures 39 + 44, reading 0.9, recorder 0.8). With the minimap
  showing it is one capture, as before. Both fit the 330 ms frame interval.

### Reader

`src-tauri/src/bigmap.rs`; the choice per frame is `choose` in `minimap.rs`.

1. Capture the minimap corner. If the minimap is read (walls or marker, as before)
   and its player marker is where the minimap draws it, use the minimap.
2. Else capture the big map around the player (a 638x556 px area). If its marker is
   found within 16 px of where the big map draws it and at least 40 wall cells
   remain, use the big map: message "Reading big map".
3. Else the minimap's own reading ("Minimap not visible", or a minimap whose marker
   was not found, as before).

A big-map frame is built the way a minimap frame is: lavender pixels by the
minimap's test, 2 px cells at the minimap's scale (`SCALE` 1.0; another value
resamples), cells with at least two such pixels, then groups under 12 cells
dropped. It has the minimap frame's size (300x259) with the player on the minimap's
player cell, and reports the minimap's capture size, so the recorder sees no
difference.

### Limits

- Measured at 3840x2160 with the owner's settings. Other resolutions use the same
  fractions of the game window; untested.
- If the game lets the big map be zoomed, `SCALE` changes and big-map frames stop
  matching. Not seen; the fit above is at the default zoom.
- The minimap's player cell comes from a quarter-size screenshot (±2 cells). The
  recorder follows a reported player cell within 4 cells, so this is enough for one
  track; a full-size minimap screenshot would pin it.
- Movement across the two views is tested on synthetic pixels only; the live check
  is in [the overlay test](overlay-test.md#big-map-reading).
