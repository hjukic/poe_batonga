# Campaign navigation rules: source audit

Reviewed 2026-11-12. Question: can authored campaign instructions support
navigation beyond Riverbank and Grelwood, including Acts 2–4 and the Interludes?

**Scope amendment, 2026-11-12:** the owner clarified that the product targets
**PoE 2 1.0, Acts 1–6**, without Interludes. Interlude findings below record the
earlier source audit only; they are not an implementation backlog. Revalidate
Act 1–4 candidates for 1.0. Act 5–6 rule coverage is still pending verified 1.0
material and observed gameplay; do not substitute old Interlude routes for it.

**Finding:** there is enough published material to start a per-zone rule catalog.
There is not yet evidence of a complete, current, machine-readable catalog that
automatically and reliably produces a heading for every campaign objective.
Geometric recognition and in-world visual cues need different detectors.

## Sources and actual freshness

| Source | Date/version checked | Useful content | Limits |
| --- | --- | --- | --- |
| Lolcohol / Campaign Codex: [Act 1](https://mobalytics.gg/poe-2/guides/campaign-layout-act-1), [Act 2](https://mobalytics.gg/poe-2/guides/campaign-layout-act-2), [Act 3](https://mobalytics.gg/poe-2/guides/campaign-layout-act-3) | Page dates: 2026-05-23. Each still warns about incoming 0.5 changes. | Zone patterns, visual clues, variants, confidence descriptions and exceptions. | A page timestamp is not a current-patch validation date for each rule. Act 3 still contains Cruel references and route assumptions that require reconciliation. |
| Lolcohol: [Act 4](https://mobalytics.gg/poe-2/guides/campaign-layout-act-4) | Page date: 2025-12-08. Text mixes 0.3 descriptions and 0.4 amendments. | General routes and links to detailed runner-authored documents. Credits CrimsonCasts, GuyThatDies, Lundburgerr, Angormus and FireMCG. | Not a verified 0.5/0.5.5 catalog. Some zones explicitly lack a simple solution. |
| Lundburgerr: [Whakapanu Sorted layouts](https://docs.google.com/document/d/e/2PACX-1vR9HqTCBAh23yQGVby1X4LSfZq0NGIQ9xrIFEI25UM1u-rJqy2vM9IB74MUmXxgieSjtqFm5noqJQN1/pub) | Public document read; no reliable last-edit date or patch tag visible. | Eight layout families, rotated/mirrored examples and a branching recognition procedure using features such as early crab beach, hooks and middle-section length. | Undated candidate rules; diagram metadata also includes game terrain paths. No extraction workflow or image reuse permission is inferred. |
| Lundburgerr: [Singing Caverns Sorted Layouts](https://docs.google.com/document/d/e/2PACX-1vSvXE2KPkz_QJWS9RlQYMp6Wxn7dLno71hM4q0HbPWQmHuUjAo18obJXUDgy8JRW5yU3t8SdLon9ML0/pub) | Public document read; last-edit date/patch not established. | Four layout families and an explicit decision procedure based on the waypoint-side road and central lake geometry. | Requires recognizing anchors and several features; neither current-patch validity nor detector performance measured. |
| Angormus: [Abandoned Prison](https://docs.google.com/document/d/1CNtBGap-1ujlNNI4kS0RFiSvhNpv45nok2X5Zb0dXQ4/edit) | Linked by the Act 4 guide. The document body could not be read with the available web reader. | Primary-author lead for the four variants described by Mobalytics. | Do not claim to have verified the document's algorithm, date or patch. The advanced Red Vale and Farmlands Mobalytics links also returned no readable body in this review. |
| [Kami-Guru / Path of Levelling 2](https://github.com/Kami-Guru/path-of-levelling-2) | Revision `4fb074906b2dbdd6b60d97c07cd8957efb6b869c`; release 0.1.45 on 2026-05-28. Act 2 source notes last changed 2026-05-27; generated reference notes 2026-05-23. | Editable per-zone text, geometry hints and the 0.5 campaign-cue additions. MIT repository. | Inspected generated JSON has 69 note entries; reference table has 70 entries, including only three Interlude chapter summaries, not all Interlude zones. Riverbank text still names Hillock, demonstrating the need for review. Source TXT and generated JSON have different revision dates. |
| [CobraWeed / POE2LevelingGuides](https://github.com/CobraWeed/POE2LevelingGuides) | Revision `0bc1f50ecd34b84c2313b91aff51f18113dbee6a`; Generic JSON last modified [2026-09-02](https://github.com/CobraWeed/POE2LevelingGuides/commit/e21940dc8baf3d2a1ecf43712cd0e42b039d9949). README targets 0.5. | Four main-act and three Interlude groups; 109 guide pages, with some direction hints. Credits build information to GuyThatDies and FubGun. | Many pages are objectives or build/town advice rather than geometric rules. Even the Generic guide contains wolf-form movement advice; it needs filtering for a universal companion. No explicit repository reuse licence identified. |
| [UmbraMalik / Act Companion](https://github.com/UmbraMalik/poe2-act-companion-overlay) | Revision `b22ee1e638471a4931dc4bc66561f0ef62c03f70`; `src/data/guide.json` last modified [2026-10-11](https://github.com/UmbraMalik/poe2-act-companion-overlay/commit/b2d482f4cd251eae5cdd214dc334717049a84d0e). Release 0.6.1 dated 2026-10-25. | Inspected JSON contains 93 cards: 70 across Acts 1–4 and 23 across the three Interludes. Includes updated zone transitions, objectives and some landmarks. | Cards are not 93 solved navigation rules. Interactive maps in the release are area/route graphs driven by the log, not live within-zone localization. No explicit repository reuse licence identified. |
| [domistae / poe2-leveling](https://github.com/domistae/poe2-leveling) | Revision `f682e2b99c4a5a4f31882f3674bba4dcefc6ab46`; Act pages touched 2026-11-08 by a substantial UI redesign. Targets 0.5.5; reconciliation still in progress. | Acts 1–4 and Interlude routing, some explicit cue blocks. Original text/code under MIT; already a selected route reference in this project. | Author describes an AI-assisted, multi-source compilation. Recent redesign is not proof that every navigation fact was independently retested. Use as an index/cross-check, not independent confirmation of repeated community claims. |

The runner documents are the strongest discovery for converting instructions
into branching recognition logic. The more recently maintained overlays are
useful for current campaign order and gaps, but their newer dates do not by
themselves establish stronger layout knowledge. No videos were watched end to
end, and no new current-patch runs were recorded during this source audit.

## Initial rule candidates by act

These are brief factual summaries and proposed detector assignments. None are
validated runtime rules. Source confidence labels describe the author's judgment,
not measured probability or our detector confidence.

| Act / zone | Candidate relationship | Observation needed | Source / status |
| --- | --- | --- | --- |
| 1 / Grelwood | Use the diamond-like POI arrangement as a hypothesis; distinguish objectives with associated clues. | Confirmed POI identities and local geometry; roots/mushrooms require scene recognition. | Mobalytics Act 1; common pattern, not universal guarantee. |
| 1 / Hunting Grounds | Directional environmental clues associate different destinations with a signpost, tracks or a locust trail. | In-world signs and motion, plus visible map anchors. | Mobalytics Act 1 / Kami notes; current-patch test required. |
| 2 / Traitor's Passage | A junction near the lore pillar splits toward Balbala and onward progression; wall seals indicate the boss branch. | Pillar/book marker and in-world paper seals. | Mobalytics Act 2; exceptions and weak global direction documented. |
| 2 / Halani Gates | The first bridge supplies an orientation reference for the narrow connected route. | Entrance, bridge axis and traversable passage. | Mobalytics Act 2; promising geometry candidate. |
| 2 / Lost City | Recognize the connecting bridge between two larger sections, then search the later section's far boundary. | Bridge and region connectivity; not just a compass bearing. | Mobalytics Act 2; approximate pattern. |
| 2 / Bone Pits | The entrance-side bone arch indicates the boss direction. | In-world bone arch and its orientation. | Kami Act 2 TXT; candidate cue, not verified here. |
| 3 / Jungle Ruins | NPC interaction can reveal the onward destination; visible clues must override generic edge-search advice. | Revealed map marker; confirmed NPC interaction. | Mobalytics Act 3 / 0.5 cue notes; route needs patch reconciliation. |
| 3 / Chimeral Wetlands | Entrance/arena-facing shapes can distinguish destinations even when their positions differ. | Oriented landmark shape and known entry. | Mobalytics Act 3; do not hard-code one wall for every variant. |
| 3 / Jiquani's Machinarium | The second section's main corridor suggests the onward passage; the soul-core side is not solved by this. | Section boundary, corridor axis, visible objective markers. | Mobalytics Act 3; separate search from directional evidence. |
| 4 / Whakapanu | Narrow eight candidate families using early beach, hook and path-shape observations. | Multiple geometric features and established local frame. | Lundburgerr document; explicit decision tree, undated. |
| 4 / Singing Caverns | Narrow four families using the initial road and central lake, then choose the relevant side/route. | Waypoint anchor, road and lake boundary. | Lundburgerr document; explicit decision tree, undated. |
| 4 / Abandoned Prison | Four variants require recognition of multiple tells. | Detailed template/clue review still needed. | Mobalytics links Angormus; primary body unreadable here. |
| 4 / Solitary Confinement | Follow the boundary associated with incoming waves. | Scene water/wave recognition and map boundary association. | Mobalytics Act 4 / Kami notes; scene detector needed. |
| Interludes / Kriar Peaks | Owls and a likely sector are candidate cues for Elder Maddox. | In-world owl recognition and confirmed orientation. | Inspected Umbra guide; unverified, not a fixed absolute heading. |
| Interludes / remaining zones | Most reviewed entries establish what to complete and which area follows, not the exact branch to take. | A zone-specific rule or an explicit exploration policy. | Coverage gap remains. A route card is insufficient to authorize a directional arrow. |

## How to turn the research into useful navigation

Proposed catalog entry: zone ID, objective, source/revision, patch tested,
preconditions, required observations, branching conditions, next intermediate
goal, exceptions, and status (`candidate`, `verified`, `retired`). Store the
author's stated strength separately from live recognition quality.

Use a small set of reusable behaviors: follow a connected corridor, follow an
identified boundary, cross a confirmed bridge, classify a landmark/template,
follow a visible directional cue, or search unexplored reachable branches. Each
zone selects and sequences these behaviors; a single global compass rule cannot
cover all objectives.

Our current capture analyses only the top-right minimap. It does not yet recognize
world-scene clues such as wall seals, bone arches, mushrooms, insects, waves or
owls. Those instructions can initially be displayed as hints, but cannot become
automatic bearings until the required observations have real detectors.

For an unresolved zone, label a suggested unexplored passage as exploration.
Keep a heading toward a known target distinct from a search recommendation.
Suppress guidance when placement, connectivity or the applicable rule is unknown.

Build and validate the Act 1–4 catalog first, then add verified Act 5–6 rules for
the six-act 1.0 release. Do not spend implementation effort closing historical
Interlude gaps. Evaluate on independent instances and revalidate after patches.
No source files were added to the runtime dataset and no navigation code
was changed in this research task. Downloaded inspection copies remain under the
ignored `experiments/layouts/output/rule-review/` folder.
