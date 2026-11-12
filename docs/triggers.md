# Triggers: checklist steps that tick themselves

A step in `data/routes/<zone>.json` may carry a `trigger`: a line of the game's log
(`Client.txt`) that ticks it. Steps without one are ticked by hand (Ctrl+Shift+Space); a hand
tick also stores the last 30 dialogue lines in `step-observations.jsonl` beside
`progress.json`, so play shows which lines really come before each step.

## How a trigger works

Two kinds, read by `src-tauri/src/tracker.rs`:

```json
"trigger": { "type": "zoneEntered", "area": "G1_town", "after": "grelwood-una-seals" }

"trigger": {
  "type": "npcLine",
  "area": "G1_7",
  "lines": [
    { "id": "BitterGuyDeath", "speaker": "Lachlann of Endless Lament", "text": "Together... at last..." }
  ]
}
```

- `zoneEntered`: the log's zone-generation line for `area`.
- `npcLine`: an NPC line, logged as `Speaker: text` in `[INFO Client N]` lines (chat starts
  with `#`, `@`, `$`, `%` or `&` and is ignored). It counts only while the player is in `area`,
  the step's own zone. `lines` lists every line that ticks the step (random and class variants,
  several speakers); any one of them does. The older one-line form, `"speaker"` + `"text"`,
  still works (the Riverbank's two triggers use it).
- `after` (both kinds): the trigger counts only once that step was done before this line.
- Texts compare in a shown form: `<i>{word}` (and `<b>`, `<italic>`) becomes `word`, other
  tags (`<continue>`, `<br/>`) and runs of whitespace become one space, on both sides. The
  trigger keeps the shown form.
- `id` names the game-table row of the line (`NPCTextAudio` or `NPCTalkDialogueTextAudio`).
  The tracker does not use it; the checker does.
- Next to the trigger, `evidence` says which rows it rests on and why, and `evidenceKind`
  how sure it is:
  - **seen**: the line is in the owner's log;
  - **bark**: world speech (not part of a dialogue panel), the kind the log is known to show;
    from the game data, not yet seen;
  - **panel**: text of a dialogue panel (a quest talk). Not known to be logged. Allowed when
    the line is said once, at the step's completion, so it is harmless if never logged.

What qualifies (rules used for the 81 triggers added on 2026-11-12): a boss's own death line,
another character's line about that death, a line that ends a scripted scene the step is
about, or a quest hand-in line that the quest flags of `NPCTalk` tie to the step (the flags an
option needs, hides on and sets). Not: fight barks and taunts, lines for the player's death,
greetings and vendor lines, lines also said earlier or elsewhere, and anything whose timing the
data does not show. The same line in two zones is told apart by `area` (Finn's line after the
King in the Mists is said in Freythorn and in town; only the town talk is the hand-in).

Measured and assumed: every NPC line in the owner's log so far (36 lines: the Wounded Man,
Mortimer, The Bloated Miller, Renly, Una) is an `NPCTextAudio` row with the same text and its
NPC's `Name` as the speaker. That bosses' death lines and the other barks are logged the same
way is assumed (same table, same kind of line). That the log shows the full `Name` where it
differs from `ShortName` is assumed. Whether panel text is logged, and whether a long panel
line with `<continue>` breaks is logged whole or page by page, is not known: if it is logged
by page, those triggers stay silent and the observations will show it.

## After a game patch

```sh
scripts/check-triggers.sh          # dumps the dialogue tables from the install and checks
```

It runs `scripts/check-triggers.mjs` on a fresh `poe_data_tools` dump (read-only, in a
temporary folder) and prints, per step: rows that are gone, texts that changed (old and new),
speakers that no longer match, and warnings for variants a trigger lacks, triggers bound to
another zone and an `evidenceKind` that does not match the tables. Exit code 1 when a line no
longer matches. Fix the trigger's `text`, `speaker` or `lines`, or drop the trigger when the
line is gone. Tests: `node --test scripts/check-triggers.test.mjs` (with
`TRIGGER_TABLES=<dump folder>` it also runs the full check).

To add a trigger: find the line in the tables (`id`, text, NPC `Name`), check when the game
says it (the `NPCTalk` option's flags, the dialogue's steps, the id), add it with `area`,
`evidence` and `evidenceKind`, and run the checker. A step ticked by hand whose
`step-observations.jsonl` entries show the same line before it every time is a candidate.

## Numbers

Objective steps: 402. Kinds: **kill** has a boss part; **talk** says "talk"; **other** is the
rest.

| | Kill | Talk | Other | All |
| --- | --- | --- | --- | --- |
| With a trigger, before (v0.1.50) | 1 | 1 | 145 | 147 |
| With a trigger, now | 20 | 46 | 162 | 228 |
| Ticked by hand, now | 54 | 15 | 105 | 174 |

The 83 `npcLine` triggers by evidence: seen 2 (the Riverbank's), bark 24, panel 57.
The 145 `zoneEntered` triggers are unchanged.

## Steps still ticked by hand

"No line found" means the tables were searched for the step's NPCs and ids and nothing said at
completion turned up; "not looked up" means the step was not searched in depth.

### Boss kills (54)

| Zone | Step | Reason |
| --- | --- | --- |
| G1_2 | `kill-beira` Kill Beira | her death line (CarrionCroneDeath) has no text |
| G1_3 | `kill-devourer` Kill The Devourer | no NPC row speaks for it: no line at all; Renly, Una and Finn react in town, later |
| G1_4 | `kill-gerung` Kill Gerung | no NPC row speaks for it: no line at all |
| G1_4 | `kill-areagne` Kill Areagne at the hut | no NPC row speaks for it: no line at all |
| G1_5 | `kill-rust-king` Kill The Rust King | only fight barks; no death line |
| G1_6 | `kill-ervig` Kill Ervig | no NPC row speaks for it: no line at all; Una's reaction is a town talk, later |
| G1_8 | `kill-draven` Kill Draven | only fight barks; no death line; Lachlann's reaction is a talk in the Cemetery |
| G1_9 | `kill-asinia` Kill Asinia | only fight barks; no death line; Lachlann's reaction is a talk in the Cemetery |
| G1_11 | `kill-crowbell` Kill The Crowbell | no NPC row speaks for it: no line at all |
| G1_13_2 | `kill-executioner` Kill The Executioner | only fight barks; no death line |
| G1_15 | `kill-candlemass` Kill Candlemass | no NPC row speaks for it: no line at all |
| G1_10 | `kill-root-hollow-druid` Kill The Rotten Druid | no NPC row speaks for it: no line at all |
| G2_1 | `kill-rathbreaker` Kill Rathbreaker | no NPC row speaks for it: no line at all; Zarka's 'You defeated the raiders?' is the next step's |
| G2_5_2 | `kill-iktab-ekbab` Kill Iktab & Ekbab | only fight barks; no death line (Iktab); Ekbab has no row |
| G2_4_1 | `kill-kabala` Kill Kabala (Venom Pit) | only fight barks; no death line |
| G2_4_2 | `kill-ninth-treasure` Kill Ninth Treasure | no NPC row speaks for it: no line at all |
| G2_7 | `kill-zalmarath` Kill Zalmarath | no NPC row speaks for it: no line at all |
| G2_8 | `kill-vultures` Kill 2 Vultures (side area) | no NPC row speaks for it: no line at all |
| G2_9_2 | `kill-tor-gul` Kill Tor Gul | no NPC row speaks for it: no line at all |
| G3_1 | `kill-rootdredge` Kill Rootdredge | no NPC row speaks for it: no line at all |
| G3_3 | `kill-silverfist` Kill Mighty Silverfist | no NPC row speaks for it: no line at all; Gwendolyn's reaction is a later talk |
| G3_5 | `kill-noxious-behemoth` Kill Noxious Behemoth | no NPC row speaks for it: no line at all |
| G3_5 | `kill-xyclucian` Kill Xyclucian | no NPC row speaks for it: no line at all |
| G3_6_1 | `kill-blackjaw` Kill Blackjaw, use his Core | only fight barks; no death line; the step goes on (use his Core) |
| G3_6_2 | `kill-zicoatl` Kill Zicoatl | no NPC row speaks for it: no line at all; Alva's line comes when the core is placed, before the fight |
| G3_2_2 | `kill-narag` Kill Narag (hut) | no NPC row speaks for it: no line at all |
| G3_7 | `kill-ignagduk` Kill Ignagduk, use Skull | only fight barks; no death line; the step goes on (use the Skull) |
| G3_9 | `kill-mektul` Kill Mektul | only fight barks; no death line |
| G3_11 | `kill-queen-of-filth` Kill Queen of Filth | only fight barks; no death line |
| G4_3_1 | `kill-great-white-one` Kill Great White One | no NPC row speaks for it: no line at all |
| G4_3_1 | `kill-clawcrunch` Kill Clawcrunch | no NPC row speaks for it: no line at all |
| G4_7 | `kill-scourge-of-the-skies` Kill Scourge of the Skies | no NPC row speaks for it: no line at all |
| G4_5_1 | `kill-forael` Kill Forael (Chapel) | no NPC row speaks for it: no line at all |
| G4_5_2 | `kill-the-prisoner` Kill The Prisoner | only fight barks; no death line |
| G4_1_1 | `kill-mimok` Kill Mimok (Beast Pen) | no NPC row speaks for it: no line at all |
| G4_1_1 | `kill-blind-beast` Kill Blind Beast | no NPC row speaks for it: no line at all |
| G4_1_2 | `kill-magma-twins` Kill Magma Twins | no NPC row speaks for it: no line at all |
| G4_1_2 | `kill-krutog` Kill Krutog | only fight barks; no death line |
| G4_2_1 | `kill-tombgrowl` Kill Tombgrowl (Tidal Cay) | no NPC row speaks for it: no line at all |
| G4_2_1 | `kill-doran` Kill Doran (near entrance) | only fight barks; no death line |
| G4_2_2 | `kill-omniphobia` Kill Omniphobia (totem) | no NPC row speaks for it: no line at all; The Raven's lines come when it spawns |
| G4_8b | `kill-torvian` Kill Torvian | no NPC row speaks for it: no line at all |
| G4_11_1b | `kill-mad-butcher` Kill Mad Butcher (big hut) | no NPC row speaks for it: no line at all |
| P1_1 | `kill-isolde-heldra` Kill Isolde & Heldra (east) | NPC rows without lines |
| P1_4 | `kill-godwin-sigbert` Kill Godwin & Sigbert | no NPC row speaks for it: no line at all |
| P1_5 | `kill-oswin` Kill Oswin, use the Ledger | has death lines, but the step goes on (use the Ledger, the reward) |
| P1_6 | `kill-wulfric-elswyth` Kill Wulfric & Elswyth | a pair: Elswyth's death lines may come while Wulfric lives; he has none |
| P2_1 | `kill-akthi-anundr` Kill Akthi & Anundr | no NPC row speaks for it: no line at all |
| P2_5 | `kill-vornas` Kill Vornas | no NPC row speaks for it: no line at all |
| P2_7 | `kill-bloodbilge` Kill Bloodbilge (vase) | no NPC row speaks for it: no line at all |
| P3_3 | `kill-rakkar` Kill Rakkar (north-west) | no NPC row speaks for it: no line at all |
| P3_4 | `kill-yeti` Kill the Yeti | no NPC row speaks for it: no line at all |
| P3_6 | `kill-stormgore` Kill Stormgore (far end) | only fight barks; no death line (the map version's row) |
| P3_7 | `kill-zolin-zelina` Kill Zolin & Zelina | a pair: 'Enough, we submit!' may come when Zolin falls first |

### Talks (15)

| Zone | Step | Reason |
| --- | --- | --- |
| G1_town | `town-talk-finn` Talk to Finn | which of Finn's talks the guide means is not clear |
| G1_town | `town-talk-all` Talk to all NPCs | several NPCs, no one line |
| G2_town | `caravan-asala-twice` Talk to Asala twice | two talks; the second (a reward) has no line |
| G2_town | `caravan-shambrin` Talk to Shambrin | Shambrin's talk for this step not identified |
| G2_town | `caravan-asala-horn` Talk to Asala | no Asala talk needs the horn sounded |
| G2_town | `caravan-asala-spires` Talk to Asala twice | two talks of unknown order |
| G2_10_2 | `mine-risu` Talk to Risu (cage) | no cage talk found; her 'break the lock' shout is used for Rudja |
| G3_town | `ziggurat-doryani-again` Talk to Doryani again | the second Doryani talk not identified |
| G3_12 | `kopec-alva` Talk to Alva (lore) | the lore talk not identified |
| G4_town | `kingsmarch-farrow` Talk to Farrow: Runeseeker | Farrow's talks depend on league content |
| MapHideoutShoreline_Claimable | `hideout-ange` Talk to Ange | her talk after the clear not identified; 'Intruders' comes before it |
| G4_4_3 | `navali-hinekora` Talk to Navali & Hinekora | the talk's zone (Halls of the Dead or the Trial) not clear |
| G4_2_2 | `journeys-end-freya` Talk to Freya | Freya's lines are in Tujen's summon talk (the step before) |
| G4_13 | `plunders-point-dannig` Talk to Dannig | summoned Dannig's talk not identified |
| P2_1 | `khari-risu` Talk to Risu | Risu's talk may be in the town, not in this zone |

### Other (105)

| Zone | Step | Reason |
| --- | --- | --- |
| G1_2 | `abandoned-stash` Abandoned Stash (campsite) | loot, chest or secret: no line |
| G1_4 | `grelwood-waypoint` Take the waypoint | waypoint: no line |
| G1_4 | `runic-seals` Activate 3 Runic Seals | no line at the third seal; Una's 'The Wound' is the next step's |
| G1_7 | `ancient-ruins` Sarcophagus by checkpoint | loot, chest or secret: no line |
| G1_8 | `forgotten-riches` Forgotten Riches cache | loot, chest or secret: no line |
| G1_9 | `haunted-treasure` Haunted Treasure secret | loot, chest or secret: no line |
| G1_11 | `hunting-grounds-waypoint` Take the waypoint | waypoint: no line |
| G1_11 | `small-ritual` Do the small ritual | no line found |
| G1_12 | `ritual-altars` Activate 3 Ritual Altars | no campaign line found (the King's ritual lines are endgame) |
| G1_12 | `ominous-altars` Cleanse the Ominous Altars | no campaign line found (the King's ritual lines are endgame) |
| G1_13_1 | `unas-lute` Una's Lute Box (house) | loot, chest or secret: no line |
| G1_13_1 | `crop-circle` Rare dogs in crop circle | loot, chest or secret: no line |
| G1_13_2 | `smithing-tools` Smithing Tools (house) | loot, chest or secret: no line |
| G1_13_2 | `blacksmiths-chest` Blacksmith's Chest | loot, chest or secret: no line |
| G1_14 | `gallows-secret` Rope on the hanging man | loot, chest or secret: no line |
| G2_1 | `vastiri-back-to-start` Portal back to the start | travel |
| G2_town | `caravan-to-traitors-passage` Caravan: Traitor's Passage | travel |
| G2_town | `sound-the-horn` Sound the Horn (top) | no line found |
| G2_town | `deliver-lihl-lima-letter` Letter to Shambrin | loot, chest or secret: no line |
| G2_10_1 | `faridun-war-cache` Faridun War Cache | loot, chest or secret: no line |
| G2_2 | `ancient-seal-door` Ancient Seal + Runic Seals | no line found |
| G2_2 | `bell-chest` Bell Chest in dead end | loot, chest or secret: no line |
| G2_3 | `summon-asala-gates` Summon Asala, open gates | no line found |
| G2_3 | `halani-stairs-down` Stairs down to checkpoint | travel |
| G2_13 | `trial-waypoint` Take the waypoint | waypoint: no line |
| G2_13 | `trial-pick-barya` Lvl 28: pick Deshar Barya | loot, chest or secret: no line |
| G2_13 | `trial-complete` Complete the trial | Balbala's 'It is done!' is said in a trial room, not known to be G2_13 |
| G2_5_1 | `shrine-of-bones` Shrine of Bones (Effigy) | loot, chest or secret: no line |
| G2_5_2 | `sun-clan-relic` Hyenas: Sun Clan Relic | loot, chest or secret: no line |
| G2_4_1 | `kabala-clan-relic` Snakes: Kabala Clan Relic | loot, chest or secret: no line |
| G2_4_1 | `abandoned-prayer` Abandoned Prayer (stairs) | travel: no line |
| G2_4_2 | `golden-tomb` Golden Tomb | loot, chest or secret: no line |
| G2_4_3 | `guarded-sarcophagus` Guarded sarcophagus | loot, chest or secret: no line |
| G2_4_3 | `choose-offering` Choose an Offering | loot, chest or secret: no line |
| G2_4_3 | `down-to-heart-of-keth` Down to the Heart of Keth | travel |
| G2_4_3 | `ignite-water-goddess` Ignite the Water Goddess | no line found |
| G2_6 | `valley-ancient-seals` Find 3 Ancient Seals | no line found |
| G2_6 | `relic-altar` Relic Altar: place relics | no line found |
| G2_8 | `fallen-dekhara` Fallen Dekhara (tower) | loot, chest or secret: no line |
| G2_8 | `the-unremembered` The Unremembered skeleton | loot, chest or secret: no line |
| G2_9_1 | `hushed-urn` Hushed Urn encounter | loot, chest or secret: no line |
| G2_9_2 | `sisters-of-garukhan` Sisters of Garukhan shrine | loot, chest or secret: no line |
| G3_1 | `orok-campsite-basket` Basket at Orok Campsite | loot, chest or secret: no line |
| G3_1 | `hanging-tree-corpse` Hanging tree corpse | loot, chest or secret: no line |
| G3_town | `take-the-gateway` Take the Gateway | travel |
| G3_3 | `jungle-grave-servi` Jungle Grave: summon Servi | no line found |
| G3_3 | `troubled-camp-gwendolyn` Troubled Camp (Gwendolyn) | loot, chest or secret: no line |
| G3_3 | `jungle-ruins-waypoint` Waypoint by Venom Crypts | waypoint: no line |
| G3_3 | `large-soul-core-altar` Soul Core on Stone Altar | no line found |
| G3_4 | `crypts-sarcophagus` Sarcophagus event | loot, chest or secret: no line |
| G3_4 | `orok-poison` Loot Orok Poison (corpse) | loot; Servi's talk there is optional and comes after |
| G3_4 | `a-reprieve` Head to A Reprieve | travel |
| G3_2_1 | `barrens-waypoint` Waypoint near Alva | waypoint: no line |
| G3_2_1 | `troubled-camp-sebastian` Troubled Camp (Sebastian) | loot, chest or secret: no line |
| G3_5 | `ravaged-camp-logbook` Ravaged Camp: Logbook | loot, chest or secret: no line |
| G3_10_Airlock | `temple-of-chaos-waypoint` Take the waypoint | waypoint: no line |
| G3_10 | `chaos-trial-complete` Complete the trial | no Trialmaster line found at the end |
| G3_6_1 | `small-soul-cores` Loot the other Soul Cores | loot |
| G3_2_2 | `pressure-pads` Step on the pressure pads | no line found |
| G3_2_2 | `bog-corpse-servi` Corpse by the Bog: Servi | loot, chest or secret: no line |
| G3_7 | `flameskin-rituals` Flameskin rituals | loot, chest or secret: no line |
| G3_8 | `drowned-quarters` Drowned Quarters rooms | loot, chest or secret: no line |
| G3_11 | `apex-cauldron` Mushrooms for the Cauldron | loot, chest or secret: no line |
| G3_14 | `sacrificial-heart` Heart from Vaal Goliaths | loot, chest or secret: no line |
| G3_14 | `utzaal-side-doors` Side doors: idols | loot, chest or secret: no line |
| G3_16 | `sacrificial-altar` Find the Sacrificial Altar | no line found |
| G3_16 | `place-the-heart` Place the Heart, stab it | no line found |
| G3_17 | `doryani-loot` Wait ~30 s for the loot | loot |
| G4_town | `shark-fin-kaimana` Shark Fin to Kaimana | where Kaimana takes the fin not settled |
| G4_3_1 | `petrified-pirate` Petrified Pirate (coast) | loot, chest or secret: no line |
| G4_3_2 | `beckoning-clam` Beckoning Clam: Pearl | loot, chest or secret: no line |
| MapHideoutShoreline_Claimable | `clear-hideout` Clear the hideout | no line found |
| G4_7 | `corpse-nest` Corpse Nest (Impaled Karui) | loot, chest or secret: no line |
| G4_5_1 | `chapel-key` Kill mobs for Chapel Key | loot |
| G4_5_1 | `armoury-cells` Armoury, Forgotten Cells | loot, chest or secret: no line |
| G4_1_1 | `flayed-sailor` Flayed Sailor (shore) | loot, chest or secret: no line |
| G4_1_1 | `doomed-mine` Doomed Mine (delve) | loot, chest or secret: no line |
| G4_1_1 | `beast-pen` Beast Pen | loot, chest or secret: no line |
| G4_4_1 | `waterfall-chest` Chest behind waterfall | loot, chest or secret: no line |
| G4_4_1 | `three-tests` Tests: Fire, Nature, Cold | no line found |
| G4_4_1 | `silent-hall` Pay Respects (Silent Hall) | loot, chest or secret: no line |
| G4_4_2 | `trial-of-ngakanu` Trial of Ngakanu: Tattoos | no line found |
| G4_2_1 | `dead-mans-chest` Dead Man's Chest or Stash | loot, chest or secret: no line |
| G4_2_1 | `abandoned-ship` Abandoned Ship (loot) | loot, chest or secret: no line |
| G4_2_2 | `journeys-end-freya-spikes` Spikes to Freya | not looked up |
| ExpeditionSubArea_Kalguur_Act4 | `summon-farrow` Summon Farrow | Farrow's lines depend on league content |
| G4_8b | `follow-lorandis` Follow Lorandis | no line found |
| G4_8b | `destroy-the-shield` Destroy the shield | no line found |
| G4_8b | `arastas-bells` Evening or Morning Bells | loot, chest or secret: no line |
| G4_10 | `excavated-cache` Excavated Cache | loot, chest or secret: no line |
| G4_11_1b | `ngakanu-abyssal-depths` Abyss: Abyssal Depths | loot, chest or secret: no line |
| G4_11_2 | `meeting-house-chests` Meeting House chests | loot, chest or secret: no line |
| P1_2 | `runed-megaliths` Activate 6 Runed Megaliths | not looked up |
| P1_4 | `soul-of-the-ferryman` Soul of the Ferryman | loot, chest or secret: no line |
| P2_1 | `molten-ones-gift` Molten One's Gift (NE) | loot, chest or secret: no line |
| P2_3 | `sanctuary-waypoint` Take the waypoint | waypoint: no line |
| P2_3 | `two-wishes` Two Wishes: place Baryas | not looked up |
| P2_6 | `orbalas-pillar` Orbala's Pillar: a Boon | not looked up |
| P2_6 | `summon-jado` Summon Jado, open the door | no line found |
| P2_7 | `sacred-wells` 2 Sacred Wells (vials) | loot, chest or secret: no line |
| P2_7 | `grand-barya` Grand Barya, Jado, Asala | no line found |
| P3_Town | `tusks-to-hilda` Icy Tusks to Hilda | not looked up |
| P3_1 | `ancient-monument` Find the Ancient Monument | not looked up |
| P3_5 | `elder-maddox` Find Elder Maddox | not looked up |
| P3_7 | `sealed-vaults` Sealed Vaults: Soul Cores | loot, chest or secret: no line |
