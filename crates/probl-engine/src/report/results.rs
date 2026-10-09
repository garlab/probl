//! What each report says, as numbers. The renderer formats these, and the
//! `probl` library gives them to programs that embed Probl, so the two can't
//! disagree (docs/library.md).

use super::{Acc, Format, Sink, summary_quantile};
use crate::continuous::Mixture;
use crate::value::Value;
use crate::weight::Weight;
use probl_sema::ir::{ReportKind, ReportSite};

/// One report's results.
#[derive(Clone, Debug)]
pub struct ReportResult {
    /// The share of the worlds' weight (or of the runs) that reached the
    /// report. `None` when every visit counts, when nothing reached it, when
    /// no world finished, or when worlds failed before evidence they'd have
    /// met, so their weight can't be compared.
    pub reach: Option<Reach>,
    /// One group per `by` key, in key order; one keyed `()` without `by`.
    pub groups: Vec<GroupResult>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reach {
    pub share: f64,
    /// When sampling: its standard error.
    pub se: Option<f64>,
}

/// What one report saw for one key.
#[derive(Clone, Debug)]
pub struct GroupResult {
    pub key: Value,
    /// When every reported value was a fact: the chance that it's true.
    pub fact: Option<Quantity>,
    /// Unless a value is continuous: each value's probability, in the order
    /// of the values (facts as `true` and `false`, for tables that mix them
    /// with other values).
    pub values: Option<Vec<(Value, Quantity)>>,
    /// The values and their probabilities, among the resolved worlds, in the
    /// order of the values (facts count as `true` and `false`).
    pub distribution: Vec<(Value, f64)>,
    /// For real numbers, and continuous values: their summaries.
    pub numeric: Option<Numeric>,
    /// The share of the group's weight that isn't resolved, as the renderer
    /// counts it.
    pub unresolved_share: f64,
    /// When sampling: how many runs it rests on.
    pub support: Option<Support>,
}

/// How many sampled runs a group rests on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Support {
    /// Independent runs that reached it; several visits in one run count once.
    pub contributing_runs: u64,
    /// The effective sample size of its denominator.
    pub effective: f64,
}

/// One reported quantity: a probability, a mean, a quantile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quantity {
    /// Over the resolved worlds (or runs).
    pub point: Option<f64>,
    /// No unresolved weight, or missing mass, affects it.
    pub complete: bool,
    /// When it isn't complete and enumerating: bounds on the quantity over
    /// all the weight, as if the unresolved weight had gone either way.
    pub bounds: Option<(f64, f64)>,
    /// When sampling: its Monte Carlo error.
    pub sampling: Option<Uncertainty>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Uncertainty {
    pub status: Status,
    /// When `Estimated`, and zero when `IntegratedZero`.
    pub se: Option<f64>,
    /// A 95% Wilson interval, where its rules allow one.
    pub wilson: Option<(f64, f64)>,
    pub support: Support,
}

/// What's known about a sampled quantity's Monte Carlo error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Its standard error is estimated.
    Estimated,
    /// The runs can't establish a useful estimate: they all agreed, without
    /// integrated outcomes.
    NotEstimable,
    /// The engine doesn't compute one for this statistic.
    NotComputed,
    /// No empirical error: every run integrated the same outcomes.
    IntegratedZero,
}

/// The summaries of a group of real numbers, or of continuous values.
#[derive(Clone, Debug)]
pub struct Numeric {
    pub mean: Quantity,
    pub sd: Quantity,
    /// Every value was a probability (shown as percentages).
    pub percent: bool,
    shape: Shape,
    /// For the quantiles: the group's completeness and sampling support.
    complete: bool,
    support: Option<Support>,
}

#[derive(Clone, Debug)]
enum Shape {
    /// A continuous marginal, possibly mixed with point masses.
    Mixture(Mixture),
    /// Point masses, as values (integers keep their exact value for printing).
    Points(Vec<(Value, f64)>),
}

impl Numeric {
    /// The continuous marginal, when there's one.
    pub fn mixture(&self) -> Option<&Mixture> {
        match &self.shape {
            Shape::Mixture(m) => Some(m),
            Shape::Points(_) => None,
        }
    }

    /// The `q` quantile, by the report convention: the median is the
    /// midpoint of the middle values, the others select an outcome. `q` must
    /// be in [0, 1].
    pub fn quantile(&self, q: f64) -> Option<Quantity> {
        let point = match &self.shape {
            Shape::Mixture(m) => Some(if q == 0.5 { m.median() } else { m.quantile(q) }),
            Shape::Points(points) => summary_quantile(points, q).and_then(|v| v.as_f64()),
        }?;
        Some(Quantity {
            point: Some(point),
            complete: self.complete,
            bounds: None,
            sampling: self.support.map(not_computed),
        })
    }

    /// The `q` quantile of point masses as a value, for printing integers
    /// exactly.
    pub fn quantile_value(&self, q: f64) -> Option<Value> {
        match &self.shape {
            Shape::Points(points) => summary_quantile(points, q),
            Shape::Mixture(_) => None,
        }
    }

    /// The values and their probabilities, as `f64`.
    pub(super) fn points(&self) -> Option<Vec<(f64, f64)>> {
        match &self.shape {
            Shape::Points(points) => Some(
                points
                    .iter()
                    .map(|(v, p)| (v.as_f64().unwrap_or(f64::NAN), *p))
                    .collect(),
            ),
            Shape::Mixture(_) => None,
        }
    }
}

fn not_computed(support: Support) -> Uncertainty {
    Uncertainty {
        status: Status::NotComputed,
        se: None,
        wilson: None,
        support,
    }
}

/// The results of every report, in source order. `unresolved` is the weight
/// the run left unresolved, which decides what's complete; `format` is what
/// the renderer uses.
pub fn results(sites: &[ReportSite], sinks: &[Sink], format: Format, unresolved: Weight) -> Vec<ReportResult> {
    sites
        .iter()
        .zip(sinks)
        .map(|(site, sink)| {
            // A simple report's facts are judged as reached once per world;
            // a table's by how its site is reached.
            let kind = if site.key_label.is_none() {
                ReportKind::Once
            } else {
                site.kind
            };
            ReportResult {
                reach: reach(site.kind, sink, format),
                groups: sink
                    .groups
                    .iter()
                    .map(|(key, acc)| group(key, acc, format, kind, unresolved))
                    .collect(),
            }
        })
        .collect()
}

fn reach(kind: ReportKind, sink: &Sink, format: Format) -> Option<Reach> {
    if kind == ReportKind::PerVisit || sink.groups.is_empty() || format.program_total.is_zero() || !format.reach_known {
        return None;
    }
    if let Some(all_squares) = format.run_squares {
        // The share of the runs' weight that reached the report, counting
        // each run once, and its standard error (section 14).
        let total = format.program_total;
        let p = sink.reached.ratio(total);
        let squares = sink.reached_squares;
        let spread = squares.scale((1.0 - p) * (1.0 - p)) + all_squares.saturating_sub(squares).scale(p * p);
        let se = spread.ratio(total * total).sqrt();
        return Some(Reach { share: p, se: Some(se) });
    }
    Some(Reach {
        share: sink.reach().ratio(format.program_total),
        se: None,
    })
}

pub(super) fn group(key: &Value, acc: &Acc, format: Format, kind: ReportKind, unresolved: Weight) -> GroupResult {
    let sampled = acc.sampled();
    let support = sampled.then(|| Support {
        contributing_runs: acc.contributing_runs(),
        effective: acc.effective(),
    });
    let complete = (unresolved + acc.missing).is_zero();
    let distribution = acc.distribution();
    let fact = acc
        .is_event()
        .then(|| fact(acc, format, kind, complete, unresolved, support));
    let continuous = distribution
        .iter()
        .any(|(v, _)| matches!(v, Value::Analytic(_) | Value::Continuous(_)));
    let values = (!continuous).then(|| values(acc, &distribution, complete, unresolved, support));
    let numeric = if acc.is_event() {
        None
    } else {
        numeric(acc, &distribution, complete, support)
    };
    GroupResult {
        key: key.clone(),
        fact,
        values,
        numeric,
        unresolved_share: acc.unresolved_share(format.unresolved),
        distribution,
        support,
    }
}

/// The chance that a reported fact is true.
fn fact(
    acc: &Acc,
    format: Format,
    kind: ReportKind,
    complete: bool,
    unresolved: Weight,
    support: Option<Support>,
) -> Quantity {
    let p = acc.chance();
    let sampling = support.map(|support| {
        let se = acc.chance_se();
        // A Wilson interval needs ordinary, equally weighted observations, and
        // is given when the standard error isn't useful: every run agreed, or
        // too few of them contributed.
        let wilson = acc
            .chance_interval95(kind)
            .filter(|_| !format.weighted)
            .filter(|_| p == 0.0 || p == 1.0 || support.contributing_runs < 30);
        let integrated = acc.moments.as_ref().is_some_and(|m| m.integrated);
        let (status, se) = if se != 0.0 {
            (Status::Estimated, Some(se))
        } else if integrated {
            (Status::IntegratedZero, Some(0.0))
        } else {
            (Status::NotEstimable, None)
        };
        Uncertainty {
            status,
            se,
            wilson,
            support,
        }
    });
    Quantity {
        point: Some(p),
        complete,
        bounds: (!complete && support.is_none()).then(|| acc.chance_bounds(unresolved)),
        sampling,
    }
}

/// Each reported value's probability, with bounds over the unresolved
/// weight: as if it had all gone to that value, or none of it.
fn values(
    acc: &Acc,
    distribution: &[(Value, f64)],
    complete: bool,
    unresolved: Weight,
    support: Option<Support>,
) -> Vec<(Value, Quantity)> {
    let u = unresolved + acc.missing;
    let resolved = Weight::sum(acc.values.values().copied()) + acc.facts;
    distribution
        .iter()
        .map(|(value, p)| {
            let bounds = (!complete && support.is_none()).then(|| {
                // Facts are counted apart from the other values.
                let w = match value {
                    Value::Bool(true) => acc.yes,
                    Value::Bool(false) => acc.facts.saturating_sub(acc.yes),
                    _ => acc.values.get(value).copied().unwrap_or_default(),
                };
                let total = resolved + u;
                (w.ratio(total), (w + u).ratio(total).min(1.0))
            });
            let sampling = support.map(|support| {
                let se = acc.value_se(value, *p);
                let (status, se) = if se == 0.0 {
                    (Status::NotEstimable, None)
                } else {
                    (Status::Estimated, Some(se))
                };
                Uncertainty {
                    status,
                    se,
                    wilson: None,
                    support,
                }
            });
            let quantity = Quantity {
                point: Some(*p),
                complete,
                bounds,
                sampling,
            };
            (value.clone(), quantity)
        })
        .collect()
}

/// The mean and standard deviation of real numbers, or of a continuous
/// marginal mixed with point masses, as the renderer prints them.
fn numeric(acc: &Acc, distribution: &[(Value, f64)], complete: bool, support: Option<Support>) -> Option<Numeric> {
    let summary = |mean: f64, sd: f64, mean_sampling: Option<Uncertainty>, percent: bool, shape: Shape| {
        // NaN: a continuous outcome whose moments have no formula.
        let quantity = |x: f64, sampling| Quantity {
            point: (!x.is_nan()).then_some(x),
            complete,
            bounds: None,
            sampling,
        };
        Some(Numeric {
            mean: quantity(mean, mean_sampling),
            sd: quantity(sd, support.map(not_computed)),
            percent,
            shape,
            complete,
            support,
        })
    };
    if let Some(m) = super::analytic_mixture(distribution) {
        let (mean, sd) = (m.mean(), m.sd());
        return summary(mean, sd, support.map(not_computed), false, Shape::Mixture(m));
    }
    let nums: Vec<(f64, f64)> = distribution
        .iter()
        .map(|(v, p)| match v {
            Value::Int(n) => n
                .to_f64()
                .filter(|x| n.cmp_f64(*x).is_some_and(|c| c.is_eq()))
                .map(|x| (x, *p)),
            Value::Float(x) | Value::Prob(x) => Some((*x, *p)),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let percent = distribution.iter().all(|(v, _)| matches!(v, Value::Prob(_)));
    let total: f64 = nums.iter().map(|(_, p)| p).sum();
    let mean = nums.iter().map(|(x, p)| x * p).sum::<f64>() / total;
    let sd = (nums.iter().map(|(x, p)| (x - mean).powi(2) * p).sum::<f64>() / total).sqrt();
    if !mean.is_finite() || !sd.is_finite() {
        return None;
    }
    let mean_sampling = support.map(|support| {
        let se = acc.mean_se().1;
        let (status, se) = if se > 0.0 {
            (Status::Estimated, Some(se))
        } else {
            (Status::NotEstimable, None)
        };
        Uncertainty {
            status,
            se,
            wilson: None,
            support,
        }
    });
    summary(mean, sd, mean_sampling, percent, Shape::Points(distribution.to_vec()))
}
