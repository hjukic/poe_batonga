# Where we stopped — 2026-11-13

Current version: **0.1.99**: character-selection recognition fixed for every
list position, the slider and OCR misreads (details in
`docs/character-identity.md`, v0.1.95–v0.1.99). Rebuild before testing:
`npm run desktop:build`, then run `src-tauri/target/release/poe-batonga.exe`
with `BATONGA_IDENTITY_DIAGNOSTICS=artifacts/character-identity/runtime-creation.log`.

## Done and checked live (2026-11-13)

- Per-character quest progress, manual checkbox corrections, isolated legacy
  progress and zone-wide `quests completed` behavior.
- Creation: one labelled new character through Riverbank entry (v0.1.91).
- Selection, live v0.1.98: 20 of 29 clicks recognized across Standard, Runes of
  Aldur and Forbidden Rites, including the first row under the league tooltip,
  rows after scrolling with the slider, long names split by OCR
  (SchumacherGewint…), `Æ`/`0`/`l` misreads. The remaining nine were an open
  league dropdown, a selection scrolled out of view, the Hangul name
  (unsupported: only en-US OCR) and two cases fixed in v0.1.99.
- Settings → **Always show character name**, on by default.
- Tests: 37 recognizer/checklist tests, 7 identity tests; `tsc` and the
  Windows release build pass.

## Next live test — do this first (v0.1.99)

1. Selection, quickly: `AAAABATONGA` (two reads used to disagree) and clicking
   a character while the league dropdown is open. Not yet seen live.
2. **Entry**, not yet accepted live: enter once with **Play** and once by
   **double-clicking** a row (v0.1.97 treats a row double-click as Play). In the
   zone the overlay must say `Character: <name>`, not `not recognized`.
3. **Progress isolation**: tick a checkbox on A, return to the menu, enter with
   B, then A again. A's ticks must come back, B's stay separate.

Known limits: a name that OCR misreads identically in both reads can still be
remembered as a separate name (no immutable character ID). An overlay restart
clears the live identity; select the character again.

The owner's local remembered-name list was cleaned of three OCR variants;
backup `%APPDATA%/com.poebatonga.overlaytest/progress.json.bak-before-0.1.99`.

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
