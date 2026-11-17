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

Push `main` and the version tag together so the remote branch contains the
release commit when validation starts (`vX.Y.Z` is a placeholder):

```sh
git push --atomic origin main refs/tags/vX.Y.Z
```

Pushing only the tag does not update GitHub's `main`, even when the release commit
is already on your local `main`.

Contributors can check packaging locally:

```sh
cargo test --all --locked
cargo publish --workspace --dry-run --locked
```

Use the latest stable Rust release. Before publishing, compile all targets with
the declared minimum as described in the [toolchain policy](testing.md#rust-toolchain).
If the release raises the Rust minimum, update the workspace `rust-version`,
installation instructions and changelog. Keep the digest-pinned Docker build
toolchain at or above that minimum.

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

If validation says the tagged commit is not on `origin/main`, push or merge that
exact commit to `main`, then rerun the failed jobs of the original release run.
Keep the tag unchanged. A retry publishes the original tagged commit; newer
commits on `main` need their own version and tag to be included in a release.

## Editor grammar

[`tree-sitter-probl`](../editors/tree-sitter-probl/README.md#release) has its own
Cargo workspace and version. The [Grammar release workflow](../.github/workflows/grammar-release.yml)
runs when a stable `tree-sitter-probl-vX.Y.Z` tag is pushed. The Probl `vX.Y.Z`
workflow continues to release only the compiler workspace.

The grammar tag must identify a commit on `origin/main` and match both
`editors/tree-sitter-probl/Cargo.toml` and the `metadata.version` field in
`editors/tree-sitter-probl/tree-sitter.json`. Publication waits for the same
checks used in grammar CI: stable and minimum Rust on macOS, Linux and Windows,
packaged-crate builds/tests, reproducible generation, corpus tests and compiler
comparisons. The publishing job rechecks the remote tag, then uses short-lived
crates.io credentials in the existing `release` environment. Build jobs do not
receive publishing credentials.

Configure a GitHub trusted publisher once in the crate's
[crates.io settings](https://crates.io/crates/tree-sitter-probl/settings):

| Field | Value |
| --- | --- |
| Repository owner | `garlab` |
| Repository name | `probl` |
| Workflow filename | `grammar-release.yml` |
| Environment | `release` |

The workflow filename is the basename, without `.github/workflows/`. Existing
deployment approvals for `release` apply. Any tag restrictions on that environment
must allow `tree-sitter-probl-v*`. No long-lived Cargo token secret is needed;
see [trusted publishing](https://crates.io/docs/trusted-publishing).

For each new grammar version:

1. Update both grammar version fields, review `cargo package --list`, and refresh
   the grammar and compiler comparison lockfiles:

   ```sh
   cargo update --workspace --manifest-path editors/tree-sitter-probl/Cargo.toml
   cargo update --workspace --manifest-path editors/highlighting-tests/Cargo.toml
   ```

2. Commit the version update on `main`, then tag and push that commit together
   with the branch (`0.1.1` is an example):

   ```sh
   git tag -a tree-sitter-probl-v0.1.1 -m "Release tree-sitter-probl 0.1.1"
   git push --atomic origin main refs/tags/tree-sitter-probl-v0.1.1
   ```

The tag's commit must contain the workflow. The existing `0.1.0` publication and
tag remain the initial manual release. To validate a future grammar tag without
publishing, dispatch `Grammar release` on that tag with `dry_run` enabled, its
default. A retry skips an already-published version; registry failures stop the
job instead of being treated as a missing version. After a failed run, keep the
tag on the tested commit and rerun its failed jobs.

Grammar sources are also included in Probl's source archive; native consumers
use the grammar's independently published registry version.

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
