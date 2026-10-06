//! The library as an embedder uses it: only `probl`'s supported API
//! (docs/library.md).

use probl::{
    Data, Date, ErrorKind, Estimate, EvidenceKind, Files, MemoryFiles, Mode, Options, Outcome, SamplingStatus,
    Snapshots, SummaryError,
};
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn run(source: &str, options: &Options) -> Outcome {
    let program = probl::compile("test.probl", source).unwrap_or_else(|e| panic!("{}", e.render(false)));
    program.run(options).unwrap_or_else(|e| panic!("{}", e.render(false)))
}

/// The probability of a report without `by`.
fn probability<'a>(outcome: &'a Outcome, label: &str) -> &'a Estimate {
    let report = outcome.report(label).unwrap_or_else(|| panic!("no report {label:?}"));
    report.groups()[0].probability().unwrap()
}

fn close(a: f64, b: f64, tolerance: f64) {
    assert!(
        (a - b).abs() <= tolerance,
        "{a} and {b} differ by more than {tolerance}"
    );
}

const CRAPS: &str = include_str!("../../../examples/02_craps.probl");

// ── Known answers ────────────────────────────────────────────────────────

#[test]
fn craps_has_a_point_despite_its_tiny_tail() {
    let outcome = run(CRAPS, &Options::new());
    let win = probability(&outcome, "win");
    // The loop is cut where the worlds left weigh less than 1e-12: the point
    // is over the resolved worlds, and bounds hold the exact answer.
    close(win.point().unwrap(), 244.0 / 495.0, 1e-9);
    assert!(!win.is_complete());
    let bounds = win.bounds().unwrap();
    assert!(
        bounds.lower() <= 244.0 / 495.0 && 244.0 / 495.0 <= bounds.upper(),
        "{bounds:?}"
    );
    assert!(bounds.upper() - bounds.lower() < 1e-10, "{bounds:?}");
    assert!(win.sampling().is_none());
    assert!(!outcome.unresolved().is_zero());
    assert!(outcome.unresolved().log_weight_upper_bound() < (1e-12f64).ln());
}

#[test]
fn a_complete_enumeration_is_exact() {
    let outcome = run("let d ~ d6\nreport d > 3", &Options::new());
    let p = probability(&outcome, "d > 3");
    close(p.point().unwrap(), 0.5, 1e-15);
    assert!(p.is_complete() && p.bounds().is_none() && p.sampling().is_none());
    assert!(outcome.unresolved().is_zero());
    assert_eq!(outcome.unresolved().log_weight_upper_bound(), f64::NEG_INFINITY);
    assert!(outcome.evidence().is_none() && outcome.sampling().is_none());
}

#[test]
fn numeric_summaries_and_quantiles() {
    let outcome = run("let a ~ d6\nlet b ~ d6\nreport a + b as \"sum\"", &Options::new());
    let group = &outcome.report("sum").unwrap().groups()[0];
    assert!(group.probability().is_none() && group.key().is_none());
    let numeric = group.numeric().unwrap();
    close(numeric.mean().unwrap().point().unwrap(), 7.0, 1e-12);
    close(numeric.sd().unwrap().point().unwrap(), (35.0f64 / 6.0).sqrt(), 1e-12);
    let quantile = |q| numeric.quantile(q).unwrap().point().unwrap();
    assert_eq!(quantile(0.5), 7.0);
    // The smallest value whose cumulative probability reaches q.
    assert_eq!(quantile(0.05), 3.0);
    assert_eq!(quantile(0.0), 2.0);
    assert_eq!(quantile(1.0), 12.0);
    for bad in [-0.1, 1.5, f64::NAN, f64::INFINITY] {
        assert_eq!(numeric.quantile(bad), Err(SummaryError::InvalidQuantile));
    }
    // The values in numeric order, as Probl prints them.
    let distribution = group.distribution().unwrap();
    let labels: Vec<&str> = distribution.iter().map(|(v, _)| v.as_str()).collect();
    assert_eq!(labels, ["2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12"]);
    close(distribution[5].1.point().unwrap(), 6.0 / 36.0, 1e-12);
    let total: f64 = distribution.iter().map(|(_, e)| e.point().unwrap()).sum();
    close(total, 1.0, 1e-12);
}

#[test]
fn continuous_reports_have_summaries_but_no_table() {
    let source = include_str!("../../../examples/19_analytic_continuous.probl");
    let outcome = run(source, &Options::new());
    let mut continuous = 0;
    for report in outcome.reports() {
        for group in report.groups() {
            let Some(numeric) = group.numeric() else { continue };
            if group.distribution().is_some() {
                continue;
            }
            // A continuous marginal: no finite table, never an empty one.
            continuous += 1;
            let mean = numeric.mean().unwrap().point().unwrap();
            let median = numeric.quantile(0.5).unwrap().point().unwrap();
            assert!(mean.is_finite() && median.is_finite());
            let low = numeric.quantile(0.05).unwrap().point().unwrap();
            let high = numeric.quantile(0.95).unwrap().point().unwrap();
            assert!(low <= median && median <= high, "{low} {median} {high}");
        }
    }
    assert!(continuous > 0, "{}", outcome.text());
}

// ── Uncertainty stays explicit ───────────────────────────────────────────

const GEOMETRIC: &str = "var n = 0\nwhile d6 != 6 {\n  n += 1\n}\n";

#[test]
fn large_cutoffs_give_wide_bounds_around_the_point() {
    let source = format!("{GEOMETRIC}report n < 2");
    let outcome = run(&source, &Options::new().epsilon(0.01));
    let p = probability(&outcome, "n < 2");
    let (point, bounds) = (p.point().unwrap(), p.bounds().unwrap());
    assert!(!p.is_complete());
    assert!(bounds.lower() < point && point < bounds.upper(), "{point} {bounds:?}");
    // The exact answer, 1/6 + 5/36, is inside.
    let exact = 1.0 / 6.0 + 5.0 / 36.0;
    assert!(bounds.lower() <= exact && exact <= bounds.upper(), "{bounds:?}");
    assert!(outcome.text().contains("30.56%–31.43%"), "{}", outcome.text());
}

#[test]
fn every_group_has_its_own_bounds() {
    let source = format!("let d ~ d6\n{GEOMETRIC}report n < 2 by d mod 2");
    let outcome = run(&source, &Options::new().epsilon(0.01));
    let groups = outcome.reports()[0].groups();
    assert_eq!(groups.iter().map(|g| g.key().unwrap()).collect::<Vec<_>>(), ["0", "1"]);
    for group in groups {
        let p = group.probability().unwrap();
        let bounds = p.bounds().unwrap();
        assert!(!p.is_complete());
        assert!(bounds.lower() <= p.point().unwrap() && p.point().unwrap() <= bounds.upper());
    }
}

#[test]
fn values_and_means_of_an_incomplete_population() {
    let source = format!("{GEOMETRIC}report n");
    let outcome = run(&source, &Options::new().epsilon(0.01));
    let group = &outcome.reports()[0].groups()[0];
    // Each value's probability is bounded; the mean is not, since the
    // unresolved worlds could take n anywhere.
    for (_, e) in group.distribution().unwrap() {
        let b = e.bounds().unwrap();
        assert!(!e.is_complete() && b.lower() <= e.point().unwrap() && e.point().unwrap() <= b.upper());
    }
    let mean = group.numeric().unwrap().mean().unwrap();
    assert!(!mean.is_complete() && mean.point().is_some() && mean.bounds().is_none());
}

#[test]
fn sampled_estimates_have_their_errors() {
    let source = "@mode sample(runs: 2000, seed: 1)\nlet a ~ d6\nlet b ~ d6\nreport a > 3\nreport a + b";
    let outcome = run(source, &Options::new());
    let sampling = outcome.sampling().unwrap();
    assert_eq!((sampling.runs(), sampling.seed()), (2000, 1));
    let fact = probability(&outcome, "a > 3");
    let s = fact.sampling().unwrap();
    assert_eq!(s.status(), SamplingStatus::Estimated);
    assert!(s.standard_error().unwrap() > 0.0 && s.confidence_interval().is_none());
    assert_eq!(s.contributing_runs(), 2000);
    assert!(fact.is_complete() && fact.bounds().is_none());
    close(fact.point().unwrap(), 0.5, 5.0 * s.standard_error().unwrap());

    let numeric = outcome.report("a + b").unwrap().groups()[0].numeric().unwrap();
    let mean = numeric.mean().unwrap();
    assert_eq!(mean.sampling().unwrap().status(), SamplingStatus::Estimated);
    close(
        mean.point().unwrap(),
        7.0,
        5.0 * mean.sampling().unwrap().standard_error().unwrap(),
    );
    // Errors the engine doesn't compute say so, rather than claiming none.
    let sd = numeric.sd().unwrap().sampling().unwrap();
    assert_eq!(sd.status(), SamplingStatus::NotComputed);
    assert_eq!(sd.standard_error(), None);
    let median = numeric.quantile(0.5).unwrap();
    assert_eq!(median.sampling().unwrap().status(), SamplingStatus::NotComputed);
}

#[test]
fn unanimous_runs_have_a_wilson_interval_and_no_error_estimate() {
    let outcome = run(
        "@mode sample(runs: 20, seed: 1)\nlet d ~ d6\nreport d > 6",
        &Options::new(),
    );
    let s = *probability(&outcome, "d > 6").sampling().unwrap();
    assert_eq!(s.status(), SamplingStatus::NotEstimable);
    assert_eq!(s.standard_error(), None);
    let ci = s.confidence_interval().unwrap();
    assert_eq!((ci.level(), ci.method()), (0.95, probl::ConfidenceMethod::Wilson));
    assert_eq!(ci.interval().lower(), 0.0);
    close(ci.interval().upper(), 0.1611, 1e-4);
    assert!(s.effective_runs() < 30.0);
}

#[test]
fn integrated_outcomes_are_told_apart_from_unestimable_errors() {
    let integrated = run("@mode sample(runs: 1000, seed: 7)\nreport d6 > 3", &Options::new());
    let s = *probability(&integrated, "d6 > 3").sampling().unwrap();
    assert_eq!(s.status(), SamplingStatus::IntegratedZero);
    assert_eq!(s.standard_error(), Some(0.0));
    assert!(s.confidence_interval().is_none());

    let weighted = run(
        "@mode sample(runs: 1000, seed: 7)\nlet x ~ d6\nobserve true from bernoulli(prob(if x == 6 { 90% } else { 10% }))\nreport true",
        &Options::new(),
    );
    let s = *probability(&weighted, "true").sampling().unwrap();
    assert_eq!(s.status(), SamplingStatus::NotEstimable);
    assert!(s.confidence_interval().is_none());
}

// ── Evidence ─────────────────────────────────────────────────────────────

#[test]
fn tiny_evidence_keeps_its_scale() {
    let source = "for i in 1..2000 {\n  observe true from bernoulli(50%)\n}\nreport true";
    let outcome = run(source, &Options::new());
    let evidence = outcome.evidence().unwrap();
    // 2⁻²⁰⁰⁰ is far below the smallest f64.
    close(evidence.log_value().unwrap(), -2000.0 * std::f64::consts::LN_2, 1e-9);
    assert_eq!(evidence.kind(), EvidenceKind::Probability);
    assert!(evidence.is_complete() && evidence.log_bounds().is_none());
    assert_eq!(evidence.sampling_status(), None);
}

#[test]
fn incomplete_evidence_has_log_bounds() {
    let source = format!("{GEOMETRIC}observe n < 5\nreport n");
    let outcome = run(&source, &Options::new().epsilon(0.01));
    let evidence = outcome.evidence().unwrap();
    assert!(!evidence.is_complete());
    let bounds = evidence.log_bounds().unwrap();
    assert_eq!(bounds.lower(), evidence.log_value().unwrap());
    assert!(bounds.upper() > bounds.lower());
    // P(n < 5) = 1 − (5/6)⁵, up to rounding.
    let exact = (1.0 - (5.0f64 / 6.0).powi(5)).ln();
    assert!(
        bounds.lower() <= exact + 1e-12 && exact <= bounds.upper(),
        "{bounds:?} {exact}"
    );
}

#[test]
fn densities_are_not_probabilities() {
    let source = "@mode sample(runs: 100, seed: 1)\nobserve 0.1 from normal(0.1, 0.01)\nreport true";
    let outcome = run(source, &Options::new());
    let evidence = outcome.evidence().unwrap();
    assert_eq!(evidence.kind(), EvidenceKind::Density);
    // The normal density at its mean: 1 / (0.01 √(2π)), about 39.9.
    let density = 1.0 / (0.01 * (2.0 * std::f64::consts::PI).sqrt());
    close(evidence.log_value().unwrap(), density.ln(), 1e-9);
    assert_eq!(evidence.sampling_status(), Some(SamplingStatus::Estimated));
    assert_eq!(evidence.relative_standard_error(), Some(0.0));

    let once = run(
        "@mode sample(runs: 1, seed: 1)\nobserve true from bernoulli(50%)\nreport true",
        &Options::new(),
    );
    let evidence = once.evidence().unwrap();
    assert_eq!(evidence.sampling_status(), Some(SamplingStatus::NotEstimable));
    assert_eq!(evidence.relative_standard_error(), None);
}

// ── Data ─────────────────────────────────────────────────────────────────

const PILOT: &str = "type Day = { day: date, signups: int }\nlet pilot: list[Day] = read(\"pilot.csv\")\nreport sum(map(pilot, d -> d.signups)) as \"total\"";

/// Files that count how often they're opened, and can change.
#[derive(Clone, Default)]
struct Counted {
    opened: Arc<AtomicUsize>,
    text: Arc<Mutex<String>>,
}

impl Files for Counted {
    fn resolve(&mut self, path: &str) -> Result<String, String> {
        Ok(path.to_string())
    }

    fn open(&mut self, _: &str) -> Result<Box<dyn Read + Send>, String> {
        self.opened.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(std::io::Cursor::new(
            self.text.lock().unwrap().clone().into_bytes(),
        )))
    }
}

fn pilot(signups: &[u32]) -> String {
    let mut csv = "day,signups\n".to_string();
    for (i, n) in signups.iter().enumerate() {
        csv.push_str(&format!("2026-09-{:02},{n}\n", i + 1));
    }
    csv
}

fn total(outcome: &Outcome) -> String {
    outcome.report("total").unwrap().groups()[0].distribution().unwrap()[0]
        .0
        .clone()
}

#[test]
fn data_is_loaded_once_and_reused() {
    let program = probl::compile("pilot.probl", PILOT).unwrap();
    assert!(program.reads_data());
    let files = Counted::default();
    *files.text.lock().unwrap() = pilot(&[9, 6, 11]);
    let data = program.load(&mut files.clone(), &Options::new()).unwrap();
    assert_eq!(data.sources().len(), 1);
    assert_eq!(data.sources()[0].identity(), "pilot.csv");
    assert_eq!(data.sources()[0].sha256().len(), 64);
    for seed in [1, 2, 3] {
        let outcome = program.run(&Options::new().seed(seed).data(data.clone())).unwrap();
        assert_eq!(total(&outcome), "26");
        assert_eq!(outcome.data(), data.sources());
    }
    // Running never reads files: only the load did.
    assert_eq!(files.opened.load(Ordering::Relaxed), 1);
}

#[test]
fn data_must_be_the_program_s() {
    let program = probl::compile("pilot.probl", PILOT).unwrap();
    let e = program.run(&Options::new()).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Usage);
    assert!(e.to_string().contains("wasn't loaded"), "{e}");
    assert_eq!(e.diagnostics()[0].line_column().0, 2);

    let other = probl::compile(
        "other.probl",
        "let xs: list[int] = read(\"xs.txt\", format: \"lines\")\nreport sum(xs)",
    )
    .unwrap();
    let mut files: MemoryFiles = [("xs.txt", "1\n2\n")].into_iter().collect();
    let data: Data = other.load(&mut files, &Options::new()).unwrap();
    let e = program.run(&Options::new().data(data.clone())).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Usage);
    assert!(e.to_string().contains("another program"), "{e}");
    // A program that reads nothing doesn't take another's data either.
    let none = probl::compile("none.probl", "report d6").unwrap();
    assert_eq!(
        none.run(&Options::new().data(data)).unwrap_err().kind(),
        ErrorKind::Usage
    );
}

#[test]
fn the_run_s_limits_still_apply_to_loaded_data() {
    let program = probl::compile(
        "names.probl",
        "let names: list[str] = read(\"names.txt\", format: \"lines\")\nreport len(names)",
    )
    .unwrap();
    let mut files: MemoryFiles = [("names.txt", "a-rather-long-name\nb\n")].into_iter().collect();
    let data = program.load(&mut files, &Options::new()).unwrap();
    let mut limits = probl::Limits::default();
    limits.max_string_bytes = 4;
    let e = program.run(&Options::new().data(data).limits(limits)).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Limit, "{e}");
}

#[test]
fn snapshots_keep_the_data_until_cleared() {
    let program = probl::compile("pilot.probl", PILOT).unwrap();
    let counted = Counted::default();
    *counted.text.lock().unwrap() = pilot(&[1, 2]);
    let mut files = Snapshots::new(counted.clone());
    let first = program.load(&mut files, &Options::new()).unwrap();
    *counted.text.lock().unwrap() = pilot(&[10, 20]);
    let again = program.load(&mut files, &Options::new()).unwrap();
    assert_eq!(first.sources(), again.sources());
    assert_eq!(counted.opened.load(Ordering::Relaxed), 1);
    files.clear();
    let reread = program.load(&mut files, &Options::new()).unwrap();
    assert_ne!(first.sources(), reread.sources());
    let outcome = program.run(&Options::new().data(reread)).unwrap();
    assert_eq!(total(&outcome), "30");
}

#[test]
fn a_read_cut_short_by_a_limit_keeps_nothing() {
    let program = probl::compile(
        "xs.probl",
        "let xs: list[int] = read(\"xs.txt\", format: \"lines\")\nreport sum(xs)",
    )
    .unwrap();
    let counted = Counted::default();
    *counted.text.lock().unwrap() = "1\n".repeat(200);
    let mut files = Snapshots::new(counted.clone());
    let mut limits = probl::Limits::default();
    limits.max_input_bytes = 100;
    let e = program.load(&mut files, &Options::new().limits(limits)).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Limit, "{e}");
    assert!(program.load(&mut files, &Options::new()).is_ok());
    assert_eq!(counted.opened.load(Ordering::Relaxed), 2);
}

#[test]
fn memory_files_have_no_standard_input() {
    let mut files = MemoryFiles::new();
    files.insert("a.txt", "1\n");
    assert!(files.resolve("-").is_err());
    assert!(files.resolve("b.txt").unwrap_err().contains("`b.txt`"));
    assert_eq!(files.resolve("a.txt").unwrap(), "a.txt");
}

// ── Options, errors and the rest ─────────────────────────────────────────

#[test]
fn modes_are_chosen_as_on_the_command_line() {
    let program = probl::compile("m.probl", "@mode sample(runs: 500, seed: 2)\nreport d6 > 4").unwrap();
    assert!(matches!(program.mode(), Mode::Sample { runs: 500, seed: 2, .. }));
    let sampled = |options: Options| program.run(&options).unwrap().sampling().map(|s| (s.runs(), s.seed()));
    assert_eq!(sampled(Options::new()), Some((500, 2)));
    assert_eq!(sampled(Options::new().seed(9)), Some((500, 9)));
    assert_eq!(sampled(Options::new().enumerate().runs(100)), None);
    let plain = probl::compile("p.probl", "report d6 > 4").unwrap();
    assert_eq!(plain.mode(), Mode::Auto);
    let runs = plain.run(&Options::new().runs(1000)).unwrap();
    assert_eq!(runs.sampling().map(|s| (s.runs(), s.seed())), Some((1000, 0)));
    assert!(
        plain
            .run(&Options::new().sample())
            .unwrap()
            .text()
            .starts_with("sample · 10,000 runs · seed 0")
    );
}

#[test]
fn errors_say_what_kind_and_where() {
    let e = probl::compile("bad.probl", "let x = 1\nreport y").unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Compile);
    assert!(e.to_string().contains("`y`"), "{e}");
    let d = &e.diagnostics()[0];
    assert_eq!(d.severity(), probl::Severity::Error);
    assert_eq!(d.line_column(), (2, 8));
    assert_eq!(&"let x = 1\nreport y"[d.span()], "y");
    assert!(e.render(false).contains("bad.probl:2:8"), "{}", e.render(false));

    let program = probl::compile("index.probl", "let xs = [1, 2]\nreport xs[5]").unwrap();
    let e = program.run(&Options::new()).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Language);
    assert_eq!(e.diagnostics()[0].line_column().0, 2);
    let e = probl::compile("today.probl", "report today")
        .unwrap()
        .run(&Options::new())
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Language);
    let mut limits = probl::Limits::default();
    limits.max_worlds = 10;
    let e = run_err(
        "let a ~ d6\nlet b ~ d6\nreport a * 10 + b",
        Options::new().limits(limits),
    );
    assert_eq!(e.kind(), ErrorKind::Limit);
    let cancel = probl::Cancel::new();
    cancel.cancel();
    let e = run_err("report d6", Options::new().cancel(&cancel));
    assert_eq!(e.kind(), ErrorKind::Limit);
}

fn run_err(source: &str, options: Options) -> probl::Error {
    probl::compile("e.probl", source).unwrap().run(&options).unwrap_err()
}

#[test]
fn dates_and_today() {
    let date: Date = "2024-02-29".parse().unwrap();
    assert_eq!(date, Date::from_ymd(2024, 2, 29).unwrap());
    assert_eq!(date.to_string(), "2024-02-29");
    assert!("2026-02-29".parse::<Date>().is_err() && Date::from_ymd(2026, 2, 29).is_none());
    let outcome = run("report today.add_months(1)", &Options::new().today(date));
    assert_eq!(outcome.today(), Some(date));
    assert!(outcome.text().contains("2024-03-29"), "{}", outcome.text());
    assert!(Date::today_utc().is_some());
}

#[test]
fn printing_is_collected_or_streamed() {
    let program = probl::compile("p.probl", "print(\"hello\")\nprint(\"world\")\nreport d6 > 4").unwrap();
    let outcome = program.run(&Options::new()).unwrap();
    assert_eq!(outcome.printed(), ["hello", "world"]);
    let mut lines = Vec::new();
    let outcome = program
        .run_with(&Options::new(), &mut |line: &str| lines.push(line.to_string()))
        .unwrap();
    assert_eq!(lines, ["hello", "world"]);
    assert!(outcome.printed().is_empty());
}

#[test]
fn some_reports_render_alone() {
    let outcome = run("report d6 > 4 as \"high\"\nreport d6 < 3 as \"low\"", &Options::new());
    assert_eq!(outcome.reports().len(), 2);
    assert_eq!(outcome.render(1..2), "low    33.33%\n");
    assert!(outcome.text().ends_with(&outcome.render(0..2)));
    assert_eq!(outcome.render(2..9), "");
}

#[test]
fn progress_is_told_after_each_batch() {
    let told = Arc::new(Mutex::new(Vec::new()));
    let sink = told.clone();
    let options = Options::new()
        .runs(2500)
        .progress(move |done, total| sink.lock().unwrap().push((done, total)));
    run("report d6 > 4", &options);
    assert_eq!(*told.lock().unwrap(), [(1000, 2500), (2000, 2500), (2500, 2500)]);
}

#[test]
fn exact_updates_are_counted() {
    let source = include_str!("../../../benches/ab_test.probl");
    let outcome = run(source, &Options::new().runs(2000));
    let a = outcome
        .stats()
        .exact_updates()
        .iter()
        .find(|u| u.variable() == "a")
        .unwrap();
    assert_eq!(
        (a.line(), a.delayed(), a.observations(), a.drawn()),
        (10, 2000, 60_000, 2000)
    );
}

#[test]
fn the_engine_checks_change_the_work_not_the_answer() {
    use probl::__internal::{EngineChecks, engine_checks};
    // Worlds with the same total merge after each roll.
    let program = probl::compile(
        "p.probl",
        "var total = 0\nrepeat 5 {\n  let r ~ d6\n  total += r\n}\nreport total > 20",
    )
    .unwrap();
    let merged = program.run(&Options::new()).unwrap();
    let checks = EngineChecks {
        merge: false,
        memoize: false,
        solve: false,
    };
    let unmerged = program.run(&engine_checks(Options::new(), checks)).unwrap();
    assert_eq!(merged.text(), unmerged.text());
    assert!(unmerged.stats().world_steps() > merged.stats().world_steps());
}

// ── The reports agree with the text ──────────────────────────────────────

/// The number after a report's label on its line, as printed.
fn printed_number(line: &str, after: &str) -> Option<f64> {
    let rest = &line[line.find(after)? + after.len()..];
    let token = rest.split_whitespace().next()?;
    token.trim_end_matches('%').replace(',', "").parse().ok()
}

#[test]
fn every_example_s_numbers_are_the_ones_it_prints() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "probl"))
        .collect();
    paths.sort();
    let mut checked = 0;
    for path in paths {
        let program = probl::compile(&path.display().to_string(), &std::fs::read_to_string(&path).unwrap()).unwrap();
        let mut options = Options::new().today("2026-09-29".parse().unwrap());
        if program.reads_data() {
            let data = program.load(&mut probl::LocalFiles::next_to(&path), &options).unwrap();
            options = options.data(data);
        }
        let outcome = program.run(&options).unwrap();
        let sampled = outcome.sampling().is_some();
        for report in outcome.reports() {
            let [group] = report.groups() else { continue };
            if group.key().is_some() {
                continue;
            }
            let label = report.label();
            let Some(line) = outcome
                .text()
                .lines()
                .find(|l| l.strip_prefix(label).is_some_and(|rest| rest.starts_with("  ")))
            else {
                continue;
            };
            let after = &line[label.len()..];
            if let Some(p) = group.probability() {
                let Some(shown) = printed_number(after, "") else {
                    continue;
                };
                if after.trim_start().contains('–') {
                    // A range: the bounds, printed.
                    let b = p.bounds().unwrap();
                    close(shown, b.lower() * 100.0, 0.005);
                } else {
                    let tolerance = if sampled { 0.5 } else { 0.005 };
                    close(shown, p.point().unwrap() * 100.0, tolerance);
                }
                checked += 1;
            } else if let Some(numeric) = group.numeric() {
                if !after.contains("mean ") || after.contains("mean ≈") || after.contains('%') {
                    continue;
                }
                let shown = printed_number(after, "mean ").unwrap();
                let mean = numeric.mean().unwrap().point().unwrap();
                let tolerance = if mean.abs().max(numeric.sd().unwrap().point().unwrap()) < 100.0 {
                    0.005
                } else {
                    0.5
                };
                close(shown, mean, tolerance + 1e-9);
                checked += 1;
            }
        }
    }
    assert!(checked >= 20, "only {checked} reports were checked");
}
