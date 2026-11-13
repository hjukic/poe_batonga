# Automatic character identity

Owner clarification, 2026-11-13: both a newly created character and a login with
an existing one must select the correct quest history automatically. Repeated
manual profile selection is not the requested product. v0.1.88 implements
separate storage and an explicit selector, not automatic recognition.

## Current implementation: v0.1.89

Windows now has an input-triggered screen recognizer and automatic profile
binding. The shared recognizer in `src/character-screen.mjs` is used by both
the diagnostic CLI and the application. Native code captures the foreground
PoE 2 window and uses Windows.Media.Ocr in process, with no PowerShell startup
or network OCR. Images remain in memory; only names, league, provenance and
this application's progress are saved. Raw key/text input is not logged.

Passive OS hooks observe real keyboard/mouse input and always forward it.
Menu edits and selection clicks are debounced to read their rendered result;
Play/Continue or Enter requests the pre-transition frame. Capture and OCR run
outside the hook callback. An immediately preceding name frame may finish OCR
after activation; any intervening edit, a newer request or session invalidation
rejects an older result. Frames are queued with a small bound. Identity OCR
does not run from a screen-reading timer or on ordinary bound gameplay input.
Escape and its following menu click provide a probe for return to selection.

A name is only pending until activation and actual zone entry. The selected
profile then restores its own ticks, and pending observations remain local to
that login. Complete creation names are remembered after entry; complete list
names may be learned without selecting those profiles. Inferred tooltip names
are never learned as new complete observations. The saved `activeSource`
distinguishes manual, complete-screen and history-prefix bindings.

Logged login/disconnect boundaries, game closure and recognized menu returns
clear identity. An application restart keeps all profiles but clears the live
binding, since the character might have changed while the overlay was closed.
Start the overlay before character selection/creation; if recognition is missed,
manual selection remains available. C-screen recovery needs a representative
screen and is not implemented. Linux retains manual selection.

Validation on 2026-11-13: the **native** OCR path plus shared recognizer returns
the expected candidate for all four owner-provided images, including the
tooltip case after learning its complete predecessor. The screenshots and
OCR dumps remain local/ignored. Frontend and Rust checks also cover ambiguity,
case changes, activation/entry gating, stale frames and isolated quest history.
Live hook/capture timing, fast create/Play, cancelled/rejected creation,
Escape-menu returns (including paths without a log boundary), double-click
entry and other aspect ratios/UI scales still need real gameplay acceptance.
No immutable character ID is available: rename/name reuse remains a limitation
of these name-based profiles. This implements the requested route for testing;
it does not claim all live paths have been accepted.

## Creation failure and diagnostic build: v0.1.90

The owner's first live v0.1.89 test did not identify either creation or Continue.
The in-zone panel stayed at the waiting message. The menu had shown no status
because the frontend rendered identity messages only for an unbound live zone;
v0.1.90 also renders them while idle/in menus. This display fix does not by
itself establish why the live name read failed.

Opt-in `BATONGA_IDENTITY_DIAGNOSTICS=<local .log path>` records a bounded local
JSON-lines trace of initialization, hook installation, consumed input requests,
capture failures/dimensions, OCR timing/menu anchors, selected name-field text,
frontend results, stale-result rejection, session/zone changes and binding.
It does not log raw keys, arbitrary full-screen OCR/chat or screenshots. The
file rotates at 2 MB. The current debug session uses an ignored artifact log.

`BATONGA_IDENTITY_REFERENCE=<local P6 PPM path>` runs a one-shot diagnostic after
the actual frontend listener registers. Native OCR sends the reference through
the webview event and IPC path with `dryRun: true`; its result is logged without
learning names, selecting a profile or changing progress. This runtime probe
successfully returned the expected creation name/league from the supplied
reference in v0.1.90. Both OS hooks reported successful installation. A labelled
live creation attempt is still needed to locate the failure in the remaining
input/capture/transition path.

The subsequent labelled live attempt read the creation name correctly, but its
zone-entry trace still had `submitted: false`. A click departing the game had
advanced the input sequence, while no activation request was recorded for the
later Continue action. The foreground-only mouse filter could ignore the very
click that focuses the game and activates its button. v0.1.91 instead checks the
root window under the real click, ignores clicks outside the game, and briefly
waits for OS focus metadata before the one requested capture. Screenshot and
focus-switch modifier shortcuts no longer count as name edits.

Live field reads also sometimes appended `I` while the input caret was visible.
v0.1.91 masks only narrow pixel bars that rise above the text and are taller
than real glyphs, then repeats both full/crop OCR on those pixels. It never
strips an OCR string's final character. Tests preserve a real letter I and
read the supplied Druid and Witch creation references correctly after adding
a synthetic tall caret.

The repeated v0.1.91 live creation now passed: real typing produced caret-mask
events and the correct final name, Continue was recorded as activation, and the
first Riverbank entry bound and saved the profile with source `screen-complete`.
The progress file contains that active profile and `G1_1`, with legacy progress
preserved. This validates one labelled creation-to-zone flow; the specific
focus-changing Continue path, other entry/cutscene inputs and existing-character
switch/progress isolation still need live acceptance. Raw names remain in the
ignored runtime trace, not these committed examples.

v0.1.93 follow-up: selection OCR sometimes merges a title and its league tooltip
into one line, dropping that row from name selection. The recognizer now separates
the title using word positions and detects tooltip words within merged lines.
Potentially clipped titles still require unique same-league history. Regression
tests exercise every one of five visible rows, merged tooltips, ambiguity and
the supplied second/third-row captures. Live acceptance of the updated build is
still pending.

## Evidence checked on 2026-11-13

- Read the local Client.txt login/zone records from the recent sessions, and
  searched relevant configuration keys without copying account names into this
  repository. The observed login sequence contains login-server connections and
  zone generation, but no character-selection name. The configuration includes
  an account name and league preferences, not a verified active-character name.
- Level-up/death/reward messages can contain names, but are not emitted on every
  login. A named message also needs evidence that it describes the local player.
  Waiting for a level-up does not satisfy detection on login or creation.
- A subsequent labelled new-character test at 01:09–01:10 also contains no
  character name/ID from login through zone entry, item pickup and checkpoint
  activation. The `[Client …]` number was verified as the OS process ID, not a
  character ID. See [the test findings](log-findings.md#labelled-new-character-test-2026-11-13-01090110-local).
- The official [Account Characters API](https://www.pathofexile.com/developer/docs/reference#characters)
  documents `GET /character/poe2`, requiring `account:characters`. Character
  objects have a unique `id`, a `name` and an optional `current` flag. The listed
  flag alone does not establish its freshness or behaviour across live character
  switches; validate those before trusting it to bind progress.
- The official [authorization guide](https://www.pathofexile.com/developer/docs/authorization)
  requires a registered public OAuth client and Authorization Code with PKCE
  for desktop applications. No client configuration or API adapter exists in
  this repository. Do not use someone else's client ID or browser session cookie.
- GGG's [registration page](https://www.pathofexile.com/developer/docs/index#gettingstarted)
  currently says it cannot process new applications. An existing registered
  client for this project would permit testing the API route; its availability
  is an outstanding question, not an assumed prerequisite already met.

## Acceptance before enabling automatic binding

1. Establish a verified identity source available on every creation/login,
   including switching between two characters of the same class and level in
   the same zone. A successful zone match alone is insufficient.
2. If using the API, validate `current` and cache/update timing against labelled
   creation, login, logout, reconnect and character-switch sequences. A stale
   response from an earlier session must never bind the new one. Respect the
   returned rate limits; network errors leave identity unresolved.
3. Persist by account/realm plus immutable character ID when available. A
   rename should retain progress; a deleted character's reused name must not
   inherit the old character's quests. Existing name-only profiles require
   explicit migration rather than an automatic guess.
4. Keep pending observations scoped to the login session. Bind them only after
   that session's identity is confirmed, and preserve separate histories after
   application restart and zone revisits.

At the initial investigation, screen recognition was a separate possible source.
It needs representative selection/creation screens, a trustworthy
selection-to-entry link and a permitted trigger. The current
[GGG overlay rules](https://www.pathofexile.com/developer/docs/index#policy)
limit screen/clipboard reads to a manual key press. Do not introduce periodic
OCR, simulated game inputs, memory access or packet interception as an implicit
solution to the missing login identity.

The initial investigation did not complete automatic recognition. The current
implementation and remaining acceptance checks are described above.

## Repository review and the C-screen proposal (2026-11-13)

Public source was cloned to temporary folders and read, not executed. Reviewed
revisions and relevant behaviour:

| Project / source | What it actually does | Limit for our requirement |
| --- | --- | --- |
| [Qutm, `File.ts`](https://github.com/Vyary/qutm/blob/d18b4e6abee3f87c90342222918c6b08422967c5/src/lib/File.ts#L75) | Extracts name/class/level from `is now level`; fills an empty character name. Settings also accept a typed name. | Does not establish the character at each login. The existing stored name can remain selected. |
| [poe2-upgrade-checker, route parser](https://github.com/ets-public/poe2-upgrade-checker/blob/ed897954ab11f49d851eb807ee6efa89f4638a1b/src/core/route.js#L55) and [profile switching](https://github.com/ets-public/poe2-upgrade-checker/blob/ed897954ab11f49d851eb807ee6efa89f4638a1b/src/features/gear.js#L208) | Reads named level-up/reward/death lines and login boundaries; switches linked profiles from evidence, with area/level heuristics. Its code explicitly notes that a party member's death has the same log format. | A useful reference for event parsing/session invalidation, but area/level guesses cannot distinguish two similar characters reliably at login. |
| [PoENavi OCR helper](https://github.com/buri34/poenavi/blob/e90a4e6492544b8337c88a0cf00c57fe811c2352/tools/ExpeditionWindowsOcr/Program.cs) | Uses local `Windows.Media.Ocr` and `RecognizeAsync` on image data for other PoE2 OCR features. | Evidence that a local Windows OCR adapter is practical, not an implementation of character-name recognition. |
| [Overwolf PoE2 events](https://dev.overwolf.com/ow-electron/live-game-data-gep/supported-games/path-of-exile-2/#me) | Documents `me.character_name` for the local player, and character-sheet open state. [dIVa's own docs](https://myla-llc.github.io/tutorials/docs/dIVa/class-detection/) describe its Overwolf-based PoE2 detection. | A distinct provider/integration route, not information supplied by Client.txt. It is not installed or integrated here; platform, distribution and compatibility work would need separate assessment. |

The OAuth path is therefore not the only conceivable source. The missing piece
for our existing standalone overlay is a verified identity adapter, rather than
an assumption that all automatic tools get their names from Client.txt.

### Earlier C-screen probe

The owner suggested manually opening the character sheet with C. First verify
that this exact panel in the current client displays the actual character name,
and locate it. A class label, account name or build name is not enough.

If the name is present:

1. Observe the user's real, unmodified character-sheet key while PoE2 has focus;
   let it reach the game normally. Do not synthesize or consume game input.
2. Take one delayed screenshot per key press, after the sheet opens. Validate
   panel landmarks before reading the name crop; pressing C to close the sheet,
   typing C into chat or a rebinding must not select a profile accidentally.
3. Run OCR locally on that frame. Microsoft's
   [OCR API](https://learn.microsoft.com/en-us/uwp/api/windows.media.ocr.ocrengine)
   returns words and positions, but does not expose a generic per-word
   confidence score. Validate exact character-name reads on representative
   names (underscores, similar letters/digits, Unicode) rather than inventing a
   confidence threshold or silently fuzzy-matching the nearest saved profile.
4. Associate the result only with the request's still-current login session and
   game window. Discard a result that returns after logout or character switch.
   Then select/create that name's local profile and attach that session's pending
   observations. Do not carry the binding across a new login without new evidence.
5. Compare two characters in the same zone and test existing/new profiles,
   closing the sheet, chat, Alt+Tab, different UI scales and app restart.

The capture trigger must remain one per manual key press under the current GGG
overlay rules. OCR can process multiple crops of that same frame without adding
periodic captures. This is also a small, bounded workload, but its actual latency
and effect on gameplay are not yet measured. Linux needs its own OCR adapter;
Windows OCR is not a cross-platform implementation.

Opening C once per login would remove typing/selecting the character manually,
but it is not recognition before that screen has been opened. A name read from
pixels also does not provide an immutable ID: renames/reused names remain a
separate identity/migration limitation.

### Current owner workflow: creation/Continue first

This supersedes C as the first check. The owner initially proposed C first,
then explicitly changed the priority on 2026-11-13:

1. **New character:** recognize the creation screen and read its name field.
   The final accepted candidate must come from the field before the transition
   into the opening animation. Replace earlier candidates after edits, including
   backspace/paste/Unicode input. Do not infer the final name from the first or
   penultimate OCR result, keyboard text, an animation frame or a name rejected
   as unavailable.
2. **Existing character:** recognize the character-selection/Continue screen
   and read the character actually selected for entry. A list containing several
   names is not enough; the selected row/portrait must be identified. Selection
   changes invalidate the preceding candidate.
3. **Entry:** retain that candidate through the animation/loading screen and
   bind the session's pending progress after confirmed entry into a zone. A
   delayed OCR result from another screen/session cannot replace the identity.
4. **Return to main menu:** invalidate the active binding, outstanding OCR
   requests and candidate, while preserving every saved profile. Resume creation
   or Continue recognition for the next character, even within the same game
   process. Zone loading or Alt+Tab alone is not a main-menu transition.
5. **Recovery only:** if the entry candidate is absent, ambiguous or stale, leave
   identity unresolved and use a user-opened C sheet as a fallback. Do not make
   C a required first step in the normal flow.

Creation cancellation and failed name validation must never bind a profile.
The recognizer needs reference captures of the creation name field, the selected
existing character, and the main menu/transition states. Capturing the correct
last pre-transition frame and distinguishing those states are unverified; this
document records the required behaviour, not a completed implementation.

Use the manual-input-triggered capture constraint described above. A rolling
timer-based screen capture is not implicitly authorized as the implementation
of "read the last frame". The exact event/capture timing must be validated so
normal typing/Continue input is not delayed or intercepted.

### Probe status

- Local Windows OCR recognizers `en-US` and `hr-HR` are installed (queried through
  the OS API).
- The live-screen computer-use helper could not connect, including after its
  documented recovery: `Computer Use native pipe is unavailable` (OS error 2).
  No character-screen screenshot or OCR result was obtained in this review.
- The owner subsequently supplied creation and single-character selection
  screenshots. Their offline OCR probe is recorded below. Main-menu/transition,
  multi-character selection and C recovery references are still unverified.

## Supplied-screen OCR probe (2026-11-13)

The owner supplied two PNGs (2559×1439): an existing character on the Play
screen, and a different name in the creation field above Continue. Both were
read locally with Windows OCR, using both the full image and focused crops.

Results:

- Both names were transcribed with the expected letters. The focused reads
  capitalized both names; one full-image read used mixed case. Compare names
  without case differences and retain any existing profile's display spelling.
- The league was also readable on both screens.
- Full-image menu anchors distinguish selection (`PLAY`, `CREATE`, `DELETE`)
  from creation (`CONTINUE` with the central league field).
- Crops agreed with the full-image name after case normalization. No account
  API, game log, memory access or network OCR was involved.

The reproducible offline prototype is:

```sh
node scripts/read-character-screen.mjs <screenshot.png>
node --test scripts/character-screen.test.mjs
```

`probe-character-screen.ps1` provides local OCR words and pixel rectangles.
`read-character-screen.mjs` validates screen anchors and name-field positions,
then returns a **candidate**, not a confirmed active character. It invokes OCR
on the provided file only and never captures or controls the game. The initial
seven tests covered both screens, an empty field, multiple names, full/crop
disagreement, unrecognized screens and case normalization. Both supplied images
produced the correct screen kind, normalized name and league. The expanded
multi-character/tooltip tests below supersede the initial multiple-name rejection.

The regions are calibrated to these references: selection crop now covers the
visible list at `(0.788, 0.215, 0.202, 0.510)` and creation crop is `(0.397, 0.723,
0.210, 0.110)` as x/y/width/height fractions. Other UI scales/aspect ratios remain
unmeasured. The first selection image has only one character; the subsequent
three-character references establish the highlight measurements below.

Remaining integration work is the real manual-input capture trigger, retention
of the final pre-animation frame and session-safe binding on zone entry/main-menu
return. Selected-row discrimination is tested on the supplied images, not yet
live gameplay. OCR processing may finish after the
transition if its captured frame and session are still valid; capture timing
and live latency cannot be established from static screenshots. The standalone
PowerShell probe's process startup is diagnostic overhead, not a design for
spawning a new OCR process on every key press. No production listener or
automatic binding was enabled by this offline test.

## Multiple characters and a league tooltip (2026-11-13)

The owner supplied two more 2559×1439 selection images: three visible character
rows with the middle row selected, and the same selection partly covered by the
league tooltip. The offline prototype correctly identifies that selected row on
both. The first image reads the complete name and records it in a local cache;
the second restores that name using the visible prefix and that cache.

Selected-row evidence is measured from gold glyph pixels, excluding neutral
white tooltip text. The selected name has mean gold brightness about 159–160
and a bright-glyph fraction of 0.67–0.69; the other names are about 103 and 0.
The prototype requires a clear lead over the other rows. No clear lead or two
bright rows means no selected candidate. Mouse hover on a different row and
other graphics/UI configurations still need real references before broad use.

History completion rules:

- Only use the selected row's text. A different complete name in the list is
  never a substitute for the obscured selected one.
- Detect another text block overlapping the name-title band geometrically;
  do not depend on OCR spelling the league tooltip correctly.
- Require agreement between full-frame and focused reads. For a known tooltip
  edge, ignore at most the final OCR glyph, which can be half hidden/misread.
  Do not allow general edit-distance correction or arbitrary substring matches.
- Require at least six visible prefix characters and at least half the known
  name, one unique case-insensitive prefix match, and the same league. Similar
  known names, insufficient text or absent history leave identity unresolved.
- Cache only complete, unobstructed names whose two reads agree. Never add the
  truncated read or a history-inferred completion as a new complete observation.
- A completion remains a candidate with provenance `history-prefix`, not an
  immutable ID or independently confirmed login. Unknown characters can share
  prefixes with remembered ones; live binding must retain that distinction.

Reproduce the sequential image test with an explicitly selected local cache:

```sh
node scripts/read-character-screen.mjs <unobstructed.png> --history artifacts/character-identity/screen-test-history.log
node scripts/read-character-screen.mjs <tooltip.png> --history artifacts/character-identity/screen-test-history.log
node --test scripts/character-screen.test.mjs
```

All 18 tests pass, including selected first/middle/last rows, two highlighted
rows, tooltip recovery, corrupted tooltip OCR, unknown/ambiguous/wrong-league
history, short prefixes, mismatched frames and case normalization. Unit-test
names are synthetic; supplied image files and the name cache stay local. The
`.log` cache is ignored by Git. This expands the offline recognizer; it does not
yet add live game capture or write quest progress from inferred names.

v0.1.95 follow-up: the owner's live v0.1.92 trace showed why the **first** row
failed. The league tooltip opens under the mouse, over the clicked title, and
OCR turned the half-hidden last glyph into a symbol (`FOULWICTHCOVEN&`). The
row was dropped as "not a name", leaving the selection ambiguous. A title with
one or two trailing symbols is now kept as a clipped prefix: it resolves only
by unique same-league history (`history-prefix`) and is never learned as a
complete name. Second/third rows already used the same history path. A test
covers the symbol edge on rows 1–3; live acceptance is pending.

v0.1.96, from the owner's live v0.1.95 test (Forbidden Rites list and a
five-row Standard list with a slider):

- First row still failed: the tooltip text can glue onto the title word
  (`NAME&orbidden`). A symbol inside or after a title now yields a clipped
  prefix (history only); a leading icon symbol (`•NAME`, `i' NAME`) is dropped.
- OCR reads the small-caps title font inconsistently (`EvoTllZV0Ll`,
  `EvoTllZVOL1`, `FÉMBOY`/`FEMBOY`). Comparisons now use `matchKey` (diacritics
  removed, 0→o, 1/l/|→i); the displayed/saved spelling stays the known one.
- A tooltip only hides a title when its text starts within 1.5% of the frame
  width of the title's right edge; one opening well to the right of a short
  title no longer forces the history-only path.
- A row without measurable gold (half scrolled, misread) no longer makes the
  whole list ambiguous; the highlight must still lead every measured row.
- Diagnostics now log each list line's width, word count and gold style for
  both the full frame and the focused crop.

v0.1.97, from the owner's live v0.1.96 test (Standard list scrolled with the
slider, then entering Clearfell):

- Entering by **double-clicking** a row lost the name: the second click cleared
  the pending candidate and its own frame was already the loading screen. A
  second click within 500 ms/8 px on the list now counts as activation, and the
  first click's frame is captured before it and stays current
  (`superseding_input`, `Recognition::current`).
- A long title under the tooltip was split by OCR into pieces
  (`SCH U MACH ERG EWINTWStandard`). Pieces closer than 0.8 glyph height join
  into one clipped title. Clipped reads resolve from their **common** prefix,
  or when the remainder is exactly the start of `<league> League`.
- `Æ`/`Œ` read for `E` folds in `matchKey`; without a remembered spelling the
  plainer read is kept.
- Not supported: Hangul/CJK names (only the en-US OCR language is installed).

v0.1.98, from the owner's live v0.1.97 test across all leagues:

- Joining OCR pieces at up to 0.8 glyph height also glued the tooltip's
  `Standard` onto `MOBILEDEPROVISIONINC` (G misread as C), so no prefix
  matched. Pieces now join only within 0.5 glyph height, and a clipped read
  falls back to the first OCR piece of each frame.
- With the tooltip over the first row the focused crop sometimes drops that
  title line. The crop now starts at y 0.205 instead of 0.215. A clipped title
  missing from the crop resolves from the full frame alone, history only; a
  complete name still needs both reads. A lone unhighlighted row in a read is
  no longer taken as the selection.

v0.1.99, from the owner's live v0.1.98 run (20 of 29 selection clicks
recognized; the rest were an open league dropdown, a selection scrolled out of
view or the unsupported Hangul name, plus the two cases below):

- An open league dropdown lights the hovered item; items have under 250 gold
  pixels against 950+ for titles, so only rows with 300+ are ranked.
- Disagreeing reads of an unclipped title (`AAAABATONGA`/`ANAABATONGA`) resolve
  only when exactly one read equals a remembered same-league name of the same
  length; otherwise ambiguous.
- Earlier versions remembered OCR variants as separate names. Variants equal
  under `matchKey` are no longer learned. The owner's local known list was
  cleaned of `ANAABATONGA`, `ODNÆSENNASINE`, `EVOTllZV0L1` (backup:
  `progress.json.bak-before-0.1.99`). A misread on which both reads agree can
  still be learned; there is no immutable ID to arbitrate.


## A turning screen, row double-clicks and any league (v0.1.109)

From the owner's live v0.1.108 run (pid 19224 in the diagnostics log):

- **The selection screen turns as a whole** at times: -3.9° on the double-click
  (frames 32/33), about -2° just after login, +10.4° with the league dropdown
  open. PLAY, CREATE and DELETE fit one rotation plus shift within a few pixels,
  so the fixed DELETE box missed by 1 px and the frame was `unknown`.
  `steadyView` finds the three buttons anywhere near their places, fits rotation,
  scale and shift (least squares), checks all three fit, and moves every word back
  to the still layout. On such a `moved` frame the crop is no second read and
  OCR is worse, so only a name already remembered is taken (`complete-moved`) and
  nothing is learned. The native gold measure and the diagnostics fields cover a
  wider area for the turned titles.
- **Row double-click:** the hook detected it (v0.1.108), but neither frame was
  readable. The double-click's point now travels with its frames (`click`); when
  the highlight cannot be told, the character row under the click is taken
  (`…-clicked-row`). Only a row with a level line under it: a league dropdown item
  lying over the list has none. Single clicks send no point.
- **Leagues are no condition any more.** A character name is unique across all
  leagues, and leagues change every few months and are cut short by the game
  (`HC SSF Forbidden Rites Lea...`: the OGLEDALO character was never accepted).
  - A row is shown to be a character's by its `LEVEL <1-100> <CLASS>` line
    (also as misread: `I_EVEL`, `[EVEL`), not by its league.
  - `leagueText` reads `<league> League` with a misread or cut "League"
    (`i_eague`, `Lea...`), or the row's cut last line by position. It knows no
    league name.
  - A candidate or a learned name may come without a league (remembered empty;
    a later read fills it in). Leagues compare by prefix and case-free
    (`sameLeague`); an unread league narrows nothing, the history match must still
    be unique.
  - A character moved to Standard when its league ends is learned again under
    Standard from any whole read; the old entry stays.

Limits: the buttons must read `PLAY`/`CREATE`/`DELETE` (English client, en-US
OCR) on a 16:9 layout; the creation screen still needs one of the twelve class
names; a covered title on a turning screen resolves only from history.

## Linux (v0.1.112)

Linux reads the character as Windows does; the code that captures, queues and hands
frames to the recognizer is now shared (`identity_native.rs`), and each platform
brings its input, its "game in front" check and its OCR:

- **Input:** Wayland lets no program watch other programs' input. On Hyprland the app
  adds non-consuming binds (`bindn`) to the `event` dispatcher while the game runs and
  the player is in the menus: left click, Enter/KP_Enter and the name-editing keys
  (plain and with Shift). Hyprland passes every input on to the game and writes
  `custom>>batonga-click|enter|edit` to its event socket; a click's place comes from
  `cursorpos` and the active window. Play, row double-clicks (500 ms, 6 px) and the
  rest follow the Windows rules. In a zone (once bound) the binds are removed, so
  gameplay input is not observed; so still one screenshot per real input, no timer.
- **Capture:** `grim` of the game window (as the map reader); the game must be on the
  visible workspace.
- **OCR:** `tesseract` (English) on two areas: the right panel and the bottom middle,
  scaled so the menu text is about 40 px, brightest channel inverted. The name list or
  name field is read a second time thresholded, for the two independent reads the
  recognizer compares. TSV words are regrouped into lines by baseline and gap
  (`identity_tesseract.rs`, tested on every platform). Gold is measured as on Windows.
- Without Hyprland or `tesseract` (or with `BATONGA_SCREEN_RECOGNITION=0`) Linux keeps
  the last chosen character, as before (`identity::recognises`).

Not yet compiled or run on Linux at the time of writing (written on Windows, where
the shared and pure parts are tested). Expected first-run checks: Hyprland accepts
`bindn , mouse:272, event, …` from `keyword`; tesseract's reads of the gold small caps;
OCR time per click (two to three tesseract runs).
