//! Display units and per-run uncertainty for time-valued reports. Canonical
//! literals and typed JSON values keep their original precision.
use super::{GroupResult, ReportStatistic, Status, Support, collection::OnlineNumbers};
use crate::{duration, value::Value, weight::Weight};

fn offset(value: &Value, origin: &Value) -> Option<f64> {
    match (value, origin) {
        (Value::DateTime(a), Value::DateTime(b)) => Some((a.total_nanos() - b.total_nanos()) as f64),
        (Value::Duration(a), Value::Duration(b)) if a.squared == b.squared => {
            match (a.amount.as_integer(), b.amount.as_integer()) {
                (Some(a), Some(b)) => a.sub(b).ok()?.to_f64(),
                _ => Some(a.amount.as_float()? - b.amount.as_float()?),
            }
        }
        _ => None,
    }
    .filter(|x| x.is_finite())
}

#[derive(Clone, Debug)]
pub(super) struct Run {
    origin: Value,
    values: OnlineNumbers,
    invalid: bool,
}

impl Run {
    pub fn new(value: &Value) -> Self {
        Self {
            origin: value.clone(),
            values: OnlineNumbers::default(),
            invalid: false,
        }
    }

    pub fn record(&mut self, value: &Value, share: f64) {
        if share == 0.0 {
            return;
        }
        if let Some(x) = offset(value, &self.origin) {
            self.values.add(x, false, Weight::new(share));
        } else {
            self.invalid = true;
        }
    }
}

/// For run means x_i and visit weights u_i = w_i b_i, the ratio estimator's
/// variance is Σ u_i²(x_i − μ)² / (Σ u_i)². Two central-moment accumulators
/// evaluate it without subtracting large raw moments. Exact relative origins
/// preserve nanosecond spreads even for far-future dates and large durations.
#[derive(Clone, Debug, Default)]
pub(super) struct Sampling {
    origin: Option<Value>,
    ordinary: OnlineNumbers,
    squared: OnlineNumbers,
    invalid: bool,
}

impl Sampling {
    pub fn same_kind(&self, value: &Value) -> bool {
        match (&self.origin, value) {
            (Some(Value::DateTime(_)), Value::DateTime(_)) => true,
            (Some(Value::Duration(a)), Value::Duration(b)) => a.squared == b.squared,
            _ => false,
        }
    }

    pub fn effective(&self) -> Option<f64> {
        (!self.invalid && !self.squared.weight.is_zero())
            .then(|| (self.ordinary.weight * self.ordinary.weight).ratio(self.squared.weight))
    }

    pub fn add(&mut self, run: &Run, weight: Weight) {
        self.invalid |= run.invalid;
        let origin = self.origin.get_or_insert_with(|| run.origin.clone());
        let Some(shift) = offset(&run.origin, origin) else {
            self.invalid = true;
            return;
        };
        let x = shift + run.values.mean;
        let u = weight * run.values.weight;
        self.ordinary.add(x, false, u);
        self.squared.add(x, false, u * u);
    }

    pub fn absorb(&mut self, mut other: Self) {
        self.invalid |= other.invalid;
        let Some(origin) = &other.origin else {
            return;
        };
        let Some(mine) = &self.origin else {
            other.invalid |= self.invalid;
            *self = other;
            return;
        };
        let Some(shift) = offset(origin, mine) else {
            self.invalid = true;
            return;
        };
        other.ordinary.mean += shift;
        other.squared.mean += shift;
        self.ordinary.absorb(other.ordinary);
        self.squared.absorb(other.squared);
    }

    pub fn uncertainty(&self, support: Support) -> Uncertainty {
        let se = (!self.invalid && !self.ordinary.weight.is_zero())
            .then(|| {
                let spread = libm::hypot(self.squared.sd, self.squared.mean - self.ordinary.mean);
                spread
                    * self
                        .squared
                        .weight
                        .ratio(self.ordinary.weight * self.ordinary.weight)
                        .sqrt()
            })
            .filter(|x| x.is_finite());
        let status = match se {
            Some(x) if x > 0.0 && support.contributing_runs > 1 => Status::Estimated,
            Some(_) => Status::NotEstimable,
            None => Status::NotComputed,
        };
        let squared = matches!(&self.origin, Some(Value::Duration(d)) if d.squared);
        Uncertainty {
            status,
            standard_error: se
                .filter(|_| status == Status::Estimated)
                .and_then(|x| duration::from_number(Value::Float(x), squared).ok()),
            support,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Uncertainty {
    pub status: Status,
    /// Duration for datetime/duration means; squared duration for squared-time means.
    pub standard_error: Option<Value>,
    pub support: Support,
}

struct Display {
    unit: &'static str,
    base: u64,
    factor: f64,
    squared: bool,
    decimals: usize,
    clock_step: i128,
}

impl Display {
    fn new(values: &[&Value], scale_values: &[&Value]) -> Self {
        let durations: Vec<_> = values
            .iter()
            .filter_map(|v| match v {
                Value::Duration(d) => Some(d),
                _ => None,
            })
            .collect();
        let squared = durations.first().is_some_and(|d| d.squared);
        let minimum = durations
            .iter()
            .filter_map(|d| d.amount.as_float())
            .map(f64::abs)
            .filter(|x| *x > 0.0 && x.is_finite())
            .reduce(f64::min);
        let power = if squared { 2 } else { 1 };
        let (unit, base) = if let Some(minimum) = minimum {
            probl_syntax::duration::UNITS
                .into_iter()
                .find(|(_, n)| minimum >= (*n as f64).powi(power))
                .unwrap_or(("ns", 1))
        } else {
            ("s", 1_000_000_000)
        };
        let factor = (base as f64).powi(power);
        let scale = scale_values
            .iter()
            .filter_map(|v| match v {
                Value::Duration(d) => d.amount.as_float(),
                _ => None,
            })
            .map(|x| x.abs() / factor)
            .fold(0.0_f64, f64::max);
        let decimals = if scale < 100.0 { 2 } else { 0 };
        // Calendar labels use decimal fractions of a second, never arbitrary
        // multiples of minutes. Preserve fine clocks when the spread is tiny.
        let clock_step = if minimum.is_none() {
            1
        } else {
            let exponent = (libm::log10(factor) - decimals as f64).floor().clamp(0.0, 9.0) as u32;
            10_i128.pow(exponent)
        };
        Self {
            unit,
            base,
            factor,
            squared,
            decimals,
            clock_step,
        }
    }

    fn duration(&self, d: &probl_syntax::duration::Duration, compact: bool) -> String {
        let Some(n) = d.amount.as_float().filter(|x| x.is_finite()) else {
            // Huge exact integers have no binary64 approximation. Keep them
            // exact rather than turning a valid value into infinity.
            return d.format();
        };
        let text = if let Some(integer) = d.amount.as_integer().filter(|i| i.bits() > 53) {
            self.exact_integer(integer, compact)
        } else {
            let n = n / self.factor;
            let decimals = if compact && n.fract() == 0.0 { 0 } else { self.decimals };
            super::fixed(n, decimals)
        };
        format!("{}{}{}", text, self.unit, if self.squared { "^2" } else { "" })
    }

    fn exact_integer(&self, n: &probl_number::Integer, compact: bool) -> String {
        use probl_number::Integer;
        let scale = Integer::from(self.base).pow(if self.squared { 2 } else { 1 }).unwrap();
        let (mut whole, remainder) = n.abs().div_mod(&scale).unwrap();
        let places = if compact && remainder.is_zero() {
            0
        } else {
            self.decimals
        };
        let factor = 10_i64.pow(places as u32);
        // Multiply only the bounded remainder, not the potentially huge value.
        let (digits, remainder) = remainder.mul(&Integer::from(factor)).unwrap().div_mod(&scale).unwrap();
        let mut digits = digits.to_i64().unwrap();
        let twice = remainder.mul(&Integer::from(2)).unwrap();
        if twice > scale || (twice == scale && if places == 0 { whole.is_odd() } else { digits % 2 == 1 }) {
            digits += 1;
        }
        if digits == factor {
            whole = whole.add(&Integer::ONE).unwrap();
            digits = 0;
        }
        let sign = if n.is_negative() && (!whole.is_zero() || digits != 0) {
            "-"
        } else {
            ""
        };
        let fraction = if places == 0 {
            String::new()
        } else {
            format!(".{digits:0places$}")
        };
        format!("{sign}{}{fraction}", super::integer_text(&whole))
    }

    fn value(&self, value: &Value, compact: bool) -> String {
        match value {
            Value::Duration(d) => self.duration(d, compact),
            Value::DateTime(d) => {
                let n = d.total_nanos();
                let rounded = n.div_euclid(self.clock_step) * self.clock_step
                    + if n.rem_euclid(self.clock_step) * 2 > self.clock_step {
                        self.clock_step
                    } else {
                        0
                    };
                crate::dates::DateTime::from_total_nanos(rounded).unwrap_or(*d).format()
            }
            _ => super::display(value),
        }
    }
}

pub(super) fn scalar(value: &Value) -> String {
    Display::new(&[value], &[value]).value(value, true)
}

/// Resolve values once, then choose a common unit for the displayed row.
pub(super) struct Summary<'a> {
    columns: &'a [ReportStatistic],
    values: Vec<Option<Value>>,
    se: Option<&'a Value>,
    display: Display,
}

impl<'a> Summary<'a> {
    pub fn new(group: &'a GroupResult, columns: &'a [ReportStatistic]) -> Option<Self> {
        let kind = super::temporal_kind(&group.distribution)?;
        let moments = crate::time_stats::moments(&group.distribution, None).ok().flatten();
        let values: Vec<_> = columns
            .iter()
            .map(|s| match s {
                ReportStatistic::Mean => moments.as_ref().map(|m| m.mean.clone()),
                ReportStatistic::StandardDeviation => moments.as_ref().and_then(|m| m.sd.clone()),
                _ => super::statistic_value(group, *s),
            })
            .collect();
        let se = group
            .temporal_mean_sampling
            .as_ref()
            .and_then(|s| s.standard_error.as_ref())
            .filter(|_| columns.contains(&ReportStatistic::Mean));
        let mut shown: Vec<_> = values.iter().flatten().collect();
        shown.extend(se);
        let clock_spread = if kind == "datetime" && !shown.iter().any(|v| matches!(v, Value::Duration(_))) {
            let clocks: Vec<_> = shown
                .iter()
                .filter_map(|v| match v {
                    Value::DateTime(d) => Some(d.total_nanos()),
                    _ => None,
                })
                .collect();
            clocks
                .iter()
                .min()
                .zip(clocks.iter().max())
                .and_then(|(lo, hi)| duration::from_number(Value::Int((hi - lo).into()), false).ok())
        } else {
            None
        };
        shown.extend(clock_spread.as_ref());
        let scale: Vec<_> = columns
            .iter()
            .zip(&values)
            .filter_map(|(s, v)| {
                matches!(s, ReportStatistic::Mean | ReportStatistic::StandardDeviation)
                    .then_some(v.as_ref())
                    .flatten()
                    .filter(|v| matches!(v, Value::Duration(_)))
            })
            .collect();
        let display = Display::new(&shown, if scale.is_empty() { &shown } else { &scale });
        Some(Self {
            columns,
            values,
            se,
            display,
        })
    }

    pub fn cell(&self, index: usize) -> String {
        let Some(value) = &self.values[index] else {
            return "unavailable".into();
        };
        let column = self.columns[index];
        let compact = !matches!(column, ReportStatistic::Mean | ReportStatistic::StandardDeviation);
        let text = self.display.value(value, compact);
        if column == ReportStatistic::Mean
            && let Some(Value::Duration(se)) = self.se
        {
            let coefficient = se.amount.as_float().unwrap_or(0.0) / self.display.factor;
            if coefficient >= 0.5 * libm::pow(10.0, -(self.display.decimals as f64)) {
                return format!("{text} ± {}", self.display.duration(se, false));
            }
        }
        text
    }

    pub fn line(&self) -> String {
        self.columns
            .iter()
            .enumerate()
            .map(|(i, s)| format!("{} {}", s.label(), self.cell(i)))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}
