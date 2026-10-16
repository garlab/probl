# Releases

Probl releases are built from version tags on commits that belong to `main`.
The tag must match the workspace version. A release goes through automated tests
and packaging checks before it is published.

## What a release contains

The [release workflow](../.github/workflows/release.yml) publishes:

- The Rust library and CLI crates on [crates.io](https://crates.io/crates/probl).
- A container image at `ghcr.io/garlab/probl`, built and tested on Linux x86-64
  and ARM64 runners.
- A complete source archive on [GitHub Releases](https://github.com/garlab/probl/releases),
  including the workspace's `Cargo.lock`.

Each GitHub release includes:

| Asset | Contents |
| --- | --- |
| `probl-X.Y.Z-source.tar.gz` | Source files from the release commit |
| `release.json` | Version tag, source commit, archive filename and SHA-256 |
| `SHA256SUMS` | SHA-256 checksums of the archive and manifest |
| `probl-X.Y.Z-browser.json` | The engine and playground for browsers ([browser distribution](#browser-distribution)) |
| `probl-X.Y.Z-browser.json.sha256` | SHA-256 checksum of the browser bundle |

These assets receive build-provenance attestations identifying the repository,
workflow, tag and commit that produced them. Assets are attached before the
release is published; release immutability then prevents changes to its assets
and tag. The automation does not replace an existing asset with different bytes.
These guarantees apply to releases produced by this workflow; earlier releases
may not include the manifest or attestations.

## Homebrew distribution

After publication, the [Homebrew tap](https://github.com/garlab/homebrew-tap)
verifies the source checksum and provenance, then opens a formula-update PR.
The tap builds and tests prebuilt packages (Homebrew's *bottles*) on Apple Silicon
macOS and Linux x86-64. Intel macOS installations build from source; Homebrew
may also need to build the Rust dependency.

Packages are published only after checks pass for the exact update being
published. Build jobs do not receive publishing credentials. Changes to the
formula's build instructions require a separately reviewed contribution; a
routine release update only changes its source reference and package metadata.

Homebrew packages can appear after the source release while their builds finish.
Other supported platforms can build the formula from source.

## Contributing to a release

A version update changes `[workspace.package].version`, the exact internal
workspace dependency versions, `Cargo.lock` and `CHANGELOG.md` together. See
[Library packaging](library.md#packaging-and-publication) for the crate structure.
Changes are reviewed and tested on `main` before a maintainer pushes the version
tag to start publication.

Contributors can check packaging locally:

```sh
cargo test --all --locked
cargo publish --workspace --dry-run --locked
```

The workflow supports validation-only runs on existing version tags. Changes to
the release helpers also have focused tests:

```sh
python3 -m unittest discover -s ci -p 'test_*.py'
```

Publication across crates.io, container registries and GitHub is not atomic.
A failed run can leave some destinations published before others; retries reuse
the same version and verify existing assets. The current automation publishes
versions in increasing order and does not publish older maintenance releases
over a newer release.

## Browser distribution

New releases also contain `probl-X.Y.Z-browser.json` and its `.sha256` checksum
file. The versioned bundle contains the matching WASM, JavaScript adapter and
highlighter, the built playground (which uses the same WASM, packaged once),
changelog, licenses and native reference results for the bundled examples. Files are base64-encoded with individual
SHA-256 hashes; the schema is versioned. No package installation or Rust compiler
is needed to consume a bundle.

The release workflow builds with the committed lockfiles, compares native and
WASM results, runs the full playground Chrome tests, then packages the output.
A separate job attests the exact bundle. Publication waits for these checks and
attestation, and includes the browser assets before making the release immutable.
The existing source manifest and source checksums retain their format.

The bundle is built with the current stable Rust. Re-running only a failed run's
failed jobs reuses the bundle it built. Re-running all jobs builds it again; after
a Rust release that WASM can differ, and publication then refuses to replace the
bundle already uploaded to the draft.

The website deployment workflow checks daily for stable releases. It verifies
the bundle's checksum and provenance against this repository, release workflow,
tag and commit, tests website examples and the packaged playground, and publishes
both sites only after success. Playground source remains here in `web/`; this
repository does not need website deployment credentials. Older immutable releases
without browser assets are not modified; this distribution starts with the first
release containing these workflow changes.
