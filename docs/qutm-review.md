# Qutm reference review

Reviewed 2026-11-11 by reading source; the application was not installed or run.
Pinned revision: [Vyary/qutm d18b4e6](https://github.com/Vyary/qutm/tree/d18b4e6abee3f87c90342222918c6b08422967c5).
This is implementation evidence, not a sample of our current PoE 2 logs.

## What the log parser uses

[File.ts](https://github.com/Vyary/qutm/blob/d18b4e6abee3f87c90342222918c6b08422967c5/src/lib/File.ts)
consumes batches emitted by a Rust file tailer:

| Signal | Behavior in Qutm | Implication for our app |
| --- | --- | --- |
| `level <number> area "<id>"` | Extracts internal zone ID and zone level; changes guide/layout zone | Concrete candidate pattern to validate in our samples |
| `<name> (<class>) is now level <number>` | Learns name if unset; updates matching character's level/class | Level-up evidence, not reliable login/character-switch identification |
| Configured dialogue/reward text | Exact matches set a flag under the current zone; character name can be normalized | Some progress may be observable without screen recognition |
| Startup, focus, closing and `Set Source` messages | Changes overlay visibility | Candidate visibility hints; verify availability and behavior on both OSes |

The parser does not extract player coordinates or a map seed. This does not prove
those fields never exist in the game log; it establishes only what this parser uses.

[guide.json](https://github.com/Vyary/qutm/blob/d18b4e6abee3f87c90342222918c6b08422967c5/src/assets/guide.json)
contains zone-specific dialogue and reward conditions, including passive-point and
Spirit reward messages. These are useful hypotheses for labelled log collection.
Do not equate a narrative line with item pickup or quest reward without testing the
trigger, speaker/channel, repeatability and exact game version/language.

[Content.ts](https://github.com/Vyary/qutm/blob/d18b4e6abee3f87c90342222918c6b08422967c5/src/components/widgets/zone/state/Content.ts)
selects guide groups by previous zone and visited-zone prerequisites, then shows or
hides tasks based on flags or zone history. This is conditional guide filtering;
it is not a complete inventory/quest-state feed or an audited per-objective ledger.

## Layout and platform findings

[LayoutsState.ts](https://github.com/Vyary/qutm/blob/d18b4e6abee3f87c90342222918c6b08422967c5/src/components/widgets/layout/LayoutsState.ts)
stores editable diagrams with normalized icon positions and connecting lines.
[Layouts.tsx](https://github.com/Vyary/qutm/blob/d18b4e6abee3f87c90342222918c6b08422967c5/src/components/widgets/layout/Layouts.tsx)
lets users select/cycle variants. In this path, diagram coordinates are authored
positions, not observations of the player's live location. It does not implement
our automatic map alignment and directional arrow.

The project uses Tauri 2, Rust, SolidJS and TypeScript. Its window configuration
requests transparency and always-on-top. The README describes Windows as primary
and invites compatibility reports for Linux/macOS. This supports using Tauri as a
prototype candidate, but is not evidence that our Hyprland requirements pass.

## Lessons for implementation

Source inspection found pitfalls we should avoid:

- The callback in `File.ts` uses `return` inside the loop over a batch. Recognizing
  a zone/focus/level event can skip every later line in that batch. Our parser must
  process every line independently, preserving order.
- The tailer in
  [lib.rs](https://github.com/Vyary/qutm/blob/d18b4e6abee3f87c90342222918c6b08422967c5/src-tauri/src/lib.rs)
  starts at EOF and retains a partial line, but does not implement rotation,
  truncation recovery or cancellation. Starting after a reward event cannot
  reconstruct it from this tail alone.
- Character and tracker state are saved under single `char`/`tracker` store keys.
  Our character/session isolation needs a stronger design.
- Broad substring checks and exact English quotes need structured parsing,
  channel validation, localization/version handling and duplicate protection.

The README's claim that the app only reads Client.txt does not describe all the
reviewed functionality: `lib.rs` also sends copy keystrokes and mouse actions;
[Inventory.tsx](https://github.com/Vyary/qutm/blob/d18b4e6abee3f87c90342222918c6b08422967c5/src/components/widgets/inventory/Inventory.tsx)
invokes those commands during scan recording/replay. Those mechanisms conflict
with our no-game-input scope and should not be adopted. A repository's compliance
claim is not confirmation from GGG.

Its layout diagrams were later imported as `data/layouts/qutm/layouts.json`.

## Recommendation and next evidence

Keep our own small core and use Qutm as a reference, rather than importing its
full app. Prioritize log-driven zone tracking and validated reward/dialogue
evidence before OCR for the checklist. Keep screen-based navigation separate.
The owner accepted this direction and explicitly excluded inventory scanning;
see decisions D12-D13. This does not change the pending log-validation work.

For step 4, collect a labelled sequence covering zone entry, level-up, quest item
pickup, item use/turn-in, reward claim and character switch. Compare before/after
lines and record which exact event each message proves. Include multiple events
in one batch, repeated dialogue and a restart after an objective is completed.
Then document verified fields and objective coverage. Step 4 remains incomplete
until actual logs have been inspected; this review cannot establish full automatic
completion or current-patch reliability.
