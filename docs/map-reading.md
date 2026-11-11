# Offline map-reading experiment (Phase 2, step 7)

Status on 2026-11-11: an offline baseline is implemented and tested. It is not
ready for real-game navigation. Step 7 remains open.

**Live minimap reading (owner decision, 2026-11-11).** The owner chose to enable
continuous live reading of the minimap without waiting for GGG's answer, on the
condition that no game files or memory are accessed. `src-tauri/src/minimap.rs`
takes a screenshot of only the top-right minimap corner of the game's monitor about three
times a second (Linux/Hyprland via `hyprctl` and `grim`; other platforms not yet),
only while the game is the visible workspace. It finds explored walls by their
lavender line colour (about RGB 91,91,121) and the player by the orange cross,
and shows a live preview in the overlay. It can be switched off in settings ("Read
minimap"). Frames stay in memory; nothing is saved or uploaded. It sends no
input to the game and produces no arrow yet. Best results with the game's map
landscape transparency turned off. The policy notes below still describe the
unresolved question with GGG.

## Policy findings

GGG's [official developer policy](https://www.pathofexile.com/developer/docs/index#policy)
explicitly permits log reading when the user knows how the data is used. This
establishes a basis for the project's informed, read-only local log workflow.
The same policy distinguishes independent tools from prohibited game interaction
and constrains input macros. Our scope sends no game inputs. These provisions do
not specifically establish permission for continuous visual layout inference or
navigation. Keep those live features gated pending clarification of the exact
behavior under the [Terms of Use](https://www.pathofexile.com/legal/terms-of-use-and-privacy-policy).

[A clarification draft](ggg-clarification-draft.md) is prepared but has not been sent.
This review does not claim GGG has approved the app.

### Pre-game review (2026-11-11)

Rechecked the official policy, Terms of Use and the staff reply in
[About translations](https://www.pathofexile.com/forum/view-thread/3590784/filter-account-type/staff).
The staff reply declines tool-specific assurances and advises against tools
providing an in-game advantage, interacting with game files or automating play.
It does not resolve permission for our planned navigation feature.

Source inspection of the current native app finds separate desktop windows and
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
