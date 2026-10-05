//! Not part of the supported API: what the workspace's own tools need, with
//! no compatibility promise between releases. `probl-cli` and `probl-bench`
//! depend on the matching version of `probl`.

use crate::Options;

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
}

impl Default for EngineChecks {
    fn default() -> EngineChecks {
        EngineChecks {
            merge: true,
            memoize: true,
            solve: true,
        }
    }
}

/// `options`, with these switches.
pub fn engine_checks(mut options: Options, checks: EngineChecks) -> Options {
    options.checks = checks;
    options
}
