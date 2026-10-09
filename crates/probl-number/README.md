# probl-number

Integer representation shared by [Probl](https://github.com/garlab/probl)'s syntax and runtime. Small integers are stored inline; large integers use shared storage. Size limits bound parsing and individual allocations.

**This is an internal Probl component with no stable API.** It is published to support the other Probl crates, and its API may change with each release. Applications embedding Probl should depend on [`probl`](https://crates.io/crates/probl).

To run Probl programs, install [`probl-cli`](https://crates.io/crates/probl-cli) or try the [browser playground](https://playground.probl.dev).

[Internal API reference](https://docs.rs/probl-number) · [Source](https://github.com/garlab/probl/tree/main/crates/probl-number) · [Architecture](https://github.com/garlab/probl/blob/main/docs/architecture.md)

## License

Licensed under either the [MIT license](https://github.com/garlab/probl/blob/main/LICENSE-MIT) or [Apache License, Version 2.0](https://github.com/garlab/probl/blob/main/LICENSE-APACHE), at your option.
