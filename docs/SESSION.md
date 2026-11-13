# Where we stopped — 2026-11-13

> **v0.1.109 (Windows, 2026-11-13):** from the owner's live v0.1.108 test: Play entry
> works (DEADEYEOPLOOOOL, AAAABATONGA bound); the row double-click was detected but both
> its frames were unreadable because the whole selection screen had turned by -3.9°;
> OGLEDALOGOTOVOOOOO was read but rejected for its cut league ("HC SSF Forbidden Rites
> Lea..."). v0.1.109 reads a turned screen back on its still layout (only remembered
> names from it), takes the double-clicked row when the highlight is unclear, and no
> longer needs the league at all (any league, cut or unread; a level line shows a
> character row). Details: `docs/character-identity.md`, last section.
>
> v0.1.108 fixed the review's findings 1–9 (`docs/review-2026-11-13-character-identity.md`).
> Since v0.1.99 the Linux side also parked the radar (v0.1.104) and added Act 1 zone
> guides (v0.1.105–107; `PLAN.md`, next: Acts 2–6).

> **v0.1.110 (2026-11-13):** character recognition on Linux/Hyprland (written on
> Windows, untested on Linux; see "Linux test" below). The capture and frame queue
> moved to the shared `identity_native.rs`; Windows behaves as in v0.1.109.

Current version: **0.1.110**. Rebuild before testing:
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
- Entry with **Play** (live v0.1.108, two characters).
- Tests (v0.1.109): 206 Rust, 74 Node; `tsc` and the Windows release build pass.

## Next live test — do this first (v0.1.109)

1. **Double-click** a row to enter, a few times (also right after moving the mouse).
   In the zone the overlay must say `Character: <name>`.
2. **OGLEDALOGOTOVOOOOO** (HC SSF league): select it, enter with Play.
3. New characters (a few): create, enter, return, select each again.
4. Clicking a character while the league dropdown is open.
5. **Progress isolation**: tick a checkbox on A, return to the menu, enter with
   B, then A again. A's ticks must come back, B's stay separate.

## Linux test — Sue (v0.1.110, not yet compiled or run on Linux)

v0.1.110 adds character recognition on Linux (Hyprland only): non-consuming
`bindn` binds while in the menus, `grim` capture, `tesseract` OCR. Needs
`tesseract` with English data (`docs/overlay-test.md`, Run). First run:

1. `cargo build` on Linux; fix whatever does not compile (`identity_linux.rs`).
2. Run with `BATONGA_IDENTITY_DIAGNOSTICS=/tmp/identity.log`; in the menus,
   `hyprctl binds | grep batonga` must list the binds, in a zone none.
3. Select a character, enter with Play and with a double-click; create a new one.
   The overlay must say `Character: <name>`. Send the diagnostics log.
4. Clicks and keys must still reach the game normally.

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
