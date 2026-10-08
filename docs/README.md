# Probl documentation

Start with the [repository README](../README.md) to install and run Probl, or the [language overview](language-overview.md) for a guided tour. The [reference semantics](semantics.md) is authoritative when descriptions disagree.

## Using Probl

| Document | Read it for |
|---|---|
| [Language overview](language-overview.md) | Syntax, distributions, evidence, reports, built-ins and links to every example |
| [Reference semantics](semantics.md) | Types, identity, inference, approximation and resource contracts |
| [Reading data](data-input.md) | Typed CSV, JSON and line input; schemas, limits and host authority |
| [Rust library](library.md) | Compile/load/run, structured results, uncertainty and embedding |
| [Inference](inference.md) | Supported conjugate updates, likelihood-weighting limitations and deferred methods |
| [Use cases and gaps](use-cases.md) | Decisions, queues, predictive checks, monitoring and correlated risks |

The [examples directory](../examples) contains runnable models with expected output. The roadmap example is illustrative, and the BB84 example is a classical probability model of a specified attack, not a quantum-amplitude engine.

## Working on Probl

- [Contributing](../CONTRIBUTING.md): bug reports, checks before a pull request, where tests go and commit messages.
- [Architecture and priorities](architecture.md): crate responsibilities, execution model, host boundaries and next work.
- [Testing](testing.md): commands, regression contracts, independent checks and remaining coverage gaps.
- [Playground](playground.md): local development, browser tests and deployment.
- [Benchmarks](benchmarks.md): dated measurements and the optimizations they motivated; these are not current performance guarantees.
- [Package publication](library.md#packaging-and-publication): the published crates, internal version pins and how to release.

## Research and historical rationale

- [Imports and model tests](design/imports-and-tests.md): static reusable definitions, acyclic dependencies, isolated `.test.probl` files and explicit assertions over weighted populations.
- [Error handling](design/error-handling.md): a near-term proposal for specific local catches and total/partial failure modes (partial by default when sampling), with explicit failure accounting and partial-result reporting.
- [Portals and external effects](design/portals-and-effects.md): a deferred proposal for an implicit prologue, read-only joint population views and midway host interaction; better reporting comes first.
- [Symbolic inference](design/symbolic-inference.md): assessment of the Dice paper and an evaluation plan; no symbolic backend is implemented.
- [Complex values and quantum simulation](design/quantum-and-complex.md): the implemented scalar foundation and an unimplemented quantum-engine proposal.
- [Original project audit](design/project-audit.md): historical findings against a specific revision, with a [resolution map](architecture.md#original-audit-resolution-map). It is not a list of current vulnerabilities.

Superseded proposal reviews and the pre-redesign operator survey were removed during curation. Their settled contracts are in the reference and regression suites; their historical text remains in Git. Outstanding design questions are consolidated in the architecture, use-case and inference documents.
