# poe_batonga

A planned Path of Exile 2 campaign leveling overlay for Windows and Linux.
The target experience includes automatic progress tracking and layout-based
navigation, conditional on GGG compliance and technical feasibility.
An initial Tauri overlay is available. For The Riverbank it ticks objectives
automatically by reading the game's `Client.txt` locally (read-only); the route
and its triggers are in [data/routes/riverbank.json](data/routes/riverbank.json).
Set `POE2_CLIENT_LOG` if the log is not found in a default Steam or standalone path.

## Run the overlay experiment

Install the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), then:

```sh
npm ci
npm run desktop
```

**Ctrl+Space** shows and hides the overlay; change it under the settings (gear)
button. The lock button unlocks the panel to move it, open settings or tick a step
by hand; while locked, only the header buttons take clicks. On Wayland the overlay is
a layer-shell surface, so it stays above a fullscreen game and passes clicks through;
this needs the `gtk-layer-shell` system library (Arch: `pacman -S gtk-layer-shell`).
On Hyprland the global shortcut fires while the game (an XWayland window) has focus. See the
[test procedure and results](docs/overlay-test.md) for Windows and Linux checks.

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
- [Data sources](docs/data-sources.md) and [third-party notices](THIRD_PARTY_NOTICES.md).
- [License](LICENSE): MIT license for this repository's original work. External
  routes, game data and images need their own source and license review before use.

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
