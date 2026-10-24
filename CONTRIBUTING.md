# Contributing

Contributions of every kind are welcome: bug reports, restoration work, fixes, documentation, or your
own gameplay built on moly.

## Ways to contribute

- **Report a problem**: open an [issue](https://github.com/empty-sekai/moly/issues) with your
  platform, version, the steps to reproduce it and the relevant lines of the game or launcher log.
- **Work in your own fork**: anyone can fork the repository, change it freely and ship their own
  version. Open a pull request when you want a change merged into the main line.
- **Work on a branch of this repository**: members with write access can create and push branches
  here directly. `master` takes changes through pull requests only; other branches are open.

Suggested branch names:

| Prefix | Use |
|---|---|
| `feat/<topic>` | a feature meant for `master` |
| `fix/<topic>` | a fix meant for `master` |
| `docs/<topic>` | documentation |
| `play/<name>/<topic>` | your own gameplay branch; it may live on its own and need not be merged |

## Original behaviour and extended gameplay

Changes fall into two groups with different requirements:

- **Original behaviour**: sites, characters, furniture, weather and screens are restored as the
  original game has them. The standard for these changes is "the same as the original". In the pull
  request, state what the change is based on and how you checked that the result matches.
- **Extended gameplay**: gameplay the original does not have, such as new character behaviour,
  multiplayer or lifted limits. On `master` it must be an option that is off by default, and turning
  it off leaves the original behaviour unchanged. It can also stay on your own branch or fork.

See the [roadmap](ROADMAP.en.md) for direction and plans.

## Game resources

Neither the repository nor the release packages contain game resources: models, textures, motions,
audio, master data and anything derived from them are never committed. Tests use synthetic or
redistributable inputs and run without game resources. Run the open-source boundary check before
committing:

```sh
node tools/boundary/check.mjs
```

## Commits and pull requests

- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) in English
  with a lowercase type, for example `fix(launcher): keep the resource directory across upgrades`.
  Describe the change and its visible effect.
- Keep one pull request to one change; the static checks must pass before it is merged.
- Local checks, tests and CI are described in [Development](docs/development.md); building from
  source is in [Building](docs/building.md).

## License

Contributions are licensed under the repository's [AGPL-3.0-only](LICENSE) license. If you
distribute a modified version, or let others use one over a network, the license requires you to
offer them its source code.
