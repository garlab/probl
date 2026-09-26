//! Collecting `report` values across worlds, and printing them.

use crate::value::{Value, fmt_float};
use probl_sema::ir::Program;
use rustc_hash::FxHashMap;
use std::collections::BTreeMap;
use std::fmt::Write;

/// Everything one report site saw for one key.
#[derive(Clone, Debug, Default)]
pub struct Acc {
    /// Weight of the worlds that reached the report.
    pub total: f64,
    /// Σ weight × p over reported probabilities.
    pub chance: f64,
    /// Weight of the reported probabilities.
    pub chance_weight: f64,
    /// Weighted counts of every other value.
    pub values: FxHashMap<Value, f64>,
}

impl Acc {
    fn add(&mut self, value: &Value, weight: f64) {
        match value {
            Value::Prob(p) => {
                self.chance += weight * p;
                self.chance_weight += weight;
            }
            Value::Dist(d) => {
                for (x, q) in &d.outcomes {
                    self.add(x, weight * q);
                }
            }
            other => *self.values.entry(other.clone()).or_insert(0.0) += weight,
        }
    }

    pub fn is_chance(&self) -> bool {
        self.values.is_empty() && self.chance_weight > 0.0
    }

    pub fn chance(&self) -> f64 {
        self.chance / self.total
    }

    /// The reported values as a sorted distribution (probabilities count as
    /// `true`/`false` when they're mixed with other values).
    pub fn distribution(&self) -> Vec<(Value, f64)> {
        let mut pairs: Vec<(Value, f64)> = self.values.iter().map(|(v, w)| (v.clone(), *w / self.total)).collect();
        if self.chance_weight > 0.0 {
            pairs.push((Value::Prob(1.0), self.chance / self.total));
            pairs.push((Value::Prob(0.0), (self.chance_weight - self.chance) / self.total));
        }
        pairs.retain(|(_, p)| *p > 0.0);
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs
    }
}

/// All values reported at one site, per `by` key (`()` without `by`).
#[derive(Clone, Debug, Default)]
pub struct Sink {
    pub groups: BTreeMap<Value, Acc>,
}

impl Sink {
    pub fn add(&mut self, key: Value, value: &Value, weight: f64) {
        let acc = self.groups.entry(key).or_default();
        acc.total += weight;
        acc.add(value, weight);
    }

    /// For a report without `by` of a probability: the overall chance.
    pub fn chance(&self) -> Option<f64> {
        let acc = self.groups.get(&Value::Unit)?;
        acc.is_chance().then(|| acc.chance())
    }

    /// For a report without `by`: the reported values and their probabilities.
    pub fn distribution(&self) -> Vec<(Value, f64)> {
        self.groups.get(&Value::Unit).map(Acc::distribution).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Format {
    /// Also print probabilities as fractions, like `244/495`.
    pub fractions: bool,
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
        let label = &site.label;
        if site.key_label.is_none() {
            let text = match sink.groups.values().next() {
                None => "(never reached)".to_string(),
                Some(acc) => value_text(acc, format),
            };
            simple.push((label.clone(), text));
            continue;
        }
        if !simple.is_empty() {
            flush(&mut simple, &mut out);
        }
        if !out.is_empty() {
            out.push('\n');
        }
        writeln!(out, "{label}").unwrap();
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

fn value_text(acc: &Acc, format: Format) -> String {
    if acc.is_chance() {
        return pct(acc.chance(), format);
    }
    let dist = acc.distribution();
    if dist.len() == 1 {
        return display(&dist[0].0);
    }
    if let Some(stats) = numeric_stats(&dist) {
        return stats;
    }
    if dist.iter().all(|(v, _)| matches!(v, Value::Date(_))) {
        let [a, b, c] = [0.05, 0.5, 0.95].map(|q| display(&quantile(&dist, q)));
        return format!("5% {a} · median {b} · 95% {c}");
    }
    categorical(&dist, format)
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

/// `mean · sd · 5% · median · 95%`, and a sparkline for small integer ranges.
fn numeric_stats(dist: &[(Value, f64)]) -> Option<String> {
    let nums: Vec<(f64, f64)> = dist
        .iter()
        .map(|(v, p)| match v {
            Value::Int(_) | Value::Float(_) => v.as_f64().map(|x| (x, *p)),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let total: f64 = nums.iter().map(|(_, p)| p).sum();
    let mean = nums.iter().map(|(x, p)| x * p).sum::<f64>() / total;
    let sd = (nums.iter().map(|(x, p)| (x - mean).powi(2) * p).sum::<f64>() / total).sqrt();
    let decimals = if mean.abs().max(sd) < 100.0 { 2 } else { 0 };
    let [a, b, c] = [0.05, 0.5, 0.95].map(|q| number(&quantile(dist, q), decimals));
    let mut text = format!(
        "mean {} · sd {} · 5% {a} · median {b} · 95% {c}",
        fixed(mean, decimals),
        fixed(sd, decimals)
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
    if hi - lo + 1 > 25 || hi == lo {
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

    if groups.iter().all(|(_, a)| a.is_chance()) {
        header = Vec::new();
        for (key, (_, acc)) in keys.iter().zip(&groups) {
            rows.push(vec![key.clone(), pct(acc.chance(), format)]);
        }
    } else {
        let dists: Vec<Vec<(Value, f64)>> = groups.iter().map(|(_, a)| a.distribution()).collect();
        let numeric = dists
            .iter()
            .all(|d| d.iter().all(|(v, _)| matches!(v, Value::Int(_) | Value::Float(_))));
        if numeric {
            header = ["5%", "25%", "median", "75%", "95%"].map(String::from).to_vec();
            for (key, d) in keys.iter().zip(&dists) {
                let nums: Vec<f64> = d.iter().filter_map(|(v, _)| v.as_f64()).collect();
                let scale = nums.iter().fold(0.0f64, |m, x| m.max(x.abs()));
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
            for (key, d) in keys.iter().zip(&dists) {
                let mut row = vec![key.clone()];
                for c in &columns {
                    let p = d.iter().find(|(v, _)| v == c).map_or(0.0, |(_, p)| *p);
                    row.push(pct(p, format));
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
                return format!("{text} ({n}/{d})");
            }
        }
    }
    text
}

/// The simplest fraction within 1e-13 of `x` with a denominator of at most a
/// million. Exact-mode probabilities are rationals computed in floating point,
/// so this recovers answers like 244/495; the tight tolerance and the bound
/// keep irrational numbers from matching a close-but-wrong fraction.
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
    // Group the integer part in thousands.
    let (int, frac) = match text.find('.') {
        Some(i) => (&text[..i], &text[i..]),
        None => (text.as_str(), ""),
    };
    let (sign, digits) = int.strip_prefix('-').map_or(("", int), |d| ("-", d));
    format!("{sign}{}{frac}", group(digits))
}

fn thousands(i: i64) -> String {
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

    #[test]
    fn fractions() {
        assert_eq!(fraction(244.0 / 495.0), Some((244, 495)));
        assert_eq!(fraction(1.0 / 6.0), Some((1, 6)));
        assert_eq!(fraction(0.5), Some((1, 2)));
        assert_eq!(fraction(std::f64::consts::FRAC_1_SQRT_2), None);
    }

    #[test]
    fn numbers() {
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(-1000), "-1,000");
        assert_eq!(thousands(999), "999");
        assert_eq!(fixed(3.375, 2), "3.38");
        assert_eq!(fixed(-0.001, 2), "0.00");
        assert_eq!(fixed(92282.9, 0), "92,283");
        assert_eq!(pct(0.4929292929, Format::default()), "49.29%");
    }
}
