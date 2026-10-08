# Changelog

All notable changes to the published crates are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, a release that breaks compatibility increments the minor version.

## [Unreleased]

### Added

- Failure modes ([semantics](docs/semantics.md#failure-modes)). A fault that depends on a world's values, like a division by zero or an index past the end, always ends that world. In partial mode the other worlds finish, and the result says how much failed and where; in total mode the run stops, as before. Choose with `@on_error total` or `@on_error partial`, `probl run --on-error`, `Options::on_error` or the playground's "On error" option.
- A partial result: `probl run` prints what the other worlds gave, with a `failed` section, and exits with status 3. `Program::run` still returns an error, with the result in `Error::partial`.
- In the `probl` crate: `FailureMode`, `Options::on_error`, `Program::failure_mode`, `Error::partial`, `Failure`, and `Outcome::failure_mode`, `failures`, `failed_share` and `finished`.

### Changed

- Sampling is partial by default: a run that faults no longer stops the whole sample, and the result is partial. Enumeration stays total by default. `--on-error total` or `@on_error total` keeps the old behavior.
- Explicit conversions out of range, like `prob(1.5)`, are now faults; a declared type that a value fails, like `let p: prob = 1.5`, still always stops the run.

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
