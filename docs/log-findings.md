# Local PoE 2 log findings

Read-only inspection on 2026-11-11 of Steam's
`steamapps/common/Path of Exile 2/logs/Client.txt`: 514,334,668 bytes,
last modified 2026-09-10. Examined the last 15,000 lines only.
Raw logs and character names were not copied into this repository.

Observed in the September 10 records:

- Zone-generation messages contain area level, internal area ID and numeric seed.
  Campaign examples include P3_5, P3_6, P3_7 and P3_Town.
- P3_6 appears with different seeds on separate visits. Town/hideout entries
  can use seed 1, so a seed alone is not a globally unique instance identity.
- Level-up messages contain name, class and level. The observed class
  `Disciple of Varashta` contains spaces; Qutm's single-word class regex misses it.
- Explicit messages report receipt of 2 Passive Skill Points and separately
  2 Weapon Set Passive Skill Points at the same timestamp. Preserve every line
  in a batch. Reward text alone does not identify the quest that supplied it.

These are verified historical inputs, not current-patch guarantees. Player
coordinates, quest-item pickup, complete objective coverage and character-switch
detection remain unverified. A seed does not provide player position or decode a
layout by itself. Step 4 remains open pending broader and labelled checks.

## Full-file investigation: quest items and navigation

Follow-up on 2026-11-11 scanned all 3,072,602 lines, spanning 2025-09-09 to
2026-09-10. Streaming searches covered pickup/acquisition/inventory/reward terms,
quest-item names, position/coordinate fields and layout/terrain/tile terms.
Matches were deduplicated and inspected with selected surrounding records;
this was not a manual review of every line. Chat, graphics capability names and
engine errors were distinguished from actionable game events.

### Quest item pickup: no general event found

No generic quest-item-acquired record was identified by these searches. There
are indirect quest-stage signals and explicit reward messages:

- Lines 212803 and 212814 in G3_6_2 contain Alva's dialogue about an uncharged
  Soul Core and then a charged core. Line 212815 says it still needs to be
  removed from the construct. Thus the charged-core line must not be interpreted
  as proof the player has picked it up.
- Line 224284 reports that activating a Stone Altar requires a Small Soul Core;
  a requirement/error message is not evidence of acquisition.
- Lines 163658-163659 explicitly report passive and weapon-set passive points.
  Other records explicitly report Spirit, resistances and permanent bonuses.
  Named reward recipients must be matched to the active character; do not assume
  every named message describes the local player.
- The file contains some inventory-related consumption messages (Omens), but
  these do not establish a general quest-item inventory feed.

Recommendation: record separate facts for quest stage, reward received and item
acquired. Validate each dialogue trigger with labelled play before using it for
automatic completion. Missing messages leave an item unknown. Keyword-search
absence is not proof that no patch or special quest ever logs pickup.

### Player position: no usable movement feed found

Position/location matches predominantly describe engine failures, invalid effect
or summoned-entity positions, and ladder rankings. For example, line 108508 has
an invalid BeamEffect location `(0, -1)`, not the player's coordinates. No usable
continuous player-position records were identified. These diagnostics cannot
support a navigation arrow.

### Layout: metadata exists, geometry was not found

The complete file contains 19,623 generation records, 19,623 Tile hash records
and 19,623 Doodad hash records (counts, not a claim of validated one-to-one pairing).
Selected generation sequences also contain:

- `Client-Safe Instance ID` (e.g. line 163647).
- Internal zone ID, area level and seed (line 163648).
- Tile and doodad hash values (lines 163649-163650).
- Human-readable scene name and Height Map Texture dimensions (163653-163654).

These values could help identify or group observations of instances. Their
stability, collision behavior and relation to reusable layout variants are not
validated. A hash is not a list of tiles or exits. Texture dimensions are not
walkable geometry. Neither these hashes nor a seed provide a reconstruction
method or the player's position on their own. No usable map geometry, exit
coordinate list or tile-placement table was identified by this investigation.

## Effect on the plan

Use logs for zone context and explicitly supported reward/quest-stage facts.
Investigate instance ID and hashes as supplemental matching metadata, without
assuming they identify a known template. The automatic arrow still needs a
separate permitted source of visible map geometry and player position.
Step 4 has substantial historical evidence but remains open for labelled item
pickup/turn-in tests and current-patch validation. Raw logs remain outside Git.

## Labelled new-character test (2026-11-13, 01:09–01:10 local)

The owner created a new character and reported entering the game around 01:09,
picking up a Voltaic Staff, Channeling Talisman and Volcano skill, and activating
a checkpoint around 01:10. A byte offset was recorded before the test; the
unmodified new bytes were exported locally under `artifacts/log-review/` as a
`.log` file (ignored by Git). The reviewed excerpt has 53 lines, from 01:08:51
through 01:11:22; no raw chat, account/character names or addresses are copied here.

Observed in the excerpt:

- 01:08:51: connection to the login server.
- 01:09:38: an instance ID, generation of level 1 area `G1_1` with a seed,
  tile/doodad hashes, and scene name `The Riverbank`.
- 01:09:44: the Riverbank loading screen finished (reported duration about 5.9 s).
- 01:09:56, 01:10:01 and 01:10:08: the Wounded Man's three lines. The last line
  matches our existing `talk-wounded-man` trigger.
- 01:10:47: the game window lost focus.

No record naming the reported items/skill, their pickup, the checkpoint/waypoint,
or the new character's name/ID was found in this excerpt. The item-class errors
name unrelated types and are not pickup events. There are chat and guild
messages, which must not be mistaken for local-character identity.

The number in `[Client …]` was checked against the live OS process and is its
process ID. `Client-Safe Instance ID` identifies the instance, and the generation
seed is map metadata; neither establishes character identity. This labelled
trial supports the absence of those events in this specific sequence, not a
claim that no game version or other situation can ever log them.
