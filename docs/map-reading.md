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

## Arrow on the game's layout graphs (2026-11-12, replaced the same night)

The first graph arrow laid the path walked since arrival on every layout as one
piece, from the arrival exit, and took the layout with the smallest mean distance
to its corridors. It needed one unbroken path from the arrival point (the Clearfell
visit is 17 unrelated parts), kept nothing across parts, re-entries or log events,
and its evidence was weak (Clearfell: `ClearFell_W_S_N` mean 42 units, runner-up
`W_N_E` 50). The navigator below replaces it.

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
  frame centre (150, 129.5). That is 284 px from both the right and the top edge,
  13.15% of the window's height: the game sizes the minimap by height and anchors it
  to the right edge, so the capture (600x518 px at 2160 px height) and this estimate
  are placed that way, and a 3440x1440 window gets a 400x346 px capture with the
  cross expected at about 3251, 189 px.
- **Time per frame** with the big map open (two grim captures, reading, recorder;
  20 live frames): debug build 107 ms (captures 40 + 41, reading 19, recorder 8),
  release 85 ms (captures 39 + 44, reading 0.9, recorder 0.8). With the minimap
  showing it is one capture, as before. Both fit the 330 ms frame interval. After the
  checks of the first fix pass (cross blobs, fills, line width, icons; a 685x603 px
  capture): debug 127 ms (captures 34 + 39, reading 45, recorder 8), release 85 ms
  (captures 40 + 41, reading 2.5, recorder 0.7).

### Reader

`src-tauri/src/bigmap.rs`; the choice per frame is `choose` in `minimap.rs`.

1. Capture the minimap corner. It counts as the minimap only with its player cross
   where the minimap's cross was last seen (24 px around it; the estimate until one
   is seen; the spot follows accepted minimap frames). Then use the minimap.
2. Else capture the big map around the player. If its cross is found within 16 px of
   where the big map draws it and its lines pass the checks below, use the big map:
   message "Reading big map".
3. Else not visible. With the big map open the corner shows big-map lines, which
   earlier went to the recorder as a minimap whenever the big map was not read (the
   same way the window-border strip did); a corner without its cross is never a
   minimap now. The message says why the big map was not read either, for example
   "Minimap not visible. Big map not read: player marker not found".

When the corner cannot be captured at all (on Windows: another window covers it),
the big map is still tried; only when that is not read either does the corner's
error count (and the game is located again).

A big-map frame is built the way a minimap frame is: lavender pixels by the
minimap's test, 2 px cells at the minimap's scale (`SCALE` 1.0; another value
resamples), cells with at least two such pixels, then groups under 12 cells
dropped. It has the minimap frame's size (300x259) with the player on the minimap's
player cell, and reports the minimap capture's own size (also when that is odd), so
the recorder stores one capture size whatever the view.

The 12-cell rule is not applied to minimap frames: in recorded minimap walls only
0.19–0.36% of cells (Clearfell parts 0, 13–15) lie in groups under 12 cells, so it
would not change how well frames of the two views match. Screenshots are taken with
`grim`, which is stopped after 2 s, so a hung screenshot does not stall the reader.

**The player cross** is found the same way in both views: of the orange blobs whose
centre is within 16 px (at 2160 px height) of where the view draws the player, the
nearest one of about the cross's size (58 px; 0.4–2.5 times that). Fire, item labels
or an exit icon next to it no longer move it, and a speck of orange is no cross. On
the minimap this replaces the average of every orange pixel in the corner, which an
exit icon in view (360 px against the cross's 58) pulled tens of cells off.

**The player cell follows the minimap.** The reader learns where the minimap's cross
really is from accepted minimap frames (averaged, kept within 24 px of the estimate)
and reports that cell in both views: big-map frames place their walls around it. Within
4 cells of each other the views would not step anyway (each places its walls around
the cell it reports, and the recorder follows a reported cell within 4 cells), but a
larger error (another aspect ratio, a misplaced estimate) made the recorder keep one
view's cell for the other's walls: with the cross painted 6 and 5 cells off the
estimate, the trail jumped by that at the first switch. Now it does not (test).

**Zoom.** A big map drawn at another scale than the minimap's would not match the
recorded walls. Big-map frames whose lines are not about as wide as at the default
zoom are not read ("map lines 5.0 px wide, not 3.25-4.75 (zoomed?)"). The width is wall
pixels per edge pixel times two, per 2160 px of height. At the default zoom 15 places
of a Clearfell screenshot gave 3.41–4.54; the fixture drawn 0.8 times as large gives
3.08 and 1.5 times 4.96, outside the accepted 3.25–4.75. Drawn 1.25 times as large it
gives 4.31, which cannot be told from the default this way: a zoom of 1.1–1.4 is not
caught. Whether the game draws zoomed lines wider at all, and whether the big map can
be zoomed, is not known (the morning check). A better check compares a big-map
frame's scale with the recorded walls; that belongs in the recorder.

**Filled areas are not walls.** The line colour also passes for bluish-grey floors,
ice or snow showing through the translucent map. Lines are at most two cells thick,
so a group of cells of which more than a quarter are inside cells (all eight
neighbours occupied) is a filled area and is dropped; a 60x60 px fill gives no wall
cells, while its outline drawn as a line stays. And when walls cover more than 15% of
the frame (Clearfell: 1.7%), the frame is "not read: too much of the view looks like
map lines". Only one real scene (Clearfell) has been measured; the scenes to check are
listed in [the overlay test](overlay-test.md#big-map-reading).

**Wide frames (on since 2026-11-12, pass 3).** Big-map frames are 600x518 cells
(1200x1036 px) centred on the player, four times a minimap frame: the big map shows
everything explored around the player, so each frame hands the recorder that much of
the map. The recorder takes the reported player cell when the frame size changes
(minimap ↔ big map; `Live::frame_size`), and stores the minimap's capture size for
both views. Minimap-sized big-map frames (`BigFrame::LikeMinimap`) stay available and
tested.

**After a gap** (frames not read for a while, the player running on), the recorder
used to search only 12 cells around where it expected the player and 32 around the
last place; anything further was lost, and after 9 lost frames a new piece of the
visit began. Now a frame that cannot be placed there is looked for within 150 cells
of the last place, the way pieces are joined (coarse search, unambiguous only, 60% of
its walls on known walls): `locate_piece` with a bound, called from `on_frame`. In a
simulated walk through a 900x700-cell area with fog (only walls within 120 cells of
where the player has been show), frames unread while the player runs on:

| Gap | Before, either size | Minimap-sized frames | Wide frames |
| --- | --- | --- | --- |
| 32 cells | 8 unplaced, new piece | 0 unplaced, one piece | 0 unplaced, one piece |
| 48 cells | 8 unplaced, new piece | 0, one piece | 0, one piece |
| 80 cells | 8 unplaced, new piece | 0, one piece | 0, one piece |
| 120 cells | 8 unplaced, new piece | 8 unplaced, new piece | 0, one piece |

**Joining pieces** (a waypoint jump 360 cells away, beyond the 150-cell search, then
walking back; pieces are tried every 3 s): wide frames joined the two pieces 21 frames
after the jump, minimap-sized frames after 51 (tries every 9 frames, so ±9). A piece
joins once 60% of its walls are on the live map; a wide frame covers that much of an
old piece from four times as far.

**Cost**, live, big map open (20 frames; `tauri dev` builds at opt-level 0): 175 ms a
frame (captures 35 + 44, reading 71, recorder 25); release 90 ms (reading 7, recorder
2). The reading was made cheaper for this: frame cells by column and row tables, and
icons looked for on every second pixel (icons are 20-30 px wide). 30 s live with the
big map open: 91 frames, all placed, slowest 227 ms.

### Map icons

Each frame also lists the map icons around the player, in either view: zone exits
(orange), checkpoints and waypoints, as minimap cells from the player
(`bigmap::Icon`, handed to `Recorder::on_icons` before each frame). The search takes
one byte per pixel (about 0.4 MB for a big-map capture, not 5.6 MB). Colours and
least sizes, measured on the big map: exits are orange blobs in the player cross's
colour, 359–373 px (the cross is 58, so it never counts); a checkpoint is a cyan
crystal over a cyan base (about 100,170,220; 145–185 px, joined within 20 px); a
waypoint is a pale blue ring (about 170,180,225, B−R 24–72); labels are neutral grey
and do not pass.

**Icons across the whole big map (pass 3).** With the big map open every icon found so
far is on screen, mostly far beyond the wall frame. Every third big-map frame the
reader captures the whole game window instead of the area around the player, reads the
wall frame from it as usual, and looks for icons over all of it, on every second pixel,
except where panels sit (fractions of the window: this app's checklist panel at its
default place, top left 19% x 40%; the zone and quest text, right 21% x 46%; the life
orb and flasks, left 32% of the bottom 22%; the mana orb and skills, right 28% of the
bottom 22%; the bottom 8%). This app's panel can be dragged anywhere: on whole-window
frames the reader asks `panel_rect` (`main.rs`; layer surface only) where it is and
leaves out that rectangle plus 8 logical px instead of the default place (a test pins
that a moved panel's orange boss name otherwise reads as exits). Without a layer
surface (Windows) the default place stays. The compass (needle #ffd27a, fainter when
rough, gray #9a9a9a when unsure, faint ring, label #ffe2a6 or #c4c4c4) passes none of
the colour tests over plain ground, at any opacity. Over an icon it can only widen that
icon (the needle over an exit stays orange), with one exception pinned in a test: the
gray "unsure" label's edge pixels half over a checkpoint's cyan blend into the
waypoint's pale blue, too few for a waypoint (it needs 80 px). The compass is not left
out, since that would lose the icons nearest the player. Those frames hand the recorder (`Recorder::on_icons`) every icon on
screen, in cells from the player; the other frames only the ones around the player.
If the whole-window capture fails, the area around the player is read as usual.
Live (Clearfell, idle near the Grelwood exit): every one of 7 whole-window scans found
all 6 icons on screen (2 exits, 3 checkpoints, the waypoint; the Mud Burrow exit 823
cells right of the player) and nothing else. Fixtures: the two real crops pasted at
their places on a window-sized canvas with orange text in the panels give the 5 icons
they hold and nothing from the panels. Cost at opt-level 0: whole-window frames 319 ms
(read 294, recorder 25), others 179 ms, 226 ms on average, so three frames a second
hold; release 127 and 88 ms.

**Exit labels (pass 4).** The big map writes each exit's zone name centred above its
icon. Widths alone cannot tell the names apart ("The Grelwood" 102 px, "Mud Burrow" 96
px), but the names an exit can have are known: the zone's `connectionNames`. So each
candidate is drawn in the game's own font (Fontin Regular, from the game's data,
`data/fonts/`, `scripts/fetch-fonts.sh`) and compared with the label's light-grey
pixels (`src-tauri/src/labels.rs`):

- Font and size found on the live screen: Fontin Regular at 17 px (16.8-17.2 tried) per
  2160 px of height; the text's top 25 px above the icon centre (searched ±6 px across,
  ±4 down).
- Score: 2PR/(P+R), P the share of label pixels in the name's box within a pixel of the
  drawn name, R the share of the drawn name within a pixel of label pixels, best over
  sizes and places. "Checkpoint" and "Waypoint" are always scored too.
- Read only when a zone name is best, scores at least 0.90 and leads the next by 0.10.

| Label (live fixtures) | Best | Next |
| --- | --- | --- |
| The Grelwood exit | The Grelwood 0.996 | Mud Burrow 0.773 |
| Mud Burrow exit (crystal over its M) | Mud Burrow 0.954 | Checkpoint 0.788 |
| Checkpoints | Checkpoint 0.970 | Waypoint 0.818-0.878 |
| Waypoint | Waypoint 0.919 | Checkpoint 0.825 |
| The Grelwood, left half covered | The Grelwood 0.744 | unread |
| The Grelwood with The Grelwood zone's names | The Red Vale 0.850 | lead 0.08, unread |

Live (7 whole-window scans): both exits read right every time, checkpoints and the
waypoint get no label. Labels are read only for exit icons, on whole-window frames,
and kept per icon place; a first read costs about 12 ms (opt-level 2). The reader
reads nothing until `minimap::set_label_candidates` gets the zone's names, which the
recorder's zone entry must call. The corner minimap draws labels too (the quarter-size
screenshot shows "The Grelwood" and "Checkpoint" by their icons, at the big map's
scale); reading them there is not built (no full-size minimap screenshot to test on).

Quality: on the whole Clearfell screenshot all six map icons are found (2 exits, 3
checkpoints, 1 waypoint) at their centres within a pixel or two, with no false ones
in the areas the reader captures. False hits appear only outside them: the life and
mana orbs, the skill bar, the desktop bar and the overlay's own checklist panel. On
the minimap (a quarter-size screenshot scaled up) the Grelwood exit and the checkpoint
are found, and the player cross is not taken for an exit. Plain scene and the
corner's zone text give none.

### Icons by their sprite (2026-11-12, third build)

The game's own icon pictures (`data/icons/`, [data sources](data-sources.md#map-icons-datamined))
are compared with the screen: a boss, a dead one, a rare monster or a portal is named by its sprite,
and what colour reads as an exit, a checkpoint or a waypoint is checked against them
(`bigmap.rs`, `identify`).

**Scale (measured).** Each sprite matched at scales 0.44–0.60 (screen pixels per pixel of the 64 px
sheet cell, at 2160 px height) onto the real icons of the Clearfell screenshot
(`sprite_scores_on_the_screenshot`):

| Icon | Best scale | Score there | At 0.48 |
| --- | --- | ---: | ---: |
| Exit (`Entrance`), both | 0.48 (0.46–0.50 within 0.01) | 0.98 (correlation 0.99, colour off 0.001–0.004) | 0.98 |
| Checkpoints, three | 0.52–0.54 | 0.73–0.83 | 0.72–0.78 |
| Waypoint | 0.52 | 0.80 | 0.47 |
| The Bloated Miller's own icon (`MillerActive`, the Riverbank) | 0.52–0.53 | 0.78 (correlation 0.93) | 0.63 |

(Scores before the hue term below, the Miller's with it.) So the exit icon is drawn at 0.48 of its sprite
and checkpoints, the waypoint and a boss's own icon larger, at about 0.53; the reader tries exits at 0.48,
checkpoints, waypoints and bosses' own icons at 0.53, and what has not been seen on a screen yet (skulls,
portals) at both.

**The first real boss icon (2026-11-12, live).** When the Bloated Miller first spoke, the app kept a
capture of the big map (`samples/boss-bigmap-*.ppm`; fixture `bigmap-boss-miller.ppm`): the player in his
arena, and right by the player's cross (13 cells) **his own icon, `MillerActive`, under its place name
"Besieged Encampment"** (the table's `Name` of `MillerActive`), no skull. The label's top sits 24 px above
the icon's centre, as for exits. So a campaign boss with a pair of its own shows that pair with its place
name, at least while alive and near; whether the icon is shown before the player gets there, follows the
boss, or turns grey after the kill is still not seen. The matcher names it at both strides
(`the_first_real_boss_icon_is_the_millers_own`): `MillerActive` 0.78, its grey twin 0.58, every other
sprite lower; the old reader saw nothing there. The threshold was lowered from 0.85 to **0.72** for it:
real icons other than exits score like checkpoints, not like exits.

**What the old reader made of other icons (drawn, I2).** Each sprite drawn onto the real big-map
fixture (alpha-blended, at a dark spot, on a map line, on the lit ground by the player) at 0.40–0.96,
read at every pixel and every second (`boss_icons_to_the_old_reader`, `an_orange_skull_was_an_exit_and_is_a_boss`):

| Drawn | Old reader |
| --- | --- |
| Orange skull (a unique monster) | nothing at 0.40–0.48; **an exit** at 0.52 on every second pixel (3 of 4 places), at 0.60 and 0.72 too |
| Yellow skull (rare), grey skulls, boss icons (Grim Tangle, Miller, Cemetery), NPC, quest icons, boss checkpoint | nothing at any scale |
| Town portal (blue) | **a checkpoint** at every scale, also a waypoint from 0.52 |
| `EntrancePortal` (red swirl) | an exit from 0.48 (it is one) |

Both were bugs (a skull counted as exit evidence; a portal as a checkpoint or a waypoint, which the
arrival check uses). Both are fixed and pinned.

**The match.** A sprite is drawn at the scale (4x4 samples per pixel, bilinear, premultiplied), at four
half-pixel phases. Only its core counts (alpha at least 0.9): the ground and map lines behind its soft
edges do not. Over the core, three numbers: the normalised correlation of all colour values in one
vector (so colour counts as much as shape), the distance of the mean colours (0–1) and the mean per-pixel
hue distance (red and blue against green, 0–1; it tells a grey icon from a dark coloured one, which the
correlation hardly does: the Miller's live and grey icons differed by 0.11 without it, 0.22 with).
Value = correlation − 3 × colour − 2 × hue. The search: around the guessed centre (4 px; 8 px for
colour groups, whose pale part lies high), every second pixel on half the core, then the pixels around
the best, then the phases; a sprite whose mean colour is more than 0.15 off everywhere near is not
searched.

**Where it looks.**
1. Every icon colour found: compared with the generic sprites and the area's boss pairs. When another
   kind's sprite scores at least 0.72 and leads every other kind by 0.15 it takes that kind (never an
   exit, checkpoint or waypoint made from something else); else it stays as it was and carries its
   own best sprite's id from 0.5 (`Entrance`, `Checkpoint`, `Waypoint`). Two colour icons of one
   object (the portal) become one.
2. Groups of orange (not an exit), warm and pale blobs, joined within half an icon, at least 8% of an
   icon's area, at most 1.6 icons across, largest first, 16 at most: the skulls (the orange skull is
   36% cross-orange and 25% warm, the yellow 57% warm, the grey 42% pale).
3. Below each line of pale letters that belongs to no icon found (the label's top 25 px above the
   icon's centre, as measured for exits): a boss's place name. Only in an area with boss pairs.
4. Where this visit saw a boss pair's icon (`set_icon_hints`, from the recorder), within 10 px: most
   boss icons are dark (the Miller 13% bright pixels, the Executioner 2%), and grey once killed with no
   label (the table gives inactive icons no name), so neither colour nor a label finds them.

A new place is named only as a kind colour cannot make (boss, boss-dead, dead, rare, portal, npc,
quest), at 0.72 with a lead of 0.15.

**Scores (measured, Clearfell screenshot, `matcher_on_the_screenshot`).** With the area's pairs
(Clearfell) the reader finds the same six icons as before and nothing else; each carries its sprite.
Real icons now: exits 0.95, checkpoints 0.55–0.66, the waypoint 0.67 (at 0.53). At the 22 places tried
on the whole screenshot (the six icons, colour groups by the player and the zone's name label), the best
of **all 108 sprites** of a kind that is not there scored at most **0.64** (`G4_10_BossInactive` by the
player's name label; skulls at most 0.59). The threshold 0.72 lies between that and the real Miller's
icon (0.78); drawn sprites score 0.95–1.0, real exits 0.95. On the four Riverbank captures it names
nothing but the Miller. The fixtures with every area's pairs give no
boss, dead, rare or portal (`no_boss_or_monster_on_the_real_fixtures_in_any_area`).

**Drawn, not seen (pinned in tests).** The orange skull is a boss at 0.48 and 0.52 at every place and both
strides (drawn bosses' own icons at 0.53); the yellow skull a rare; both grey skulls "dead"; the portal a portal; the Grim Tangle's icon is
`GrimTangleBossActive` in G1_6 and nothing elsewhere; the Miller's dark icon is found below a label or at a
hint, and its grey icon at a hint 2 px off.

**The grey skulls are one picture.** `UniqueMonsterDead` and `RareMonsterDead` differ by one pixel of
offset in their cells (mean difference 0.9 of 255 after it). A grey skull is therefore `"dead"`: a dead
unique or a dead rare, never by itself a dead boss.

**Cost (measured, release, best of 5).** Whole window every second pixel: 6.5 ms without the matcher,
19 ms with; the area around the player (685 x 603 px): 0.3 ms and 5.7 ms. About +12 ms on every third
frame and +5.5 ms on the others; the frame interval is 330 ms.

**Kinds (`bigmap::Icon`).** `kind` is `exit`, `checkpoint`, `waypoint` (colour, as before), or what the
matcher named: `boss` (an orange skull, a unique monster alive; or a boss's own icon, `…Active`),
`boss-dead` (a boss's own grey icon, `…Inactive`), `dead` (a grey skull), `rare` (a yellow skull),
`portal`, `npc`, `quest`. `sprite` carries the sprite's id (`GrimTangleBossActive`), so a boss's own icon
is known by name; `data/icons/index.json` says whose. Rares, portals and the rest are read only so they
are not taken for anything else: the navigator uses exits and, from I5 on, bosses.

**The corner minimap** goes through the same reading (every pixel, `bigmap::icons`), with the same
scales: the table gives every icon taken here `MinimapIconRadius` = `LargemapIconRadius` (2), which is
read as "drawn the same size". Not measured: no full-size minimap capture with an icon exists yet
(`the_corner_minimap_names_icons_too` pins the reading on a synthetic minimap).

**Captures to check it on.** Besides the boss-line captures, the reader keeps a picture whenever it names
a boss, a dead one or a rare (`samples/icon-<kind>-<sprite>-<view>-<time>.ppm`, at most one a minute, 12
in all, counted over restarts): the minimap corner it was read in, or 400 x 400 px of the whole window
around the icon on the big map.

**Assumed.** That campaign bosses show these icons at all, when, and that the grey one stays after the
kill (the icon table's word); that skulls and boss icons are drawn at 0.48–0.53 like exits and
checkpoints; that a boss's place name is drawn above its icon like the exit labels; that the corner
minimap draws icons at the big map's size (the table's `MinimapIconRadius` equals `LargemapIconRadius`
for all of them; not measured). The boss captures (`samples/boss-*.ppm`) are where this is first checked.

### Limits

- Measured at 3840x2160 with the owner's settings. Other sizes use the same fractions
  of the window's height (minimap, from the right edge) and of its size (big map);
  untested on a real game.
- If the game lets the big map be zoomed, `SCALE` changes and big-map frames stop
  matching. The line-width check catches 0.8 or 1.5 times, not 1.25 (see "Zoom").
- The minimap's estimate comes from a quarter-size screenshot (±2 cells); the reader
  learns the real spot from the minimap. A full-size minimap screenshot would pin it.
- Movement across the two views is tested on synthetic pixels only; the live check
  is in [the overlay test](overlay-test.md#big-map-reading).

## The navigator (2026-11-12, night build)

`src-tauri/src/layout/nav.rs`: one belief over (layout, position) per zone visit,
updated with every measured step and log anchor, never reset by a tracking loss.
Areas with layout graphs (`data/layouts/ggg/`, built per area on first entry) use
it; Clearfell Encampment keeps poe2way's terrain arrow. The compass shows a needle
in every area that has graphs or terrain.

- **Belief:** per layout ~200 weighted particles (x, y in the file's units, a scale
  error around 1) and three kinds of log evidence: the walked line's (`path`), the
  anchors' and exit sightings' (`anchors`), and the fit of the last whole scan's
  named exits (`named`). Layout posterior `p_k ∝ prior_k · exp(path_k + anchors_k +
  named_k)`.
- **Start:** at the nodes of the exit the player came through (spread 40 units), else
  the layout's start nodes, else along all corridors; 10% always along all corridors
  (30% when coming from a town: portals and waypoints are the usual way back).
- **Movement:** each frame's move (minimap cells, turned to world units, divided by
  scale and the particle's scale error) moves the particles, with noise of 20% of
  the distance; moves under 2 cells (registration jitter) do not count as walked.
  Every 15 units walked they are weighted by their distance to the nearest corridor:
  full weight within 60 units, a Gaussian of 60 beyond, over a floor of 0.1.
  Resampled when the effective sample size drops below half; when nothing fits, 10%
  are redrawn along the corridors.
- **What the walked line may claim:** each weighing counts half, and the path's lead
  of any layout over another is capped so the path alone gives the top layout at
  most 60%. On Clearfell 2693856399 the walk prefers a wrong layout (`W_S_N`; the big
  map shows `W_E_N`): the graph's straight corridors differ from the real ones in
  ways that do not average out.
- **Tracking lost:** `lost(seconds)` spreads the particles by what a player walks in
  that time (60 units/s, at most 2,400): 80% along their corridor, mostly the way the
  player was heading, 20% anywhere within reach; after 20 s, 30% are redrawn along all
  corridors. Evidence stays; the exit sightings and icons in hand are dropped. The
  recorder calls it only for a new part (within a part a pause is a measured move).
- **Log anchors:** a checklist step with a target that the tracker marks as completed
  by the log is `seen(Exit | Tag)`: agreement with being at a matching node
  (Gaussian of 80 units, 150 for a tag) adds to `anchors`, then the particles are
  placed there (10% stay); a layout without such a node loses 7 nats. Leaving the
  zone is `seen(Left)`: soft (floor 0.3, nothing moved, not an anchor).

### Exit icons

The reader finds the exit icons in every frame (minimap cells from the player);
every third big-map frame scans the whole window and reads the exits' labels (the
zone they lead to). `Recorder::on_icons(icons, whole)` hands them over before
`on_frame`; exits outside the wall frame from the last whole scan are kept in the
part's coordinates for the 3 frames until the next one (a scan without them clears
them).

- **Sightings:** an icon's place is the player's dead-reckoned track since the last
  loss plus its offset; icons within 40 units are one exit. A new sighting counts in
  full; a label read for one already counted is an upgrade, counted once at half
  weight; the same sightings again count only after 60 units walked, at half weight.
  A sighting keeps its label in the unlabelled frames between scans. Only a new
  sighting that the belief held before it predicted (50%) is absolute evidence.
- **Evidence:** an icon at offset `d` says the player stands at (an exit) − `d`. Each
  particle is weighed by how well every icon lands on an exit node of its layout
  (Gaussian of 100 world units, floor 0.05; a labelled icon only on the exit to
  that area, an unlabelled one on any exit). Two unlabelled icons also test the
  vector between two of the layout's exits. The update adds to `anchors` the log of
  the better of the two (at least 0.005, the same for every layout). When the belief
  does not explain the icons, 30% of the particles are redrawn where they put the
  player; a layout already at 50% or more is not redrawn and only half-follows.
- **Named exits decide:** two or more exits read by name give a position-free fit
  per layout (do its named exits lie as the icons do), which replaces the last one
  in `named` (one scan counts once); a pair that does not fit costs ln(1e-4). This
  overrides whatever was believed before: the owner's Clearfell ends at `W_E_N`.
- **Remembered exits:** where an icon put an exit is kept per layout (within 100
  units of the node, blended) and used only for the way to a target icon, never as
  evidence: learnt under each layout's own belief, it had made every layout agree
  with its own past sightings (the v0.1.41 regression).
- **Two unlabelled icons are not enough** in Clearfell: with three exits a layout has
  six pairings, and other layouts fit the owner's pair as well as the true one.

### Needle

- **Target:** the first open checklist target some layout has (exit nodes, or nodes
  whose tag or keys match), then the onward exit by area order. A target no layout
  has gives no needle.
- **Along the route:** for every layout and particle the route along the corridors to
  the target (per corridor point: the point 120 units ahead and the distance to go;
  straight to an exit the graph leaves unconnected). The needle points where most of
  the belief points (the densest 90° sector of the weighted directions);
  `confidence = |V|`, `V = Σ p_k Σ w_i u_i`.
- **At the target's icon:** when the label (or 95% of the belief) says an icon is the
  target's exit (of equally good icons the nearest; a name maps to all its area ids),
  the route starts where the icon puts the player in each layout. The needle points
  at the icon once that route is under 1.2 times the straight line, and back along
  the route above 1.5 (hysteresis). The distance shown is the icon's.
- **Gold ("estimated"):**
  - at the icon: within 200 cells, when its label was read the same in two scans and
    no layout at 90% or more puts it on another exit (then gold in every area, also
    unmeasured ones and dungeons), or unlabelled with one layout at 90%;
  - along the route: `|V| ≥ 0.9`, one layout at 90%, and absolute evidence this visit
    (a log anchor or a confirming sighting); the walked line alone is never gold;
  - inferred gold only in the areas replayed against a known truth (The Riverbank,
    Clearfell): elsewhere at most rough, and unconfirmed wherever a dungeon layout
    carries belief; any gold turns gray after 2 s without a placed frame.
  - "rough" at `|V| ≥ 0.5`, else "unconfirmed" (gray). `variant` names the top layout
    at 50%.
- **Live (v0.1.43, the owner's screen):** Clearfell 2693856399, app restarted, big map
  open, The Grelwood (−136, 83) and Mud Burrow (823, 110) read by name: `W_E_N` 0.997,
  the needle gold at the Grelwood exit. The test
  `two_named_exits_win_whatever_was_believed_before` replays it from the file v0.1.41
  wrote (W_E_N was 0.006 there): 0.999, gold at 148.6° (the icon's bearing), also with
  the same stale beliefs in a current file and with no memory.

### Memory

`layouts/<area>/<seed>.nav.json` beside the recording, version 1 (older files are
ignored: their anchors and exits were counted the old way): the posterior, the
`path`, `anchors` and `named` evidence per layout, remembered exits, the units
walked, and for layouts above 1% 32 particles, with the time of the last data.
Written only when the navigator got data (a log catch-up leaves it untouched). A
re-entry starts from the kept evidence (anchors capped on load like the path's); an
app restart inside the instance (no zone change, data under 10 minutes old) resumes
the particles, spread by what the player could have walked since. Gold waits for
this visit's own evidence.

**Dungeons (`.dgr`):** one file unit is `roomUnitSize − 1` world units (derived, not
checked in play); their particles get a scale prior from 1/2.5 to 2.5. Inferred
needles there are never gold.

### Simulation (`src-tauri/src/layout/nav_sim.rs`)

A walker harder than the owner and independent of the navigator: heavy-tailed
sideways wander (σ 35, stretches of σ 120 a third of the time; its distance to the
corridors, percentiles 25/50/75/90/95, is 13/29/55/90/121 against the recorded
14/38/63/91/115 and 9/26/67/120/150), speed 0.6-1.4 times 60 u/s, standing still 2-10
s, backing up 50-150 units, a wrong turn at one junction in three, start offset up to
40, scale error up to ±6%, whole minimap cells, tracking losses of 5-30 frames (one
per ~33 s), and in 30% of the runs a wrong arrival. Its route is its own shortest
path; the truth is the point 120 units ahead on its own plan. Exit icons: the exits
sit up to 100 units from their nodes, are seen within the minimap's view (±150 x
±130 cells) or twice that (wide), 20% missed, 2-cell noise, a false icon in 1% of
frames; labels, when on, are always right and on every frame (the live every-third-
frame pattern and misreads are covered by unit tests, not here). Runs: every layout
of G1_1 and G1_2, from each exit to each other exit (Riverbank: start ↔ Miller exit).
`cargo test` runs one seed; `cargo test --release nav_sim::tests::report -- --ignored
--nocapture` three.

`report` at three seeds on the final build (v0.1.43; 30% of runs told a wrong
arrival unless noted). Gold steps out of all steps; "off" = gold more than 60° from
the truth:

| Case | Runs (not walkable) | Gold steps | Gold off | Within 45° at 60% | Top layout true for good |
| --- | --- | ---: | ---: | ---: | ---: |
| No icons, arrival known | 579 (45) | 0 of 57,321 | — | 30% | 22% |
| No icons, arrival told right | 588 (36) | 0 | — | 32% | 30% |
| No icons, arrival unknown | 588 (36) | 0 | — | 13% | 11% |
| No icons, 30 s loss at 40% | 579 (45) | 0 | — | 26% | 12% |
| No icons, movement turned 90° | 579 (45) | 0 | — | 20% | 3% |
| No icons, no tracking losses | 579 (45) | 0 | — | 32% | 28% |
| Icons unlabelled, minimap view | 579 (45) | 16 | 0 | 35% | 34% |
| Icons unlabelled, wide view | 579 (45) | 387 (0.7%) | 8 (2.1%) | 52% | 42% |
| Icons labelled, minimap view | 579 (45) | 2,347 (4.0%) | **202 (8.6%)** | 38% | 37% |
| Icons unlabelled, arrival unknown | 588 (36) | 8 | 0 | 24% | 30% |
| Icons unlabelled, 30 s loss at 40% | 579 (45) | 0 | — | 47% | 17% |
| Icons unlabelled, movement turned 90° | 579 (45) | 0 | — | 23% | 3% |

- **Gate A (at most 3% of gold off, no layout over 10%):** met where there is gold
  without labels (0 of 16, 8 of 387). **Not met with labelled icons: 8.6% of gold
  off** at three seeds (one seed, wide view: 3.9%, which `cargo test` pins at 4%).
  Without icons there is no gold at all, so nothing to check, and the planted 90°
  turn gives no gold either (the bite test runs with labelled wide icons: 82% off).
  Likely cause: a labelled icon pins the position to the exit's node, and the real
  exit sits up to 100 units from it; near junctions and walls the way from there
  differs from the walker's. Note the simulation's labels are perfect and on every
  frame, unlike the live ones (read on every third frame, twice before gold).
- **Gate B (85% within 45° at 60%): not met** in any case (13-52%); see below.

- **Gate B (85% within 45° at 60%) is not reachable on these graphs:** another layout
  still has corridors under the walk until a median 59% of it (75th percentile 75%),
  and in 186 walks 112 rival layouts fit the whole walk (league variants, mostly).
- **Mutations** (`mutations`, one seed, labelled wide icons, before S7): `GOLD` 0.5
  with the top layout at 0.5 (16.5% of gold wrong) and gold from the walk (7.5%) fail
  Gate A's share; `FLAT` 120 and no top-layout requirement pass the share but fail per
  layout; `MISSING` 0 changes nothing (the walker sees no anchors).

### Real recordings

`RECORDINGS=<dir> cargo test --release navigator_on_recordings -- --ignored
--nocapture` feeds every part's trail as the app would: parts in order (a new part
is a loss of unknown length, a jump inside a joined part a 30 s loss), "left" points
as `seen(Left)`, "arrived" points as a new visit from the instance's memory. It
prints the posterior after each part and the final needles, and asserts Clearfell's
truth.

- **Clearfell** seed 2693856399 (17 parts). Truth (chair, from the big-map
  screenshot with both exits in view): The Grelwood − Mud Burrow = (−959, −28)
  cells; `ClearFell_W_E_N` predicts (−875, −89), 103 cells off, every other layout
  417 or more. The first pass put `ClearFell_W_S_N` at 1.00 from the walk: wrong and
  sure. Now no layout passes 0.21 and `W_E_N` keeps 0.049 (uniform 0.030; the walk
  ranks it 5th, so the walk alone cannot single it out). The test asserts the truth
  at least uniform after part 0, never under 2%, and no layout above 60%.
- **The Riverbank** 1580033459: at most 0.31 for any layout. 2090944099:
  `Riverbed_4` at most 0.54 (0.93 before), where a batch fit of its long part
  prefers `Riverbed_6`; there is no truth for these instances.

### The way through the recorded map (2026-11-12, day build)

`src-tauri/src/layout/route.rs`; used by `Navigator::guide_on`. The recorder lends the
navigator the live part's walls and trail and the player's cell at each needle
(`nav::Place`, borrowed, about once a second); nothing is copied per frame.

**The search.** From the player's cell to the target's, the shortest way that crosses
no recorded wall, on a grid of 3x3 minimap cells:

- A search cell is a wall when it holds a cell seen at least twice (`n >= 2`) and its
  3x3 search cells hold at least 3 such cells: lines always do; specks of one or two
  cells, and cells seen once, never do. A wall closes its 8 neighbours too, so a line
  is about 9 minimap cells thick.
- Gaps (closed room round the target, one gap facing the player, three grid offsets):
  every gap of 1-8 cells is closed, every opening of 11 or more is open, 9-10 depend on
  where the grid falls. The grid lies on multiples of 3 cells (fix pass 1, review B-3),
  so the same walls give the same doors on every call: before, its origin moved with
  the search box, and a 9-cell gap opened and shut as the player walked
  (`the_same_walls_give_the_same_doors_wherever_the_player_stands`). A closed diamond of one-cell diagonal lines does not leak
  (moves never pass a closed cell diagonally).
- Cells the player walked through are open whatever was drawn there (trail steps up to
  30 cells; longer ones are jumps). The recordings need it: within a cell of a recorded
  wall lie 16% of the walked line of Clearfell's part 0, 8-46% in other parts of the
  owner's recordings (Riverbank 8-31%; Clearfell parts 12 and 16 0%). That is the
  recording, not the ground: the big-map screenshot shows these outlines as obstacles
  the player walks round. Likely cause: the minimap's player cell was estimated badly
  before the cross was found by its blob (see "Big map").
  A line opens only where the walk crossed it (fix pass 1, review B-4): the walk went
  into closed cells from open ground and came out on open ground that open ground does
  not join to where it went in within 30 cells. A walk that dips into a line and comes
  back out on the same side, or stays inside its band (running beside it with drift),
  opens nothing: before, its chords punched one-cell doors into lines the player only
  grazed (`a_trail_that_runs_beside_a_wall_opens_no_door`: a walk 4 cells off a line,
  wobbling 4 cells over it twice; the way goes round the line's end). A cost on opened
  cells instead (the review's other fix) pulled the way back off the walk on the real
  map (within 20 cells of it: 0.78 against 0.90).
- A player in a closed cell (next to a line) starts from the open cells within 3
  search cells reached without crossing a wall cell; a target in one (an exit drawn
  on a cliff line, a point on a line drawn twice) is reached from either side (that
  found a way for one more of the 44 real pairs below).
- Fog, where nothing is recorded, costs what open ground costs: an unrecorded area may
  be open ground nobody has seen, and on the minimap open ground and fog look the
  same. The alternative, a higher cost away from where the player has been, was not
  built: it bends a way across fog towards the trail, where straight is the best guess.
- The search box is the box round player and target, widened by 0.75 times their
  distance (at least 150, at most 600 cells), cut to the recorded map's box widened by
  60 cells (fix pass 1, review B-7; it was 6, so a wall ending at the edge of what was
  seen could not be gone round). It gives up after 60,000 search cells, and a grid of
  more than 1.5 million search cells (a target some 3,700 cells off: a wrong merge, a
  bad sighting) is not built at all.
  No way found (target walled in, or past the bound): `None`, and the needle is what
  it was before.
- The result: the way's corners (cut by line of sight over up to 64 cells), its
  length, its cells, and where the needle looks: the farthest point of the way within
  40 cells that the player sees in a straight line (review B-9). The point 40 cells
  along, past a corner, made the needle cut the jamb: a door 13 cells off with the way
  turning down behind it gave 64° against the corner's 4°; now 9°
  (`the_needle_looks_at_the_corner_not_through_the_jamb`).
- Hysteresis: the cells of the last way to the same target (and their neighbours)
  cost 90%, so the needle keeps its way unless another is clearly shorter (an island
  between: a step that makes the other side 3% shorter keeps the way, 15% switches).
  A loss forgets the way.

| Synthetic map (tests in `route.rs`) | Result |
| --- | --- |
| Wall between, 20-cell doorway 70 cells to the side | needle −28° (doorway −30°, straight −90°), 157 cells (straight 80) |
| U-shaped corridor, target in the other arm 64 cells away | needle down the arm (86°), 243 cells |
| Target in fog 192 cells away | straight: same length and bearing |
| Specks (1 cell in 200) and a whole line seen once across the way | straight: 200 for 200 |
| A walked trail across a closed room's wall | in along the trail; a 120-cell jump opens nothing |

**Time** (release; the app's crate builds at opt-level 2): a map of 40,000 wall cells
(room outlines over 1,600x1,600 cells): a target 330 cells off 0.4 ms (815 search cells
expanded, a grid of 40,000), 2,150 cells across 4.7 ms (41,000 of 329,000), a walled-in
target 5.5 ms (gives up at 60,000). The tests pin the expanded counts and the grid
size, and times at about ten times the release ones (the test profile is opt-level 1). On Clearfell's
part 0 (21,500 walls seen twice) a `guide_on` call with a named icon 300 cells off:
median 0.5 ms, slowest 2.0 ms (25 places along the trail; 0.31 and 1.8 before fix pass 1). It runs on the recorder's
lock about once a second.

**Real walls** (`clearfell-part0-map.txt`, Clearfell 2693856399 part 0: walls and
trail, 52 KB). 44 pairs of trail points 40 points apart along stretches without a jump
(300-500 cells walked):

- To a target ahead (the walls and the trail up to the player only): a way for 38 of
  44 (41 before lines opened only where the walk crossed them; for the others the walk
  crossed recorded lines, or ran inside their band); 9 of the 38 go through a line the
  walk crossed. The straight lines cross 404 unwalked wall cells, the ways none; median
  way 1.09 times the straight line. Where the player walked across a recorded line
  (above) the way goes round it, so on this recording it can be longer than the walk.
- Going back (the trail recorded): a way for 43 of 44, 86% of it within 20 cells of the
  trail (median), half the walked length (the walk wanders; the way cuts across open
  ground); 20 of the 43 go through a line the walk crossed.
- **Where the player went** (fix pass 1, review B-8;
  `on_real_walls_the_needle_points_where_the_player_went`): the same pairs with the map
  cut to what the recorder had then (the walls within the minimap's view of the walk so
  far, the walk so far), the needle's bearing against the walk's next 5 trail points
  (40-60 cells). A way for 37 of 44; 2 are about straight across fog (no map needle); of
  the 35 map needles 16 are within 45° of where the player went, against 21 for the
  straight line to the target on the same pairs. Gold-eligible ones (half by recorded
  ground, at most 1.5 times the straight line): 14 of 29, straight 18. Where the walk's
  next stretch crosses a recorded line (drift; 15 pairs) 5 against 8, elsewhere 11 of 20
  against 13. So on this recording the map does not beat the straight line: the drift
  puts lines where the player walked, and the way goes round them. The test pins 40%
  for both. The proxy is rough (the walk wanders: the straight line itself agrees only
  60%); a recording made after the cross fix (Clearfell parts 12 and 16 have no walk on
  a line) or the owner's play should decide whether the map's gold stays.
- Gaps: of 30 lines (groups of 20 or more cells seen twice), 8 have a gap of 1-8 cells
  to another line (widths 1, 1, 1, 3, 3, 5, 5, 6), which the raw grid would leak
  through; all 8 are closed. Two more have a narrowest gap of 9-11 cells, left open.

**Shadow mode for ways ahead** (fix pass 2, the chair's decision). On the only real
recording the map needle matched where the player went less often than the straight
line (16 against 21 of 35; gold-eligible 14 against 18 of 29; "Where the player went"
above). The recording is old and its walls drift, so that does not condemn the way,
but it does not support showing it. So for a target placed from an icon in view or a
sighting (a way ahead) the way is still worked out, but the needle shown is exactly
what `guide_on` gives without the map (as on v0.1.46). What the map would have shown
goes to the flight recorder on that needle line, as `shadow` (its bearing, length,
look-ahead, share over unrecorded ground, would-be gold), and the replay scores it
against the exit taken in a third row, "map (shadow)" (see "Replay"). One constant,
`nav::MAP_AHEAD_LIVE` (false), switches ways ahead on; the tests of their live
behaviour switch it on for themselves (`Navigator::map_ahead_live`), and
`ways_ahead_run_in_shadow` pins the default. The way back to a trusted arrival stays
live: it rests on ground the player walked, not on inferred doors.

**What would switch ways ahead on** (proposed): over at least five real visits scored
against the exit taken (recorded after the cross fix), the shadow within 45° at least
as often as the needle shown on the same frames, and the would-be gold shadow wrong
(more than 45° off) on at most 3% of its frames. The replay's last line gives both
numbers over all visits.

**The needle** (`guide_on`). The target's place in the part's coordinates is known
when (in this order):

1. its exit icon is in view and the label (or 95% of the belief) says it is the
   target's, as for the needle at the icon;
2. it is the exit the player came in by (the target's `exit` is the area the live
   part's arrival came from): where the player appeared, when that was by the exit
   (fix pass 1, review B-2). The recorder notes at the first placed frame how long
   after the zone entry it came and which icons were by it. Not trusted: from a town,
   more than 5 s after the entry (the player may have walked on), or a waypoint icon
   within 15 cells (a waypoint journey). Sure (gold allowed) only with an exit icon
   within 40 cells, unread or read as that zone; else rough. The point is gone with
   the part (a new part has new coordinates). Tests:
   `an_arrival_counts_as_the_exit_only_soon_and_without_a_waypoint`,
   `an_arrival_is_the_exits_place_only_by_its_exit_and_soon_after_the_entry`;
3. it was seen earlier in this part: a sighting's place on the dead-reckoned track is
   turned back into cells from the player (sightings end at every loss, and a new part
   starts with one). Not after a placement jump (a frame placed more than 20 cells from
   the last: a wide re-find, which may be a wrong match and would leave the sighting
   off against the walls by as much; fix pass 1, review B-6). It still counts as
   evidence (`a_placement_jump_ends_the_sighting_as_a_target_on_the_map`).

Then the way there. If one is found and is at most 4 times the straight line (or 150
cells more), the needle looks 40 cells along it and the distance shown is its length.

**Recorded ground** (fix pass 1, review B-1): a way's search cells count as recorded
within 15 cells of a wall cell seen twice or of the walked trail. A way that is about
the straight line (no corner a search cell off it, at most 5% longer) and lies less
than half over recorded ground is not taken: across fog the map knows nothing, and the
layouts' route (which may say "round the river") stays the needle, as without a map
(`a_straight_way_across_unrecorded_ground_leaves_the_layout_needle`: a U-shaped layout,
an empty map). A way less than half over recorded ground is never gold. The navigator
reports what the last needle took from the map (`Navigator::last_way`: the look-ahead
offset from the player and the share over unrecorded ground) for the flight recorder.

**Long ways** (review B-5): gold only for a way at most 1.5 times the straight line; a
longer one (up to the limit of 4) is rough. A long way leans on a door the recorder may
have made (a gap, a line the walk grazed) or on fog. The way back is exempt where 80% of
it lies within 15 cells of the walk: every door on it was walked through. The doorway
case (1.96 times) is rough on purpose: its door is known only as a gap in one recorded
line, the inference most likely to be wrong, and gold must stay rarely wrong; the needle
still points at the door. Tests: `a_long_way_round_is_rough` (a recorded U-corridor,
2.2 times), the way back round a wall (gold along a 30-cell U-walk, 5 times; rough
across a 100-cell U where the way leaves the walk).

Gold keeps the icon's conditions (named and read the same twice, not against a sure
belief, within 200 cells; or unlabelled with one layout at 90%, measured areas only)
but no longer needs the graph route to be about straight: the way is the route. The
way back to a sure arrival point is gold in every area, dungeons too: walked and recorded.
Otherwise the needle is as before. The flight recorder marks such needles `way`; its
replay has no walls and does not compare them (it counts them apart).

Before and after, the wall-between case (`a_named_icon_behind_a_wall_is_reached_through_the_doorway`):
The Grelwood's icon 80 cells up, read twice, a wall across halfway with a 20-cell
doorway 70 cells to the right. Before: −87° (the layout's corridors from where the icon
puts the player: through the wall), rough, 80 cells. After: −28°, at the doorway (−30°),
157 cells; rough since fix pass 1 (no walk recorded, so most of the way lies over
unrecorded ground). Walked 30 cells right with the icon out of view: −45° to the
doorway, from the sighting. In a recorded corridor with a pillar ahead (way 88% over
recorded ground) the needle goes round the pillar, gold.

**Simulation.** The walker's world has no walls, so the 4-9% of gold needles more than 60°
off the walkable route (above) cannot be checked against this there. It would need a
walled world per layout: the corridors drawn as outlines at a walkable width (60-120
units, the recorded distances) with islands and side pockets, exits off the corridor
ends as in play; the walker's own trail; the walls the minimap would show within its
view around each step, with the recorder's faults (registration drift of a few cells,
one-cell gaps, specks, the trail over a line now and then); and the truth as the shortest
way through that world, not the graph. Then Gate A's "gold off" with and without the map.

**Limits.** The way trusts the recording: a misplaced line (16% of Clearfell part 0's
walk lies on one) makes it go round an obstacle that is not there, or not where drawn.
How often that misleads a gold needle on today's recordings needs the owner's play: a
named exit behind a wall or river, the needle checked against the walk. The arrival
point is where the player appeared, a few cells from the exit icon.

## Flight recorder (2026-11-12, day build)

Every zone visit with layout graphs leaves behind exactly what the navigator was fed,
as JSON lines in `layouts/<area>/<seed>.obs.jsonl` beside the recording
(`src-tauri/src/layout/obs.rs`). A later visit of the same instance appends (a new
`start`). Writes are buffered and reach the file at most once a second and at the
visit's end; a write error is printed once and never stops the frame loop. A visit
without a placed frame (a log catch-up at start) writes nothing. When the app starts,
files older than 180 days are deleted, then the oldest while all of them come to more
than 300 MB (about 250 hours of play); the terminal says how many went. They are the
only way to score the navigator on real play: copy those to keep out of `layouts/`.

Each line has `t` (unix ms) and `k`. After `start`, lines have a frame `f` (placed
minimap frames of the visit so far), left out where it follows: a `move` or `lost`
line starts the frame after the last line's, any other line is in the last line's
frame. A frame without a line (no movement, the same exits as last written) still
counts, so the replay can hand the navigator every `icons` call the app made.

| `k` | fields | written |
|---|---|---|
| `start` | `area`, `seed`, `from` (arrival area id, `""` = a live arrival from no zone, `null` = unknown), `scale`, `version` (package.json's), `memory`, `resumed`; with a memory `remembered` (the instance's `.nav.json` as the navigator took it, about 6 KB) and `age` (its age in seconds) | at the zone change |
| `move` | `dx`, `dy` (cells, as passed to `Navigator::moved`) | every non-zero move |
| `lost` | `seconds` | first frame of a new part |
| `icons` | `whole` (absent = false), `size` (frame cells, when it changed), `icons`: `kind` (absent = exit), `x`, `y`, `label` | when the exits differ from the last written, on every whole-window scan with an exit or while far exits are kept, for a checkpoint or waypoint not yet written in this part |
| `seen` | `exit` or `tag`, `left` | each log anchor |
| `targets` | `targets` (`label`, `exit`, `tag`, `optional`) | when the list asked for changes |
| `needle` | `target`, `degrees`, `source`, `confidence`, `variant`, `distance`; `kept` (tracking lost, the last needle stayed), `stale` (no frame for 2 s), `way` (it followed a way through the recorded map); on `way` lines `ahead` (the look-ahead point from the player, cells) and `fog` (share of the way over unrecorded ground); with ways ahead in shadow, `shadow`: what the map's way would have shown (`degrees`, `length`, `ahead`, `fog`, `gold` when it would have been gold) | when source, target or variant change or it turns more than 15° (the shadow too: it comes, goes, turns more than 15° or changes gold), else every 5 s while frames come |
| `end` | | when the zone is left |
| `gap` | `f` (always written) | before the next line after a failed write: the lines between are lost |

A write that fails (full disk, the folder not makeable) loses its lines; the next
write starts with a newline (a torn line ends there), the `start` line again if it
never reached the file, and a `gap` line that names its frame, so the lines after it
number theirs right. The replay marks such a visit **incomplete**, also when a line
it cannot read is followed by more of the visit, and leaves it out of the totals
(`a_failed_write_leaves_a_gap_and_the_visit_is_incomplete`,
`a_torn_middle_line_keeps_the_lines_after_it`).

Exits are written exactly as handed over (the navigator's input); checkpoints and
waypoints, which the navigator does not use and which don't move, only the first time
each is seen in a part.

From the synthetic visit (`a_simulated_visit_replays_as_it_was_fed`, 152 lines,
14 KB for 69 frames) and its run with losses:

```
{"t":1762945000000,"k":"start","area":"G1_2","seed":11,"from":"G1_town","scale":0.78,"version":"0.1.44","memory":false,"resumed":false}
{"t":1762945000333,"k":"move","dx":24,"dy":7}
{"t":1762945000333,"k":"icons","whole":true,"size":[100000,100000],"icons":[{"x":-24.458593,"y":16.249884,"label":"G1_town"},{"x":-338.5581,"y":-952.7067,"label":"G1_4"},{"x":746.6004,"y":-519.0581,"label":"G1_3"}]}
{"t":1762945000333,"k":"targets","targets":[{"label":"Mud Burrow","exit":"G1_3"}]}
{"t":1762945000333,"k":"needle","target":"Mud Burrow","degrees":-34.8,"source":"rough","confidence":1.0,"variant":"ClearFell_W_S_N","distance":909.0}
{"t":1762945000666,"k":"move","dx":25,"dy":4}
{"t":1762945000666,"k":"icons","icons":[{"x":-51.398148,"y":11.930754},{"x":-365.62192,"y":-957.4076},{"x":722.2359,"y":-528.5994}]}
{"t":1762945000999,"k":"lost","seconds":3.0}
{"t":1762945000999,"k":"icons","icons":[]}
{"t":1762945022977,"k":"seen","exit":"G1_3","left":true}
{"t":1762945023077,"k":"end"}
```

Size: a simulated hour (the walker moves on 99% of frames, a needle line every 1.6 s)
is 1.18 MB with the big map open 70% of the time and 1.31 MB with it always open
(`an_hour_of_play_stays_near_a_megabyte`); moves are 45%, needles 25%. Real hours
stand, fight and read more, and towns write nothing. Cost: 801 µs a frame with the
recorder against 808 without (`obs_cost`, noise ±25 µs).

### Replay

`OBS=<file or folder> cargo test --release replay_obs -- --ignored --nocapture`
replays every `.obs.jsonl` (folders recursively): each `start` makes a `Navigator`
as the app did (from the `remembered` memory at its `age`, so a resumed position is
resumed again), then every line goes to it as the app gave it (`moved`,
`lost`, the frame's exits through the recorder's own `exit_list` with its far
icons, `seen`); the needle is asked with the logged targets after every frame. Per
visit it prints:

```
== G1_2 seed Some(11) (from G1_town, v0.1.47, scale 0.780): 70 frames, 1 part(s), 23 s, walked 2064 cells (2646 units)
   top layout: 0s ClearFell_W_S_N 1.00
   truth layout (3 exits read by name): ClearFell_W_S_N 1.00; the replay's last top agrees
   left through G1_3 at (771, -515); its labelled icon was there
   needle within 45° of the exit taken (last part):
     shown by the app: gold 100% of 5, rough 98% of 63 (0 through the recorded map); right for good after 14 s, 710 cells (910 units); 0 to another target
     replayed, no map, every frame: gold 100% of 6, rough 98% of 62; right for good after 14 s, 687 cells (881 units); 1 to another target
     map (shadow), the way ahead the app did not show: no samples; never right for good
   app's needles: 13 logged, 13 agree with the current code; 0 through the recorded map (not replayed)
```

- **top layout**: each time the replay's top layout changed, seconds from the start
  and its share then.
- **truth layout**: the layouts whose exits lie as the exits read by name do (two
  or more names in one part; relative places by dead reckoning, the navigator's
  named-fit formula, summed over parts); "do not fix" below 0.9. Layouts with the
  same exit places tie (`A = B`). A name counts as the navigator trusts one: read
  the same in two whole-window scans of that exit in a row; each name's place is the
  median of those readings, so one misread scan changes nothing
  (`one_misread_scan_leaves_the_truth_as_it_was`).
- **left through**: the `seen` … `left` line, where the player stood (cells in the
  last part), and whether an exit icon (with that name) was seen within 30 cells of
  it. With no exit icon there the player left by portal, waypoint or death: the area
  left to is no exit taken, the visit is "left to … not scored" and counts under
  "unscored exits" in the last line (`a_portal_out_is_not_scored`).
- **needle within 45°**: in the part the player left from, every frame's needle to
  the exit taken against the straight line from the dead-reckoned place to the
  leaving spot (frames within 10 cells of it left out), by source: gold
  (`estimated`), rough, gray (`unconfirmed`), none. Frames aiming at another
  target are counted apart. **Right for good**: the first such frame after which
  every one was within 45°, seconds and cells walked from the start. Two rows:
  **shown by the app**, the needle the player saw on each frame (the last `needle`
  line, `kept` and `way` ones included; how many went through the recorded map);
  **replayed**, the current code's needle without the recorded map (the log holds
  no walls); and **map (shadow)**, with ways ahead in shadow, what the map's way
  would have shown on each frame (the last needle line's `shadow`), by would-be gold
  and rough: this is how ways ahead are judged on real visits before they are shown
  (`the_map_in_shadow_is_scored_apart`).
- **app's needles**: the `needle` lines (not `kept`, not `way`: the log holds no
  walls, so a needle through the recorded map is counted apart) against what the current code
  gives at the same point (same target, source, layout, within 5°; a `stale` gold
  counts as gray). A visit that started from a memory the file does not hold (files
  before v0.1.48) is left out of the agreement and counted apart. With unchanged code they agree; a difference on old visits shows
  a change of the navigator. The one other cause: the needle's switch between
  pointing at an icon and along the route has hysteresis, and the app asked about
  once a second where the replay asks every frame.
- A last line: visits, how many have a truth and an exit, the needle's shares over
  all visits, shown, replayed and map (shadow), and the agreement
  (`needles_through_the_map_are_scored_as_shown`).

The synthetic visit above is `a_simulated_visit_replays_as_it_was_fed` (the
simulator's walker on `ClearFell_W_S_N`, exits named on every third frame);
`the_flight_recorder_logs_what_the_navigator_is_fed` runs the recorder itself and
replays its file. The simulated walk ends at the exit's graph node, its icon up to
100 units off: the test walks on to the icon before leaving, as the player does.

### Layout history

When the named exits fix a visit's layout (the top layout at 0.9 or more and the
best of the navigator's named fit), checked at every save of the memory (15 s), one
line goes to `layouts/layout-history.jsonl`, once per visit and layout: `t`, `area`,
`seed`, `layout`, `share`, `named` (its share of the named fit alone), `by`
(`named`: the names alone give 0.9; `named+belief`: they favour it, and with the
rest it reaches 0.9), `version`. It is for testing later whether the layout is a
function of the seed.

## Boss lines as "near" evidence (v0.1.50)

A boss of the zone speaks in the log when the player gets to it. The checklists name the bosses (steps with
a `boss` part); the step's target says where the layout graphs put it (an exit, a tag), else a node tagged
`boss`. The first line of a boss in a visit is handed to the navigator as `Seen::NearTag` / `NearExit`:

- **Soft and wide**: each layout is weighed by how much of its belief lies within about 250 units of that
  node, with a floor of 0.3. No particle is moved and it is not an anchor (it cannot make the needle gold): a
  boss may speak before the player reaches the arena, and some zones have two nodes with the tag.
- A layout without such a node loses nothing; a tag no layout has says nothing.
- The speaker must contain the checklist's name ("Beira of the Rotten Pack" contains "Beira"); names under
  four letters never match. Lines replayed from the log's history at start do not count.
- 58 of the 74 boss steps in the checklists have a node to match (Act 1: Beira, The Devourer, Gerung,
  Areagne, Ervig, Lachlann, Draven, Asinia, Candlemass, Count Geonor, the Miller at his exit).
- The flight recorder writes it as a `seen` line with `"near":true`, and the replay feeds it the same way.

When a boss speaks, the reader also keeps one capture of the minimap corner and one of the big map's area
(`samples/boss-minimap-*.ppm`, `boss-bigmap-*.ppm`): nobody has yet looked at whether the map draws a boss
icon that could be read like the exits.

## Boss icons as evidence and as kills (third build)

*Changed by the review's fix pass: boss icons are in shadow and one dead rule is left; see
[fix pass 1](#fix-pass-1-review-ri). What follows is the first version, kept for its measurements.*

What the reader names a boss icon ([icons by their sprite](#icons-by-their-sprite-2026-11-12-third-build))
goes to the navigator like an exit icon, and a grey one can tell the checklist that a boss is dead. Nothing
of this has been seen on a real screen: no boss icon has been read yet.

### Evidence for the navigator

`layout::boss_candidates` says which of the zone's bosses (`Recorder::set_bosses`, from the checklists) an
icon may be:

- a boss's own icon (`MillerActive`, `GrimTangleBossInactive`) only in its own area, and then the boss its
  pair names (`data/icons/index.json`, `boss`), else (pair unnamed) any of the zone's bosses; never a pair
  known to be no checklist boss's (`CarnivorousPlant` in The Grelwood);
- an orange skull (a unique monster alive): any of the zone's bosses that has no icon of its own (a pair
  that names it, or its zone's one boss with a pair): the Miller showed his own icon, not a skull;
- a grey skull, a rare, anything else: none (a grey skull is a dead unique or a dead rare).

Each place (`Boss::place`: the exit to an area, a tagged node, else a node tagged `boss`) becomes
`nav::BossAt`, and the icon an `ExitIcon` with `boss` set: its candidate nodes are those places' nodes
instead of the exits. One place is an exact sighting (as an exit icon is); several are weighed as an
unlabelled exit icon is over the exits (the nearest candidate counts). A tagged place is an arena's centre:
the icon may stand 150 units from it (`BOSS_TOL`, as a log anchor at a tag), an exit place 100 (`ICON_TOL`).
An exit and a boss's icon at one place (the Miller by his exit) are two sightings. Boss icons are never
labelled, so they never make the "named exits" fit and never the label's gold. Two choices go further
than "as an unlabelled exit icon", because the simulation said so (below): a new boss sighting the belief
predicted does **not** confirm it (`BOSS_CONFIRMS`, no anchor from a boss icon), and what bosses' icons add
to a layout is kept apart (`Bank::boss`): it is in the posterior and the needle, **not** in the 90% share
gold needs (`BOSS_GOLD`). They move the particles as exit icons do (`BOSS_PARTICLES`). A needle to a boss
target points at its icon once the belief says it is the target's (`icon_is_target`).

**Simulation** (`nav_sim::what_boss_icons_add`, one seed in `cargo test`; `boss_icons_report`, two seeds,
release): the same walks, unlabelled exit icons, with and without the boss's icon (the Miller by the
Riverbank's way to the encampment; Beira at one of Clearfell's boss nodes, up to 100 units off it):

| View | Boss icons | Within 45° at 60% | Top layout true for good | Gold steps | Gold > 60° off |
| --- | --- | ---: | ---: | ---: | ---: |
| Minimap, 2 seeds | no | 36.2% | 35% | 14 | 0 |
| Minimap, 2 seeds | yes | 38.0% | 45% | 10 | 2 (20%) |
| Wide, 2 seeds | no | 51.9% | 46% | 295 | 5 (1.7%) |
| Wide, 2 seeds | yes | 61.5% | 54% | 412 | 11 (2.7%) |
| Wide, 1 seed | no | 52.8% | 50% | 173 | 1 (0.6%) |
| Wide, 1 seed | yes | 61.1% | 54% | 183 | 7 (3.8%) |

Boss icons help the needle (+9 points within 45° in the wide view) and the layout (+8–10 points), and make
gold worse: Gate A's 3% holds at two seeds, not at one. Letting them count for gold's share made it worse
still (minimap view 5 of 22 off), letting them confirm changed nothing more, keeping them off the
particles was no better (wide view 6 of 145 off, 58.5% within 45°). The test pins the numbers above so
they cannot get worse unnoticed; the simulation's labels are perfect and its boss is always where the
checklist says.

Recorder: `exit_list` hands the navigator exits and boss icons (kept from whole-window scans for 3 frames
like far exits). Flight recorder: boss icons are written with exits (`kind`, `sprite`), the `start` line
carries the zone's `bosses`, and the replay places them the same way
(`boss_icons_are_logged_and_replayed_as_fed`). Grey skulls are written once per part, as checkpoints are.

Each frame the recorder tells the reader where this part saw each boss pair (`set_icon_hints`), so a
boss's dark grey icon, which has no label and little colour, is looked for there.

Tests: `a_boss_icon_is_weighed_at_the_boss_places_not_the_exits` (Clearfell `W_E_N`, just arrived from the
encampment, Beira's icon in view: the layout gains more than from the walk; the same place read as an
exit says something else), `a_boss_icon_never_confirms_the_belief` (alone, six frames: no anchor, no
gold; after two named exits fixed the layout, a boss icon where the belief expects it confirms only with
`BOSS_CONFIRMS` switched on; the needle to Beira points at it), `a_boss_icon_counts_only_for_the_bosses_it_may_be`.

### "This boss is dead" (`Recorder::dead_bosses`)

The current zone's bosses (names as `set_bosses` gave them) the map showed dead in this visit, for the
checklist (the chair wires it). Strict: a false "dead" ticks a step not done. A boss is dead when

- **a**: its own pair's grey icon is read and the zone has one boss;
- **b**: the grey icon's pair names it;
- **c**: a grey icon (its own or a grey skull) lies within 12 cells of where this part saw it alive by its
  own icon (a or b);
- **d**: with a trusted layout (the top one at 90%), a grey skull lies at its node in at least 90% of the
  belief, and at no other boss's node in 50%.

A grey skull alone, an orange skull turning grey (an unrelated unique, as far as the map can tell), a pair
of another area or of no checklist boss count for nothing
(`the_map_names_a_boss_dead_only_when_the_grey_icon_is_its`). Places are per part: a new part starts
without them; the names stay for the visit.

Which of the 74 boss steps each rule can attribute (from `data/icons/index.json`, the checklists and the
layout graphs; c applies wherever a or b does):

| Rule | Steps | Bosses |
| --- | ---: | --- |
| b (own icon names it) | 24 | The Bloated Miller, The Devourer, Draven, Asinia, The Crowbell, The Executioner, Candlemass, Rathbreaker, Balbala, Kabala, Tor Gul, Rudja, Rootdredge, Xyclucian, Blackjaw, Zicoatl, Ignagduk, Queen of Filth, Viper Napuatzi, Doryani, Blind Beast, Captain Hartlin, Great White One, Clawcrunch |
| a (its area's pair, one boss) | 17 | Beira, Ervig, Lachlann, Jamanra (×2), Azarian, Iktab & Ekbab, Zalmarath, Mighty Silverfist, Ketzuli, Diamora, Yama the White, The Prisoner, Scourge of the Skies, Torvian, Benedictus, Tavakai |
| d only (grey skull at its node, trusted layout) | 24 | Gerung, Areagne, Count Geonor, Magma Twins, Krutog, Omniphobia, Tombgrowl, Doran, Ninth Treasure, Narag, Mektul, 2 Vultures, Mad Butcher, Forael, the interludes' Godwin & Sigbert, Oswin, Akthi & Anundr, Elzarah, Bloodbilge, Azmadi, Rakkar, the Yeti, Stormgore, Zolin & Zelina |
| none | 9 | King in the Mists, The Rust King, The Rotten Druid, Noxious Behemoth, Mimok, Vornas, Lythara, Isolde & Heldra, Wulfric & Elswyth |

"d" needs the navigator at 90% on one layout, which outside the two measured areas is rare; in practice
those 24 stay hand-ticked. Two pairs are in zones with two bosses and name neither (the Count's in Ogham
Manor, the Volcanic Warrens'): by elimination the Count's is Count Geonor's (the Gargoyle's names
Candlemass), not built.

## Scene pixels read as map (third build, I9)

Seen live on the owner's Riverbank (2026-11-12): the flight recorder of a zone without a waypoint logged about
3,900 waypoint and 470 checkpoint icons, and the map preview showed specks and streaks along the river. Both
come from the game scene showing through the translucent big map. Material: three big-map captures and the
boss capture the app kept (`samples/`), copied by the chair.

**False icons.** Pale blue water foam, a spell's glow and the checkpoint's own world object (a glowing
pillar) pass the waypoint's and checkpoint's colours. A colour-found checkpoint or waypoint is now kept only
when it looks like its own sprite (`OWN` 0.47; an icon cut by the image's edge is scored on its visible
part when that is at least 60% of it):

| | Real | False (scene) |
| --- | ---: | ---: |
| Checkpoints, own sprite score | 0.55–0.66 (Clearfell 3, Riverbank 4, one cut by the edge 0.67) | at most 0.06 |
| Waypoint | 0.67 (Clearfell) | at most 0.39 |

On the four Riverbank captures the old reader made 18 waypoints and 9 checkpoints that are not there (every pixel; 16 and 9 every second); now
none, and all four real checkpoints stay (`scene_pixels_are_no_waypoint_or_checkpoint`, fixtures
`bigmap-riverbank-foam.ppm`, `bigmap-riverbank-glow.ppm`). Clearfell's six icons are unchanged. Exits are
kept whatever they score (no false exit was seen).

**False walls.** The line colour test passes the real map lines cleanly (the map's teal river lines do not
pass) and also white-blue foam, wet stones, the character's clothes and a tutorial panel. On the frames
read from the captures, by the groups' mean colour (map lines: blue 118 or more, blue − red 30 or more,
green − red 5 or less; checked by eye on the overlays of `live_walls`): 2,467 / 3,852 / 5,570 line cells
against 260 / 1,001 / 60 scene cells kept by the reader. Shape alone does not part them: the 12-cell rule
already drops specks, and the scene groups left (12–62 cells, 4–20 cells across) are as large as real
fragments of line (47 cells, 15 x 16), and the character's clothes pass the colour as well as the lines.

What parts them is movement: a map line stays on its map cell while the player walks, the scene scrolls
several times faster, and the character (and any panel) moves with the player. So the recorder now
**holds a big-map wall cell back until it was seen from two player positions 24 cells apart** (either axis;
more than the character is tall in cells). Held cells still place frames (registration uses every cell),
but they are left out of the recording, the overview, the map's ways and the scale measure (`Live::kept`).
Corner-minimap cells are kept at once, as before: the fault was not seen there. Joining pieces
(`locate_piece`) looks for a saved piece (kept cells only) on the live map (every cell), so it works as
before; a joined piece's cells are kept. At a zone's start with the big map open nothing is kept until the
player has walked 24 cells: the way back to the arrival has no walls to follow until then (it is a straight
line over unrecorded ground, never gold).

A walk built from each capture's own cells (lines fixed on the map, the scene scrolling 5 times as fast,
what lies on the character moving with the player; 20 frames of 4 x 1 cells,
`scene_pixels_on_the_big_map_are_not_recorded`):

| Capture | Scene cells recorded, before | after | Line cells kept, of those seen |
| --- | ---: | ---: | ---: |
| Zone start (stones, the character) | 4,413 | 0 | 2,358 of 2,463 (96%) |
| By the bridge (foam, panel, character) | 13,965 | 209 | 3,521 of 3,846 (92%) |
| Middle (stones) | 1,142 | 0 | 5,286 of 5,559 (95%) |

**A new character's spawn.** The owner's new character spawned in the Riverbank after the old one had left
Clearfell: the recording said "arrived from G1_2" (the log's last zone). An arrival from a zone that is no
neighbour (no exit of the zone leads there) is now the zone's start (`""`): the navigator already started at
the layout's start node then; the recording's arrival point, the flight recorder's `from` and the scale
measure now say so too (`an_arrival_from_no_neighbour_is_the_zones_start`). A waypoint arrival from a far
zone is still taken as the start, as before.

The line cells not kept were seen only from places less than 24 cells apart (at the view's edge as the
walk ended); a longer walk keeps them. Measured on real cell shapes, but the walk is built, not recorded:
the next real visit's recording is where to check it.

## Checkpoints as soft evidence (third build, I8)

*Capped and deduplicated by the review's fix pass; see [fix pass 1](#fix-pass-1-review-ri).*

A checkpoint icon is a sighting of *some* landmark node of the layout (`Navigator::checkpoints`).

- **Landmarks**: exit nodes, and nodes whose tag names a place rather than a corridor's shape: every tag but
  `""`, `end`, `block`, `source`, `sink`, `connection`, `join`, `endcap`, `deadend`, `fill`, `blank`, `cc`,
  `dune_edge`, `water_source`/`_sink` and the like (`PLAIN`, the most common tags of all graphs sorted by
  hand). Bosses' places are tagged (`boss`, `arena`), so they count. In `ClearFell_W_E_N`: the three exits,
  two bridges, the boss node and the campsite.
- **Why**: on the one instance whose layout is known (Clearfell 2693856399 = `ClearFell_W_E_N`) the three
  checkpoints lie 50 cells from `exit_woods`, 42 from `mud_entrance` and 94 from `campsite` (next
  `bridge_small` at 111); the waypoint 61 from `mud_entrance`.
- **Weight**: each layout by how much of its belief puts the checkpoint within 120 units (94 cells) of a
  landmark: floor 0.3, counted half (`CHECKPOINT_TOL`, `_FLOOR`, `_WEIGHT`). A checkpoint once per sighting
  (40 units on the track, until a loss); one within 60 cells of an exit icon of the same frame adds nothing
  (the exit counted; Clearfell's: 28 and 42 cells). Never an anchor, never in gold's share (`Bank::checkpoints`
  is left out of `gold_posterior`), no particle moved. Only checkpoints that look like their sprite reach it
  (I9).
- **Flight recorder**: checkpoints were already written once per part; the replay feeds them on the frame of
  their line only (the app sees them on every frame, which counts once). Old logs replay as before, with their
  checkpoints now fed too.

Tests (`a_checkpoint_is_soft_evidence_of_a_landmark`): arrived from the encampment, Clearfell's middle
checkpoint in view (from the screenshot's measured places) lifts `ClearFell_W_E_N` from 0.030 to 0.041, the
top layout; seen again it adds nothing; by an exit icon nothing; far from every landmark of every layout the
posterior does not move.

Simulation (`what_checkpoints_add`, one seed; checkpoints by half the landmark nodes, 50–120 units off,
seen like exit icons):

| View | Checkpoints | Within 45° at 60% | Top layout true for good | Gold steps | Gold > 60° off |
| --- | --- | ---: | ---: | ---: | ---: |
| Minimap | no | 37.3% | 34% | 12 | 0 |
| Minimap | yes | 34.2% | 38% | 2 | 0 |
| Wide | no | 52.8% | 50% | 173 | 1 |
| Wide | yes | 51.3% | 55% | 205 | 1 |

They help the layout (+4–5 points) and add no wrong gold, but the needle at 60% is no better (−1.5 to −3
points at one seed): a checkpoint says "some landmark", and a wrong one shifts the belief as much as a right
one helps.

**The owner's Riverbank (seed 3615065120, `truth/riverbank-3615065120.md`)**, the only zone played today: one
exit, no layout fixed by names, the instance one of `Riverbed_1/3/6`. Its two real checkpoints, from the
boss capture (the player in the Miller's arena, about (1148, −77) cells from the spawn): (−74, 21) and
(−267, 275) cells from the player. Placed from each layout's start node, as measured and scaled by the
layout's own spawn-to-town length (the measured walk is 1.14–1.18 times the graphs'):

| Layout | Checkpoint by the arena | The other |
| --- | --- | --- |
| Riverbed_3 (best fit) | 75 cells from the town exit (118 scaled) | 65 from `lumberjack` (71 from `tutorial2_creekbend` scaled) |
| Riverbed_1 | 203 from `lumberjack` (51 scaled) | 210 (151 scaled) |
| Riverbed_6 | 119 from the town exit (106 scaled) | 179 (82 scaled) |

In the best-fitting layout both lie within the 94 cells the tolerance allows; the check is rough while the
Riverbank's scale is in question.

## Fix pass 1 (review RI)

The review (four blocking findings) and a new real capture: the Riverbank arena 4 minutes after the Miller's
last line (`samples/bigmap-1794482737.ppm`, fixture crop `bigmap-miller-dead.ppm`): his icon **grey and
without its place name at the same place**, and a new exit icon "Clearfell Encampment" by it. So for this
boss the table's word is now seen: the pair's live icon with its place name while he lives, the grey twin
without a name after the kill.

**Reader.**
- **The grey twin.** It scored 0.847 (`MillerInactive`) against the live twin's 0.774: the 0.15 lead between
  kinds rejected it. A pair's two icons are now weighed apart: the best must lead its own twin by `TWIN_LEAD`
  0.04 (alive: 0.78 against 0.58; grey: 0.85 against 0.77) and every other kind by 0.15. The reader finds it
  only where the recorder hints the pair's place (`the_millers_grey_icon_after_the_kill_is_found_where_he_was`):
  without a hint nothing points the matcher there (dark, no label, little colour).
- **Checkpoints** must score their sprite 0.55 (`OWN_CHECKPOINT`): the glow of Clearfell Encampment's waypoint
  stone scored 0.478 (fixture `bigmap-encampment-stone.ppm`); real checkpoints 0.55–0.67. The waypoint keeps
  0.47 (real 0.65–0.68, scene at most 0.39).
- **No candidate on the character** (`CHARACTER`: ±50 px, from 300 to 15 px above the cross at 2160 px): a grey
  boss sprite reached 0.71 on the player's hair.
- **Scale 0.53 for every icon but the exit**: every non-exit icon seen on a real screen (checkpoints, the
  waypoint, the Miller alive and grey) is drawn at 0.52–0.53; skulls are assumed so.
- **Cost.** A sure exit (own sprite ≥ 0.9) and a checkpoint or waypoint below its threshold skip the full
  ranking; colour groups are capped at 8 and compared only with sprites that have the group's main colour.
  Release, best of 4, the owner's 19 captures (`scan_samples`): big-map area (1286 x 1122) every second pixel
  1.0–1.3 → 2.7–5.8 ms (+1.7–4.6 ms; the review measured +10–19), every pixel 3.9–4.8 → 6.1–11.2 ms; the corner
  minimap (600 x 518) with no icon 0.9 → 2.4 ms every pixel (+1.5 ms), with an exit and the waypoint 0.8–0.9 →
  3.9–5.5 ms. Clearfell's whole screenshot 5.1 → 10.1 ms (was 18.8), around the player 0.27 → 2.7 ms.
- **Scan of all 19 captures** (`SAMPLES=… LIVE_AREA=G1_1 cargo test --release scan_samples -- --ignored`):
  every exit, the 6 real checkpoints and the 8 real waypoints found, nothing else named but the Miller alive
  (and the grey Miller where hinted). The highest boss/dead/rare score at any place tried, of all 108 sprites:
  0.654 (`UniqueMonsterDead` on pale stones), 0.60 on the minimap captures; on the Clearfell screenshot 0.58.
- The template cache keeps no colour planes outside tests and is cleared when the area changes.

**Recorder.**
- **Two frames.** A boss's icon, a grey one, a skull or a rare is used (fed, logged, kept for the dead rule)
  only once read with the same sprite at the same map place (4 cells) in two frames within 12
  (`confirm_named`): an icon stays where it is; scene a sprite was matched on does not.
- **Boss icons in shadow** (`BOSS_EVIDENCE` false). Only a pair's own icon could count (the orange skull never:
  an ordinary unique walks about, and no campaign boss was seen as one), read in two frames, off the particles
  (`BOSS_PARTICLES` false), out of gold's share. Still, two seeds (`boss_icons_report`, the simulated icon also
  needs two frames):

  | View | Boss icons | Within 45° at 60% | Top layout true for good | Gold | Gold > 60° off |
  | --- | --- | ---: | ---: | ---: | ---: |
  | Minimap, 2 seeds | no / yes | 36.2% / 36.4% | 35% / 41% | 14 / 6 | 0 / 2 (33%) |
  | Wide, 2 seeds | no / yes | 51.9% / 58.1% | 46% / 52% | 295 / 326 | 1.7% / 4.3% |
  | Minimap, 3 seeds | no / yes | 35.4% / 37.1% | 34% / 40% | 16 / 6 | 0 / 2 (33%) |
  | Wide, 3 seeds | no / yes | 51.8% / 59.1% | 42% / 50% | 387 / 443 | 2.1% / 4.1% |

  The review allowed half a point: they stay in shadow (read, logged, the dead rule, the reader's hints; the
  navigator gets nothing; the replay does the same).
- **One dead rule** (`dead_bosses`): this part saw the pair's live icon at a place, and now reads its grey twin
  within 12 cells and no live icon of that pair; the boss is the one the pair names, or the zone's only boss.
  No grey twin never seen alive, no grey skull, no layout rule. The real pair of captures, through the reader
  and the recorder (`the_real_miller_alive_then_dead_through_the_recorder`; wall frames
  `riverbank-frame-*.json`): the second capture is placed 35, −25 cells from the first; its first frame still
  hints from the old place, the second finds the grey twin, the third names the Miller dead. The review's
  cases (a grey skull beside the live icon, a grey twin never seen alive, a grey skull at the node) name
  nobody (`the_map_names_a_boss_dead_only_when_the_grey_icon_is_its`). It is cheap (a short list): ask it
  whenever the overlay's state is refreshed, about once a second.
- **Checkpoints exactly as the flight recorder writes them**: the recorder hands the navigator each
  checkpoint once a part (`new_checkpoints`, 4 cells, as the log's marks), so the replay, which feeds a
  checkpoint on the frame of its line, feeds exactly what the app fed. Logs now say on their `start` line that
  their checkpoints were checked against their sprite (`sprites`); older logs' checkpoints (scene ones among
  them) are not fed. On the owner's live folder the replay agrees with `main` on 400 of 400 needles.
- **Kept cells**: `Live::kept` holds the cells not held back, so the map's ways borrow it instead of a copy
  of all cells each frame. The hold's comment says what holds the character out (its small patches, not its
  height).
- The flight recorder marks only checkpoints, waypoints and grey skulls (not rares, portals, NPCs, quest icons).

**Navigator, checkpoints** (`checkpoints_take_at_most_the_cap_and_count_once`): what checkpoints take from a
layout against another is capped at 1.5 nats (`CHECKPOINT_CAP`), at most 6 count in a visit
(`CHECKPOINT_MOST`), and one the belief puts within 150 units of a counted one is that one again, also after a
tracking loss (`CHECKPOINT_SAME`). Fed 50 checkpoints, 6 count and no layout's odds move by more than 1.5.

**Fix pass 2 (the recheck).** Thresholds from all 24 of the owner's captures and the Clearfell screenshot: a
waypoint must score its sprite 0.55 like a checkpoint (real 0.653–0.680 on 15 captures; a hen on a blue rune
circle 0.496, fixture `bigmap-hen-rune.ppm`), and an **exit** must score `Entrance` 0.83 (real 0.930–0.948 on
17 captures, 0.95 on Clearfell, 0.94–0.96 drawn with the player's cross on them; lit rock 0.73 and 0.62,
`bigmap-lit-rock.ppm`; a brazier behind a label 0.05, `minimap-brazier.ppm`). An orange blob that does not
look like an exit and is no other icon is dropped. Real labels sit above the icon and did not lower a real
exit's score; a label drawn over six of its rows would (0.72), not seen on a screen. The dead rule counts a
grey twin's frame only when no live icon of its pair is read in it.
