//! Worlds that ended at a fault, when the failure mode is partial
//! (docs/semantics.md, section 11).

use crate::error::{Fault, RuntimeError};
use crate::weight::Weight;
use probl_syntax::Span;

/// The worlds that failed, grouped by where and how.
#[derive(Clone, Debug, Default)]
pub struct Failures {
    /// In the order they first failed (in batch order, when sampling).
    pub groups: Vec<Failure>,
    /// Their total weight: when sampling, of the runs that failed.
    pub weight: Weight,
    /// When sampling: Σ w² over the runs that failed.
    pub squares: Weight,
    /// When sampling: how many runs failed.
    pub runs: u64,
    /// Whether evidence could still have been applied to a world that
    /// failed, had it gone on. Its weight then stops short of what the
    /// finished worlds' includes, and the two can't be added up.
    pub before_evidence: bool,
    /// When enumerating: how many densities the failed weight includes, as
    /// the main program's worlds count them, and whether that's as many in
    /// each.
    pub densities: Option<u32>,
    pub mixed_densities: bool,
    /// Whether the failed weight includes another number of densities than
    /// the finished worlds', which puts it in other units: it can't be added
    /// to theirs either.
    pub other_units: bool,
}

/// The worlds that failed at one place, with one kind of fault.
#[derive(Clone, Debug)]
pub struct Failure {
    /// The first of them.
    pub error: RuntimeError,
    pub weight: Weight,
    /// When sampling: Σ w² over the runs that failed here.
    pub squares: Weight,
    /// When sampling: how many runs failed here, and the first, in batch
    /// order.
    pub runs: u64,
    pub first_run: Option<u32>,
    /// Whether evidence could still have been applied to them.
    pub before_evidence: bool,
}

impl Failures {
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Whether the failed weight can be added to the finished worlds'.
    pub fn comparable(&self) -> bool {
        !self.before_evidence && !self.other_units
    }

    fn note_densities(&mut self, n: u32) {
        match self.densities {
            Some(m) if m != n => self.mixed_densities = true,
            _ => self.densities = Some(n),
        }
    }

    /// A world of this weight, with `densities` in it, failed with `error`.
    /// `run` is its run, when sampling.
    pub fn record(
        &mut self,
        error: RuntimeError,
        weight: Weight,
        run: Option<u32>,
        before_evidence: bool,
        densities: u32,
    ) {
        self.note_densities(densities);
        let runs = run.is_some() as u64;
        let squares = if run.is_some() { weight * weight } else { Weight::ZERO };
        self.weight += weight;
        self.squares += squares;
        self.runs += runs;
        self.before_evidence |= before_evidence;
        let (span, fault) = (error.span, error.fault);
        match self.group(span, fault) {
            Some(g) => {
                g.weight += weight;
                g.squares += squares;
                g.runs += runs;
                g.before_evidence |= before_evidence;
            }
            None => self.groups.push(Failure {
                error,
                weight,
                squares,
                runs,
                first_run: run,
                before_evidence,
            }),
        }
    }

    /// Add what failed inside a call, or in a loop's state, `times` as
    /// often. When sampling, a call follows one path: whatever failed in it
    /// failed in the caller's run `run`. `before_evidence`: whether evidence
    /// can follow where it's added.
    pub fn absorb(&mut self, other: &Failures, times: Weight, run: Option<u32>, before_evidence: bool) {
        for theirs in &other.groups {
            self.absorb_group(theirs, times, run, before_evidence, other.densities.unwrap_or(0));
        }
        self.mixed_densities |= other.mixed_densities;
    }

    /// Add one group of what failed inside a call, as [`absorb`](Self::absorb)
    /// does, from a world with `densities` in it.
    pub fn absorb_group(
        &mut self,
        theirs: &Failure,
        times: Weight,
        run: Option<u32>,
        before_evidence: bool,
        densities: u32,
    ) {
        self.note_densities(densities);
        let weight = theirs.weight * times;
        let squares = theirs.squares * times * times;
        let before_evidence = theirs.before_evidence || before_evidence;
        self.weight += weight;
        self.squares += squares;
        self.runs += theirs.runs;
        self.before_evidence |= before_evidence;
        match self.group(theirs.error.span, theirs.error.fault) {
            Some(g) => {
                g.weight += weight;
                g.squares += squares;
                g.runs += theirs.runs;
                g.before_evidence |= before_evidence;
            }
            None => self.groups.push(Failure {
                error: theirs.error.clone(),
                weight,
                squares,
                runs: theirs.runs,
                first_run: run.or(theirs.first_run),
                before_evidence,
            }),
        }
    }

    /// Add a later batch's failures.
    pub fn append(&mut self, later: Failures) {
        self.weight += later.weight;
        self.squares += later.squares;
        self.runs += later.runs;
        self.before_evidence |= later.before_evidence;
        if let Some(n) = later.densities {
            self.note_densities(n);
        }
        self.mixed_densities |= later.mixed_densities;
        self.other_units |= later.other_units;
        for theirs in later.groups {
            match self.group(theirs.error.span, theirs.error.fault) {
                Some(g) => {
                    g.weight += theirs.weight;
                    g.squares += theirs.squares;
                    g.runs += theirs.runs;
                    g.before_evidence |= theirs.before_evidence;
                }
                None => self.groups.push(theirs),
            }
        }
    }

    fn group(&mut self, span: Span, fault: Option<Fault>) -> Option<&mut Failure> {
        self.groups
            .iter_mut()
            .find(|g| g.error.span == span && g.error.fault == fault)
    }
}
