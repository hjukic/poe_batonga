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


## Flight recorder (2026-11-12, day build)

Every zone visit with layout graphs leaves behind exactly what the navigator was fed,
as JSON lines in `layouts/<area>/<seed>.obs.jsonl` beside the recording
(`src-tauri/src/layout/obs.rs`). A later visit of the same instance appends (a new
`start`). Writes are buffered and reach the file at most once a second and at the
visit's end; a write error is printed once and never stops the frame loop. A visit
without a placed frame (a log catch-up at start) writes nothing. Files older than
30 days are deleted when the app starts.

Each line has `t` (unix ms) and `k`. After `start`, lines have a frame `f` (placed
minimap frames of the visit so far), left out where it follows: a `move` or `lost`
line starts the frame after the last line's, any other line is in the last line's
frame. A frame without a line (no movement, the same exits as last written) still
counts, so the replay can hand the navigator every `icons` call the app made.

| `k` | fields | written |
|---|---|---|
| `start` | `area`, `seed`, `from` (arrival area id, `""` = a live arrival from no zone, `null` = unknown), `scale`, `version` (package.json's), `memory`, `resumed` | at the zone change |
| `move` | `dx`, `dy` (cells, as passed to `Navigator::moved`) | every non-zero move |
| `lost` | `seconds` | first frame of a new part |
| `icons` | `whole` (absent = false), `size` (frame cells, when it changed), `icons`: `kind` (absent = exit), `x`, `y`, `label` | when the exits differ from the last written, on every whole-window scan with an exit or while far exits are kept, for a checkpoint or waypoint not yet written in this part |
| `seen` | `exit` or `tag`, `left` | each log anchor |
| `targets` | `targets` (`label`, `exit`, `tag`, `optional`) | when the list asked for changes |
| `needle` | `target`, `degrees`, `source`, `confidence`, `variant`, `distance`; `kept` (tracking lost, the last needle stayed), `stale` (no frame for 2 s) | when source, target or variant change or it turns more than 15°, else every 5 s while frames come |
| `end` | | when the zone is left |

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
replays every `.obs.jsonl` (folders recursively): each `start` makes a fresh
`Navigator` (no memory), then every line goes to it as the app gave it (`moved`,
`lost`, the frame's exits through the recorder's own `exit_list` with its far
icons, `seen`); the needle is asked with the logged targets after every frame. Per
visit it prints:

```
== G1_2 seed Some(11) (from G1_town, v0.1.44, scale 0.780): 69 frames, 1 part(s), 23 s, walked 1979 cells (2538 units)
   top layout: 0s ClearFell_W_S_N 1.00
   truth layout (3 exits read by name): ClearFell_W_S_N 1.00; the replay's last top agrees
   left through G1_3 at (748, -434); no exit icon seen there (a portal or waypoint is possible)
   needle within 45° of the exit taken (last part): gold 80% of 5, rough 98% of 62; 1 samples to another target
   never right for good
   app's needles: 12 logged, 12 agree with the current code
```

- **top layout**: each time the replay's top layout changed, seconds from the start
  and its share then.
- **truth layout**: the layouts whose exits lie as the exits read by name do (two
  or more names in one part; relative places by dead reckoning, the navigator's
  named-fit formula, summed over parts); "do not fix" below 0.9. Layouts with the
  same exit places tie (`A = B`).
- **left through**: the `seen` … `left` line, where the player stood (cells in the
  last part), and whether an exit icon (with that name) was seen within 30 cells of
  it; leaving by portal or waypoint shows as "no exit icon".
- **needle within 45°**: in the part the player left from, every frame's needle to
  the exit taken against the straight line from the dead-reckoned place to the
  leaving spot (frames within 10 cells of it left out), by source: gold
  (`estimated`), rough, gray (`unconfirmed`), none. Frames aiming at another
  target are counted apart. **Right for good**: the first such frame after which
  every one was within 45°, seconds and cells walked from the start.
- **app's needles**: the `needle` lines (not `kept`) against what the current code
  gives at the same point (same target, source, layout, within 5°; a `stale` gold
  counts as gray). With unchanged code they agree; a difference on old visits shows
  a change of the navigator. The one other cause: the needle's switch between
  pointing at an icon and along the route has hysteresis, and the app asked about
  once a second where the replay asks every frame.
- A last line: visits, how many have a truth and an exit, the needle's shares over
  all visits and the agreement.

The synthetic visit above is `a_simulated_visit_replays_as_it_was_fed` (the
simulator's walker on `ClearFell_W_S_N`, exits named on every third frame);
`the_flight_recorder_logs_what_the_navigator_is_fed` runs the recorder itself and
replays its file. In the simulation the last steps count as wrong: the walker ends
at the exit's graph node, its icon lies up to 100 units off.

### Layout history

When the named exits fix a visit's layout (the top layout at 0.9 or more and the
best of the navigator's named fit), checked at every save of the memory (15 s), one
line goes to `layouts/layout-history.jsonl`, once per visit and layout: `t`, `area`,
`seed`, `layout`, `share`, `named` (its share of the named fit alone), `by`
(`named`: the names alone give 0.9; `named+belief`: they favour it, and with the
rest it reaches 0.9), `version`. It is for testing later whether the layout is a
function of the seed.
