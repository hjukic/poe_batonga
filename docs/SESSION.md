# Where we stopped — 2026-11-13

Current version: **0.1.94** (the character-identity work, tested locally as 0.1.93, rebased onto the remote minimap v0.1.88-v0.1.93 and renumbered). Rebuild before the next live test.

## Implemented and checked

- Per-character quest progress, manual checkbox corrections, isolated legacy
  progress and zone-wide `quests completed` behavior.
- Windows creation/selection screen OCR, with entry-gated profile binding.
  One labelled new-character creation through Riverbank entry was confirmed
  live in v0.1.91, including persisted identity and profile data.
- Missing focus-changing mouse clicks and screenshot/focus shortcut invalidation
  were fixed. Tall creation-field carets are masked before OCR so they do not
  become an extra `I`; real letters are preserved.
- Settings → **Always show character name** (v0.1.92), enabled by default and
  remembered. `Character` means a bound profile; `Recognized` is pending entry.
- Latest v0.1.93 fix: Windows OCR can merge a name and league tooltip into one
  line. Separate words by position and retain conservative same-league history
  completion. Tests cover every one of five visible rows, merged tooltips and
  the owner's supplied second/third-row screenshots.
- Frontend: 24 recognizer/checklist tests pass; frontend and Windows release
  builds pass. Earlier native checks: 7 identity/input/caret tests, 6 profile
  tests and 30 tracker tests pass. Native reference/caret probes also pass.

## Next live test — do this first

The owner reported first/third-row selection failing in **v0.1.92**. The fix is
now running in **v0.1.93**, but has not yet been accepted in a live test.

1. Select the first, second and third visible characters, including tooltip
   overlap; verify the displayed name matches the selected row.
2. Use Play/Continue and confirm the name becomes `Character` in the zone.
3. Tick a checkbox on A, switch to B, then return to A. Verify separate progress
   and restoration. Observe logged/session menu invalidation during this flow.

Creation already has one successful live flow; existing-character switching
and complete progress-isolation acceptance remain outstanding. Fast entry,
other focus/cutscene paths and different UI configurations are not fully tested.
An overlay restart intentionally clears live identity; select the character
again instead of assuming the saved active name is current.

## Deferred scope

Final app name/message wording and Windows VSync compatibility stay in
`docs/scope.md` TODO. VSync Off is the owner's confirmed workaround. C-screen
recovery is not implemented. Do not expand into those tasks without selection.

## Local diagnostics and files

The launched app has opt-in diagnostics at
`artifacts/character-identity/runtime-creation.log` (ignored by Git). The trace
contains capture/OCR/result/session/binding stages. Reference probe results have
`dryRun: true` and never select a profile. Real screenshots and raw OCR dumps
remain local. `experiments/` is pre-existing unrelated untracked work; preserve it.

See `docs/character-identity.md` for implementation details, evidence and limits.
Keep game logs read-only; no game memory, injected game inputs or game-file edits.
