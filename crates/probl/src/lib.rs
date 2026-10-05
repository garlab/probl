//! Compile and run [Probl](https://probl.dev) programs from Rust.
//!
//! Probl is a small language for explicit random choices and weighted worlds:
//! a draw explores its outcomes, each in a world of its own, weighted by how
//! likely it is, and a `report` collects what the worlds say. This crate is
//! its supported API; the command line (`probl-cli`) and the playground are
//! built on it.
//!
//! ```
//! let source = "let a ~ d6\nlet b ~ d6\nreport a + b >= 10 as \"ten or more\"";
//! let program = probl::compile("dice.probl", source)?;
//! let outcome = program.run(&probl::Options::new())?;
//!
//! let ten = outcome.report("ten or more").unwrap().groups()[0].probability().unwrap();
//! assert!((ten.point().unwrap() - 6.0 / 36.0).abs() < 1e-12);
//! assert!(ten.is_complete());
//! assert_eq!(outcome.text(), "enumerated\n\nten or more    16.67%\n");
//! # Ok::<(), probl::Error>(())
//! ```
//!
//! A run's [`Outcome`] has its [`text`](Outcome::text), as `probl run` prints
//! it, and each report's numbers, with what's known about their accuracy:
//! whether unresolved weight affects them, their bounds, and when sampling,
//! their Monte Carlo error. A program that reads data has it loaded first,
//! with [`Program::load`], from [`Files`] the host chooses.
//!
//! The crates `probl-syntax`, `probl-sema` and `probl-engine` are internal:
//! their APIs change with every release.

#[doc(hidden)]
#[path = "internal.rs"]
pub mod __internal;
mod error;
mod files;
mod options;
mod outcome;
mod program;

pub use error::{Diagnostic, Error, ErrorKind, Severity};
#[cfg(not(target_arch = "wasm32"))]
pub use files::LocalFiles;
pub use files::{Data, DataSource, Files, MemoryFiles, Snapshots};
pub use options::{Cancel, Date, Limits, Options, ParseDateError};
pub use outcome::{
    ConfidenceInterval, ConfidenceMethod, Estimate, Evidence, EvidenceKind, ExactUpdates, Group, Interval,
    NumericSummary, Outcome, Report, Sampling, SamplingStatus, SamplingUncertainty, Stats, SummaryError, Unresolved,
};
pub use program::{Mode, Program, compile};

/// This version of Probl.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
