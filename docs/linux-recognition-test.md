# Linux character recognition — task for Sue's agent (v0.1.110)

Written 2026-11-13 by Jurica's agent (Claude, on Windows). If you are the agent working
with Sue on Linux, this is your task. Sue reported that character names are not
recognized on Linux: until v0.1.110 Linux had no screen recognition at all (it only kept
the last chosen character). v0.1.110 adds it, but it was written on Windows and has
**never been compiled or run on Linux**. Your job is to make it build and work.

## How it works (read `docs/character-identity.md`, section "Linux (v0.1.110)")

- `src-tauri/src/identity_linux.rs`: Linux input and OCR. While the game runs and the
  player is in the menus, it adds non-consuming Hyprland binds (`bindn`, dispatcher
  `event`) for left click, Enter/KP_Enter and the name-editing keys (plain and with
  SHIFT), and listens on `.socket2.sock` for `custom>>batonga-click|enter|edit`. Every
  real input gives one `grim` capture of the game window, read by `tesseract` (TSV,
  `--psm 11`). In a zone the binds are removed.
- `src-tauri/src/identity_tesseract.rs`: image preparation and TSV → words/lines
  (tested on every platform).
- `src-tauri/src/identity_native.rs`: shared with Windows (capture queue, frames,
  click/Play logic). `src/character-screen.mjs`: the shared recognizer.

Rules that must stay (GGG overlay rules, `docs/character-identity.md`): one screen read
per real user input, no periodic OCR, no simulated game input, no memory reads, no
packet interception. The binds must never consume or change input to the game.

## Steps

1. `git pull`. Install `tesseract` with English data (Arch:
   `pacman -S tesseract tesseract-data-eng`) and check `grim` is present.
2. `cd src-tauri && cargo build --release` and `cargo test --release`. Fix compile
   errors; they are expected only in `identity_linux.rs` (and its use of
   `identity_native.rs`). Do not change Windows behaviour.
3. Run the overlay with
   `BATONGA_IDENTITY_DIAGNOSTICS=/tmp/identity.log` (as in `docs/overlay-test.md`).
   Start the game. In the character selection, `hyprctl binds | grep -i batonga`
   (or `hyprctl binds` and look for `event` binds) must list them; in a zone none.
   If Hyprland rejects a bind (check the app's stderr and `hyprctl` output), fix the
   syntax in `binds()`; the uncertain parts are `bindn` with `mouse:272` and no
   modifier, the `event` dispatcher and `keyword unbind`.
4. Ask Sue to: select a character and enter with **Play**; enter with a
   **double-click** on a row; create a **new character** and enter. Each time the
   overlay must say `Character: <name>` in the zone. Clicks and typing must reach the
   game normally (nothing swallowed, no double input).
5. If names are not read, look at `/tmp/identity.log`: the lines tesseract returned
   per frame. Likely tuning points: `AREAS`, `scale_for`, `Prep` thresholds in
   `identity_tesseract.rs`, and `group_lines` (one line per name row). Also note the
   OCR time per click.
6. Write what you found and changed in `docs/SESSION.md` (a short "Linux test"
   result under the v0.1.110 section), commit with a version bump (0.1.111), and
   tell Sue to push or send the log to Jurica.
