//! Explicit summary columns. Collection remains exact; this selects what to
//! compute for display, not a smaller-memory accumulator.
use super::{GroupResult, Numeric, Quantity, summary_quantile};
use crate::value::Value;
use std::{fmt, str::FromStr};

/// One requested report column. A median uses midpoint ties; a quantile uses
/// the weighted inverse CDF, including at level 0.5.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReportStatistic {
    Mean,
    StandardDeviation,
    Median,
    Quantile(f64),
}

impl ReportStatistic {
    pub fn label(self) -> String {
        match self {
            Self::Mean => "mean".into(),
            Self::StandardDeviation => "sd".into(),
            Self::Median => "median".into(),
            Self::Quantile(q) => {
                // Shift the shortest decimal representation, avoiding 0.29's
                // multiplication roundoff in a column headed "29%".
                if q == 1.0 {
                    return "100%".into();
                }
                let raw = q.to_string();
                let mut digits = raw.strip_prefix("0.").unwrap_or("0").to_string();
                while digits.len() < 2 {
                    digits.push('0');
                }
                let (whole, fraction) = digits.split_at(2);
                let whole = whole.parse::<u32>().unwrap_or(0);
                let fraction = fraction.trim_end_matches('0');
                if fraction.is_empty() {
                    format!("{whole}%")
                } else {
                    format!("{whole}.{fraction}%")
                }
            }
        }
    }

    pub fn token(self) -> String {
        match self {
            Self::Quantile(q) => format!("q:{q}"),
            _ => self.label(),
        }
    }
}

/// Checked, ordered summary columns. Empty selections, duplicates, more than
/// 32 columns and nonfinite/out-of-range quantiles are rejected.
#[derive(Clone, Debug, PartialEq)]
pub struct ReportSummary(Vec<ReportStatistic>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportSummaryError(String);
impl fmt::Display for ReportSummaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ReportSummaryError {}

impl ReportSummary {
    pub fn new(statistics: Vec<ReportStatistic>) -> Result<Self, ReportSummaryError> {
        if statistics.is_empty() || statistics.len() > 32 {
            return Err(ReportSummaryError("a report summary needs 1 to 32 columns".into()));
        }
        for (i, s) in statistics.iter().enumerate() {
            if matches!(s, ReportStatistic::Quantile(q) if !q.is_finite() || !(0.0..=1.0).contains(q)) {
                return Err(ReportSummaryError(
                    "quantile levels must be finite numbers within 0..1".into(),
                ));
            }
            if statistics[..i].contains(s) {
                return Err(ReportSummaryError(format!("duplicate summary column `{}`", s.token())));
            }
        }
        Ok(Self(statistics))
    }

    /// The article's 10/25/50/75/90 presentation, with a midpoint median.
    pub fn forecast() -> Self {
        Self::bands(0.1, 0.9)
    }
    /// The existing grouped 5/25/50/75/95 presentation.
    pub fn standard() -> Self {
        Self::bands(0.05, 0.95)
    }
    pub fn moments() -> Self {
        Self(vec![ReportStatistic::Mean, ReportStatistic::StandardDeviation])
    }
    fn bands(lo: f64, hi: f64) -> Self {
        use ReportStatistic::{Median, Quantile};
        Self(vec![Quantile(lo), Quantile(0.25), Median, Quantile(0.75), Quantile(hi)])
    }
    pub fn statistics(&self) -> &[ReportStatistic] {
        &self.0
    }
}

impl FromStr for ReportSummary {
    type Err = ReportSummaryError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() > 4096 {
            return Err(ReportSummaryError("report summary text exceeds 4096 bytes".into()));
        }
        match text.trim() {
            "standard" => return Ok(Self::standard()),
            "forecast" => return Ok(Self::forecast()),
            "moments" => return Ok(Self::moments()),
            _ => {}
        }
        let statistics = text
            .split(',')
            .map(|part| {
                let token = part.trim();
                match token {
                    "mean" => Ok(ReportStatistic::Mean),
                    "sd" => Ok(ReportStatistic::StandardDeviation),
                    "median" => Ok(ReportStatistic::Median),
                    _ => token
                        .strip_prefix("q:")
                        .and_then(|q| q.parse().ok())
                        .map(ReportStatistic::Quantile)
                        .ok_or_else(|| {
                            ReportSummaryError(format!(
                                "unknown summary column `{token}`; use mean, sd, median or q:LEVEL"
                            ))
                        }),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(statistics)
    }
}

impl Numeric {
    pub fn statistic(&self, statistic: ReportStatistic) -> Option<Quantity> {
        match statistic {
            ReportStatistic::Mean => Some(self.mean),
            ReportStatistic::StandardDeviation => Some(self.sd),
            ReportStatistic::Median => self.quantile(0.5),
            ReportStatistic::Quantile(q) => self.inverse_quantile(q),
        }
    }
}

/// Exact discrete values for quantile and median columns (including dates and
/// integers which cannot be represented by an f64 summary).
pub fn statistic_value(group: &GroupResult, statistic: ReportStatistic) -> Option<Value> {
    if group.numeric.as_ref().is_some_and(|n| n.mixture().is_some()) {
        return None;
    }
    match statistic {
        ReportStatistic::Median => summary_quantile(&group.distribution, 0.5),
        ReportStatistic::Quantile(q) => crate::stats::quantile(&group.distribution, q).cloned(),
        _ => None,
    }
}
