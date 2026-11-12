# poe_batonga

A planned Path of Exile 2 campaign leveling overlay for Windows and Linux.
The release target is **PoE 2 version 1.0, Acts 1–6**. Interludes are outside the
product scope. Existing Early Access guides are research references; rules and
objectives must be validated for 1.0, including new Act 5 and Act 6 coverage.
The target experience includes automatic progress tracking and layout-based
navigation, conditional on GGG compliance and technical feasibility.
An initial Tauri overlay is available. It shows a checklist for the zone you are in
(The Riverbank, Clearfell Encampment, Clearfell) and ticks objectives automatically
by reading the game's `Client.txt` locally (read-only); steps without a known log
line are ticked by hand. The checklists and their triggers are in
[data/routes/](data/routes/), one file per zone (steps from Domistae's Act 1 guide).
Set `POE2_CLIENT_LOG` if the log is not found in a default Steam or standalone path.

## Run the overlay experiment

Install the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), then:

```sh
npm ci
npm run desktop
```

**Ctrl+Space** shows and hides the overlay; change it under the settings (gear)
button. The lock button unlocks the panel to move it, open settings or tick a step
by hand; while locked, only the lock button takes clicks. The restart button (unlocked)
pulls the latest commit, runs `npm run desktop:build` and replaces the running app
with the new build; build output goes to the app's terminal, and a failure leaves the
current version running with the reason under the top bar. On Wayland the overlay is
a layer-shell surface, so it stays above a fullscreen game and passes clicks through;
this needs the `gtk-layer-shell` system library (Arch: `pacman -S gtk-layer-shell`).
On Hyprland the global shortcut fires while the game (an XWayland window) has focus. See the
[test procedure and results](docs/overlay-test.md) for Windows and Linux checks.

Windows minimap capture is available for windowed/borderless PoE 2 and starts
automatically with the application. It pauses when the game loses focus or the
minimap is covered, then resumes when conditions allow. There is no read-map
toggle. See [Windows minimap support](docs/windows-minimap.md) for limits
and outstanding gameplay verification.

`npm run build` validates the frontend; `npm run desktop:build` builds the native
executable. Native/game behavior requires the manual checks above.

The separate [offline map experiment](docs/map-reading.md) runs with
`npm run vision:test` and `npm run vision:experiment`. Its real-image results are
not sufficient for game navigation; it is not connected to the overlay.

## Project documents

- [Plan](PLAN.md): ordered work and completion criteria.
- [Scope](docs/scope.md): requirements, boundaries and conditional features.
- [Architecture](docs/architecture.md) and [decisions](docs/decisions.md): proposed
  design and experiments needed before implementation.
- [Documentation](docs/README.md): documentation status and repository setup checks.
- [Data sources](docs/data-sources.md): route, zone and layout data, including
  poe2way's zone terrain (walls and points of interest) for the first zones in
  `data/layouts/poe2way/`; `node scripts/render-terrain.mjs` draws it.

## Contributing

Clone the repository and create a branch for your change:

```sh
git clone https://github.com/hjukic/poe_batonga.git
cd poe_batonga
git switch -c your-change
```

After editing, commit the relevant files and push your branch:

```sh
git add <changed-files>
git commit -m "Describe your change"
git push -u origin your-change
```

Open a pull request into `main` on GitHub. Collaborators need write access before
they can push branches to this repository.

### Version number

The overlay shows the app version bottom right (`v0.1.1`). It comes from `version` in
`package.json` (Tauri reads the same field). Raise the patch version
(`0.1.0` → `0.1.1` → `0.1.2` → …) with every update you push, and name it in the
commit message, so a running app shows whether it has the latest change.
