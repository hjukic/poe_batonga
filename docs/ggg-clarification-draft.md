# Draft: clarification of PoE 2 companion behavior

Not sent. This is a technical description to review before contacting GGG.

We are developing an independent Windows/Linux PoE 2 campaign companion. It would
run as a separate desktop overlay without injection, memory access, network
interception or game-file modification. It would not send game inputs, move the
character, pick up items, interact with quests or automate combat. No inventory
scanner is planned.

The intended behaviors are:

- Read Client.txt locally, with the user's knowledge, for zone/level and explicit
  reward/dialogue evidence. Update only the companion's own checklist state.
- Subject to clarification, use OS-authorized local capture of the visible game
  map, detect the player marker and compare explored geometry with independently
  authored layout templates. No gameplay frames would be uploaded.
- Display an estimated direction towards a known campaign objective, clearly
  abstaining when the match is uncertain. No hidden instance data would be read
  and no unexplored geometry would be extracted from the client.

Questions:

1. Is continuous local screen analysis of the visible map permitted for this
   independent, non-interactive overlay?
2. Does displaying a direction inferred from visible explored geometry and known
   templates cross a restriction on navigation assistance or in-game advantage?
3. Is automatic updating of the companion's own quest checklist from permitted
   log/visible UI evidence acceptable without generating any game inputs?
4. Are there additional restrictions on these exact behaviors or on distributing
   the application? If you cannot approve tools individually, is there published
   guidance addressing these behavior categories?

Reference: https://www.pathofexile.com/developer/docs/index#policy
