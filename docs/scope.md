# Scope: Path of Exile 2 leveling companion

Phase 1, step 2. Requirements recorded on 2026-11-11 from the project owner;
agreement by the second contributor is still pending.

## Product and platforms

Build a desktop overlay for the Path of Exile 2 campaign, initially validated on
Act 1, that helps the player level and follow campaign objectives. It must work
on Windows and Linux, including the planned Hyprland test. Endgame, trading,
build optimization, consoles and Path of Exile 1 are outside this scope.

## Desired experience

- Show a checklist of mandatory quest items, quest objectives and progression
  points. Also support optional quest items and rewards as a separately
  identifiable, selectable part of the route; mandatory objectives take priority.
- Update the companion's checklist automatically when reliable evidence confirms
  an objective. Remember progress per character. A zone change alone does not
  prove that an item was collected or a quest completed. Unknown states stay
  unknown; manual correction is a recovery option, not the normal workflow.
- Show known layout variants and an arrow towards the next required objective,
  quest item or exit, and towards selected optional objectives.
- Determine the visible map and player position automatically, then compare
  explored map geometry and landmarks with known layouts. No Print Screen,
  screenshot uploads, manual position entry or manual layout selection should
  be needed in the target experience. Local automatic screen capture is a
  candidate input, subject to the compliance and feasibility checks below.
- Keep captured frames and recognition local; do not upload gameplay images.
  Matching is an estimate, not knowledge of the current unexplored map. Hide
  directional guidance when evidence is ambiguous, stale or unavailable.

## Hard boundaries and feasibility

GGG compliance takes priority over every feature. No game memory access, code
injection, client/file modification, network interception, hidden map extraction,
or automated movement, combat, item pickup, quest interaction or game inputs.
Automatic checklist updates change only this app's state.

The public terms do not establish permission for our proposed continuous screen
analysis, layout inference or automatic progress detection. These are desired
features, not approved capabilities. Before enabling them in live gameplay,
document a permitted basis for the exact behavior, seeking written clarification
from GGG where unclear. No claim of GGG approval is made for this project.

Phase 2 must establish what PoE 2 actually exposes through permitted inputs:
zone/log events, visible quest and item evidence, player/map visibility, layout
matching reliability, and Windows/Linux capture and overlay support. Do not
assume logs contain coordinates, seeds, inventory or quest completion. Screen
capture cannot recover map information that the game does not display.

The arrow and full automatic tracking remain conditional on both compliance
and technical evidence. If they fail either check, retain a checklist/map panel
fallback and explicitly revisit scope with both contributors; do not silently
replace the requested automatic experience with manual operation.

## Completion and reference

Step 2 is complete when both contributors agree to this page, including its
conditional features. Owner requirements recorded; second-contributor agreement
pending. Implementation and GGG clarification have not been completed.

Reference checked 2026-11-11: [GGG Terms of Use for Path of Exile and Path of
Exile 2](https://www.pathofexile.com/legal/terms-of-use-and-privacy-policy),
especially clauses 7 (restrictions) and 8 (gameplay recording). The recording
clause is not treated as permission for automated gameplay analysis.
