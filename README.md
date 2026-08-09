# Libro

A cross-platform, pure-client media hub for books, audiobooks, and your personal library.

## Product authority

Product obligations and outcomes are defined in
[jrmoulckers/product](https://github.com/jrmoulckers/product). Cite obligations by stable ID
(for example `PROD-REL-001`); pin to a commit SHA when exact wording matters. Roadmaps, metrics,
experiments, and compliance evidence stay in this repository and cite the obligation they
satisfy.

Engineering mechanisms are defined in
[jrmoulckers/engineering](https://github.com/jrmoulckers/engineering), design and interface in
[jrmoulckers/studio](https://github.com/jrmoulckers/studio), and automation and shared agent
assets in [jrmoulckers/.github](https://github.com/jrmoulckers/.github).

See [`AGENTS.md`](AGENTS.md) for how libro applies these, including its recorded deviations.

Lint, format, and TypeScript configuration are consumed from `jrmoulckers/engineering` as
published packages rather than restated here — see
[docs/adopting.md](https://github.com/jrmoulckers/engineering/blob/main/docs/adopting.md).
Reading `@jrmoulckers/*` from GitHub Packages needs a `read:packages` credential; `.npmrc` sets
the registry but deliberately contains no token.
