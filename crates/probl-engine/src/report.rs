//! Collecting `report` values across worlds, and printing them
//! (docs/semantics.md, sections 9, 10 and 14).

use crate::value::{Value, fmt_float};
use crate::weight::Weight;
use probl_sema::ir::{Program, ReportKind};
use rustc_hash::FxHashMap;
use std::collections::BTreeMap;
use std::fmt::Write;

/// Everything one report site saw for one key.
#[derive(Clone, Debug)]
pub struct Acc {
    /// Weight of the worlds that reached the report.
    pub total: Weight,
    /// Of the reported facts, the weight where they're true.
    pub yes: Weight,
    /// Weight of the reported facts.
    pub facts: Weight,
    /// Weight of every other reported value.
    pub values: FxHashMap<Value, Weight>,
    /// Weight of reported distributions' missing mass.
    pub missing: Weight,
    /// When sampling: what each run of the current batch reported.
    runs: FxHashMap<u32, RunStat>,
    /// When sampling: sums over the finished runs, for standard errors.
    moments: Option<Moments>,
}

impl Default for Acc {
    fn default() -> Acc {
        Acc {
            total: Weight::ZERO,
            yes: Weight::ZERO,
            facts: Weight::ZERO,
            values: FxHashMap::default(),
            missing: Weight::ZERO,
            runs: FxHashMap::default(),
            moments: None,
        }
    }
}

/// Sums over sampled runs, for the standard error of an estimate
/// Σ wᵢaᵢ / Σ wᵢbᵢ by the delta method (docs/semantics.md, section 14):
/// Σ wᵢ²aᵢ² and Σ wᵢ²aᵢbᵢ, the second split into its positive and negative
/// terms (weights can't be negative).
#[derive(Clone, Copy, Debug, Default)]
struct Sums {
    aa: Weight,
    ab: Weight,
    ab_negative: Weight,
}

/// Σ wᵢbᵢ and Σ wᵢ²bᵢ², shared by the estimates with the same bᵢ.
#[derive(Clone, Copy, Debug, Default)]
struct Base {
    b: Weight,
    bb: Weight,
}

impl Base {
    fn add(&mut self, w: Weight, b: f64) {
        self.b += w.scale(b);
        self.bb += (w * w).scale(b * b);
    }

    fn absorb(&mut self, other: Base) {
        self.b += other.b;
        self.bb += other.bb;
    }
}

impl Sums {
    fn absorb(&mut self, other: Sums) {
        self.aa += other.aa;
        self.ab += other.ab;
        self.ab_negative += other.ab_negative;
    }

    fn add(&mut self, w: Weight, a: f64, b: f64) {
        let w2 = w * w;
        self.aa += w2.scale(a * a);
        if a * b >= 0.0 {
            self.ab += w2.scale(a * b);
        } else {
            self.ab_negative += w2.scale(-a * b);
        }
    }

    /// √(Σ wᵢ²(aᵢ − p bᵢ)²) / Σ wᵢbᵢ, expanded into the sums.
    fn standard_error(&self, base: &Base, p: f64) -> f64 {
        if base.b.is_zero() {
            return 0.0;
        }
        let d = base.b * base.b;
        let (aa, bb) = (self.aa.ratio(d), p * p * base.bb.ratio(d));
        let ab = self.ab.ratio(d) - self.ab_negative.ratio(d);
        let v = aa - 2.0 * p * ab + bb;
        // Below the rounding error of its terms, the variance is zero: every
        // run reported the same thing.
        if v <= 1e-12 * (aa + bb) {
            return 0.0;
        }
        v.sqrt()
    }
}

/// What the finished runs reported for one key, summed.
#[derive(Clone, Debug, Default)]
struct Moments {
    /// Σ w and Σ w² over the runs that reached the report.
    weight: Weight,
    squares: Weight,
    /// Facts: a is P(true), b the probability of a fact.
    facts: Base,
    yes: Sums,
    /// Numbers: a is the value (times its probability), b the probability of
    /// a number; Σ wᵢaᵢ in positive and negative parts, for the mean.
    numbers: Base,
    sum: Sums,
    sum_positive: Weight,
    sum_negative: Weight,
    /// Each value's probability: a is its share of a run's visits, b the
    /// number of visits.
    visits: Base,
    values: Vec<(Value, Sums)>,
}

impl Moments {
    fn absorb(&mut self, other: Moments) {
        self.weight += other.weight;
        self.squares += other.squares;
        self.facts.absorb(other.facts);
        self.yes.absorb(other.yes);
        self.numbers.absorb(other.numbers);
        self.sum.absorb(other.sum);
        self.sum_positive += other.sum_positive;
        self.sum_negative += other.sum_negative;
        self.visits.absorb(other.visits);
        for (v, sums) in other.values {
            match self.values.iter_mut().find(|(x, _)| *x == v) {
                Some((_, mine)) => mine.absorb(sums),
                None => self.values.push((v, sums)),
            }
        }
    }
}

/// What one sampled run reported for one key, added up over its visits: the
/// aᵢ and bᵢ of docs/semantics.md, section 14.
#[derive(Clone, Debug)]
pub struct RunStat {
    /// The run's weight (its final weight: no observation follows a report).
    pub weight: Weight,
    pub visits: u32,
    /// The probability that the reported fact was true.
    pub yes: f64,
    /// The probability that the reported value was a fact.
    pub facts: f64,
    /// The reported numbers, times their probabilities.
    pub sum: f64,
    /// The probability that the reported value was a number.
    pub numbers: f64,
    /// The probability of each other value (dates excluded).
    pub others: Vec<(Value, f64)>,
}

impl RunStat {
    fn record(&mut self, value: &Value, share: f64) {
        match value {
            Value::Bool(b) => {
                self.facts += share;
                if *b {
                    self.yes += share;
                }
            }
            Value::Dist(d) => {
                for (x, q) in &d.outcomes {
                    self.record(x, share * q);
                }
            }
            Value::Int(_) | Value::Float(_) | Value::Prob(_) => {
                self.numbers += share;
                self.sum += share * value.as_f64().unwrap();
            }
            Value::Date(_) => {}
            other => match self.others.iter_mut().find(|(v, _)| v == other) {
                Some((_, s)) => *s += share,
                None => self.others.push((other.clone(), share)),
            },
        }
    }
}

impl Acc {
    fn add(&mut self, value: &Value, weight: Weight) {
        match value {
            Value::Bool(b) => {
                self.facts += weight;
                if *b {
                    self.yes += weight;
                }
            }
            Value::Dist(d) => {
                self.missing += weight.scale(d.missing);
                for (x, q) in &d.outcomes {
                    self.add(x, weight.scale(*q));
                }
            }
            other => {
                let slot = self.values.entry(other.clone()).or_insert(Weight::ZERO);
                *slot += weight;
            }
        }
    }

    /// Whether every reported value was a fact (or a distribution of facts).
    pub fn is_event(&self) -> bool {
        self.values.is_empty() && !self.facts.is_zero()
    }

    /// The probability that the reported fact is true, among the resolved
    /// worlds that reached the report.
    pub fn chance(&self) -> f64 {
        self.yes.ratio(self.facts)
    }

    /// Bounds on that probability, given `unresolved` weight elsewhere
    /// (docs/semantics.md, section 10).
    pub fn chance_bounds(&self, unresolved: Weight) -> (f64, f64) {
        let u = unresolved + self.missing;
        let denom = self.facts + u;
        (self.yes.ratio(denom), (self.yes + u).ratio(denom).min(1.0))
    }

    /// The reported values as a sorted distribution, conditional on being
    /// resolved (facts count as `true`/`false` when mixed with other values).
    pub fn distribution(&self) -> Vec<(Value, f64)> {
        let mut pairs: Vec<(Value, Weight)> = self.values.iter().map(|(v, w)| (v.clone(), *w)).collect();
        if !self.facts.is_zero() {
            pairs.push((Value::Bool(true), self.yes));
            pairs.push((Value::Bool(false), self.facts.saturating_sub(self.yes)));
        }
        let total = Weight::sum(pairs.iter().map(|(_, w)| *w));
        let mut out: Vec<(Value, f64)> = pairs.into_iter().map(|(v, w)| (v, w.ratio(total))).collect();
        out.retain(|(_, p)| *p > 0.0);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// The share of this report's weight that isn't resolved.
    fn unresolved_share(&self, unresolved: Weight) -> f64 {
        let u = unresolved + self.missing;
        u.ratio(self.total + unresolved)
    }

    /// Add what another batch of runs reported for this key.
    fn absorb(&mut self, other: Acc) {
        debug_assert!(
            self.runs.is_empty() && other.runs.is_empty(),
            "batches end before they're combined"
        );
        self.total += other.total;
        self.yes += other.yes;
        self.facts += other.facts;
        self.missing += other.missing;
        for (v, w) in other.values {
            *self.values.entry(v).or_insert(Weight::ZERO) += w;
        }
        match (&mut self.moments, other.moments) {
            (Some(mine), Some(theirs)) => mine.absorb(theirs),
            (mine @ None, theirs) => *mine = theirs,
            (Some(_), None) => {}
        }
    }

    /// Whether the values come from sampled runs.
    pub fn sampled(&self) -> bool {
        self.moments.is_some() || !self.runs.is_empty()
    }

    /// Fold the current batch's runs into the sums: they're finished.
    fn end_batch(&mut self) {
        if self.runs.is_empty() {
            return;
        }
        let m = self.moments.get_or_insert_with(Moments::default);
        for (_, run) in self.runs.drain() {
            let w = run.weight;
            m.weight += w;
            m.squares += w * w;
            m.facts.add(w, run.facts);
            m.yes.add(w, run.yes, run.facts);
            m.numbers.add(w, run.numbers);
            m.sum.add(w, run.sum, run.numbers);
            if run.sum >= 0.0 {
                m.sum_positive += w.scale(run.sum);
            } else {
                m.sum_negative += w.scale(-run.sum);
            }
            let visits = run.visits as f64;
            m.visits.add(w, visits);
            let mut shares = run.others;
            if run.facts > 0.0 {
                shares.push((Value::Bool(true), run.yes));
                shares.push((Value::Bool(false), run.facts - run.yes));
            }
            for (v, share) in shares {
                match m.values.iter_mut().find(|(x, _)| *x == v) {
                    Some((_, sums)) => sums.add(w, share, visits),
                    None => {
                        let mut sums = Sums::default();
                        sums.add(w, share, visits);
                        m.values.push((v, sums));
                    }
                }
            }
        }
    }

    fn moments(&self) -> Moments {
        self.moments.clone().unwrap_or_default()
    }

    /// When sampling: how many equally weighted runs the estimates are worth.
    pub fn effective(&self) -> f64 {
        let m = self.moments();
        if m.squares.is_zero() {
            return 0.0;
        }
        (m.weight * m.weight).ratio(m.squares)
    }

    /// When sampling: the standard error of `chance`.
    pub fn chance_se(&self) -> f64 {
        let m = self.moments();
        m.yes.standard_error(&m.facts, self.chance())
    }

    /// When sampling: the mean of the reported numbers and its standard error.
    pub fn mean_se(&self) -> (f64, f64) {
        let m = self.moments();
        let mean = m.sum_positive.ratio(m.numbers.b) - m.sum_negative.ratio(m.numbers.b);
        (mean, m.sum.standard_error(&m.numbers, mean))
    }

    /// When sampling: the standard error of a value's probability `p`.
    pub fn value_se(&self, value: &Value, p: f64) -> f64 {
        let m = self.moments();
        let sums = m
            .values
            .iter()
            .find(|(x, _)| x == value)
            .map_or(Sums::default(), |(_, s)| *s);
        sums.standard_error(&m.visits, p)
    }
}

/// All values reported at one site, per `by` key (`()` without `by`).
#[derive(Clone, Debug, Default)]
pub struct Sink {
    pub groups: BTreeMap<Value, Acc>,
    /// When sampling: Σ w and Σ w² over the runs that reached the report,
    /// each counted once.
    pub reached: Weight,
    pub reached_squares: Weight,
}

impl Sink {
    /// Record a value reported in a world, and when sampling, by which run.
    pub fn add(&mut self, key: Value, value: &Value, weight: Weight, run: Option<u32>) {
        let acc = self.groups.entry(key).or_default();
        acc.total += weight;
        acc.add(value, weight);
        if let Some(run) = run {
            let stat = acc.runs.entry(run).or_insert_with(|| RunStat {
                weight,
                visits: 0,
                yes: 0.0,
                facts: 0.0,
                sum: 0.0,
                numbers: 0.0,
                others: Vec::new(),
            });
            stat.visits += 1;
            stat.record(value, 1.0);
        }
    }

    /// When sampling: fold the batch's runs into the sums. They're finished,
    /// so what's kept doesn't grow with the number of runs.
    pub fn end_batch(&mut self) {
        let mut runs: FxHashMap<u32, Weight> = FxHashMap::default();
        for acc in self.groups.values() {
            runs.extend(acc.runs.iter().map(|(id, r)| (*id, r.weight)));
        }
        for w in runs.into_values() {
            self.reached += w;
            self.reached_squares += w * w;
        }
        for acc in self.groups.values_mut() {
            acc.end_batch();
        }
    }

    /// Add what another batch of runs reported. Batches are combined in
    /// order, so the sums don't depend on which thread ran which batch.
    pub fn absorb(&mut self, other: Sink) {
        self.reached += other.reached;
        self.reached_squares += other.reached_squares;
        for (key, acc) in other.groups {
            match self.groups.entry(key) {
                std::collections::btree_map::Entry::Occupied(mut mine) => mine.get_mut().absorb(acc),
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(acc);
                }
            }
        }
    }

    /// For a report without `by` of facts: the probability they're true.
    pub fn chance(&self) -> Option<f64> {
        let acc = self.groups.get(&Value::Unit)?;
        acc.is_event().then(|| acc.chance())
    }

    /// For a report without `by`: the reported values and their probabilities.
    pub fn distribution(&self) -> Vec<(Value, f64)> {
        self.groups.get(&Value::Unit).map(Acc::distribution).unwrap_or_default()
    }

    /// Total weight that reached the report.
    pub fn reach(&self) -> Weight {
        Weight::sum(self.groups.values().map(|a| a.total))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Format {
    /// Also print the simplest nearby fraction of each probability.
    pub fractions: bool,
    /// Weight left unresolved by the whole run.
    pub unresolved: Weight,
    /// The total weight of the worlds that finished the program.
    pub program_total: Weight,
    /// When sampling: the sum of the runs' squared weights.
    pub run_squares: Option<Weight>,
}

/// Print every report in source order.
pub fn render(program: &Program, sinks: &[Sink], format: Format) -> String {
    let mut out = String::new();
    let mut simple: Vec<(String, String)> = Vec::new();
    let flush = |simple: &mut Vec<(String, String)>, out: &mut String| {
        let width = simple.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(0);
        for (label, text) in simple.drain(..) {
            let pad = width - label.chars().count();
            writeln!(out, "{label}{}    {text}", " ".repeat(pad)).unwrap();
        }
    };
    for (site, sink) in program.reports.iter().zip(sinks) {
        let mut label = site.label.clone();
        if site.kind == ReportKind::PerVisit {
            label.push_str(" (per visit)");
        }
        let reach = reach_note(site.kind, sink, format);
        if site.key_label.is_none() {
            let text = match sink.groups.values().next() {
                None => "(never reached)".to_string(),
                Some(acc) => format!("{}{reach}", value_text(acc, format)),
            };
            simple.push((label, text));
            continue;
        }
        if !simple.is_empty() {
            flush(&mut simple, &mut out);
        }
        if !out.is_empty() {
            out.push('\n');
        }
        writeln!(out, "{label}{reach}").unwrap();
        if sink.groups.is_empty() {
            writeln!(out, "  (never reached)").unwrap();
        } else {
            out.push_str(&table(site.key_label.as_deref().unwrap_or(""), sink, format));
        }
        out.push('\n');
    }
    flush(&mut simple, &mut out);
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// " (reached in 1.00% of worlds)" when a report sees part of the weight.
fn reach_note(kind: ReportKind, sink: &Sink, format: Format) -> String {
    if kind == ReportKind::PerVisit || sink.groups.is_empty() || format.program_total.is_zero() {
        return String::new();
    }
    if let Some(all_squares) = format.run_squares {
        // The share of the runs' weight that reached the report, counting
        // each run once, and its standard error (section 14).
        let total = format.program_total;
        let p = sink.reached.ratio(total);
        if p >= 0.99995 {
            return String::new();
        }
        let squares = sink.reached_squares;
        let spread = squares.scale((1.0 - p) * (1.0 - p)) + all_squares.saturating_sub(squares).scale(p * p);
        let se = spread.ratio(total * total).sqrt();
        return format!(" (reached in {} of runs)", estimate(p, se));
    }
    let share = sink.reach().ratio(format.program_total);
    if share >= 0.99995 {
        return String::new();
    }
    format!(
        " (reached in {} of worlds)",
        pct(
            share,
            Format {
                fractions: false,
                ..format
            }
        )
    )
}

fn value_text(acc: &Acc, format: Format) -> String {
    if acc.is_event() {
        return chance_text(acc, format);
    }
    let dist = acc.distribution();
    let mean_se = acc.sampled().then(|| acc.mean_se().1);
    let mut text = if dist.len() == 1 {
        display(&dist[0].0)
    } else if let Some(stats) = numeric_stats(&dist, mean_se) {
        stats
    } else if dist.iter().all(|(v, _)| matches!(v, Value::Date(_))) {
        let [a, b, c] = [0.05, 0.5, 0.95].map(|q| display(&quantile(&dist, q)));
        format!("5% {a} · median {b} · 95% {c}")
    } else if acc.sampled() {
        categorical_sampled(acc, &dist)
    } else {
        categorical(&dist, format)
    };
    let share = acc.unresolved_share(format.unresolved);
    if share >= 0.00005 {
        write!(
            text,
            " · {} unresolved",
            pct(
                share,
                Format {
                    fractions: false,
                    ..format
                }
            )
        )
        .unwrap();
    }
    text
}

/// A probability, or the range it lies in when unresolved weight is visible;
/// when sampling, an estimate and its standard error.
fn chance_text(acc: &Acc, format: Format) -> String {
    if acc.sampled() {
        return estimate(acc.chance(), acc.chance_se());
    }
    let (lo, hi) = acc.chance_bounds(format.unresolved);
    if hi - lo >= 0.00005 {
        let plain = Format {
            fractions: false,
            ..format
        };
        return format!("{}–{}", pct(lo, plain), pct(hi, plain));
    }
    pct(acc.chance(), format)
}

fn categorical(dist: &[(Value, f64)], format: Format) -> String {
    let mut by_chance: Vec<&(Value, f64)> = dist.iter().collect();
    by_chance.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let shown = by_chance.len().min(12);
    let mut parts: Vec<String> = by_chance[..shown]
        .iter()
        .map(|(v, p)| format!("{} {}", display(v), pct(*p, format)))
        .collect();
    if by_chance.len() > shown {
        parts.push(format!("… {} more", by_chance.len() - shown));
    }
    parts.join(" · ")
}

/// Sampled values with their probabilities and standard errors.
fn categorical_sampled(acc: &Acc, dist: &[(Value, f64)]) -> String {
    let mut by_chance: Vec<&(Value, f64)> = dist.iter().collect();
    by_chance.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let shown = by_chance.len().min(12);
    let mut parts: Vec<String> = by_chance[..shown]
        .iter()
        .map(|(v, p)| format!("{} {}", display(v), estimate(*p, acc.value_se(v, *p))))
        .collect();
    if by_chance.len() > shown {
        parts.push(format!("… {} more", by_chance.len() - shown));
    }
    parts.join(" · ")
}

/// A sampled probability and its standard error, rounded to the error's
/// precision: `46.1% ± 0.3%` (docs/semantics.md, section 14).
pub fn estimate(p: f64, se: f64) -> String {
    let (p, se) = (p * 100.0, se * 100.0);
    let decimals = if se < 0.005 {
        2
    } else {
        (-libm::log10(se).floor()).clamp(0.0, 2.0) as usize
    };
    format!("{p:.decimals$}% ± {se:.decimals$}%")
}

/// `mean · sd · 5% · median · 95%`, and a sparkline for small integer ranges.
/// Probabilities (as values, not facts) are shown as percentages. A sampled
/// mean shows its standard error when it's visible at the printed precision.
fn numeric_stats(dist: &[(Value, f64)], mean_se: Option<f64>) -> Option<String> {
    let nums: Vec<(f64, f64)> = dist
        .iter()
        .map(|(v, p)| match v {
            Value::Int(_) | Value::Float(_) | Value::Prob(_) => v.as_f64().map(|x| (x, *p)),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let percent = dist.iter().all(|(v, _)| matches!(v, Value::Prob(_)));
    let total: f64 = nums.iter().map(|(_, p)| p).sum();
    let mean = nums.iter().map(|(x, p)| x * p).sum::<f64>() / total;
    let sd = (nums.iter().map(|(x, p)| (x - mean).powi(2) * p).sum::<f64>() / total).sqrt();
    let show = |x: f64, decimals: usize| {
        if percent {
            format!("{:.2}%", x * 100.0)
        } else {
            fixed(x, decimals)
        }
    };
    let decimals = if mean.abs().max(sd) < 100.0 { 2 } else { 0 };
    let [a, b, c] = [0.05, 0.5, 0.95].map(|q| {
        let v = quantile(dist, q);
        if percent {
            show(v.as_f64().unwrap(), 2)
        } else {
            number(&v, decimals)
        }
    });
    let mean_text = match mean_se {
        Some(se) if se * if percent { 100.0 } else { 1.0 } >= 0.5 * libm::pow(10.0, -(decimals as f64)) => {
            format!("{} ± {}", show(mean, decimals), show(se, decimals))
        }
        _ => show(mean, decimals),
    };
    let mut text = format!(
        "mean {mean_text} · sd {} · 5% {a} · median {b} · 95% {c}",
        show(sd, decimals)
    );
    if let Some(spark) = sparkline(dist) {
        write!(text, " · {spark}").unwrap();
    }
    Some(text)
}

const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// `2 ▂▃▄▆▇█▇▆▄▃▂ 12` for integers spanning at most 25 values.
fn sparkline(dist: &[(Value, f64)]) -> Option<String> {
    let ints: Vec<(i64, f64)> = dist
        .iter()
        .map(|(v, p)| match v {
            Value::Int(i) => Some((*i, *p)),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let (lo, hi) = (ints.first()?.0, ints.last()?.0);
    if hi as i128 - lo as i128 + 1 > 25 || hi == lo {
        return None;
    }
    let max = ints.iter().map(|(_, p)| *p).fold(0.0, f64::max);
    let by_value: BTreeMap<i64, f64> = ints.into_iter().collect();
    let bars: String = (lo..=hi)
        .map(|i| match by_value.get(&i) {
            Some(&p) if p > 0.0 => BLOCKS[((p / max * 8.0).ceil() as usize).clamp(1, 8) - 1],
            _ => ' ',
        })
        .collect();
    Some(format!("{lo} {bars} {hi}"))
}

fn quantile(dist: &[(Value, f64)], q: f64) -> Value {
    let total: f64 = dist.iter().map(|(_, p)| p).sum();
    let mut acc = 0.0;
    for (v, p) in dist {
        acc += p / total;
        if acc >= q - 1e-12 {
            return v.clone();
        }
    }
    dist.last().unwrap().0.clone()
}

/// A report table: one row per `by` key.
fn table(key_label: &str, sink: &Sink, format: Format) -> String {
    let groups: Vec<(&Value, &Acc)> = sink.groups.iter().collect();
    let keys: Vec<String> = groups.iter().map(|(k, _)| display(k)).collect();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let header: Vec<String>;

    if groups.iter().all(|(_, a)| a.is_event()) {
        header = Vec::new();
        for (key, (_, acc)) in keys.iter().zip(&groups) {
            rows.push(vec![key.clone(), chance_text(acc, format)]);
        }
    } else {
        let dists: Vec<Vec<(Value, f64)>> = groups.iter().map(|(_, a)| a.distribution()).collect();
        let numeric = dists
            .iter()
            .all(|d| d.iter().all(|(v, _)| matches!(v, Value::Int(_) | Value::Float(_))));
        if numeric {
            header = ["5%", "25%", "median", "75%", "95%"].map(String::from).to_vec();
            for (key, d) in keys.iter().zip(&dists) {
                let scale = d
                    .iter()
                    .filter_map(|(v, _)| v.as_f64())
                    .fold(0.0f64, |m, x| m.max(x.abs()));
                let decimals = if scale < 100.0 { 2 } else { 0 };
                let mut row = vec![key.clone()];
                row.extend([0.05, 0.25, 0.5, 0.75, 0.95].map(|q| number(&quantile(d, q), decimals)));
                rows.push(row);
            }
        } else {
            let mut columns: Vec<Value> = dists.iter().flat_map(|d| d.iter().map(|(v, _)| v.clone())).collect();
            columns.sort();
            columns.dedup();
            header = columns.iter().map(display).collect();
            for ((key, d), (_, acc)) in keys.iter().zip(&dists).zip(&groups) {
                let mut row = vec![key.clone()];
                for c in &columns {
                    let p = d.iter().find(|(v, _)| v == c).map_or(0.0, |(_, p)| *p);
                    row.push(if acc.sampled() {
                        estimate(p, acc.value_se(c, p))
                    } else {
                        pct(p, format)
                    });
                }
                rows.push(row);
            }
        }
    }

    let mut all = Vec::new();
    if !header.is_empty() {
        let mut h = vec![key_label.to_string()];
        h.extend(header);
        all.push(h);
    }
    all.extend(rows);
    let ncols = all.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..ncols)
        .map(|c| {
            all.iter()
                .filter_map(|r| r.get(c))
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for row in all {
        out.push_str("  ");
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(c, cell)| format!("{}{cell}", " ".repeat(widths[c] - cell.chars().count())))
            .collect();
        out.push_str(&cells.join("   "));
        out.push('\n');
    }
    out
}

// ── Number formatting ────────────────────────────────────────────────────

/// A probability as a percentage with two decimals.
pub fn pct(p: f64, format: Format) -> String {
    let text = format!("{:.2}%", p * 100.0);
    let text = if text == "-0.00%" { "0.00%".to_string() } else { text };
    if format.fractions {
        if let Some((n, d)) = fraction(p) {
            if d > 1 {
                return format!("{text} (≈ {n}/{d})");
            }
        }
    }
    text
}

/// The simplest fraction within 1e-13 of `x` with a denominator of at most a
/// million. It helps recognize an answer; it doesn't prove the answer is
/// exactly that fraction (docs/semantics.md, section 10).
pub fn fraction(x: f64) -> Option<(u64, u64)> {
    if !(0.0..=1.0).contains(&x) {
        return None;
    }
    let (mut h0, mut h1, mut k0, mut k1) = (0f64, 1f64, 1f64, 0f64);
    let mut v = x;
    for _ in 0..64 {
        let a = v.floor();
        let (h2, k2) = (a * h1 + h0, a * k1 + k0);
        if k2 > 1e6 {
            break;
        }
        (h0, h1, k0, k1) = (h1, h2, k1, k2);
        if (h1 / k1 - x).abs() < 1e-13 {
            return Some((h1 as u64, k1 as u64));
        }
        let frac = v - a;
        if frac < 1e-15 {
            break;
        }
        v = 1.0 / frac;
    }
    None
}

fn display(v: &Value) -> String {
    match v {
        Value::Int(i) => thousands(*i),
        // Twelve significant digits hide rounding like 0.30000000000000004.
        Value::Float(f) if f.is_finite() => fmt_float(format!("{f:.11e}").parse().unwrap_or(*f)),
        Value::Float(f) => fmt_float(*f),
        other => other.to_string(),
    }
}

/// Integers with separators; floats with `decimals` decimals.
fn number(v: &Value, decimals: usize) -> String {
    match v {
        Value::Int(i) => thousands(*i),
        Value::Float(f) => fixed(*f, decimals),
        other => other.to_string(),
    }
}

fn fixed(x: f64, decimals: usize) -> String {
    let text = format!("{:.*}", decimals, x);
    let text = if text.starts_with('-') && text.trim_start_matches(['-', '0', '.']).is_empty() {
        text[1..].to_string()
    } else {
        text
    };
    let (int, frac) = match text.find('.') {
        Some(i) => (&text[..i], &text[i..]),
        None => (text.as_str(), ""),
    };
    let (sign, digits) = int.strip_prefix('-').map_or(("", int), |d| ("-", d));
    format!("{sign}{}{frac}", group(digits))
}

/// An integer with thousands separators: `50,000`.
pub fn thousands(i: i64) -> String {
    let digits = i.unsigned_abs().to_string();
    format!("{}{}", if i < 0 { "-" } else { "" }, group(&digits))
}

fn group(digits: &str) -> String {
    if digits.len() <= 3 {
        return digits.to_string();
    }
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> Format {
        Format {
            fractions: false,
            unresolved: Weight::ZERO,
            program_total: Weight::ONE,
            run_squares: None,
        }
    }

    #[test]
    fn fractions() {
        assert_eq!(fraction(244.0 / 495.0), Some((244, 495)));
        assert_eq!(fraction(1.0 / 6.0), Some((1, 6)));
        assert_eq!(fraction(0.5), Some((1, 2)));
        assert_eq!(fraction(std::f64::consts::FRAC_1_SQRT_2), None);
        let with = Format {
            fractions: true,
            ..plain()
        };
        assert_eq!(pct(244.0 / 495.0, with), "49.29% (≈ 244/495)");
    }

    #[test]
    fn numbers() {
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(-1000), "-1,000");
        assert_eq!(thousands(i64::MIN), "-9,223,372,036,854,775,808");
        assert_eq!(fixed(3.375, 2), "3.38");
        assert_eq!(fixed(-0.001, 2), "0.00");
        assert_eq!(fixed(92282.9, 0), "92,283");
        assert_eq!(pct(0.4929292929, plain()), "49.29%");
    }

    #[test]
    fn bounds_widen_with_unresolved_weight() {
        let mut sink = Sink::default();
        sink.add(Value::Unit, &Value::Bool(false), Weight::new(1e-5), None);
        let acc = &sink.groups[&Value::Unit];
        // Half a percent of the weight was cut before it could reach the report.
        let (lo, hi) = acc.chance_bounds(Weight::new(0.005));
        assert!(lo == 0.0 && (hi - 0.005 / (1e-5 + 0.005)).abs() < 1e-12);
        let format = Format {
            unresolved: Weight::new(0.005),
            ..plain()
        };
        assert_eq!(chance_text(acc, format), "0.00%–99.80%");
    }
}
