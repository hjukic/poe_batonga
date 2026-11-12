# Build log

## Night build, 2026-11-12 (v0.1.11 → v0.1.43)

The owner asked for the arrow and layout logic to be made to work with the datamined maps, a concise
checklist, and whatever else improved the app, while away for six hours. The chair (a Claude session) planned,
briefed four builders in the Herdr app, had each large package reviewed, and merged. No input was sent to the
game: checks were simulation, the owner's recordings, and screenshots of the idle game.

### What was wrong, measured first

- Tracking died whenever the big map (Tab) was open: the corner minimap is hidden then, and the owner plays
  that way. One Clearfell visit was recorded as 17 unrelated pieces.
- The layout came from one batch fit of an unbroken walked path; every tracking loss threw it away.
- Layout graphs existed for 3 zones, checklists for 3 zones.
- The overlay stayed blank after a restart until hidden and shown again.

### Packages

| Package | Builder | Passes | Result |
| --- | --- | --- | --- |
| Checklists | Claude Opus, high | 4 | 97 zones (Acts 1–4, interludes), 427 steps; 45 of 50 "go back" steps tick themselves (`after`) |
| Layout graphs | Claude Opus, high | 2 | `npm run layouts:update`: 101 areas, 584 layouts, `.tgr` and `.dgr`, sub-graphs expanded |
| Big map | Claude Opus, xhigh | 5 | Reads the big map in the minimap's units (scale 1.00), wide frames, re-find after a gap, icons across the screen, exit labels in the game's font |
| Navigator | Claude Opus, xhigh | 5 | A belief over (layout, position) per zone visit; exit icons and names as anchors; memory per instance |
| Chair | — | — | Panel redesign, tick shortcut, settings, start-up fix, bundling, `after` triggers, tick observations, wiring |

Reviews: Cursor was out of usage and Codex stopped at a folder-trust prompt (not answered for the owner), so
the three reviews were done by Claude Sonnet, another model than the builders'. They found: for the big map,
five things to fix, none blocking; for the navigator, four blocking (overconfidence from the walked line, a
stale resumed belief shown as sure, a pause counted as lost movement, gold in unmeasured zones); for the icon
anchors, five to fix, none blocking. All accepted findings were fixed in the same night.

### What the measurements say

- **The walked line alone cannot identify a layout.** On the owner's Clearfell (seed 2693856399) it ranked the
  true layout fifth; the first navigator put a wrong one at certainty. The truth came from the big map: the
  vector between the two exit icons fits `ClearFell_W_E_N` and no other layout. Path evidence is now tempered
  and capped; gold needs absolute evidence.
- **Exit icons and their names decide.** Two exits read by name fix the layout from one frame. Live, after the
  last fix: `ClearFell_W_E_N` 0.997, the needle gold at the Grelwood exit.
- **Simulated walks** (every Riverbank and Clearfell layout, a walker that strays like the recordings, 30% told
  a wrong arrival): without icons the needle is within 45° at 60% of the walk in about 30% of runs and is never
  gold; with icons 35–52%. Gold appears on up to 4% of steps. With unnamed icons 2% of gold needles are more
  than 60° off the walkable route; with named icons in the minimap view 8.6% (three seeds), in the wide view
  3.9% (one seed). The 3% bar set for gold is therefore not met with named icons. A gold needle there points
  straight at a named exit icon that is on the map; the route to it can bend (a wall, a river), and exits sit
  up to about 100 units from their graph nodes.
- Numbers and method: [map reading](map-reading.md).

### Found on the live screen, not by tests

- After an app restart inside a zone no exit names were set, so labels were not read (v0.1.37).
- Remembered exit places, learnt for every layout, fed back into the evidence and made every layout agree
  with the icons; the right layout fell to 0.006 (fixed in v0.1.42, memory files versioned).
- The compass arrowhead was nearly equilateral: its tip was ambiguous (v0.1.43).

### Open

- Movement across the minimap and the big map, a zone change, and the Grelwood (first zone not measured) need
  the owner's play: checks in [overlay test](overlay-test.md), "Big map reading".
- Gold with named icons is 4–9% off the walkable route in simulation; the bar was 3% (see above).
- Exit labels on the corner minimap are not read (no full-size minimap screenshot was available).
- Dungeon (`.dgr`) scale is derived from the files, not checked in play; needles there are gray unless a named
  target exit is in view.
- Steps without a log line are ticked by hand (Ctrl+Shift+Space). `step-observations.jsonl` collects the
  dialogue lines seen before each hand tick, to turn into triggers.
- Ideas not started: wall evidence for the navigator, a seed-to-layout table, a visit order by shortest tour.

### Run numbers

9 agents and 11 follow-up passes in about 4 hours of wall time; 419 task-minutes; parallelism 1.8; at most 4
agents at once. The navigator set the wall time (216 minutes over five passes). Fix passes went to the builder
that held the context and took 10–45 minutes each.

## Day build, 2026-11-12 (v0.1.45 → v0.1.49)

The owner asked for further improvements. Two builders (Claude Opus, xhigh), one review (Claude Sonnet), the
chair's own changes. The game was closed the whole time: nothing was checked on a live screen.

| Package | Passes | Result |
| --- | --- | --- |
| Flight recorder | 2 | Each zone visit appends what the navigator is fed to `layouts/<area>/<seed>.obs.jsonl`; a replay scores the needle on real visits; `layout-history.jsonl` lists visits whose layout the named exits fixed |
| The way through the recorded map | 3 | A way that does not cross recorded walls; live for the way back to the exit the player came in by, in shadow for ways ahead |
| Chair | — | The overlay hides while the game is off screen; real captures are kept under `samples/`; merges |

The review found one blocking problem (a straight line across unrecorded ground replaced the layout's needle and
was gold) and nine to fix (arrival points that are not exits, a search grid whose doors moved with the player, a
trail opening doors in lines it only grazed, gold for long detours, scores that counted portals as exits and
ignored the needles actually shown, torn writes). All were fixed.

**What the first real comparison said.** On the owner's Clearfell recording the map needle matched where the
player walked next in 16 of 35 cases, a plain straight line in 21. The recording is old and its walls are drawn
off the walk, so this does not settle it, but it does not support showing it. Ways ahead therefore run in
**shadow**: worked out, written to the flight recorder, scored by the replay, not shown. The bar to switch them on
(`MAP_AHEAD_LIVE` in `nav.rs`) is in [map reading](map-reading.md): over at least five real visits, the shadow
within 45° at least as often as the shown needle, and would-be gold off in at most 3%.

### Open after this round

- The owner's next sessions decide the map routing: `OBS=<app data>/layouts cargo test --release replay_obs --
  --ignored --nocapture` prints shown, replayed and shadow scores.
- `samples/` in the app's data folder will hold corner-minimap captures with an exit in view: the material for
  reading exit names on the minimap.
- The auto-hide was checked with the game closed (the overlay stays hidden); its return when the game starts
  was not.

### v0.1.50: boss lines

The owner asked for bosses. A zone boss's first line in the log now tells the navigator the player is near the
boss's place in the layout: soft evidence, never an anchor, never gold on its own. 58 of the 74 boss steps
have a place in the layout graphs. When a boss speaks, the app saves a capture of the minimap and the big map
(`samples/boss-*.ppm`), to see what the map draws for a boss.

## Third build and the first live session, 2026-11-12 (v0.1.51 → v0.1.59)

The owner asked for further improvements, then started the game with a new character and played the
Riverbank and the Clearfell Encampment while the build ran. For the first time the app was seen in play;
much of what follows came from that, not from the plan.

| Package | Builder | Passes | Result |
| --- | --- | --- | --- |
| Triggers from the game's dialogue table | Claude Opus, xhigh; a fresh Opus for the fix pass (the first pane had closed) | 2 | `npcLine` triggers with several lines and `after`; a checker against the game tables (`npm run triggers:check`); 81 triggers. Play then showed that dialogue-panel text is not logged: the 58 "panel" ones cannot fire |
| Map icons by the game's sprites | Claude Opus, xhigh | 3 | `data/icons/` from the game's sprite sheet; icons named by sprite; false exits, checkpoints and waypoints from scene pixels gone on the owner's captures; checkpoints as capped soft evidence; big-map wall cells kept only once seen from two places; `Recorder::dead_bosses` |
| Chair | — | — | The live fixes below; wiring; merges |

Reviews (Cursor still out of usage, Codex not trusted for the folder: Claude Sonnet, three reviews). The
triggers review found nothing blocking and four triggers to narrow. The icons review said not ready, four
blocking: checkpoint evidence without a cap changed 248 of 400 needles on the owner's own Riverbank
recording; a false checkpoint passed its threshold by 0.008; boss icons raised wrong gold from about 2% to 4%
in simulation; three of four "boss is dead" rules could fire on ordinary kills. After the fix pass a second
reviewer said ready, with three small findings, fixed in a last pass.

### What play showed

- The overlay returns when the game starts (not checked before). The compass was drawn over the login
  screen: it now waits for a zone.
- Three app restarts during one zone visit (the chair's updates, dev mode) cut the recording into parts
  and left the navigator on a wrong layout. Updates to the Rust side now wait for a town.
- Talks in a dialogue panel write nothing to the log. Steps the log cannot see are now taken as done when a
  later step of their list is logged (`imply_earlier`), and a click on a row ticks a step at any time.
- A town has no layout graphs: the needle ignored the exit the map showed by name. A named exit icon now
  leads the needle there, sure.
- Bosses do have map icons: the Miller's own icon with its place name ("Besieged Encampment") while alive,
  its grey twin without a name after the kill (two real captures). Kills are ticked from that pair only.
  Boss icons do not feed the navigator (shadow): in simulation they made gold worse.
- Blue foam, wet stones and the character's clothes were read as map walls and as waypoints on the big map.
- A first replay of a real visit (`replay_obs`): the needle pointed the right way; three look-alike
  Riverbank layouts shared 88% of the belief, none was confirmed.

### Chair's changes in the session

v0.1.51 label never cut off · v0.1.52 "Optional content" switch (off: speedrun list), hollow needle for a
likely direction · v0.1.54 the overlay hides during a boss fight · v0.1.55 named exit icons lead the needle
where there is no navigator · v0.1.56 the needle updates every frame · v0.1.57 click to tick, no shortcut,
implied ticks · v0.1.58 no compass before the first zone · v0.1.59 the icons merge, kills from the map.

### Open

- The click on a row, the fight hide, the implied ticks and the kill from the map have not been seen working
  in play yet. The hold on big-map walls has no real sequence behind it.
- Checkpoint evidence is capped (1.5 nats, six a visit) and has no real visit behind it either: check the
  replay after the next zones.
- The Riverbank's measured walk is 1.14–1.18 times the graphs' (Clearfell's exits 1.10): the scale may be
  nearer 0.9 cells per unit than 0.78.
- 54 boss kills have no log line; 41 of them have an icon pair and can tick from the map.

Run numbers: 7 agents and 2 follow-up passes in 165 minutes of wall time; 210 task-minutes; parallelism 1.3.
The icons package set the wall time (136 minutes with two fix passes).
