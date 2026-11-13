# Levelling benchmark: what fast players do and tell others to do

Collected on 2026-11-13 for the council review of the overlay (see
[the review](review-2026-11-13-levelling-council.md)). Summaries in our own words, not quotes.
Game version: Early Access 0.5.x ("0.5.5 The Forbidden Rites" is the newest named in these
sources); the app targets 1.0 (out on 2026-12-11), so zone details may be dated.

## Sources read

| Key | Source | What it is |
| --- | --- | --- |
| GTD-video | GuyThatDies, "I Spent One Month Making This Campaign Guide", YouTube `UdIsZHbhXOY`, 23 Apr 2026, ~47 min. Transcript read in full. | The route and tactics of one of the fastest players (he says: rank-one race winner): level 1 to maps in under 4 hours on a Mercenary (grenades), solo self-found, no stash. Every act, zone by zone. |
| GTD-build | GuyThatDies, "[0.5.5] Twister Monk Levelling", Mobalytics, updated 24 Oct 2026 (campaign in 3:25 with the Huntress version). | A speed-levelling build page: three level bands (1–14, 15–28, 29–58+), gear priorities, a vendor search regex, gem pickups listed by where they come from. |
| maxroll | ZiggyD (reviewed by Palsteron), "The Comprehensive League Start Leveling Guide", Maxroll, for 0.5.5, updated 8 Sep 2026. | A league-start guide that trades some speed for ease: mandatory / recommended / optional per act, gear checkpoints by character level. |
| codex | Mobalytics Campaign Codex layout guides (Lolcohol; Haddoq, Guitaraholic, CrimsonCasts, Angormus). Already used for the app's Act 1 zone guides: `docs/act1-zone-tips.md`. | Layout patterns per zone. |
| domistae | Domistae `poe2-leveling` interactive checklist (0.5.5). The source of the app's route files (`data/routes/*.json`, field `source`). | A step checklist per act; says its speedrun tips still need verification. |

## What the fastest guide (GTD-video) is made of

Counted by kind of advice, most frequent first. Examples are from Act 1 unless said.

1. **What to pick up, and what it is for.** Almost every zone names its guaranteed
   rewards and what they do for the run: a skill gem at the Clearfell campsite; a support
   gem from Areagne in the Grelwood ("a serious power spike": the first support roughly
   doubles boss damage); the next weapon tier from weapon racks in the Red Vale; support
   gems from the Grim Tangle boss and the Tomb of the Consort; a ring from the Cemetery
   sarcophagus; two passive points from Crowbell; spirit from the Freythorn boss; two
   passive points from Una's lute; the smithing tools and a chest in Ogham Village; max
   life from Candlemass. Things many checklists call optional are part of the fast route
   because they pay back in damage.
2. **What to skip.** Said just as plainly: skip the Mud Burrow; ignore almost all monsters
   on the way; leave Lachlann's ring on the ground (it forces a long cutscene and its
   unlock comes anyway); skip Balbala; skip abysses, the Trial of Chaos, the shark, the
   beetle; "only grab them if they're along your path" for minor rewards.
3. **Level targets before a zone or boss**, because of the experience penalty for being
   under-levelled ("in some cases you lose over half your experience"). Act 1: leave the
   Grim Tangle at 5+, 7 before Draven, 8 before Freythorn, 10 for the next weapon base in
   town, 13 before Ogham Manor, 14 before Geonor. Act 2: 16 for the crafted weapon, 18
   before Jamanra's first fight, 23 at the Valley of the Titans, 24½ leaving Deshar, 26
   before Tor Gul, 28½ before Jamanra. Act 3: 29 before town, 31 for the next gem level.
   He names places to catch up when behind (an abyss at Titan Grotto, the Flameskin ritual
   in Azak Bog with checkpoint resets: a level in about two minutes).
4. **Town trips with a plan.** "One of the biggest mistakes when rushing the campaign:
   staying in town too long." Before each trip decide what you need (movement-speed boots,
   damage gloves, a better weapon), do it, leave. Per trip he names the errands: identify
   rares before selling, which NPC gives what, a vendor check with a search regex, flasks
   at set levels, the next weapon base at a level.
5. **Boss openers and one or two mechanics per boss**: where to stand, when to hold the
   burst (bosses that teleport), which attack is the dangerous one. Short: two to four
   sentences per boss.
6. **Time-saving tricks tied to a place**: fight the Miller in the far corner so you can
   walk past the blockage into town; respawn at a checkpoint to clear monsters near a boss
   hut, to make a boss spawn at once, to skip a lift or a long dialogue; portal out the
   moment a quest state changes; drop a portal before the last lever; clicking a
   checkpoint travels to any other checkpoint in the zone; a movement skill to jump gaps;
   the log-out trick to get back to the caravan.
7. **Finding things in a zone** (the kind of hint the app has for Act 1): the dark-energy
   streams from an altar point to the remaining rituals in Freythorn; the Temple of Kopec
   stairs are in the far corner half the time; "hug the wall next to the ocean"; "hug the
   right wall"; a cracked floor line leads to the stairs. Fewer of these than of kinds 1–4.
8. **Crafting and currency moments**: use a transmute on the first good weapon base; save
   named currency for the level-16 weapon; when to regal/exalt; which rune goes where.
9. **Gem and link changes at named moments** ("after the Tomb of the Consort move
   multishot to …"). Build-specific; he points to a separate build guide for the rest.
10. **Zone "league mechanic" rewards**: each zone has a fixed reward; he does it only where
    the reward is worth it (a level-2 skill gem in the Red Vale, an exalt in the Hunting
    Grounds, a regal in the Cemetery) and points to a cheat sheet for the full list.

Also in it: manage the inventory while moving; the interludes' order (he does 2, 3, 1).

## What a speed-levelling build page (GTD-build) carries

- Three level bands, each with its own gear, gems and passive tree.
- **Gem pickups listed by place, not by level**: "Renly: …", "Clearfell: …", "The Rust
  King: …", "Lachlann: upgrade …", "Leitis: …", and the support gems by the boss or chest
  that gives them (Areagne, the Haunted Treasure in the Tomb of the Consort, the
  Executioner, the Gallows in the Manor Ramparts).
- Gear priority per slot in one line each (boots: movement speed first), and a regex to
  paste into the vendor search.
- Which choice to take at each quest reward that offers one (a venom, a medallion, the
  tests in the Halls of the Dead, a pillar).
- A start trick: make a second character to fetch a weapon and gems, then swap.

## What the easier league-start guide (maxroll) adds

- **Three tiers per act**: mandatory, recommended (with the permanent bonus named: a
  resistance, spirit, life, passive points), optional. It says outright that skipping the
  optional ones is fine and that the guide gives up some speed for ease.
- **Gear checkpoints by character level**: movement-speed boots tiers (10% from level 1,
  15% at 16, 20% at 33, 25% at 46), flask tiers by level (medium free in the Grelwood,
  greater at 10 from Una, grand at 16, …), resistance rings before named bosses (cold for
  Count Geonor; fire for Rudja), "Act 1 vendors cap at item level 15: check them before
  the Mawdun Quarry".
- **Town efficiency**: bundle turn-ins with shopping; keep a short list of current needs;
  vendors restock as you progress and after a level-up; ctrl-click an NPC to open the shop.
- **Tricks**: checkpoint travel within a zone; respawn at checkpoint resets the zone;
  portal out when a named line is spoken.
- **Boss prep**: hug bosses; resistance targets for a few fights; a timed fight to come
  back to if damage is low.

## Where the sources disagree with each other

- Mud Burrow / the Devourer: GTD skips it; maxroll recommends it (a level-2 gem).
- Areagne (Grelwood): GTD treats her support gem as a must; maxroll "recommended".
- Lachlann's ring, Balbala, the Trial of Chaos in the campaign: GTD skips; easier guides do them.
- What is "optional" depends on the build and on how fast the player wants to go.

## Not read yet

- Angormus: "Path of Exile 2 by Angormus in 2:55:32" at AGDQ 2026 (YouTube `LSxijR13aZ8`,
  a commentated run); his Red Vale and Ogham Farmlands layout guides, linked from the
  Mobalytics Act 1 layout guide. A March 2025 race write-up says he won a Warrior race to
  Doryani in 2h28 at level 28 using movement skills on a weapon swap; it gives no route.
- GuyThatDies: "PoE2: Act 1 Any% in 24 Minutes" (`d0Mt0YZpDuQ`, Nov 2026); his class runs
  ("Level 1 to Maps in 3:35", `Jla2a4H8XKI`, and others); his linked XP-penalty guide,
  campaign weapon-crafting guide, league-mechanic reward cheat sheet and vendor regex
  (links in the video description; not opened).
- Keyson, "Act 1 Layout guide and general strat" (`G_z8gRdOWqw`); CrimsonCasts' layout videos.
- Several "speedrun guide" pages on currency-seller sites were read and dropped: no author,
  no run, no specifics.
