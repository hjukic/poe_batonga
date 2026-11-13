# The quest tracker: kills and talks the log does not show

Since v0.1.114 (Windows) the checklist also reads the game's own quest tracker, the list
of quests under the minimap, to tick boss kills and NPC talks the moment they happen.

## Why

The owner's run of 2026-11-13 (character StoNaMeniVidisCurice):

- **The Bloated Miller** died about 16:49. The log has nothing at that moment (only
  `is now level 2` at 16:49:02). Mortimer's line `Well done! Please come inside.`, the
  step's log trigger, came at 16:51:02, when the player reached the gate, two seconds
  before the town loaded. The step stayed open the whole time in between.
- **Talk to Renly**: the reward talk wrote nothing (dialogue panels are never logged,
  `docs/triggers.md`). The step was only implied at 16:53:03, on entering Clearfell.

The tracker changes at once: after the kill, "Kill the Bloated Miller and end his rage"
becomes "Enter the logging encampment".

## Data

`data/quests/states.json` holds the campaign's quests (acts 1–6) and, for each state, the
lines the tracker shows: the state's `MapPinsText` in the game's `QuestStates` table, one
line per `\n` (measured: The Miller's state 4 shows "Enter the logging encampment", its
`Message` is "Enter town"). A completed quest has no text; its `Message` "Quest Complete"
is kept in case the tracker shows it for a moment.

The game shows a quest's first state, by `Order`, whose flags hold. Flags only grow, so a
lower `Order` is further on. A quest with branches (Draven and Asinia in either order, the
Act 4 islands) has states for each branch, so `Order` is not a single line there.

After a game patch:

```sh
poe_data_tools --patch 2 --steam "<game folder>" dump-tables --mode json <dump> \
  data/balance/quest.datc64 data/balance/queststates.datc64
node scripts/import-quest-states.mjs <dump>
cd src-tauri && cargo test every_screen_condition_names_real_tracker_lines
```

`poe_data_tools` needs nightly Rust (`cargo +nightly install --git
https://github.com/adamthedash/poe_data_tools`). A condition whose quest or line is gone
fails to load, and the test names the route file and step.

The tables `NPCTalk` (who sets which quest flag: Renly's "Mercy for the Miller Reward" sets
`MillerRewardSeen`, Una gives "Secrets in the Dark", Renly "The Devourer") were used to
work out what each talk step changes.

## Route steps

A step names in `screen` the tracker states that prove it done, any one entry is enough:

```json
"screen": [{ "quest": "TheMiller", "past": "Kill the Bloated Miller and end his rage" }]
"screen": [{ "quest": "FindTheSpearPieces", "from": "Speak to Makoru in Kingsmarch" }]
"screen": [{ "quest": "GraveyardFight", "lines": ["Pick up Draven's Memorial Key Piece", "..."] }]
"screen": [{ "quest": "TheMiller" }]
```

- `past`: every state further on than the first state showing the line, completion
  included. For quests that go one way.
- `from`: the first state showing the line and every state further on. For a line said
  again later in the quest ("Speak to Alva") or the n-th of like steps.
- `lines`: the states showing any of the lines. For branches; "Quest Complete" means the
  quest left the list.
- quest only: any state (the quest was given).

A read proves a step only when every state it could be is a proving one: "Pick up Asinia's
Memorial Key Piece" is shown with and without Draven dead, so it proves Asinia, not Draven.

`scripts/set-screen-conditions.mjs <mapping.json>` writes the `screen` lines into the
route files without reformatting them.

Covered (v0.1.114): 119 steps in acts 1–4 (`every_screen_condition_names_real_tracker_lines`).
Ticks by `screen` imply the unseen steps before them, as log ticks do.

**Not covered: bosses outside quests.** Beira, Gerung, Areagne, Ervig, The Crowbell,
Candlemass, The Rotten Druid, Rootdredge, Mighty Silverfist, Blackjaw, Kabala, Ninth
Treasure, Narag, Forael, Mimok, Clawcrunch, Magma Twins and the other optional bosses have
no quest state, so the tracker never mentions them. Also not covered: talks that change no
quest (Finn after the Cemetery, Shambrin, Ange, Freya's first talk) and pick-ups (relics,
torn maps). They stay manual or implied.

## Reading

`src-tauri/src/quest_screen.rs` (shared) and `quest_screen_windows.rs`:

- Only while the player is in a zone, a character is bound and some step with `screen` is
  open, about once a second.
- The capture is the client area's right column from 0.22 to 0.85 of its height, 0.75
  heights wide (the interface scales with the height), skipped while the game is not in
  front or another window covers it. GDI, like the minimap capture; no game memory, no input.
- Windows.Media.Ocr (en-US) reads it. A line the OCR runs across a wide gap is split there.
- Lines compare normalized (lower case, letters and digits, apostrophes dropped) and within
  one edit per 10 letters (quest names one per 8; none below 8 letters). A quest name starts
  a block; a line said by several quests counts only under its quest's name. A line wrapped
  in two is read joined.
- A quest's state counts when two reads in a row agree.
- A quest that leaves the list counts as complete when it was at one of its last two states
  (a reward talk, or the step before it when the reward is taken in under two reads), three
  reads in a row show other quests but not it, and the list cannot have cut it off: a quest
  that was below it still shows, or at most three quests show.
- A panel over the tracker (inventory) reads as no quests: nothing changes.

Checked on the owner's screenshot (2488×1336): the column reads "Reaching Clearfell",
"Enter the logging encampment" and the Miller's step ticks; with the inventory open the
column reads only the item tooltip.

## Live test

Run with `BATONGA_QUEST_DIAGNOSTICS=artifacts/quest-tracker.jsonl`: each new read (OCR
lines, the states they fit, what was confirmed) is one JSON line. A captured column can be
checked offline: save it as PPM and run
`BATONGA_QUEST_REFERENCE=<file.ppm> cargo test --release quest_reference -- --ignored --nocapture`.

Open: the tracker's behaviour is assumed from one screenshot — whether it ever hides or
collapses quests, and whether it caps how many it shows. Linux has no capture for it yet.
