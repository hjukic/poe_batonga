# poe_batonga

A planned Path of Exile 2 campaign leveling overlay for Windows and Linux.
The target experience includes automatic progress tracking and layout-based
navigation, conditional on GGG compliance and technical feasibility.
An initial Tauri window experiment is available; game tracking is not connected.

## Run the overlay experiment

Install the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), then:

```sh
npm ci
npm run desktop
```

Use **Ctrl+Alt+I** for interaction and **Ctrl+Alt+O** to show/hide. The regular
controls window also provides recovery and quit buttons. See the
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
