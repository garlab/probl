//! What each report says, as numbers. The renderer formats these, and the
//! `probl` library gives them to programs that embed Probl, so the two can't
//! disagree (docs/library.md).

use super::{Acc, Format, Sink, summary_quantile};
use crate::continuous::Mixture;
use crate::value::Value;
use crate::weight::Weight;
use probl_sema::ir::{ReportKind, ReportSite};
use std::sync::Arc;

/// One report's results.
#[derive(Clone, Debug)]
pub struct ReportResult {
    pub collection: super::ReportCollection,
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
    pub values: Option<ValueProbabilities>,
    /// The values and their probabilities, among the resolved worlds, in the
    /// order of the values (facts count as `true` and `false`).
    pub distribution: Arc<[(Value, f64)]>,
    /// For real numbers, and continuous values: their summaries.
    pub numeric: Option<Numeric>,
    /// The share of the group's weight that isn't resolved, as the renderer
    /// counts it.
    pub unresolved_share: f64,
    /// When sampling: how many runs it rests on.
    pub support: Option<Support>,
    /// For records or lists that hold outcomes of draws: each field's path
    /// (`.x`, `[0]`), with what it says reported alone. Fields can depend on
    /// each other, which their marginals don't show. The other summaries
    /// are then empty.
    pub fields: Option<Box<Fields>>,
}

/// A structured report group's fields: each one's path, with what it says
/// reported alone.
#[derive(Clone, Debug)]
pub struct Fields(pub Vec<(String, GroupResult)>);

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
    /// The mean and sd came from numerical integration, with `@integrate`,
    /// where there's no formula.
    pub integrated: bool,
    shape: Shape,
    /// For the quantiles: the group's completeness and sampling support.
    complete: bool,
    support: Option<Support>,
}

#[derive(Clone, Debug)]
enum Shape {
    Moments,
    Sketch(Arc<super::sketch::Quantiles>),
    /// A continuous marginal, possibly mixed with point masses.
    Mixture(Arc<Mixture>),
    /// Point masses, as values (integers keep their exact value for printing).
    Points(Arc<[(Value, f64)]>),
}

impl Numeric {
    pub fn approximation(&self) -> Option<super::QuantileApproximation> {
        match &self.shape {
            Shape::Sketch(view) => Some(view.metadata),
            _ => None,
        }
    }
    pub fn has_quantiles(&self) -> bool {
        !matches!(self.shape, Shape::Moments)
    }

    /// Weighted inverse CDF, including at 0.5 (unlike the midpoint median).
    pub fn inverse_quantile(&self, q: f64) -> Option<Quantity> {
        if !q.is_finite() || !(0.0..=1.0).contains(&q) {
            return None;
        }
        let point = match &self.shape {
            Shape::Moments => None,
            Shape::Sketch(view) => view.quantile(q),
            Shape::Mixture(m) => Some(m.quantile(q)),
            Shape::Points(points) => crate::stats::quantile(points, q).and_then(Value::as_f64),
        }?;
        Some(Quantity {
            point: Some(point),
            complete: self.complete,
            bounds: None,
            sampling: self.support.map(not_computed),
        })
    }

    /// The continuous marginal, when there's one.
    pub fn mixture(&self) -> Option<&Mixture> {
        match &self.shape {
            Shape::Mixture(m) => Some(m),
            Shape::Points(_) | Shape::Moments | Shape::Sketch(_) => None,
        }
    }

    /// The `q` quantile, by the report convention: the median is the
    /// midpoint of the middle values, the others select an outcome. `q` must
    /// be in [0, 1].
    pub fn quantile(&self, q: f64) -> Option<Quantity> {
        let point = match &self.shape {
            Shape::Moments => None,
            Shape::Sketch(view) => view.quantile(q),
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
            Shape::Mixture(_) | Shape::Moments | Shape::Sketch(_) => None,
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
            Shape::Mixture(_) | Shape::Moments | Shape::Sketch(_) => None,
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
                collection: sink.collection,
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
    if acc.structured {
        return structured(key, acc, format, kind, unresolved);
    }
    let sampled = acc.sampled();
    let support = sampled.then(|| Support {
        contributing_runs: acc.contributing_runs(),
        effective: acc.effective(),
    });
    let complete = (unresolved + acc.missing).is_zero();
    let distribution: Arc<[(Value, f64)]> = acc.distribution().into();
    let fact = acc
        .is_event()
        .then(|| fact(acc, format, kind, complete, unresolved, support));
    let continuous = distribution
        .iter()
        .any(|(v, _)| matches!(v, Value::Analytic(_) | Value::Continuous(_)));
    let values =
        (!continuous && acc.online.is_none()).then(|| values(acc, &distribution, complete, unresolved, support));
    let numeric = if acc.is_event() {
        None
    } else {
        numeric(acc, &distribution, complete, support, format.integrate)
    };
    GroupResult {
        key: key.clone(),
        fact,
        values,
        numeric,
        unresolved_share: acc.unresolved_share(format.unresolved),
        distribution,
        support,
        fields: None,
    }
}

/// A group of records or lists of outcomes of draws: each field summarized
/// as if it were reported alone.
fn structured(key: &Value, acc: &Acc, format: Format, kind: ReportKind, unresolved: Weight) -> GroupResult {
    let mut values: Vec<(&Value, Weight)> = acc.values.iter().map(|(v, w)| (v, *w)).collect();
    values.sort_by(|a, b| a.0.cmp(b.0));
    let mut fields: Vec<(String, Acc)> = Vec::new();
    for (v, w) in values {
        for (path, leaf) in super::fields(v) {
            let i = match fields.iter().position(|(p, _)| *p == path) {
                Some(i) => i,
                None => {
                    fields.push((path, Acc::default()));
                    fields.len() - 1
                }
            };
            let field = &mut fields[i].1;
            field.total += w;
            field.add(&leaf, w);
        }
    }
    let fields = fields
        .into_iter()
        .map(|(path, mut field)| {
            field.missing = acc.missing;
            let result = group(&Value::Str(path.as_str().into()), &field, format, kind, unresolved);
            (path, result)
        })
        .collect();
    GroupResult {
        key: key.clone(),
        fact: None,
        values: None,
        distribution: Arc::from([]),
        numeric: None,
        unresolved_share: acc.unresolved_share(format.unresolved),
        support: None,
        fields: Some(Box::new(Fields(fields))),
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
#[derive(Clone, Debug)]
pub struct ValueProbabilities {
    distribution: Arc<[(Value, f64)]>,
    complete: bool,
    // Only incomplete enumeration needs per-value bounds. Keep the original
    // weight arithmetic rather than reconstructing weights from rounded shares.
    bounds: Option<Arc<[(f64, f64)]>>,
    sampling: Option<SampledValues>,
}

#[derive(Clone, Debug)]
struct SampledValues {
    support: Support,
    visits: super::Base,
    // Numeric reports do not keep per-value sums. Categorical sums are shared
    // and sorted for lookup; sorting here does not change accumulation order.
    sums: Arc<[(Value, super::Sums)]>,
}

impl ValueProbabilities {
    /// Construct quantities as they are read, sharing the exact population
    /// with numeric queries instead of keeping a large Quantity per value.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&Value, Quantity)> {
        self.distribution
            .iter()
            .enumerate()
            .map(|(i, (value, p))| (value, self.quantity(i, value, *p)))
    }

    pub fn first(&self) -> Option<(&Value, Quantity)> {
        self.iter().next()
    }

    pub fn get(&self, value: &Value) -> Option<Quantity> {
        let i = self.distribution.binary_search_by(|(v, _)| v.cmp(value)).ok()?;
        Some(self.quantity(i, value, self.distribution[i].1))
    }

    fn quantity(&self, index: usize, value: &Value, p: f64) -> Quantity {
        let sampling = self.sampling.as_ref().map(|sample| {
            let sums = sample
                .sums
                .binary_search_by(|(v, _)| v.cmp(value))
                .map(|i| sample.sums[i].1)
                .unwrap_or_default();
            let se = sums.standard_error(&sample.visits, p);
            let (status, se) = if se == 0.0 {
                (Status::NotEstimable, None)
            } else {
                (Status::Estimated, Some(se))
            };
            Uncertainty {
                status,
                se,
                wilson: None,
                support: sample.support,
            }
        });
        Quantity {
            point: Some(p),
            complete: self.complete,
            bounds: self.bounds.as_ref().map(|bounds| bounds[index]),
            sampling,
        }
    }
}

fn values(
    acc: &Acc,
    distribution: &Arc<[(Value, f64)]>,
    complete: bool,
    unresolved: Weight,
    support: Option<Support>,
) -> ValueProbabilities {
    let bounds = (!complete && support.is_none()).then(|| {
        let u = unresolved + acc.missing;
        let resolved = Weight::sum(acc.values.values().copied()) + acc.facts;
        let total = resolved + u;
        distribution
            .iter()
            .map(|(value, _)| {
                let w = match value {
                    Value::Bool(true) => acc.yes,
                    Value::Bool(false) => acc.facts.saturating_sub(acc.yes),
                    _ => acc.values.get(value).copied().unwrap_or_default(),
                };
                (w.ratio(total), (w + u).ratio(total).min(1.0))
            })
            .collect()
    });
    let sampling = support.map(|support| {
        let moments = acc.moments();
        let mut sums = moments.values.clone();
        sums.sort_by(|(a, _), (b, _)| a.cmp(b));
        SampledValues {
            support,
            visits: moments.visits,
            sums: sums.into(),
        }
    });
    ValueProbabilities {
        distribution: distribution.clone(),
        complete,
        bounds,
        sampling,
    }
}

/// The mean and standard deviation of real numbers, or of a continuous
/// marginal mixed with point masses, as the renderer prints them.
fn numeric(
    acc: &Acc,
    distribution: &Arc<[(Value, f64)]>,
    complete: bool,
    support: Option<Support>,
    integrate: Option<f64>,
) -> Option<Numeric> {
    let summary = |mean: f64, sd: f64, mean_sampling: Option<Uncertainty>, percent: bool, shape: Shape, integrated| {
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
            integrated,
            shape,
            complete,
            support,
        })
    };
    if let Some(m) = super::analytic_mixture(distribution) {
        let (mean, sd, integrated) = mixture_moments(&m, integrate);
        return summary(
            mean,
            sd,
            support.map(not_computed),
            false,
            Shape::Mixture(Arc::new(m)),
            integrated,
        );
    }
    if let Some(n) = acc.online {
        if n.weight.is_zero() {
            return None;
        }
        let mean_sampling = support.map(|support| {
            let se = acc.mean_se().1;
            Uncertainty {
                status: if se > 0.0 {
                    Status::Estimated
                } else {
                    Status::NotEstimable
                },
                se: (se > 0.0).then_some(se),
                wilson: None,
                support,
            }
        });
        let shape = acc
            .sketch
            .as_ref()
            .and_then(|s| s.view())
            .map_or(Shape::Moments, |v| Shape::Sketch(v.clone()));
        return summary(n.mean, n.sd, mean_sampling, n.percent, shape, false);
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
    summary(
        mean,
        sd,
        mean_sampling,
        percent,
        Shape::Points(distribution.clone()),
        false,
    )
}

/// A mixture's mean and sd: from formulas, or with `@integrate`, by
/// numerical integration for the parts that have none. Whether it took
/// integration.
fn mixture_moments(m: &Mixture, integrate: Option<f64>) -> (f64, f64, bool) {
    use crate::continuous::Part;
    let (mean, sd) = (m.mean(), m.sd());
    let Some(tolerance) = integrate.filter(|_| mean.is_nan()) else {
        return (mean, sd, false);
    };
    let mut parts = Vec::with_capacity(m.parts.len());
    for (part, p) in &m.parts {
        let moments = match part {
            Part::Analytic(a) => a.moments().or_else(|| a.integrated_moments(tolerance)),
            Part::Continuous(f) => Some((f.mean(), f.variance())),
            Part::Point(x) => Some((*x, 0.0)),
        };
        match moments {
            Some(mv) => parts.push((mv, *p)),
            None => return (f64::NAN, f64::NAN, false),
        }
    }
    let total: f64 = parts.iter().map(|(_, p)| p).sum();
    let mean = parts.iter().map(|((m, _), p)| m * p).sum::<f64>() / total;
    let variance = parts
        .iter()
        .map(|((m, v), p)| p * (v + (m - mean).powi(2)))
        .sum::<f64>()
        / total;
    (mean, variance.sqrt(), true)
}
