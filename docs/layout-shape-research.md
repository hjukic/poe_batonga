# Campaign map-shape sources

Reviewed 2026-11-12 for the owner's request to find real map outlines suitable
for comparison with our recorded minimap. This is source research, not a change
to the live matcher or evidence of navigation accuracy.

## Best candidate: Exile-UI Act-Decoder

Reviewed repository revision: `f77e6e861d6dbca07eb523fd67e42b8f50f26f29`.

- [PoE 2 image inventory](https://github.com/Lailloken/Exile-UI/blob/f77e6e861d6dbca07eb523fd67e42b8f50f26f29/img/GUI/act-decoder/file-list%202.json)
- [Act-Decoder documentation](https://github.com/Lailloken/Exile-UI/wiki/Act%E2%80%90Decoder)
- [Repository licence](https://github.com/Lailloken/Exile-UI/blob/f77e6e861d6dbca07eb523fd67e42b8f50f26f29/LICENSE.md)

The GitHub tree contains 332 JPGs in `img/GUI/act-decoder/zones 2/`: one
explanation and 331 clue/layout/example images. Filenames cover 61 area IDs.
Act 1 accounts for 109 images across 14 IDs. These are image counts, not counts
of distinct complete generated layouts.

There is **no `g1_1` Riverbank image** in this inventory. Its Act 1 coverage begins
with `g1_2` (Clearfell). Similar-looking `g1_11` filenames are other areas, not
Riverbank variants. Clearfell has eight images combining clues and fuller maps.

Visually inspected examples:

| Asset | What it actually contains |
| --- | --- |
| [Clearfell clue, `g1_2 1.jpg`](https://raw.githubusercontent.com/Lailloken/Exile-UI/f77e6e861d6dbca07eb523fd67e42b8f50f26f29/img/GUI/act-decoder/zones%202/g1_2%201.jpg) | Partial white wall outlines, a highlighted bridge and arrival marker. |
| [Clearfell fuller layout, `g1_2 1_1.jpg`](https://raw.githubusercontent.com/Lailloken/Exile-UI/f77e6e861d6dbca07eb523fd67e42b8f50f26f29/img/GUI/act-decoder/zones%202/g1_2%201_1.jpg) | Detailed outer boundary, interior walls/bridges and annotated points of interest. A useful candidate for contour-matching experiments. |
| [Mud Burrow example, `g1_3 1.jpg`](https://raw.githubusercontent.com/Lailloken/Exile-UI/f77e6e861d6dbca07eb523fd67e42b8f50f26f29/img/GUI/act-decoder/zones%202/g1_3%201.jpg) | Detailed rooms and corridors with orange border. |

The documentation identifies orange-bordered images as non-conclusive examples,
and describes manual comparison/selection and rotation/mirroring. This collection
does not itself implement automatic geometric matching. The
[2026-06-30 changelog](https://github.com/Lailloken/Exile-UI/blob/f77e6e861d6dbca07eb523fd67e42b8f50f26f29/data/changelog.json)
warns that PoE 2 Act-Decoder content remains largely outdated. Patch compatibility
must therefore be established per candidate, on separate observed runs.

The repository has an MIT licence. Image-specific provenance is less simple:
the author [describes collecting examples from Campaign Codex and personal runs,
then editing them](https://www.reddit.com/r/PathOfExile2/comments/1pi9pue/campaign_efficiency_zone_layouts_for_the_whole/).
The inspected images also contain game icons. A separate image redistribution
grant was not verified. Samples remain local research files, excluded from Git;
no third-party image was added to the application's dataset.

## Other leads

| Source | Finding and usefulness |
| --- | --- |
| [Mobalytics / Lolcohol, Act 1](https://mobalytics.gg/poe-2/guides/campaign-layout-act-1) | Generalised layout patterns. The inspected Clearfell image is a simple polygon with objective icons, not detailed minimap walls. The author explicitly distinguishes these patterns from exact generated instances. The reviewed page has no Riverbank layout section. Useful for route hints rather than precise contour alignment. |
| [nicolasbagatello/poe2-helper](https://github.com/nicolasbagatello/poe2-helper) at `1da5c1bc919c1ef50084acab40c9349c58b718a2` | Packages Mobalytics images. The inspected Clearfell asset is the same generalised style. Its zone data starts at Clearfell, with no Riverbank/Miller entry found. No additional detailed Riverbank template or explicit image reuse licence established. |
| [Campaign Codex announcement](https://www.reddit.com/r/PathOfExile2/comments/1hy1j2d/poe_2_campaign_codex_decoding_the_path_of_exile_2/) | Community collecting campaign screenshots and navigation clues, and a provenance lead for Exile-UI. Discord-hosted material was not inspected; no Riverbank coverage claim can be made from the public announcement. |

## Proposed experiment

1. Start an offline contour-matching trial with the detailed Clearfell example:
   isolate walls from labels/icons, preserve screen axes, and search continuous
   scale/rotation plus reflection. Score only the explored region of our map.
2. Require several independent distinctive features (for example a bridge and a
   boundary bend), keep rival hypotheses, and reject mismatches or weak evidence.
   A visually similar section alone must not become a confident navigation arrow.
3. For Riverbank, retain automatically recorded explored geometry from our own
   runs and label arrival, arena and exit on completed visits. Keep instances and
   patches separate; evaluate on runs not used to build reference templates.
4. Report accepted-match accuracy and coverage before connecting a new matcher
   to live guidance. External examples are research candidates, not a guarantee
   that a generated instance can be matched exactly.

Local inspected samples and API inventories are in the ignored
`experiments/layouts/output/source-review/` folder. No application code was
changed by this research.
