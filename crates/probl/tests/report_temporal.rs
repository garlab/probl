#![cfg(feature = "json")]

use probl::{Limits, Options, Outcome, ReportSummary};
use probl_syntax::duration::Duration;
use serde_json::Value;

fn document(out: &Outcome) -> Value {
    serde_json::from_str(&out.reports_json(&Default::default()).unwrap()).unwrap()
}

fn run(source: &str, options: &Options) -> Outcome {
    probl::compile("time.probl", source).unwrap().run(options).unwrap()
}

#[test]
fn summaries_round_in_shared_units_without_changing_canonical_values() {
    let out = run(
        "report minutes(one_of([10,20,35])); report hours(one_of([1,2,3])); report str(hours(3))",
        &Options::new(),
    );
    assert!(
        out.text()
            .contains("mean 21.67m · sd 10.27m · 5% 10m · median 20m · 95% 35m"),
        "{}",
        out.text()
    );
    assert!(
        out.text()
            .contains("mean 120m · sd 49m · 5% 60m · median 120m · 95% 180m"),
        "{}",
        out.text()
    );
    assert!(out.text().contains("0.125d")); // str() remains canonical.
    let doc = document(&out);
    let summary = &doc["reports"][1]["groups"][0]["summary"];
    assert_eq!(summary["quantiles"][4]["value"]["value"], "0.125d");
    assert!(summary["mean_sampling"].is_null());

    let out = run("report hours(3)", &Options::new());
    assert!(out.text().contains("3h"), "{}", out.text());
    let out = run(
        "report hours(one_of([1,2,3]))",
        &Options::new().report_summary(0, "q:0.95".parse().unwrap()),
    );
    assert!(out.text().contains("95% 3h"), "{}", out.text());
}

#[test]
fn negative_squared_and_subnanosecond_summaries_keep_their_dimensions() {
    for (source, expected) in [
        (
            "report one_of(['-30m','-10m'])",
            "mean -20.00m · sd 10.00m · 5% -30m · median -20m · 95% -10m",
        ),
        ("report one_of(['1s^2','9s^2'])", "mean 5.00s^2 · sd 4.00s^2"),
        (
            "report one_of(['0ns','1ns'])",
            "mean 0.50ns · sd 0.50ns · 5% 0ns · median 0.50ns · 95% 1ns",
        ),
        ("report one_of(['0ns','0.0000002ns'])", "mean 1.00e-7ns · sd 1.00e-7ns"),
    ] {
        let out = run(source, &Options::new());
        assert!(out.text().contains(expected), "{}", out.text());
    }
    let out = run(
        "for k in 1..2 { report minutes(one_of([10,20,35])) by k }",
        &Options::new().report_summary(0, ReportSummary::moments()),
    );
    assert!(
        out.text().contains("21.67m") && out.text().contains("10.27m"),
        "{}",
        out.text()
    );

    let out = run(
        "report one_of(['9007199254740992ns','9007199254740994ns'])",
        &Options::new(),
    );
    assert!(
        out.text().contains("mean 9,007,199,254,740,993ns · sd 1ns"),
        "{}",
        out.text()
    );
}

#[test]
fn clock_precision_follows_spread_while_json_preserves_nanoseconds() {
    let out = run(
        "let a='2027-01-31T09:59:59.447021209'; report one_of([a-'300s',a+'300s'])",
        &Options::new().report_summary(0, ReportSummary::moments()),
    );
    assert!(
        out.text().contains("mean 2027-01-31T09:59:59.4 · sd 5.00m"),
        "{}",
        out.text()
    );
    assert_eq!(
        document(&out)["reports"][0]["groups"][0]["summary"]["mean_value"]["value"],
        "2027-01-31T09:59:59.447021209"
    );
    let out = run(
        "let a='1969-12-31T23:59:59.999999999'; report one_of([a,a+'1ns'])",
        &Options::new(),
    );
    assert!(
        out.text().contains("mean 1969-12-31T23:59:59.999999999 · sd 0.50ns"),
        "{}",
        out.text()
    );
    let out = run("report '9999-12-31T23:59:59.999999999'", &Options::new());
    assert!(out.text().contains("9999-12-31T23:59:59.999999999"));
}

#[test]
fn sampled_time_errors_match_independent_run_errors_in_every_dimension() {
    let program = probl::compile(
        "weighted time",
        r#"
        let a = '9999-12-31T23:59:59.999999000'
        let huge = nanoseconds(10^100)
        let x ~ d6
        score if x > 3 {90%} else {10%}
        for k in 1..x {
            let n = x + k
            report n by k mod 2
            report a + nanoseconds(n) by k mod 2
            report huge + nanoseconds(n) by k mod 2
            report n * '1ns^2' by k mod 2
        }
    "#,
    )
    .unwrap();
    let mut previous = None;
    for threads in [1, 4] {
        let mut limits = Limits::default();
        limits.max_threads = threads;
        let mut options = Options::new().runs(2100).seed(42).limits(limits);
        for id in 0..4 {
            options = options.report_summary(id, ReportSummary::moments());
        }
        let out = program.run(&options).unwrap();
        let doc = document(&out);
        for group in 0..2 {
            let numeric = &doc["reports"][0]["groups"][group]["summary"]["mean"]["sampling"];
            let expected = numeric["standard_error"].as_f64().unwrap();
            for report in 1..4 {
                let time = &doc["reports"][report]["groups"][group]["summary"]["mean_sampling"];
                assert_eq!(time["status"], "estimated");
                assert_eq!(time["contributing_runs"], numeric["contributing_runs"]);
                let se = Duration::parse(time["standard_error"]["value"].as_str().unwrap()).unwrap();
                assert_eq!(se.squared, report == 3);
                assert!(
                    (se.amount.as_float().unwrap() - expected).abs() < 1e-10,
                    "{time} vs {numeric}"
                );
            }
        }
        assert!(out.render(1..4).contains('±'), "{}", out.text());
        if let Some(previous) = previous {
            assert_eq!(doc, previous);
        }
        previous = Some(doc);
    }
}

#[test]
fn within_run_spread_is_not_mistaken_for_independent_sampling_error() {
    let out = run(
        "let x ~ d2; for n in [x,3-x] { report nanoseconds(n) by 0 }",
        &Options::new()
            .runs(400)
            .seed(7)
            .report_summary(0, ReportSummary::moments()),
    );
    let doc = document(&out);
    let summary = &doc["reports"][0]["groups"][0]["summary"];
    assert_eq!(summary["sd_value"]["value"], "0.5ns");
    assert_eq!(summary["mean_sampling"]["status"], "not_estimable");
    assert!(summary["mean_sampling"]["standard_error"].is_null());
    assert!(!out.render(0..1).contains('±'));
}
