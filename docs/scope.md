# Scope: Path of Exile 2 leveling companion

Phase 1, step 2. Requirements recorded on 2026-11-11 from the project owner
and agreed by both contributors the same day.

## Product and platforms

Build a desktop overlay for the Path of Exile 2 campaign, initially validated on
Act 1, that helps the player level and follow campaign objectives. It must work
on Windows and Linux, including the planned Hyprland test. Endgame, trading,
build optimization, consoles and Path of Exile 1 are outside this scope.
Inventory scanning, inventory scan recording/replay and price checking are also
explicitly excluded. Quest-item progress remains in scope through validated
log/quest evidence; it does not require an inventory scanner.

Owner clarification, 2026-11-12: target **Path of Exile 2 version 1.0 and its
six-act campaign (Acts 1–6)**. Interlude routes, objectives and navigation rules
are excluded from product work. Existing Early Access observations may remain
as historical research, but Interlude zones must not be relabelled as Act 5/6.
Validate Act 1–4 assumptions against 1.0 and collect Act 5–6 rules from published
1.0 sources and observed gameplay when available. Their rules are currently
unverified; full six-act support is a release goal, not an implemented capability.

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

## Overlay behaviour (owner, 2026-11-13)

- Outside an active game zone, show the application's working name instead of
  the last saved zone and its checklist. Saved log history alone is not live presence.
- On zone entry, show that zone's checklist. An area without a route must not
  reuse another area's checklist.
- Keep completed and manually corrected objectives per character, across zone
  revisits and application restarts. A checked objective stays struck through
  until that character's progress is reset or the objective is manually reopened.
- Once all objectives selected by the optional-content setting are complete
  across that zone's visits, replace the list with the working completion message.
- **Automatic character identity is required** (owner clarification, 2026-11-13):
  creating a character or logging into an existing one must identify the actual
  character and select its own quest progress without a repeated manual choice.
  The profile selector in v0.1.88 is a temporary fallback and does not complete
  this requirement. Never substitute the last saved name, class, level, zone or
  seed for verified identity. Until identity is established, keep observations
  unassigned instead of updating another character's profile.
- **Recognition order** (owner's latest clarification, 2026-11-13): for a new
  character, read the final name from the creation screen's name field before
  the transition into the opening animation. For an existing character, read
  the selected character on the selection/Continue screen. C is a recovery
  source only, not the first check or a mandatory step in the normal flow.
  Treat the screen read as a candidate; bind progress when that entry reaches a
  game zone. Returning to the main menu clears the previous binding/candidate
  and starts recognition again. An edited/rejected name or cancelled creation
  must not create or select a quest profile.
- On the existing-character screen, a league tooltip may hide the name. Complete
  the selected row's visible prefix only from a unique known same-league name;
  ambiguous/unknown fragments remain unresolved. Preserve whether a name was
  read completely or completed from history, and never store a clipped fragment
  as a newly verified character name.
- Preserve pre-profile progress separately and assign it to a profile only by
  the owner's explicit choice. Source feasibility and the outstanding dependency:
  [character identity investigation](character-identity.md).

Implementation status: v0.1.89 adds the Windows creation/selection OCR route,
input-triggered capture and entry-gated profile binding. All four supplied image
references pass native OCR. Live transitions and return-to-menu paths still need
gameplay acceptance; C-screen recovery and other UI configurations remain open.

## Scope TODO: choose later

- [ ] **Application name:** choose the final name. Working overlay title:
  `Leveling batonge`. Branding, native window titles and package names can be
  aligned once the name is chosen.
- [ ] **Message wording:** choose final status text. The working completion
  message is `quests completed`; wording choices are separate from its behaviour.
- [ ] **Windows VSync compatibility:** the visible overlay stutters with VSync
  on the owner's setup; VSync Off is the confirmed workaround. Start with a
  bounded `noRedirectionBitmap` A/B test. Broader rendering changes remain
  deferred. See [evidence and checks](overlay-test.md#windows-overlay-and-vsync-deferred-investigation).

## Hard boundaries and feasibility

GGG compliance takes priority over every feature. No game memory access, code
injection, client/file modification, network interception, hidden map extraction,
or automated movement, combat, item pickup, quest interaction or game inputs.
Automatic checklist updates change only this app's state.

GGG's [developer policy](https://www.pathofexile.com/developer/docs/index#policy)
permits log reading when the user knows how the data is used; this supplies the
basis for our informed read-only log workflow. It does not establish permission
for our proposed continuous screen
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
conditional features. Both contributors agreed on 2026-11-11; step 2 is complete.
Implementation and GGG clarification have not been completed.

Reference checked 2026-11-11: [GGG Terms of Use for Path of Exile and Path of
Exile 2](https://www.pathofexile.com/legal/terms-of-use-and-privacy-policy),
especially clauses 7 (restrictions) and 8 (gameplay recording). The recording
clause is not treated as permission for automated gameplay analysis.
