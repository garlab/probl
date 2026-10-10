//! What a run gives back: its text, its reports as numbers, with what's
//! known about their uncertainty, the evidence, and statistics.

use crate::{DataSource, Date, Diagnostic, FailureMode};
use probl_engine::report::{self, GroupResult, Status};
use probl_engine::value::Value;
use probl_sema::ir;
use probl_syntax::SourceFile;
use std::ops::Range;
use std::sync::Arc;

/// What a run gives back.
pub struct Outcome {
    program: Arc<ir::Program>,
    results: Vec<report::ReportResult>,
    format: report::Format,
    text: String,
    pub(crate) printed: Vec<String>,
    reports: Vec<Report>,
    evidence: Option<Evidence>,
    unresolved: Unresolved,
    sampling: Option<Sampling>,
    stats: Stats,
    data: Vec<DataSource>,
    today: Option<Date>,
    failure_mode: FailureMode,
    failures: Vec<Failure>,
    failed_share: Option<f64>,
    finished: bool,
}

impl Outcome {
    pub(crate) fn new(
        mut outcome: probl_engine::Outcome,
        program: &Arc<ir::Program>,
        file: &Arc<SourceFile>,
    ) -> Outcome {
        let reports = program
            .reports
            .iter()
            .zip(&outcome.results)
            .map(|(site, result)| Report {
                label: site.label.clone(),
                groups: result
                    .groups
                    .iter()
                    .map(|g| Group::new(g, site.key_label.is_some()))
                    .collect(),
            })
            .collect();
        let runs = outcome.sample.as_ref().map(|s| s.runs);
        let (failures, failed_share) = failures(&outcome, file);
        let mut text = std::mem::take(&mut outcome.output);
        if !failures.is_empty() {
            // After the reports, as a report would be.
            text.truncate(text.trim_end_matches('\n').len());
            text.push_str("\n\n");
            text.push_str(&failure_text(&failures, outcome.format));
            text.push('\n');
        }
        let unresolved = match runs {
            // Per run, as enumeration counts it per unit of prior weight.
            Some(runs) => outcome.unresolved.scale(1.0 / runs as f64),
            None => outcome.unresolved,
        };
        Outcome {
            evidence: evidence(&outcome),
            unresolved: Unresolved(unresolved),
            sampling: outcome.sample.as_ref().map(|s| Sampling {
                runs: s.runs,
                seed: s.seed,
                effective_runs: s.effective,
            }),
            stats: Stats::new(&outcome, file),
            data: outcome.data.iter().map(DataSource::new).collect(),
            today: outcome.today.map(Date::from_days),
            failure_mode: FailureMode::new(outcome.on_error),
            failures,
            failed_share,
            finished: !outcome.finished.is_zero(),
            text,
            printed: Vec::new(),
            reports,
            results: outcome.results,
            format: outcome.format,
            program: program.clone(),
        }
    }

    /// What `probl run` prints: the summary line and every report.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The reports in `reports` (indices into [`reports`](Outcome::reports)),
    /// aligned as `text` aligns them. The REPL prints the reports its newest
    /// input added this way.
    pub fn render(&self, reports: Range<usize>) -> String {
        let end = reports.end.min(self.results.len());
        let start = reports.start.min(end);
        report::render_results(
            &self.program.reports[start..end],
            &self.results[start..end],
            self.format,
        )
    }

    /// Every report, in the order of the program.
    pub fn reports(&self) -> &[Report] {
        &self.reports
    }

    /// The first report with this label. A report without `as` is labelled
    /// with its expression: `report win` is `"win"`.
    pub fn report(&self, label: &str) -> Option<&Report> {
        self.reports.iter().find(|r| r.label == label)
    }

    /// What the program printed, with [`Program::run`](crate::Program::run).
    /// Empty with [`run_with`](crate::Program::run_with), which gives each
    /// line to its function instead.
    pub fn printed(&self) -> &[String] {
        &self.printed
    }

    /// The evidence, when the program observed anything: the probability of
    /// the observations, or their density.
    pub fn evidence(&self) -> Option<&Evidence> {
        self.evidence.as_ref()
    }

    /// The weight that loops and limits cut off before it finished.
    pub fn unresolved(&self) -> &Unresolved {
        &self.unresolved
    }

    /// How the program was sampled, when it was.
    pub fn sampling(&self) -> Option<Sampling> {
        self.sampling
    }

    /// What `probl run --stats` prints: how much work the engine did.
    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// The data the program read: what its results were computed from.
    pub fn data(&self) -> &[DataSource] {
        &self.data
    }

    /// The date `today` stood for, if the run was given one.
    pub fn today(&self) -> Option<Date> {
        self.today
    }

    /// What a fault did to the other worlds in this run: the host's choice,
    /// the program's `@on_error`, or the mode's default.
    pub fn failure_mode(&self) -> FailureMode {
        self.failure_mode
    }

    /// In partial mode: the worlds that failed, grouped by where and how,
    /// in the order they first failed. Empty when none did. An outcome with
    /// failures is a partial result: [`Program::run`](crate::Program::run)
    /// returns it in its error, as [`Error::partial`](crate::Error::partial).
    pub fn failures(&self) -> &[Failure] {
        &self.failures
    }

    /// The share of the weight (or of the runs) that failed, when it can be
    /// compared with the worlds that finished: `None` when worlds failed
    /// before evidence they'd have met, and `Some(0.0)` when none failed.
    pub fn failed_share(&self) -> Option<f64> {
        self.failed_share
    }

    /// Whether any world (or run) finished the program.
    pub fn finished(&self) -> bool {
        self.finished
    }
}

/// The worlds of a partial run that failed at one place, with one kind of
/// fault (docs/semantics.md, section 11).
#[derive(Clone, Debug)]
pub struct Failure {
    diagnostic: Diagnostic,
    share: Option<f64>,
    standard_error: Option<f64>,
    weight: f64,
    runs: Option<u64>,
    first_run: Option<u64>,
}

impl Failure {
    /// The first of them, as an error: the fault, where it happened, and
    /// how much failed there.
    pub fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }

    /// The share of the weight (or of the runs) that failed here. `None`
    /// when the failed worlds could still have met evidence: their weight
    /// stops short of the evidence the finished worlds' includes.
    pub fn share(&self) -> Option<f64> {
        self.share
    }

    /// When sampling, and the share is known: its standard error.
    pub fn standard_error(&self) -> Option<f64> {
        self.standard_error
    }

    /// The weight the worlds had when they failed. When sampling, it's
    /// summed over the runs that failed here.
    pub fn weight(&self) -> f64 {
        self.weight
    }

    /// When sampling: how many runs failed here.
    pub fn runs(&self) -> Option<u64> {
        self.runs
    }

    /// When sampling: the first run that failed here, in the order runs are
    /// combined, counting from 1.
    pub fn first_run(&self) -> Option<u64> {
        self.first_run
    }
}

/// How many failures the text lists before saying how many more there are.
const LISTED_FAILURES: usize = 10;

/// The run's failures, each with a diagnostic that says how much failed,
/// and the share that failed overall when it's known.
fn failures(outcome: &probl_engine::Outcome, file: &Arc<SourceFile>) -> (Vec<Failure>, Option<f64>) {
    let all = &outcome.failures;
    let comparable = all.comparable();
    let total = outcome.finished + all.weight;
    let total_squares = outcome.sample.as_ref().map(|s| s.squares + all.squares);
    let share_of = |w: probl_engine::Weight| (comparable && !total.is_zero()).then(|| w.ratio(total));
    let pct = |p: f64| report::pct(p, outcome.format);
    let others = match (outcome.sample.is_some(), outcome.finished.is_zero()) {
        (false, false) => "the other worlds finished",
        (false, true) => "no world finished",
        (true, false) => "the other runs finished",
        (true, true) => "no run finished",
    };
    let failures = all
        .groups
        .iter()
        .map(|g| {
            let share = share_of(g.weight);
            let standard_error = match (share, total_squares) {
                (Some(p), Some(all_squares)) => {
                    let spread =
                        g.squares.scale((1.0 - p) * (1.0 - p)) + all_squares.saturating_sub(g.squares).scale(p * p);
                    Some(spread.ratio(total * total).sqrt())
                }
                _ => None,
            };
            let note = match (&outcome.sample, share) {
                (Some(s), _) => {
                    let first = g
                        .first_run
                        .map_or(String::new(), |r| format!(", first in run {}", r + 1));
                    format!(
                        "it failed in {} of {} runs{first}; {others}",
                        report::thousands(g.runs as i64),
                        report::thousands(s.runs as i64)
                    )
                }
                (None, Some(p)) => format!("it failed in {} of the worlds; {others}", pct(p)),
                (None, None) if all.other_units => {
                    format!("it failed in worlds that observed another number of continuous values; {others}")
                }
                (None, None) => format!("it failed in worlds that hadn't met all the evidence yet; {others}"),
            };
            let error = g.error.clone().with_note(note);
            Failure {
                diagnostic: Diagnostic::new(error.to_diagnostic(), file),
                share,
                standard_error,
                weight: g.weight.to_f64(),
                runs: outcome.sample.is_some().then_some(g.runs),
                first_run: g.first_run.map(|r| u64::from(r) + 1),
            }
        })
        .collect();
    let failed_share = if all.is_empty() {
        Some(0.0)
    } else {
        share_of(all.weight)
    };
    (failures, failed_share)
}

/// The failures, as `probl run` lists them after the reports:
///
/// ```text
/// failed
///   line 4: division by zero    16.67%
/// ```
fn failure_text(failures: &[Failure], format: report::Format) -> String {
    let rows: Vec<(String, String)> = failures
        .iter()
        .take(LISTED_FAILURES)
        .map(|f| {
            let (line, _) = f.diagnostic.line_column();
            let label = format!("line {line}: {}", f.diagnostic.message());
            let share = match (f.share, f.standard_error) {
                (Some(p), Some(se)) => Some(report::estimate(p, se)),
                (Some(p), None) => Some(report::pct(p, format)),
                (None, _) => None,
            };
            let amount = match (f.runs, share) {
                (Some(runs), share) => {
                    let runs = match runs {
                        1 => "1 run".to_string(),
                        n => format!("{} runs", report::thousands(n as i64)),
                    };
                    match share {
                        Some(share) => format!("{runs} ({share})"),
                        None => runs,
                    }
                }
                (None, Some(share)) => share,
                (None, None) => format!("weight {}", plain_number(f.weight)),
            };
            (label, amount)
        })
        .collect();
    let width = rows.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(0);
    let mut text = String::from("failed");
    for (label, amount) in rows {
        let pad = width - label.chars().count();
        text.push_str(&format!("\n  {label}{}    {amount}", " ".repeat(pad)));
    }
    if failures.len() > LISTED_FAILURES {
        text.push_str(&format!("\n  and {} more", failures.len() - LISTED_FAILURES));
    }
    text
}

/// A weight that isn't a probability, with four significant digits.
fn plain_number(x: f64) -> String {
    if x != 0.0 && !(1e-3..1e6).contains(&x.abs()) {
        format!("{x:.3e}")
    } else {
        let digits = if x == 0.0 {
            0
        } else {
            (3 - x.abs().log10().floor() as i32).max(0) as usize
        };
        format!("{x:.digits$}")
    }
}

impl std::fmt::Debug for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Outcome")
            .field("text", &self.text)
            .field("reports", &self.reports)
            .field("evidence", &self.evidence)
            .field("unresolved", &self.unresolved)
            .field("sampling", &self.sampling)
            .finish_non_exhaustive()
    }
}

/// One `report`: what it saw, for each of its keys.
#[derive(Clone, Debug)]
pub struct Report {
    label: String,
    groups: Vec<Group>,
}

impl Report {
    pub fn label(&self) -> &str {
        &self.label
    }

    /// One group for each key of its `by`, in the keys' order, or a single
    /// group without `by`. None when nothing reached the report.
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }
}

/// What a report saw for one key.
#[derive(Clone, Debug)]
pub struct Group {
    key: Option<String>,
    probability: Option<Estimate>,
    distribution: Option<Vec<(String, Estimate)>>,
    numeric: Option<NumericSummary>,
    fields: Option<Box<Fields>>,
}

/// A structured group's fields, each with its path.
#[derive(Clone, Debug)]
struct Fields(Vec<(String, Group)>);

impl Group {
    fn new(group: &GroupResult, by: bool) -> Group {
        Group {
            key: by.then(|| report::display(&group.key)),
            probability: group.fact.map(Estimate::new),
            // A fact's probability is its result: a table of `true` and
            // `false` would only say it again.
            distribution: match (&group.fact, &group.values) {
                (None, Some(values)) => Some(
                    values
                        .iter()
                        .map(|(value, q)| (label(value), Estimate::new(*q)))
                        .collect(),
                ),
                _ => None,
            },
            numeric: group.numeric.clone().map(NumericSummary::new),
            fields: group.fields.as_ref().map(|fields| {
                Box::new(Fields(
                    fields
                        .0
                        .iter()
                        .map(|(path, field)| (path.clone(), Group::new(field, false)))
                        .collect(),
                ))
            }),
        }
    }

    /// The `by` key, as Probl prints it; `None` without `by`.
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    /// When the report is of a fact: the probability that it's true.
    pub fn probability(&self) -> Option<&Estimate> {
        self.probability.as_ref()
    }

    /// When the report is of other values, unless they're continuous: each
    /// value, as Probl prints it, with its probability, in the order of the
    /// values (numbers and dates in numeric order). Labels are for display:
    /// two values can print alike, and both are listed.
    pub fn distribution(&self) -> Option<&[(String, Estimate)]> {
        self.distribution.as_deref()
    }

    /// When the values are real numbers, or continuous: their mean, standard
    /// deviation and quantiles.
    pub fn numeric(&self) -> Option<&NumericSummary> {
        self.numeric.as_ref()
    }

    /// When the values are records or lists that hold continuous outcomes,
    /// as enumeration reports them: each field's path, like `.x` or `[0]`,
    /// with its own summary. The fields come from one joint distribution, and
    /// can depend on each other in ways their summaries don't show. The
    /// group's other summaries are then `None`.
    pub fn fields(&self) -> Option<&[(String, Group)]> {
        self.fields.as_deref().map(|f| f.0.as_slice())
    }
}

/// A value as a label, as reports print it.
fn label(value: &Value) -> String {
    report::display(value)
}

/// One reported quantity, such as a probability or a mean, with what's known
/// about its accuracy.
///
/// Its [`point`](Estimate::point) is over the resolved weight: the worlds or
/// runs that finished. When some weight is unresolved, it's conditional on
/// what resolved, [`is_complete`](Estimate::is_complete) is false, and
/// [`bounds`](Estimate::bounds) bound the quantity over all the weight, where
/// the engine can establish them. Sampling error is separate, in
/// [`sampling`](Estimate::sampling).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    point: Option<f64>,
    complete: bool,
    bounds: Option<Interval>,
    sampling: Option<SamplingUncertainty>,
}

impl Estimate {
    fn new(q: report::Quantity) -> Estimate {
        Estimate {
            point: q.point,
            complete: q.complete,
            bounds: q.bounds.map(|(lower, upper)| Interval { lower, upper }),
            sampling: q.sampling.map(SamplingUncertainty::new),
        }
    }

    /// The quantity over the resolved weight; `None` when it can't be
    /// computed.
    pub fn point(&self) -> Option<f64> {
        self.point
    }

    /// Whether no unresolved weight affects it. A complete sampled estimate
    /// still has Monte Carlo error.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// When it isn't complete: bounds on the quantity over all the weight, as
    /// if the unresolved weight had gone either way. Given when enumerating,
    /// for probabilities; not for statistics such as a mean, which unresolved
    /// weight could move without bound.
    pub fn bounds(&self) -> Option<Interval> {
        self.bounds
    }

    /// When sampling: its Monte Carlo error.
    pub fn sampling(&self) -> Option<&SamplingUncertainty> {
        self.sampling.as_ref()
    }
}

/// What's known about a sampled quantity's Monte Carlo error.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SamplingStatus {
    /// Its standard error is estimated.
    Estimated,
    /// The runs can't establish a useful estimate: every run agreed, without
    /// integrated outcomes. It doesn't mean there's no error.
    NotEstimable,
    /// The engine doesn't compute one for this statistic. It doesn't mean
    /// there's no error.
    NotComputed,
    /// No empirical Monte Carlo error: every run integrated the same
    /// outcomes, as when a fact's probability is computed in each run. Not a
    /// guarantee for every symbolic or integrated computation.
    IntegratedZero,
}

impl SamplingStatus {
    fn new(status: Status) -> SamplingStatus {
        match status {
            Status::Estimated => SamplingStatus::Estimated,
            Status::NotEstimable => SamplingStatus::NotEstimable,
            Status::NotComputed => SamplingStatus::NotComputed,
            Status::IntegratedZero => SamplingStatus::IntegratedZero,
        }
    }
}

/// A sampled quantity's Monte Carlo error.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SamplingUncertainty {
    status: SamplingStatus,
    standard_error: Option<f64>,
    confidence_interval: Option<ConfidenceInterval>,
    contributing_runs: u64,
    effective_runs: f64,
}

impl SamplingUncertainty {
    fn new(u: report::Uncertainty) -> SamplingUncertainty {
        SamplingUncertainty {
            status: SamplingStatus::new(u.status),
            standard_error: u.se,
            confidence_interval: u.wilson.map(|(lower, upper)| ConfidenceInterval {
                interval: Interval { lower, upper },
                level: 0.95,
                method: ConfidenceMethod::Wilson,
            }),
            contributing_runs: u.support.contributing_runs,
            effective_runs: u.support.effective,
        }
    }

    pub fn status(&self) -> SamplingStatus {
        self.status
    }

    /// When [`Estimated`](SamplingStatus::Estimated), and zero when
    /// [`IntegratedZero`](SamplingStatus::IntegratedZero).
    pub fn standard_error(&self) -> Option<f64> {
        self.standard_error
    }

    /// For a fact's probability, when its rules allow one: a 95% Wilson
    /// interval, given when the standard error isn't useful (every
    /// contributing run agreed, or fewer than 30 contributed) and each run
    /// contributed one ordinary, equally weighted observation.
    pub fn confidence_interval(&self) -> Option<&ConfidenceInterval> {
        self.confidence_interval.as_ref()
    }

    /// The independent runs that reached the report with this key; several
    /// visits in one run count once.
    pub fn contributing_runs(&self) -> u64 {
        self.contributing_runs
    }

    /// The effective sample size of those runs.
    pub fn effective_runs(&self) -> f64 {
        self.effective_runs
    }
}

/// A range of numbers, both ends included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    lower: f64,
    upper: f64,
}

impl Interval {
    pub fn lower(&self) -> f64 {
        self.lower
    }

    pub fn upper(&self) -> f64 {
        self.upper
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConfidenceInterval {
    interval: Interval,
    level: f64,
    method: ConfidenceMethod,
}

impl ConfidenceInterval {
    pub fn interval(&self) -> &Interval {
        &self.interval
    }

    /// Its confidence level, such as 0.95.
    pub fn level(&self) -> f64 {
        self.level
    }

    pub fn method(&self) -> ConfidenceMethod {
        self.method
    }
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConfidenceMethod {
    /// The Wilson score interval, for ordinary independent observations of a
    /// fact.
    Wilson,
}

/// The summaries of real numbers, or of continuous values.
#[derive(Clone, Debug)]
pub struct NumericSummary {
    mean: Estimate,
    sd: Estimate,
    numeric: report::Numeric,
}

/// Why [`NumericSummary::quantile`] gave no quantile.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SummaryError {
    /// `q` isn't a finite number from 0 to 1.
    InvalidQuantile,
    /// The report has no such summary.
    Unavailable,
}

impl std::fmt::Display for SummaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SummaryError::InvalidQuantile => "a quantile must be a number from 0 to 1",
            SummaryError::Unavailable => "the report has no such summary",
        })
    }
}

impl std::error::Error for SummaryError {}

impl NumericSummary {
    fn new(numeric: report::Numeric) -> NumericSummary {
        NumericSummary {
            mean: Estimate::new(numeric.mean),
            sd: Estimate::new(numeric.sd),
            numeric,
        }
    }

    /// The mean. When sampling, its standard error is estimated.
    pub fn mean(&self) -> Option<&Estimate> {
        Some(&self.mean)
    }

    /// The standard deviation, as the population's (no sample correction).
    pub fn sd(&self) -> Option<&Estimate> {
        Some(&self.sd)
    }

    /// The `q` quantile, from 0 to 1, by the reports' convention: the median
    /// is the midpoint of the middle values, and any other quantile is the
    /// smallest value whose cumulative probability reaches `q`.
    pub fn quantile(&self, q: f64) -> Result<Estimate, SummaryError> {
        if !(0.0..=1.0).contains(&q) {
            return Err(SummaryError::InvalidQuantile);
        }
        self.numeric
            .quantile(q)
            .map(Estimate::new)
            .ok_or(SummaryError::Unavailable)
    }
}

/// What kind of number the evidence is.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EvidenceKind {
    /// The probability of the observations.
    Probability,
    /// A density: some observation was of a continuous value. It can be more
    /// than one, and isn't a percentage.
    Density,
}

/// The evidence: the probability (or density) of what the program observed,
/// as a natural logarithm, so that tiny likelihoods don't underflow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Evidence {
    kind: EvidenceKind,
    log_value: Option<f64>,
    complete: bool,
    log_bounds: Option<Interval>,
    relative_standard_error: Option<f64>,
    sampling_status: Option<SamplingStatus>,
}

fn evidence(outcome: &probl_engine::Outcome) -> Option<Evidence> {
    let z = outcome.evidence?;
    let complete = outcome.unresolved.is_zero();
    Some(match &outcome.sample {
        None => Evidence {
            kind: if outcome.densities {
                EvidenceKind::Density
            } else {
                EvidenceKind::Probability
            },
            log_value: Some(z.ln()),
            complete,
            log_bounds: (!complete).then(|| Interval {
                lower: z.ln(),
                upper: (z + outcome.unresolved).ln(),
            }),
            relative_standard_error: None,
            sampling_status: None,
        },
        Some(sampled) => {
            let known = sampled.evidence_se.is_finite();
            Evidence {
                kind: if sampled.densities {
                    EvidenceKind::Density
                } else {
                    EvidenceKind::Probability
                },
                log_value: Some(z.ln()),
                complete,
                log_bounds: None,
                relative_standard_error: known.then_some(sampled.evidence_se),
                sampling_status: Some(if known {
                    SamplingStatus::Estimated
                } else {
                    SamplingStatus::NotEstimable
                }),
            }
        }
    })
}

impl Evidence {
    pub fn kind(&self) -> EvidenceKind {
        self.kind
    }

    /// The natural logarithm of the evidence from the resolved worlds (−∞
    /// for none). Not renormalized: when it isn't complete, unresolved worlds
    /// could add to it.
    pub fn log_value(&self) -> Option<f64> {
        self.log_value
    }

    /// Whether no unresolved weight could add to it.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// When enumerating and it isn't complete: its bounds, in natural-log
    /// units, as if the unresolved weight had all fitted the observations,
    /// or none of it.
    pub fn log_bounds(&self) -> Option<Interval> {
        self.log_bounds
    }

    /// When sampling, if there was more than one run: the standard error of
    /// the evidence, relative to it (not an error in log units).
    pub fn relative_standard_error(&self) -> Option<f64> {
        self.relative_standard_error
    }

    /// When sampling: whether the relative standard error is estimated.
    pub fn sampling_status(&self) -> Option<SamplingStatus> {
        self.sampling_status
    }
}

/// The weight that loops and limits cut off before it finished: an
/// unnormalized bound, not a probability. When sampling, it's per run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Unresolved(probl_engine::Weight);

impl Unresolved {
    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    /// Its natural logarithm, which keeps very small weights: −∞ for zero.
    pub fn log_weight_upper_bound(&self) -> f64 {
        self.0.ln()
    }
}

/// How a program was sampled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sampling {
    runs: u64,
    seed: u64,
    effective_runs: f64,
}

impl Sampling {
    pub fn runs(&self) -> u64 {
        self.runs
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The effective sample size of the runs' weights: (Σ w)² / Σ w².
    pub fn effective_runs(&self) -> f64 {
        self.effective_runs
    }
}

/// How much work the engine did.
#[derive(Clone, Debug, PartialEq)]
pub struct Stats {
    peak_worlds: usize,
    world_steps: u64,
    calls: u64,
    reused_calls: u64,
    solved_loops: u64,
    chain_states: u64,
    solved_calls: u64,
    call_rounds: u64,
    exact_updates: Vec<ExactUpdates>,
}

impl Stats {
    fn new(outcome: &probl_engine::Outcome, file: &SourceFile) -> Stats {
        let s = &outcome.stats;
        Stats {
            peak_worlds: s.peak_worlds,
            world_steps: s.world_steps,
            calls: s.calls,
            reused_calls: s.memo_hits,
            solved_loops: s.solved_loops,
            chain_states: s.chain_states,
            solved_calls: s.solved_calls,
            call_rounds: s.call_rounds,
            exact_updates: outcome
                .updates
                .iter()
                .map(|(variable, u)| ExactUpdates {
                    variable: variable.name.clone(),
                    line: file.line_col(variable.span.lo).0,
                    delayed: u.delayed,
                    observations: u.exact,
                    drawn: u.drawn,
                })
                .collect(),
        }
    }

    /// The most worlds any statement ran on.
    pub fn peak_worlds(&self) -> usize {
        self.peak_worlds
    }

    /// Statement executions, counting each world separately.
    pub fn world_steps(&self) -> u64 {
        self.world_steps
    }

    pub fn calls(&self) -> u64 {
        self.calls
    }

    /// Calls whose result was reused.
    pub fn reused_calls(&self) -> u64 {
        self.reused_calls
    }

    /// Loops solved as Markov chains.
    pub fn solved_loops(&self) -> u64 {
        self.solved_loops
    }

    /// The states of those chains.
    pub fn chain_states(&self) -> u64 {
        self.chain_states
    }

    /// Calls that came back to themselves, solved by iteration.
    pub fn solved_calls(&self) -> u64 {
        self.solved_calls
    }

    /// The rounds of iteration they took.
    pub fn call_rounds(&self) -> u64 {
        self.call_rounds
    }

    /// When sampling: each variable whose draws could be delayed for exact
    /// updates, and what happened to it.
    pub fn exact_updates(&self) -> &[ExactUpdates] {
        &self.exact_updates
    }
}

/// How one variable's draws were delayed and updated exactly, over all the
/// runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactUpdates {
    variable: String,
    line: usize,
    delayed: u64,
    observations: u64,
    drawn: u64,
}

impl ExactUpdates {
    pub fn variable(&self) -> &str {
        &self.variable
    }

    /// The line it's declared on, from 1.
    pub fn line(&self) -> usize {
        self.line
    }

    /// Draws delayed.
    pub fn delayed(&self) -> u64 {
        self.delayed
    }

    /// Observations that updated it exactly.
    pub fn observations(&self) -> u64 {
        self.observations
    }

    /// Times it was drawn when first needed, from its updated distribution.
    pub fn drawn(&self) -> u64 {
        self.drawn
    }
}
