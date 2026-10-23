#![cfg(feature = "json")]

use probl::{Options, Outcome, ReportJsonError, ReportJsonLimits};
use serde_json::{Value, json};

fn run(source: &str) -> Outcome {
    probl::compile("report.probl", source)
        .unwrap()
        .run(&Options::new())
        .unwrap()
}
fn document(outcome: &Outcome) -> Value {
    serde_json::from_str(&outcome.reports_json(&ReportJsonLimits::default()).unwrap()).unwrap()
}

#[test]
fn identities_typed_keys_and_summaries_do_not_depend_on_display_text() {
    let out = run(
        "for k in [1,1.0,prob(1),\"1\",date(2026,10,19),9007199254740993] { report d6 by k as \"same\" }; report true as \"same\"",
    );
    let doc = document(&out);
    assert_eq!(doc["schema_version"], 1);
    let reports = doc["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0]["id"], 0);
    assert_eq!(reports[1]["id"], 1);
    assert_eq!(reports[0]["label"], reports[1]["label"]);
    assert_eq!(reports[0]["visits"], "per_visit");
    let groups = reports[0]["groups"].as_array().unwrap();
    let keys: Vec<_> = groups.iter().map(|g| &g["key"]).collect();
    for (kind, value) in [
        ("int", json!("1")),
        ("float", json!(1.0)),
        ("prob", json!(1.0)),
        ("str", json!("1")),
        ("date", json!("2026-10-19")),
        ("int", json!("9007199254740993")),
    ] {
        assert!(
            keys.iter().any(|k| k["kind"] == kind && k["value"] == value),
            "{keys:?}"
        );
    }
    assert!((groups[0]["summary"]["mean"]["point"].as_f64().unwrap() - 3.5).abs() < 1e-12);
    assert_eq!(groups[0]["summary"]["quantiles"][2]["estimate"]["point"], 3.5);
    assert_eq!(reports[1]["groups"][0]["probability"]["point"], 1.0);
    assert!(reports[1]["groups"][0]["key"].is_null());
    assert_eq!(doc["run"]["unresolved"]["log_weight_upper_bound"], "-infinity");
}

#[test]
fn unsafe_numeric_values_and_dates_retain_exact_quantiles() {
    let out = run("report one_of([9007199254740993,9007199254740995]); report one_of([date(2026,1,1),date(2026,1,3)])");
    let doc = document(&out);
    let numbers = &doc["reports"][0]["groups"][0];
    assert_eq!(numbers["kind"], "number");
    assert!(numbers["summary"]["mean"].is_null());
    assert_eq!(numbers["summary"]["quantiles"][0]["value"]["value"], "9007199254740993");
    assert_eq!(numbers["summary"]["quantiles"][2]["value"]["value"], "9007199254740994");
    let dates = &doc["reports"][1]["groups"][0];
    assert_eq!(dates["kind"], "date");
    assert_eq!(dates["summary"]["quantiles"][2]["value"]["value"], "2026-01-02");
}

#[test]
fn reach_visits_support_and_sampling_error_remain_distinct() {
    let out = run(
        "@mode sample(runs: 100, seed: 7)\nlet x ~ bernoulli(50%); if x { report true as \"branch\" }; for k in [1,1] { report x by k }; for year in 0..2 { report year by year }; if false { report 1 }",
    );
    let doc = document(&out);
    let reports = &doc["reports"];
    assert!(reports[0]["reach"]["share"].as_f64().unwrap() < 1.0);
    let sampling = &reports[0]["groups"][0]["probability"]["sampling"];
    assert_eq!(sampling["status"], "not_estimable");
    assert_eq!(sampling["confidence_interval"]["method"], "wilson");
    assert_eq!(sampling["confidence_interval"]["level"], 0.95);
    assert_eq!(reports[1]["visits"], "per_visit");
    assert_eq!(reports[1]["groups"][0]["support"]["contributing_runs"], "100");
    assert!(reports[1]["groups"][0]["probability"]["sampling"]["confidence_interval"].is_null());
    assert_eq!(reports[2]["visits"], "per_key");
    assert_eq!(reports[2]["groups"][0]["summary"]["deterministic"], true);
    assert_eq!(reports[3]["groups"], json!([]));
    assert_eq!(doc["run"]["sampling"]["seed"], "7");
}

#[test]
fn evidence_missing_mass_and_partial_failures_are_not_lost() {
    let out =
        run("@mode sample(runs: 100, seed: 23)\nlet x ~ bernoulli(50%); score if x { 90% } else { 10% }; report x");
    let doc = document(&out);
    assert_eq!(doc["run"]["evidence"]["kind"], "probability");
    assert!(doc["run"]["sampling"]["effective_runs"].as_f64().unwrap() < 100.0);
    let craps = document(&run(include_str!("../../../examples/02_craps.probl")));
    assert_eq!(craps["run"]["unresolved"]["zero"], false);
    assert_eq!(craps["reports"][0]["groups"][0]["probability"]["complete"], false);
    assert!(craps["reports"][0]["groups"][0]["probability"]["bounds"].is_array());
    let error = probl::compile("partial.probl", "@on_error partial\nlet n ~ d2; report 10 / (n-1)")
        .unwrap()
        .run(&Options::new())
        .err()
        .unwrap();
    let partial = document(error.partial().unwrap());
    assert_eq!(partial["run"]["partial"], true);
    assert_eq!(partial["run"]["finished"], true);
    assert_eq!(partial["run"]["failed_share"], 0.5);
    assert_eq!(partial["failures"][0]["share"], 0.5);
    assert!(partial["failures"][0]["message"].as_str().unwrap().contains("zero"));
}

#[test]
fn analytic_fields_and_categorical_fallback_are_explicit() {
    let out = run("let x ~ normal(0,1); report {a:x,b:x+2}; report \"red\" by [1,2]");
    let doc = document(&out);
    let group = &doc["reports"][0]["groups"][0];
    assert_eq!(group["kind"], "structured");
    assert_eq!(group["fields"][1]["path"], ".b");
    assert_eq!(group["fields"][1]["group"]["summary"]["mean"]["point"], 2.0);
    let fallback = &doc["reports"][1]["groups"][0];
    assert_eq!(fallback["kind"], "categorical");
    assert_eq!(fallback["key"]["kind"], "opaque");
    assert_eq!(fallback["key"]["label"], "[1, 2]");
    assert!(fallback["summary"].is_null());
}

#[test]
fn budgets_fail_without_truncation_or_mutating_the_outcome() {
    let out = run("for year in 0..3 { report d6 by year }");
    let before = out.text().to_string();
    let encoded = out.reports_json(&ReportJsonLimits::default()).unwrap();
    for (limits, error) in [
        (
            ReportJsonLimits {
                max_bytes: encoded.len() - 1,
                ..ReportJsonLimits::default()
            },
            ReportJsonError::Bytes,
        ),
        (
            ReportJsonLimits {
                max_nodes: 4,
                ..ReportJsonLimits::default()
            },
            ReportJsonError::Nodes,
        ),
        (
            ReportJsonLimits {
                max_work: 10,
                ..ReportJsonLimits::default()
            },
            ReportJsonError::Work,
        ),
    ] {
        assert_eq!(out.reports_json(&limits), Err(error));
    }
    assert_eq!(
        out.reports_json(&ReportJsonLimits {
            max_bytes: encoded.len(),
            ..ReportJsonLimits::default()
        })
        .unwrap(),
        encoded
    );
    assert_eq!(out.text(), before);
    assert_eq!(out.reports_json(&ReportJsonLimits::default()).unwrap(), encoded);
}

#[test]
fn a_string_key_costs_its_label_and_its_value_once_each() {
    // The least work that exports a report keyed by a string of `n` bytes.
    let least_work = |n: usize| {
        let out = run(&format!("for k in [\"{}\"] {{ report 1 by k }}", "x".repeat(n)));
        let fits = |max_work| {
            out.reports_json(&ReportJsonLimits {
                max_work,
                ..ReportJsonLimits::default()
            })
            .is_ok()
        };
        let (mut lo, mut hi) = (0, ReportJsonLimits::default().max_work);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if fits(mid) { hi = mid } else { lo = mid + 1 }
        }
        lo
    };
    assert_eq!(least_work(2000) - least_work(1000), 2 * 1000);
}

#[test]
fn document_size_depends_on_groups_not_number_of_trials() {
    let sample = |runs| {
        run(&format!(
            "@mode sample(runs: {runs}, seed: 42)\nfor year in 1..10 {{ let x ~ normal(0,1); report x by year }}"
        ))
    };
    let small = sample(100).reports_json(&ReportJsonLimits::default()).unwrap();
    let large = sample(2000).reports_json(&ReportJsonLimits::default()).unwrap();
    assert!(large.len() < small.len() * 2);
    assert!(!large.contains("\"distribution\""));
}

#[test]
fn empty_failed_runs_and_unicode_labels_remain_valid_documents() {
    let error = probl::compile("failed.probl", "@mode sample(runs: 20, seed: 1)\nreport 1/0")
        .unwrap()
        .run(&Options::new())
        .err()
        .unwrap();
    let failed = document(error.partial().unwrap());
    assert_eq!(failed["run"]["finished"], false);
    assert_eq!(failed["run"]["partial"], true);
    assert_eq!(failed["run"]["failed_share"], 1.0);
    assert_eq!(failed["reports"][0]["groups"], json!([]));
    assert_eq!(document(&run("let x=1"))["reports"], json!([]));
    let out = run("report true by \"é🙂\\\"\" as \"quote \\\" and newline\\n\"");
    let doc = document(&out);
    assert_eq!(doc["reports"][0]["label"], "quote \" and newline\n");
    assert_eq!(doc["reports"][0]["groups"][0]["key"]["value"], "é🙂\"");
    let len = out.reports_json(&ReportJsonLimits::default()).unwrap().len();
    assert_eq!(
        out.reports_json(&ReportJsonLimits {
            max_bytes: len - 1,
            ..ReportJsonLimits::default()
        }),
        Err(ReportJsonError::Bytes)
    );
}
