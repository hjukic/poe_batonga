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

> **Linux test (2026-11-13, Sue's side, v0.1.113-v0.1.114):** Sue's PC: Hyprland 0.56.2
> with a Lua config, 3840x2160 at scale 2, tesseract 5.
> - Binds: a Lua-config Hyprland refuses `hyprctl keyword` ("keyword can't work with
>   non-legacy parsers. Use eval."). v0.1.113 falls back to `eval`: `hl.bind(key,
>   hl.dsp.event(name), { non_consuming = true })`, each handle kept in `_G.batonga_binds`
>   and removed with `b:remove()`, never the user's own binds. Live: 89 binds in the menus,
>   removed on zone entry; a click and a row double-click arrived, one capture each, OCR ~1 s.
> - Screen: tesseract read CREATE and DELETE but missed PLAY (gold on its ornate button), so
>   neither side saw the selection screen. v0.1.114: CREATE and DELETE side by side are
>   enough (`classifyScreen`, `focus_of`).
> - Names: not reliable yet. Gold small caps on textured rows read unstably. On one
>   screenshot it helped to read the name column at 2x the menu scale, enlarge smoothly
>   (bilinear) before cutting, and cut twice: brightest channel >= 100 reads every name,
>   >= 130-145 only the highlighted row (the selected character alone). Still: names split
>   ("AJ HBDFJ ...", "M UCKODUBRE"), "M" read as "IVI", rows lost between runs, and
>   `recognizeScreen` answers "The selected character row is not unambiguous".
> - Offline check: `BATONGA_IDENTITY_PPM=<shot.ppm> BATONGA_IDENTITY_OUT=<frames.json> cargo
>   test read_a_screenshot -- --ignored` reads a screenshot as the app does; feed
>   `frames.json` to `recognizeScreen` in Node. The screenshot stays on Sue's PC.
> - Next: screenshots with each row selected and of the creation screen, then tune the cuts,
>   the sizes and `group_lines` for split names. Until then Sue picks the character in
>   Settings.

> **v0.1.112 (2026-11-13):** character recognition on Linux/Hyprland (written on
> Windows, untested on Linux; see "Linux test" below). The capture and frame queue
> moved to the shared `identity_native.rs`; Windows behaves as in v0.1.109.
> The history has two lines numbered v0.1.108–0.1.111: zone guide changes
> (pushed from the other machine) and Windows character recognition (local), merged
> as v0.1.112.

> **v0.1.113 (2026-11-13):** names no longer depend on class titles. Creation
> recognizes the Continue/league-field layout without a class whitelist or even
> readable class text. Selection also accepts a level line without its class,
> including row double-click entry. Tests cover Assassin, Duelist, unknown/corrupt
> and absent class titles. Still needs live create → enter → return → Play and
> double-click testing; any new 1.0 screen layout must be checked in the game.
> Verified: 216 Rust tests passed (27 ignored), 76 Node tests passed (1 skipped),
> TypeScript passed. Five saved native OCR references still returned the same
> names after removing creation class titles and selection class suffixes.
> Windows release build passed; v0.1.113 was restarted with identity diagnostics.

> **v0.1.114 (Windows, 2026-11-13):** quest kills and talks tick from the game's quest
> tracker (`docs/quest-tracker.md`). From the owner's StoNaMeniVidisCurice run: the
> Bloated Miller stayed open after the kill (the log's only sign is Mortimer's line at the
> gate, 2 minutes later) and "Talk to Renly" stayed open after the reward (talks are never
> logged). The overlay now reads the tracker under the minimap about once a second while a
> step it can tick is open (GDI + Windows OCR), and 119 steps in acts 1–4 name the tracker
> states that prove them (`screen` in `data/routes`, from the game's QuestStates table in
> `data/quests/states.json`). Non-quest bosses (Beira, Gerung, Crowbell, Candlemass, …)
> are not covered. Verified: Rust tests (quest_screen and tracker), the real Windows OCR
> on the owner's screenshot. Not yet live.
>
> **Next live test (v0.1.114):** run with
> `BATONGA_QUEST_DIAGNOSTICS=artifacts/quest-tracker.jsonl` as well. New character: kill
> the Miller → his row must strike within ~2 s, before the gate; in town take Renly's
> reward → "Talk to Renly" must strike within ~3 s. Then a few Act 1 quest bosses (Devourer,
> Rust King, Draven/Asinia) and talks (Una in the Grelwood). Send the jsonl if a row stays open.

> **v0.1.115 (2026-11-13):** merge of Sue's Linux fix (origin's v0.1.113: Hyprland
> binds through Lua `eval` where a Lua config refuses `keyword`, Hyprland 0.56) with
> local v0.1.113–0.1.114. No other change; the live tests below still apply.

Current version: **0.1.115**. Rebuild before testing:
`npm run desktop:build`, then run `src-tauri/target/release/poe-batonga.exe`
with `BATONGA_IDENTITY_DIAGNOSTICS=artifacts/character-identity/runtime-creation.log`
and `BATONGA_QUEST_DIAGNOSTICS=artifacts/quest-tracker.jsonl`.

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

## Linux test — Sue (v0.1.112, not yet compiled or run on Linux)

v0.1.112 adds character recognition on Linux (Hyprland only): non-consuming
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
