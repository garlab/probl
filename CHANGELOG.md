# Changelog

All notable changes to the published crates are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, a release that breaks compatibility increments the minor version.

## [Unreleased]

### Fixed

- The `probl` crate's documentation linked to probl.dev, which doesn't exist; it now links to the repository.

## [0.1.0] - 2026-10-06

The first release.

- `probl`: compile Probl programs, run them by enumeration or by sampling, and read their reports as structured results with their bounds, sampling status and evidence.
- `probl-cli`: the `probl` command, with `run`, `check`, `schema` and `repl`.
- `probl-number`, `probl-syntax`, `probl-sema` and `probl-engine`: the internal crates that `probl` depends on, with no stable API.

The [language overview](docs/language-overview.md) and the [reference semantics](docs/semantics.md) describe the language, and [status and limits](README.md#status-and-limits) lists what isn't implemented yet.

[Unreleased]: https://github.com/garlab/probl/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/garlab/probl/tree/v0.1.0
