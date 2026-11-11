# Data sources (Phase 2, step 6)

Source selection completed 2026-11-11. The machine-readable catalog is
[`data/sources.json`](../data/sources.json). Selection does not mean current-patch
content or complete Act 1 data has been validated; that belongs to steps 7 and 9.

| Data | Selected source | Terms / limits |
| --- | --- | --- |
| Campaign route | Original authored campaign instructions in [domistae/poe2-leveling](https://github.com/domistae/poe2-leveling/tree/f682e2b99c4a5a4f31882f3674bba4dcefc6ab46), starting with `poe2_act1_guide.html` | MIT; preserve copyright/license and link attribution. Only original text, no external images/extracts. |
| Zone IDs/names | Our own factual observations from the owner's local PoE 2 log | Export selected area metadata only; do not redistribute the raw log. Historical observations are not authoritative game exports. |
| Layout pictures | Contributor-authored abstract schematic diagrams from labelled explored-map observations | Project-original artwork under MIT; no third-party image copying/tracing. Game assets remain subject to their own rights. Diagrams are templates, not exact generated instances. |

## Route reference

Reviewed revision: `f682e2b99c4a5a4f31882f3674bba4dcefc6ab46`. The upstream
[README](https://github.com/domistae/poe2-leveling/blob/f682e2b99c4a5a4f31882f3674bba4dcefc6ab46/README.md)
explicitly covers original text under MIT and asks for source attribution. It
targets patch 0.5.5 and flags reconciliation still in progress, so we must review
each adapted objective. Keep mandatory progression, optional rewards and optional
detours distinct; never infer completion from a guide's route order.

No route text is imported yet. The licence is preserved in
[`licenses/domistae-MIT.txt`](../licenses/domistae-MIT.txt), and
[`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md) records attribution. Import a
reviewed snapshot when defining the route format in step 9, not mutable HTML at
application startup. Exclude upstream endgame/crafting content from this app.

## Collected zone observations

[`data/observations/campaign-zones.json`](../data/observations/campaign-zones.json)
contains 97 campaign area IDs collected from 3,072,602 local log lines spanning
2025-09-09 to 2026-09-10. It stores observed scene names, levels and generation
record counts. No character names, chat, network addresses, full log lines or
dialogue are exported. An empty name list means no nearby scene-name observation
was paired; it does not mean the zone has no name.

Reproduce offline with:

```sh
node scripts/collect-zone-observations.mjs "/path/to/logs/Client.txt" data/observations/campaign-zones.json
```

The collector accepts structured client log entries, pairs generation with a
nearby non-null scene name from the same client process and exports only campaign
ID prefixes. This is a research utility, not the application's live tailer. Do
not hard-code observed area levels as immutable: patches/variants may change
them. Unknown IDs should display the ID and keep unsupported route/layout states
explicit. See [log findings](log-findings.md) for what the file actually supplies.

## Layout references and unselected candidates

- [Exile-UI](https://github.com/Lailloken/Exile-UI) has an MIT repository licence,
  but its [changelog](https://github.com/Lailloken/Exile-UI/blob/main/data/changelog.json)
  warns of outdated PoE 2 Act-Decoder layouts. Its game screenshots also require
  separate provenance consideration. Do not import that image collection as our
  initial dataset.
- [Qutm](https://github.com/Vyary/qutm) is Apache-2.0 and useful as a reference;
  we have not selected its route/diagram assets for import. Diagram coordinates
  are authored positions, not live player coordinates.
- The exact Exile Leveling fork in the original plan was not identified and its
  PoE 2 applicability/licence was not established. It is not selected.
- PoE 2 wiki copyright pages could not be verified in this review. Do not assume
  the site's text or images are MIT; use it as a research lead only until exact
  page and asset terms are checked. No wiki material is imported.
- Direct game-data extraction is not needed for the initial zone catalog and
  remains outside the selected source workflow.

Each future schematic needs author, zone ID, game patch, observed variant and
confidence. No layout images have been created or imported yet. Offline tests may
use synthetic maps, clearly labelled synthetic; they cannot establish performance
on real gameplay. Live automatic capture/matching still needs the step-7 review.

## Remaining work

Step 6's source-choice criterion is met. Current-patch verification, Act 1 route
adaptation, the route/layout schemas and actual schematic coverage remain open.
The overlay is still a preview and does not consume this research catalog.
