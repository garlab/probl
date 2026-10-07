# Security policy

## Supported versions

Fixes go into the latest release only. Probl is before 1.0, so a fix comes as a new `0.x` release of every published crate.

## Report a vulnerability

Please don't open a public issue. Report it privately through GitHub instead: [report a vulnerability](https://github.com/garlab/probl/security/advisories/new), under the repository's Security tab.

Include the Probl version or commit, the smallest program or input that shows the problem, how you ran it (the command line, the playground or the library), and what an attacker gains. The fix is prepared privately and released with a security advisory. The advisory credits you, unless you'd rather it didn't.

## What counts

Probl runs models that someone else may have written: a shared playground link, or a service that embeds the `probl` crate. A model has no file, network or process access of its own, and the host sets its resource budgets. A vulnerability is a way around those boundaries, for example:

- a program that reads data its host didn't grant: through `MemoryFiles` or another restricted `Files` provider, or in the playground;
- a program or data file that raises a host's limits, runs past its budgets or cancellation, or crashes the host through a panic the library doesn't catch;
- a program, its output or a share link that runs script in the playground page;
- a problem in the release workflows or the published packages.

These are documented limits, not vulnerabilities:

- `LocalFiles`, which the command line uses, follows the local filesystem's rules: a program can read any file the user running it can read. It isn't a sandbox.
- Running out of memory, a stack overflow or a WebAssembly trap can end the process even within the budgets. A service that runs untrusted models should isolate each one in its own process, as [host boundaries](docs/architecture.md#host-boundaries) explains.
- A wrong probability or estimate is a correctness bug. Please report it as an ordinary [issue](https://github.com/garlab/probl/issues).
