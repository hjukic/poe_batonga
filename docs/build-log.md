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
