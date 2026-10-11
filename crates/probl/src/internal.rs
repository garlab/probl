//! Not part of the supported API: what the workspace's own tools need, with
//! no compatibility promise between releases. `probl-cli` and `probl-bench`
//! depend on the matching version of `probl`.

use crate::{Options, Stats};

pub use probl_engine::Symbolic;

/// The engine's switches for checking itself, all on by default. Turning one
/// off changes how much work a run does, not what it finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineChecks {
    /// Merge worlds that reach the same state (`--no-merge` turns it off).
    pub merge: bool,
    /// Reuse the result of a function called again with the same inputs
    /// (`--no-memo`).
    pub memoize: bool,
    /// Solve loops that cycle as Markov chains, instead of unrolling them
    /// (`--no-solve`).
    pub solve: bool,
    /// Experimental: answer programs with decision diagrams instead of
    /// worlds (`--symbolic`, docs/design/symbolic-inference.md). Off by
    /// default.
    pub symbolic: Symbolic,
}

impl Default for EngineChecks {
    fn default() -> EngineChecks {
        EngineChecks {
            merge: true,
            memoize: true,
            solve: true,
            symbolic: Symbolic::Off,
        }
    }
}

/// `options`, with these switches.
pub fn engine_checks(mut options: Options, checks: EngineChecks) -> Options {
    options.checks = checks;
    options
}

/// When a run was answered with decision diagrams: their variables and
/// nodes.
pub fn diagrams(stats: &Stats) -> Option<(u64, u64)> {
    stats.diagrams
}
