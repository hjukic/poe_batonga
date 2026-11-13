# Review of the character-identity work (v0.1.94, v0.1.99), 2026-11-13

A code review of `74c2d70` and `f2224fc`, asked for by the owner. Items 1, 2 and 10 were
checked by hand in the code; the others are from the review and are worth confirming
before fixing. Line numbers are as of `f2224fc`.

## Already changed (v0.1.101)

- **Linux keeps its progress.** Where no character can be recognised (`!cfg!(windows)`),
  `Characters::keep_chosen` binds the last chosen character after anything that clears the
  binding (start, new login, game closed); before any was chosen, a `Default` profile
  takes the legacy progress. On the owner's Linux PC every restart had put the ticks into an
  empty, unsaved progress, and the 40 earlier ticks sat in `legacy`. Windows is unchanged
  (`RECOGNISES` is true there, `keep_chosen` does nothing). Test:
  `characters::tests::without_recognition_the_progress_is_kept_across_restarts`.
- **`game::running()` keeps its answer for 3 s** (item 10 below). It was asked on every
  500 ms poll and every log batch; on Linux each call read every process's `cmdline`.
- Please build on Windows once: the Windows cross-check cannot run on Linux (tauri-winres).

## Fixed on Windows (v0.1.108)

Items 1–9 and the `save()` half of item 10 are fixed; each has a test. Built and tested on
Windows. What changed, by item:

1. `request()` stores `LAST_ACTIVATION`, so a quick Play after a selection keeps its
   selection frame. The mouse hook takes a second click on the character list (system
   double-click time and rectangle, `second_click`) as an activation.
2. `glued` needs at least three tooltip glyphs after the known name; "ANNAB" is not "Anna".
3. `game::running()` tells a gone process (false at once) from a running one whose
   "Path of Exile 2" window is missing: still running for 15 s after the window was last seen.
4. Live events read while unbound are kept with their area (`Characters::unbound`) and
   judged again against the chosen profile (`tracker::replay_unbound`), after OCR binding
   and manual selection. The start backlog is not kept: it may be another character's.
5. `clear_binding` keeps `current_area`/`shown_zone` (they belong to the log, like its
   path), so the start guard works on Windows too.
6. `Recognition::observe`: an activation frame without a name keeps the read before it.
7. "quests completed" is a summary line; a click lists the zone's steps to untick one.
8. `Characters::select` falls back to `characters::match_key` (the JS `matchKey`) when no
   profile has the same spelling up to case.
9. The hooks use `InternalGetWindowText` (no `WM_GETTEXT`).
10. `identity_observed` saves only when a name was learned, bound or the menu was reached
    from a zone. Still open: two full-frame OCRs per creation keystroke when a caret is masked.

## Open, as reviewed (line numbers as of `f2224fc`)

1. **Double-click activation is never implemented** (`identity_windows.rs:27`).
   `LAST_ACTIVATION` is read in `superseding_input()` but never written; `LAST_CLICK_MS`
   and `LAST_CLICK_AT` are unused. A row double-click arrives as two `request(80, false, ..)`,
   `begin()` leaves `identity.submitted = false`, `bind_at()` returns early on zone entry and
   the panel stays on "choose character". A selection click followed by Play within the
   80 ms debounce also loses the selection trigger (`trigger.seq != superseding_input()`).
2. **`glued` accepts a known name plus one extra character** (`character-screen.mjs:148`).
   `read.slice(key.length, -1)` is `''` for a read one character longer than the key, and
   `tooltip.startsWith('')` is true. A new "Annabel" whose clipped title reads "ANNAB" in
   both frames resolves to a known "Anna" (`history-prefix`) and its ticks merge into
   Anna's profile.
3. **One missed game-window poll unbinds the character mid-play** (`tracker.rs:1063`).
   `running() == Some(false)` → `set_in_zone(false)` → `new_session(None)` →
   `clear_binding()`. On Windows `running()` needs a top-level window titled exactly
   "Path of Exile 2"; a recreated window (display-mode change, loading) misses it. Then
   `IN_ZONE = false` makes every in-game key and click a full-window screenshot plus OCR.
4. **Events while unbound are judged against an empty progress** (`characters.rs:92`).
   Triggers with `after`, and visit-dependent rules, see no earlier ticks; `select()` later
   merges only `completed`, so such steps stay open.
5. **The "restored entry is not another visit" guard is dead** (`tracker.rs:929`, also the
   `shown_zone` recompute at 739): `clear_binding()` at start resets `current_area`, so the
   entry is always applied "on start". (On Linux `keep_chosen` now restores `current_area`
   first, so the guard works there.)
6. **An ambiguous activation frame erases a good pending candidate** (`identity.rs:210`).
   Selection gives `pending = 'Hero'`, Play's frame classifies as selection but with no
   selected row (highlight fading, hover tooltip): `pending = None`, the zone is entered
   unbound.
7. **In "completed" mode a wrong tick cannot be undone** (`main.ts:278`): the rows are
   replaced by "quests completed", so the new manual correction has nothing to untick.
8. **Profile lookup folds only case** (`characters.rs:57`). OCR misread folding (I/l/1,
   O/0, Æ/E: `matchKey`) is JS-only against `known`. "BAT0NGA" for a hand-made "Batonga"
   creates a second, empty profile. Fold the same way in `Characters::select`.
9. **The low-level hooks call `GetWindowTextW`** (`identity_windows.rs:64`) on the
   foreground or clicked window. For the overlay's own windows that is a synchronous
   `WM_GETTEXT` to its UI thread from inside the hook: global input lag when the webview
   is busy, and past `LowLevelHooksTimeout` Windows removes the hook silently. Compare the
   window's process id instead, or use `InternalGetWindowText`.
10. *(fixed, see above)* `game::running()` on every poll and log batch. Related, still open:
    on the creation screen `frames()` runs full-frame OCR twice per keystroke
    (`identity_windows.rs:327/335`), and `identity_observed` calls `inner.save()` on every
    menu OCR result.
