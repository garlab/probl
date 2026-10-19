//! Versioned report summaries, streamed to a bounded JSON buffer.
use super::Outcome;
use crate::{Date, EvidenceKind, ReportStatistic, ReportSummary, SamplingStatus};
use probl_engine::report::{self, GroupResult, Quantity, Status};
use probl_engine::value::Value;
use probl_sema::ir::ReportKind;
use serde_json::{Value as Json, json};
use std::fmt;
use std::io::{self, Write};

const COLUMNS: [ReportStatistic; 5] = [
    ReportStatistic::Quantile(0.05),
    ReportStatistic::Quantile(0.25),
    ReportStatistic::Median,
    ReportStatistic::Quantile(0.75),
    ReportStatistic::Quantile(0.95),
];

/// Budgets for [`Outcome::reports_json`], separate from model execution.
#[derive(Clone, Copy, Debug)]
pub struct ReportJsonLimits {
    /// Maximum encoded UTF-8 bytes (4 MiB by default).
    pub max_bytes: usize,
    /// Maximum report, group, field and failure nodes (10,000 by default).
    pub max_nodes: usize,
    /// Population visits reserved for summary scans, and label/key bytes
    /// inspected (20 million by default). This is not a time limit.
    pub max_work: usize,
}

impl Default for ReportJsonLimits {
    fn default() -> Self {
        Self {
            max_bytes: 4 * 1024 * 1024,
            max_nodes: 10_000,
            max_work: 20_000_000,
        }
    }
}

/// A report document exceeded one of its serialization budgets. The model's
/// outcome and existing text are still available; no partial JSON is returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReportJsonError {
    Bytes,
    Nodes,
    Work,
}

impl fmt::Display for ReportJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bytes => "structured reports exceeded the JSON byte limit",
            Self::Nodes => "structured reports exceeded the report node limit",
            Self::Work => "structured reports exceeded the summary work limit",
        })
    }
}
impl std::error::Error for ReportJsonError {}

type Result<T> = std::result::Result<T, ReportJsonError>;

struct Buffer {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for Buffer {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("report JSON byte limit"));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Buffer {
    fn raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.write_all(bytes).map_err(|_| ReportJsonError::Bytes)
    }
    fn json(&mut self, value: &Json) -> Result<()> {
        serde_json::to_writer(self, value).map_err(|_| ReportJsonError::Bytes)
    }
}
struct Budget {
    nodes: usize,
    work: usize,
    max_bytes: usize,
}
impl Budget {
    fn work(&mut self, n: usize) -> Result<()> {
        self.work = self.work.checked_sub(n).ok_or(ReportJsonError::Work)?;
        Ok(())
    }
    fn node(&mut self) -> Result<()> {
        self.nodes = self.nodes.checked_sub(1).ok_or(ReportJsonError::Nodes)?;
        self.work(1)
    }
    fn text(&mut self, text: &str) -> Result<()> {
        if text.len() > self.max_bytes {
            return Err(ReportJsonError::Bytes);
        }
        self.work(text.len())
    }
}

impl Outcome {
    /// A version-1 JSON document of report summaries, with typed group keys,
    /// reach, uncertainty and run provenance. Requires the `json` crate feature.
    ///
    /// Report IDs are their zero-based source order, stable within this compiled
    /// program; labels need not be unique. Quantiles use the report convention
    /// (5/25/50/75/95%, including the midpoint median). Categorical populations
    /// are identified but not exported; use the existing text/distribution API.
    /// No sampled population is copied into the JSON document.
    ///
    /// Limits bound encoded bytes, document nodes and summary work. On failure,
    /// this returns an error instead of a truncated document. It does not change
    /// the outcome or consume randomness. See `docs/report-json.md` for the schema.
    pub fn reports_json(&self, limits: &ReportJsonLimits) -> Result<String> {
        let mut out = Buffer {
            bytes: Vec::new(),
            limit: limits.max_bytes,
        };
        let mut budget = Budget {
            nodes: limits.max_nodes,
            work: limits.max_work,
            max_bytes: limits.max_bytes,
        };
        let sampled = self.sampling();
        let evidence = self.evidence().map(|e| json!({
            "kind": match e.kind() { EvidenceKind::Probability => "probability", EvidenceKind::Density => "density" },
            "log_value": e.log_value().map(extended), "complete": e.is_complete(),
            "log_bounds": e.log_bounds().map(|b| [extended(b.lower()), extended(b.upper())]),
            "relative_standard_error": e.relative_standard_error().filter(|x| x.is_finite()),
            "sampling_status": e.sampling_status().map(sampling_status),
        }));
        out.raw(b"{\"schema_version\":1,\"run\":")?;
        out.json(&json!({
            "mode": if sampled.is_some() { "sample" } else { "enumerate" },
            "sampling": sampled.map(|s| json!({"runs": s.runs().to_string(), "seed": s.seed().to_string(), "effective_runs": s.effective_runs()})),
            "today": self.today().map(|d| d.to_string()),
            "finished": self.finished(), "partial": !self.failures().is_empty(), "failed_share": self.failed_share(),
            "evidence": evidence,
            "unresolved": {"zero": self.unresolved().is_zero(), "log_weight_upper_bound": extended(self.unresolved().log_weight_upper_bound())},
            "integration_tolerance": self.integration(),
        }))?;
        out.raw(b",\"failures\":[")?;
        for (i, failure) in self.failures().iter().enumerate() {
            budget.node()?;
            budget.text(failure.diagnostic().message())?;
            if i > 0 {
                out.raw(b",")?;
            }
            let span = failure.diagnostic().span();
            out.json(&json!({
                "message": failure.diagnostic().message(), "span": {"start": span.start, "end": span.end},
                "share": failure.share(), "standard_error": failure.standard_error(),
                "runs": failure.runs().map(|n| n.to_string()), "first_run": failure.first_run().map(|n| n.to_string()),
            }))?;
        }
        out.raw(b"],\"reports\":[")?;
        for (id, ((site, result), public)) in self
            .program
            .reports
            .iter()
            .zip(&self.results)
            .zip(&self.reports)
            .enumerate()
        {
            budget.node()?;
            budget.text(&site.label)?;
            if let Some(label) = &site.key_label {
                budget.text(label)?;
            }
            if id > 0 {
                out.raw(b",")?;
            }
            out.raw(b"{\"id\":")?;
            out.json(&json!(id))?;
            out.raw(b",\"label\":")?;
            out.json(&json!(site.label))?;
            out.raw(b",\"key_label\":")?;
            out.json(&json!(site.key_label))?;
            let selection = self.report_summaries.get(&id);
            out.raw(b",\"summary_selection\":")?;
            out.json(&json!(
                selection.map(|s| s.statistics().iter().map(|s| s.token()).collect::<Vec<_>>())
            ))?;
            out.raw(b",\"visits\":")?;
            out.json(&json!(match site.kind {
                ReportKind::Once => "once",
                ReportKind::PerKey => "per_key",
                ReportKind::PerVisit => "per_visit",
            }))?;
            out.raw(b",\"reach\":")?;
            out.json(&json!(
                result.reach.map(|r| json!({"share":r.share, "standard_error": r.se}))
            ))?;
            out.raw(b",\"groups\":[")?;
            for (index, (g, p)) in result.groups.iter().zip(public.groups()).enumerate() {
                if index > 0 {
                    out.raw(b",")?;
                }
                let group = group(g, p.key(), index, &mut budget, 0, selection)?;
                out.json(&group)?;
            }
            out.raw(b"]}")?;
        }
        out.raw(b"]}")?;
        Ok(String::from_utf8(out.bytes).expect("JSON is UTF-8"))
    }
}

fn group(
    g: &GroupResult,
    key_label: Option<&str>,
    id: usize,
    budget: &mut Budget,
    depth: usize,
    selection: Option<&ReportSummary>,
) -> Result<Json> {
    budget.node()?;
    if depth > 64 {
        return Err(ReportJsonError::Nodes);
    }
    if let Some(label) = key_label {
        budget.text(label)?;
    }
    let columns = selection.map_or(COLUMNS.as_slice(), ReportSummary::statistics);
    let count = columns
        .iter()
        .filter(|s| matches!(s, ReportStatistic::Median | ReportStatistic::Quantile(_)))
        .count();
    // Reserve repeated scans for classification, the requested quantiles and their
    // exact typed values. Continuous queries have bounded internal solvers.
    let population = g.distribution.len().saturating_add(
        g.numeric
            .as_ref()
            .and_then(|n| n.mixture())
            .map_or(0, |m| m.parts.len()),
    );
    budget.work(population.saturating_add(1).saturating_mul(6 + 2 * count))?;
    let dates = !g.distribution.is_empty() && g.distribution.iter().all(|(v, _)| matches!(v, Value::Date(_)));
    let numbers = !g.distribution.is_empty()
        && g.distribution
            .iter()
            .all(|(v, _)| matches!(v, Value::Int(_) | Value::Float(_) | Value::Prob(_)));
    let kind = if g.fields.is_some() {
        "structured"
    } else if g.fact.is_some() {
        "probability"
    } else if dates {
        "date"
    } else if g.numeric.is_some() || numbers {
        "number"
    } else {
        "categorical"
    };
    let mut summary = Json::Null;
    if dates || numbers || g.numeric.is_some() {
        let mut quantiles = Vec::new();
        for statistic in columns {
            let (level, convention) = match statistic {
                ReportStatistic::Median => (0.5, "midpoint"),
                ReportStatistic::Quantile(q) => (*q, "inverse_cdf"),
                _ => continue,
            };
            let exact = report::statistic_value(g, *statistic);
            let estimate = g.numeric.as_ref().and_then(|n| n.statistic(*statistic));
            let value = exact.as_ref().map(|v| typed(v, None, budget)).transpose()?;
            quantiles.push(
                json!({"level":level, "convention":convention, "estimate": estimate.map(quantity), "value": value}),
            );
        }
        summary = json!({
            "mean": g.numeric.as_ref().filter(|_| selection.is_none() || columns.contains(&ReportStatistic::Mean)).map(|n| quantity(n.mean)),
            "sd": g.numeric.as_ref().filter(|_| selection.is_none() || columns.contains(&ReportStatistic::StandardDeviation)).map(|n| quantity(n.sd)),
            "quantiles": quantiles,
            "percent": g.numeric.as_ref().is_some_and(|n| n.percent),
            "integrated_moments": g.numeric.as_ref().is_some_and(|n| n.integrated),
            "quantile_method": "exact",
            "deterministic": g.distribution.len() == 1 && (dates || numbers),
        });
    }
    let mut fields = Vec::new();
    if let Some(fs) = &g.fields {
        for (index, (path, child)) in fs.0.iter().enumerate() {
            budget.text(path)?;
            fields.push(json!({"path":path, "group":group(child, None, index, budget, depth+1, None)?}));
        }
    }
    Ok(json!({
        "id":id, "key": key_label.map(|label| typed(&g.key, Some(label), budget)).transpose()?,
        "kind":kind, "probability":g.fact.map(quantity), "summary":summary,
        "unresolved_share": g.unresolved_share,
        "complete": g.fact.or_else(|| g.numeric.as_ref().map(|n| n.mean))
            .or_else(|| g.values.as_ref().and_then(|values| values.first().map(|(_, q)| *q)))
            .map(|q| q.complete),
        "support": g.support.map(|s| json!({"contributing_runs":s.contributing_runs.to_string(),"effective_runs":s.effective})),
        "fields":fields,
    }))
}

fn typed(value: &Value, label: Option<&str>, budget: &mut Budget) -> Result<Json> {
    let (kind, value) = match value {
        Value::Int(n) => ("int", json!(n.to_string())),
        Value::Float(x) => ("float", json!(x)),
        Value::Prob(x) => ("prob", json!(x)),
        Value::Date(d) => ("date", json!(Date::from_days(*d).to_string())),
        Value::Bool(b) => ("bool", json!(b)),
        Value::Str(s) => {
            budget.text(s)?;
            ("str", json!(s.as_ref()))
        }
        Value::Unit => ("unit", Json::Null),
        _ => ("opaque", Json::Null),
    };
    if let Some(text) = value.as_str() {
        budget.text(text)?;
    }
    Ok(json!({"kind":kind, "value":value, "label":label}))
}

fn quantity(q: Quantity) -> Json {
    json!({
        "point":q.point.filter(|x|x.is_finite()),
        "nonfinite": q.point.filter(|x|!x.is_finite()).map(extended),
        "complete":q.complete, "bounds":q.bounds.map(|(a,b)|[extended(a),extended(b)]),
        "sampling":q.sampling.map(|s|json!({
            "status":match s.status { Status::Estimated => "estimated", Status::NotEstimable => "not_estimable", Status::NotComputed => "not_computed", Status::IntegratedZero => "integrated_zero" },
            "standard_error":s.se.filter(|x|x.is_finite()),
            "confidence_interval":s.wilson.map(|(lower,upper)|json!({"level":0.95,"method":"wilson","lower":lower,"upper":upper})),
            "contributing_runs":s.support.contributing_runs.to_string(),"effective_runs":s.support.effective,
        }))
    })
}
fn extended(x: f64) -> Json {
    if x.is_finite() {
        json!(x)
    } else if x.is_nan() {
        json!("unavailable")
    } else if x.is_sign_positive() {
        json!("+infinity")
    } else {
        json!("-infinity")
    }
}
fn sampling_status(s: SamplingStatus) -> &'static str {
    match s {
        SamplingStatus::Estimated => "estimated",
        SamplingStatus::NotEstimable => "not_estimable",
        SamplingStatus::NotComputed => "not_computed",
        SamplingStatus::IntegratedZero => "integrated_zero",
    }
}
