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

## Live arrow correction (2026-11-12)

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
