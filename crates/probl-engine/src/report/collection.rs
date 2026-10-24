//! Explicit retention policies, separate from displayed summary columns.
use super::{ReportStatistic, ReportSummary};
use crate::{RuntimeError, error::OpError, value::Value, weight::Weight};
use probl_sema::ir::{Mode, ReportSite};
use std::{collections::BTreeMap, fmt, str::FromStr};

/// What a report retains. Exact collection is the default. More policies may
/// be added, so matches outside this crate need a wildcard arm.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReportCollection {
    #[default]
    Exact,
    /// Sampled scalar numbers or facts only. Numeric populations and quantiles
    /// are unavailable; weighted means, spreads and sampling errors remain.
    Moments,
    /// Unweighted sampled scalar numbers, once per trial/key. Retains moments
    /// and an approximate inverse-CDF sketch, not the exact population.
    /// Capacity must be within 128..=4096; 512 is the initial default.
    Quantiles { capacity: usize },
}

impl fmt::Display for ReportCollection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact => f.write_str("exact"),
            Self::Moments => f.write_str("moments"),
            Self::Quantiles { capacity } => write!(f, "quantiles:{capacity}"),
        }
    }
}

/// A collection policy that doesn't parse, or names an invalid capacity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportCollectionError(&'static str);
impl fmt::Display for ReportCollectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for ReportCollectionError {}

impl FromStr for ReportCollection {
    type Err = ReportCollectionError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "exact" => Ok(Self::Exact),
            "moments" => Ok(Self::Moments),
            "quantiles" => Ok(Self::Quantiles { capacity: 512 }),
            _ => {
                if let Some(capacity) = s.strip_prefix("quantiles:").and_then(|s| s.parse::<usize>().ok()) {
                    if (128..=4096).contains(&capacity) {
                        return Ok(Self::Quantiles { capacity });
                    }
                    return Err(ReportCollectionError("quantile capacity must be within 128..=4096"));
                }
                Err(ReportCollectionError(
                    "report collection must be exact, moments or quantiles[:CAPACITY]",
                ))
            }
        }
    }
}

pub fn validate_collections(
    sites: &[ReportSite],
    collections: &BTreeMap<usize, ReportCollection>,
    summaries: &BTreeMap<usize, ReportSummary>,
    mode: &Mode,
) -> Result<(), RuntimeError> {
    if collections.len() > 256 {
        return Err(OpError::new("at most 256 report collection overrides are allowed").at(Default::default()));
    }
    for (&id, &policy) in collections {
        let site = sites
            .get(id)
            .ok_or_else(|| OpError::new(format!("unknown report collection ID {id}")).at(Default::default()))?;
        if let ReportCollection::Quantiles { capacity } = policy {
            let message = if !(128..=4096).contains(&capacity) {
                Some("quantile capacity must be within 128..=4096")
            } else if !matches!(mode, Mode::Sample { .. }) {
                Some("quantile collection requires sample mode")
            } else {
                None
            };
            if let Some(message) = message {
                return Err(OpError::new(format!("report {id}: {message}")).at(site.span));
            }
        }
        if policy == ReportCollection::Moments {
            let message = if !matches!(mode, Mode::Sample { .. }) {
                Some("moments collection requires sample mode")
            } else if summaries.get(&id).is_some_and(|s| {
                s.statistics()
                    .iter()
                    .any(|s| matches!(s, ReportStatistic::Median | ReportStatistic::Quantile(_)))
            }) {
                Some(
                    "moments collection cannot provide median or quantile columns; choose mean and/or sd, or exact collection",
                )
            } else {
                None
            };
            if let Some(message) = message {
                return Err(OpError::new(format!("report {id}: {message}")).at(site.span));
            }
        }
    }
    Ok(())
}

/// Fill in the moments-only default without changing exact report defaults.
pub fn selected_summaries(
    collections: &BTreeMap<usize, ReportCollection>,
    summaries: &BTreeMap<usize, ReportSummary>,
) -> BTreeMap<usize, ReportSummary> {
    let mut selected = summaries.clone();
    for (&id, &policy) in collections {
        if policy == ReportCollection::Moments {
            selected.entry(id).or_insert_with(ReportSummary::moments);
        } else if matches!(policy, ReportCollection::Quantiles { .. }) {
            selected.entry(id).or_insert_with(ReportSummary::standard);
        }
    }
    selected
}

/// Preserve the exact collector's requirement that integer summaries be
/// representable without rounding. Booleans are handled separately.
pub(super) fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Int(n) => n.to_f64().filter(|x| n.cmp_f64(*x).is_some_and(|c| c.is_eq())),
        Value::Float(x) | Value::Prob(x) if x.is_finite() => Some(*x),
        _ => None,
    }
}

/// Weighted parallel central moments. Store SD instead of the squared spread
/// so finite spreads remain representable even when their squares overflow.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct OnlineNumbers {
    pub weight: Weight,
    pub mean: f64,
    pub sd: f64,
    pub percent: bool,
}

impl OnlineNumbers {
    pub fn add(&mut self, value: f64, percent: bool, weight: Weight) {
        if weight.is_zero() {
            return;
        }
        if self.weight.is_zero() {
            *self = Self {
                weight,
                mean: value,
                sd: 0.0,
                percent,
            };
            return;
        }
        let total = self.weight + weight;
        let a = self.weight.ratio(total);
        let b = weight.ratio(total);
        let root_a = a.sqrt();
        let cross = root_a * b.sqrt();
        let delta = value - self.mean;
        let spread = if delta.is_finite() {
            delta * cross
        } else {
            value * cross - self.mean * cross
        };
        // A single point has zero spread: hypot(own_sd, 0) is exactly own_sd.
        // Preserve the general merge's rounding order and overflow protection.
        self.sd = libm::hypot(self.sd * root_a, spread);
        self.mean = if delta.is_finite() {
            self.mean + b * delta
        } else {
            a * self.mean + b * value
        };
        self.weight = total;
        self.percent &= percent;
    }
    pub fn absorb(&mut self, other: Self) {
        if other.weight.is_zero() {
            return;
        }
        if self.weight.is_zero() {
            *self = other;
            return;
        }
        let weight = self.weight + other.weight;
        let a = self.weight.ratio(weight);
        let b = other.weight.ratio(weight);
        let cross = a.sqrt() * b.sqrt();
        let delta = other.mean - self.mean;
        let spread = if delta.is_finite() {
            delta * cross
        } else {
            other.mean * cross - self.mean * cross
        };
        self.sd = libm::hypot(libm::hypot(self.sd * a.sqrt(), other.sd * b.sqrt()), spread);
        self.mean = if delta.is_finite() {
            self.mean + b * delta
        } else {
            a * self.mean + b * other.mean
        };
        self.weight = weight;
        self.percent &= other.percent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn point(x: f64, w: f64) -> OnlineNumbers {
        OnlineNumbers {
            weight: Weight::ONE.scale(w),
            mean: x,
            sd: 0.0,
            percent: false,
        }
    }

    #[test]
    fn point_updates_match_general_merges_bit_for_bit() {
        let values = [
            0.0,
            -0.0,
            f64::from_bits(1),
            -f64::MIN_POSITIVE,
            1.0,
            -3.5,
            1e12,
            1e12 + 1.0,
            1e308,
            -1e308,
        ];
        let weights = [
            Weight::ZERO,
            Weight::ONE,
            Weight::new(0.3),
            Weight::new(7.0),
            Weight::from_ln(-3000.0),
            Weight::from_ln(3000.0),
        ];
        for &initial in &values {
            for &initial_weight in &weights {
                let mut fast = OnlineNumbers {
                    weight: initial_weight,
                    mean: initial,
                    sd: 0.0,
                    percent: true,
                };
                let mut general = fast;
                for (i, &value) in values.iter().cycle().take(100).enumerate() {
                    let weight = weights[i % weights.len()];
                    let percent = i % 3 != 0;
                    fast.add(value, percent, weight);
                    general.absorb(OnlineNumbers {
                        weight,
                        mean: value,
                        sd: 0.0,
                        percent,
                    });
                    assert_eq!(fast.weight, general.weight);
                    assert_eq!(fast.mean.to_bits(), general.mean.to_bits());
                    assert_eq!(fast.sd.to_bits(), general.sd.to_bits());
                    assert_eq!(fast.percent, general.percent);
                }
            }
        }
    }

    #[test]
    fn central_moments_preserve_small_spreads_and_large_finite_values() {
        let mut n = point(1e12, 1.0);
        n.absorb(point(1e12 + 1.0, 1.0));
        assert_eq!(n.mean, 1e12 + 0.5);
        assert!((n.sd - 0.5).abs() < 1e-15);
        let mut n = point(-1e308, 1.0);
        n.absorb(point(1e308, 1.0));
        assert_eq!(n.mean, 0.0);
        assert!((n.sd / 1e308 - 1.0).abs() < 1e-15);
        let mut n = point(1.0, 1.0);
        n.absorb(point(3.0, 3.0));
        assert!((n.mean - 2.5).abs() < 1e-15);
        assert!((n.sd - 0.75_f64.sqrt()).abs() < 1e-15);
    }
}
