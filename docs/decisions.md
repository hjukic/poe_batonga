# Decisions and proposals

Recorded 2026-11-11. Scope requirements come from the project owner; the second
contributor's scope agreement is pending. Technical proposals below are not
measured results or completion of Phase 2.

| ID | Status | Decision and reason |
| --- | --- | --- |
| D01 | Owner requirement | Target Path of Exile 2 campaign leveling on Windows and Linux, including Hyprland testing. Validate Act 1 first to bound initial data and test coverage. |
| D02 | Owner requirement | Guide mandatory quest items and progress, with optional objectives selectable separately so detours do not obscure campaign requirements. |
| D03 | Owner requirement | Automatic checklist and layout guidance are the desired workflow; manual correction is recovery. Do not treat manual screenshot delivery as the intended product. |
| D04 | Hard constraint | GGG compliance overrides feature scope. No memory reading, injection, client modification, interception or automated game input. Unclear automatic analysis remains gated pending a documented permitted basis. |
| D05 | Proposed | Start the overlay experiment with Tauri 2, Rust and TypeScript. Rust hosts native adapters and shared logic; web UI handles display. Confirm only after step 5; Electron is a tested fallback candidate, not a guaranteed Wayland solution. |
| D06 | Proposed | Use local OS capture adapters: Windows.Graphics.Capture and Linux portal/PipeWire where supported. Initial system permission/source selection may be unavoidable; repeated manual screenshots are not required. |
| D07 | Proposed | Combine read-only verified log evidence with local visible-screen evidence. Neither logs nor APIs are assumed to expose position, inventory, seeds or quest completion until verified. |
| D08 | Proposed | Use evidence-based, reversible progress updates and explicit unknown states. Entering a zone does not prove an objective was completed. Persist character-scoped progress in SQLite. |
| D09 | Proposed | Match multiple layout hypotheses and suppress uncertain/stale guidance. Visible geometry supports estimates, not extraction of hidden map information; arrow feasibility remains open. |
| D10 | Proposed | Keep frames in memory and analysis local, with no cloud backend or image uploads. Plain versioned JSON route/layout data makes review and patch updates straightforward; schemas are step 9. |
| D11 | Deferred | Final toolkit/language selection is step 8 after the overlay experiment. Recognition libraries, accuracy thresholds, exact supported Linux modes and data sources require Phase 2 evidence. |
| D12 | Accepted by owner | Use Qutm as a reference for zone and dialogue/reward parsing, with our own core. Prioritize log-driven automatic progress and a layout panel; keep automatic positioning and the arrow as a separate investigation. See [review](qutm-review.md). |
| D13 | Owner requirement | Exclude inventory scanning, scan recording/replay and price checking. Continue quest-item and reward tracking using validated log/quest evidence. Acceptance of this direction does not establish log coverage or GGG approval. |
| D14 | Experiment result | Tauri 2.12.5 builds and launches the two-window prototype on Windows 10; interaction and visibility shortcuts update both UIs. Actual mouse pass-through, game stacking and Hyprland behavior still need verification; final toolkit selection remains deferred. See [overlay experiment](overlay-test.md). |
| D15 | Source selection | Use the pinned Domistae MIT original campaign text for future route adaptation, our own historical log observations for zone facts and project-authored schematic artwork for layouts. Preserve attribution and review patch compatibility; no third-party layout images or direct game-data extraction are selected. See [data sources](data-sources.md). |
| D16 | Policy evidence | GGG's developer policy explicitly permits informed log reading. Record that basis for the local read-only log workflow; continuous visual inference/navigation remains unresolved. See [map experiment](map-reading.md). |
| D17 | Experiment result | An offline fixed-scale matcher passes synthetic checks, but the default pixel/marker filters fail on two historical real minimaps. Keep live navigation disabled; improve real map-line/player separation and evaluate held-out examples before enabling an arrow. |

See [architecture](architecture.md) for data flow, experiment exit conditions and
official research references. If automation cannot meet compliance or feasibility
constraints, record an explicit scope revision agreed by both contributors.
