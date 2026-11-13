# Council review: the overlay for a fast league-start run (2026-11-13)

The owner asked for a review of the app's features, design and information, "so that i can
levelup fast when playing", against the guides of the fastest players. Three reviewers (Claude
Opus, GPT Sol, Claude Sonnet standing in for Grok) judged v0.1.118 (`3b12baf`) against
[the benchmark](levelling-benchmark.md). All three: **not ready**.

## Findings

| Id | Finding | Raised by | Severity |
| --- | --- | --- | --- |
| F1 | The default route is a walkthrough (the Domistae checklist, for all classes), not the fast route: power pickups are hidden as optional (the Clearfell campsite gem, Areagne, Ervig, the Cemetery ring, the Haunted Treasure, the Gallows rope), and steps the fastest guide skips are required (looting Lachlann's ring, "Talk to Finn", "Talk to all NPCs", the Trial of Chaos). Two data errors: Shambrin's letter is optional though it pays the 2 points; the heart for Aggorat's required altar is an optional row. | all three | blocking |
| F2 | The panel shows the wrong visit when the player leaves the planned order. One Red Vale entry ticked two Grelwood steps, so the visit with Gerung and Areagne could never show. A skipped detour kept an old visit for good. A waypoint-grab entry listed the zone's full list. An unplanned town trip showed the next planned visit. | GPT, Opus | blocking |
| F3 | The panel does not know the character's level; the header's "Lv" is the zone's. Fast guides plan around level targets, and the log gives the level on every level-up. | all three | should-fix |
| F4 | Rows the log cannot tick hold the bright arrow after they are done; on Linux about half the required rows. | all three | should-fix |
| F5 | Town visits list quest talks, not the errands that keep a town trip short. | all three | should-fix |
| F6 | The schematic's glowing route is fixed per zone and can point away from the current step; it shows places of hidden rows, stays under a finished zone, uses orange for exits where the rows use orange for bosses, and its labels are under 10 px. | all three | should-fix |
| F7 | No boss lines and almost no place-bound tricks; the panel hides during the fight, so a boss line has to be on the row before it. | Opus, Sonnet | should-fix |
| F8 | A boss quiet for 30 s was struck out as killed: a false success when the player backed off. | GPT (blocking), Opus (measure first) | disputed |
| F9 | Version 1.0 lands on 2026-12-11 with Acts 5–6: guide text for Acts 2–4 written now may be redone. | all three | should-fix |
| F10 | "Reset progress" wiped the run on one click. | Opus | should-fix |
| F11 | Row look: an optional row with a reward lost its "optional" tag; rewards on required rows; the level hidden for long zone names. | all three | polish |
| F12 | Legibility at 25% opacity over bright zones is untested; whether the schematic is read at all is unknown. | all three | unknown until played |

Assumptions that fell: the imported route is a fast route; hand-ticked rows cost little; the
current visit alone is enough in town.

## The owner's decisions (2026-11-13)

1. The fast route's power pickups show by default; the Mud Burrow and other side content stay
   optional.
2. Lachlann's ring stays on the ground (it starts a long cutscene); the row says so.
3. The panel shows the character's level next to the zone's, and a level target only while the
   character is under it.
4. A boss quiet for 30 s is marked "probably done": the arrow moves on, a click confirms.
5. A reward shows on a row only where it is the reason for the row.

Order of work: visits (F2), the Act 1 route (F1), the level and its targets (F3), the arrow
(F4, F8), town errands (F5), the schematic and the rows (F6, F10, F11), boss lines (F7). Guide
text for Acts 2–4 waits for 1.0 (F9).

## Left for later

- A chosen build per character, for gem and reward choices (GPT's proposal; Opus and Sonnet
  would leave that to the build's own page).
- Ticking reward rows from the log's "received" lines (passive points, spirit): seen in the
  co-owner's log (`docs/log-findings.md`), not yet in the owner's.
- One recorded Act 1 run to settle what a glance takes in and how often the arrow is wrong.
