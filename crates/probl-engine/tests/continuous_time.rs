mod common;
use common::*;
use probl_engine::{Options, value::Value};
use probl_sema::ir::Mode;

#[test]
fn fractional_delays_round_once_toward_the_earlier_time_on_ties() {
    let text = r#"
        let t = '9999-01-01T00:00:00.000000001'
        report t + '0.5ns' == t
        report t + '0.5001ns' == t + '1ns'
        report t - '0.5ns' == t - '1ns'
        report t + '-0.5ns' == t - '1ns'
        report t - '-0.5ns' == t
        report t + '1.5ns' == t + '1ns'
        report t - '1.5ns' == t - '2ns'
        report '0.5ns' + t == t
        report '2027-01-01T23:59:59.999999999' + '0.6ns' == '2027-01-02T00:00:00'
        report (t + '9007199254740992ns') - t == '9007199254740992ns'
    "#;
    for mode in [Mode::Enumerate, Mode::Sample { runs: 10, seed: 1 }] {
        let out = exec_raw(
            text,
            &Options {
                mode: Some(mode),
                ..Options::default()
            },
        )
        .unwrap();
        for r in out.reports {
            assert_eq!(r.distribution(), vec![(Value::Bool(true), 1.0)]);
        }
    }
}

#[test]
fn normal_forecast_works_in_both_modes_with_typed_summaries() {
    let src = r#"
        let start = '2027-01-31T09:00:00.000000001'
        let x ~ normal(30, 5)
        let delay: duration = x * '1m'
        let finish: datetime = start + delay
        report delay
        report finish
        report finish < '2027-01-31T09:30:00.000000001'
        report typeof finish == "datetime" and typeof delay == "duration"
    "#;
    for mode in [Mode::Enumerate, Mode::Sample { runs: 1000, seed: 3 }] {
        let out = exec_raw(
            src,
            &Options {
                mode: Some(mode),
                ..Options::default()
            },
        )
        .unwrap();
        assert!(out.output.contains("2027-01-31T09:"), "{}", out.output);
        assert!(!out.output.contains("unavailable"), "{}", out.output);
        assert_eq!(out.reports[3].distribution(), vec![(Value::Bool(true), 1.0)]);
        let p = out.reports[2].chance().unwrap();
        assert!((p - 0.5).abs() < 0.06, "{p}");
        let dist = out.reports[1].distribution();
        assert_eq!(probl_engine::time_stats::kind(&dist), Some("datetime"));
        let moments = probl_engine::time_stats::moments(&dist, None).unwrap().unwrap();
        assert!(matches!(moments.mean, Value::DateTime(_)));
        assert!(matches!(moments.sd, Some(Value::Duration(_))));
    }
}

#[test]
fn nanosecond_bins_comparisons_conditioning_and_sampling_agree() {
    let base = r#"
        let t = '9999-01-01T00:00:00.000000001'
        let x ~ uniform(-2, 2)
        let finish = t + nanoseconds(x)
    "#;
    for (op, expected) in [
        ("==", 0.25),
        ("!=", 0.75),
        ("<", 0.375),
        ("<=", 0.625),
        (">", 0.375),
        (">=", 0.625),
    ] {
        close(chance(&format!("{base}\nreport finish {op} t")), expected);
        let src = format!("{base}\nreport finish {op} t");
        let sampled = exec_raw(
            &src,
            &Options {
                mode: Some(Mode::Sample { runs: 4000, seed: 42 }),
                ..Options::default()
            },
        )
        .unwrap();
        assert!((sampled.reports[0].chance().unwrap() - expected).abs() < 0.03);
    }
    close(chance(&format!("{base}\nobserve finish == t\nreport x < 0")), 0.5);
    close(chance(&format!("{base}\nobserve x > 0\nreport finish == t")), 0.25);
    close(chance(&format!("{base}\nreport finish - t == '0ns'")), 0.25);
    close(chance(&format!("{base}\nreport finish + '1ns' == t + '1ns'")), 0.25);
    close(chance(&format!("{base}\nreport finish - finish == '0ns'")), 1.0);
    close(chance(&format!("{base}\nreport t - finish < '0ns'")), 0.375);
}

#[test]
fn simulate_retains_typed_statistics_and_fresh_latents() {
    let src = r#"
        let start = '2027-01-31T09:00:00.000000001'
        let forecast: dist[datetime] = simulate {
            let x ~ uniform(-2, 2)
            start + x * '1ns'
        }
        report mean(forecast) == start
        report abs(sd(forecast) / '1ns' - sqrt(1.5)) < 1e-10
        report abs(variance(forecast) / '1ns^2' - 1.5) < 1e-10
        report cdf(forecast, start) == 62.5%
        report pmf(forecast, start) == 25%
        report quantile(forecast, 0.1) == start - '2ns'
        let a ~ forecast
        let b ~ forecast
        report a == start and b == start
    "#;
    let out = outcome(src);
    for r in &out.reports[..6] {
        assert_eq!(r.distribution(), vec![(Value::Bool(true), 1.0)]);
    }
    close(out.reports[6].chance().unwrap(), 0.0625);
}

#[test]
fn duration_scaling_retains_dimensions_and_latent_identity() {
    close(
        chance(
            r#"
        let x ~ normal(30, 5)
        let a: duration = minutes(x)
        let b: duration = '1m' * x
        report a == b
    "#,
        ),
        1.0,
    );
    close(
        chance(
            r#"
        let x ~ normal(30, 5)
        let a: duration = minutes(x)
        report a / '1m' < 30
    "#,
        ),
        0.5,
    );
    for src in [
        "let x ~ normal(30, 5)\nlet a: float = minutes(x)",
        "let x ~ normal(30, 5)\nreport minutes(x) + 1",
        "let x ~ uniform(-2, 2)\nreport '0001-01-01T00:00:00' + seconds(x)",
        "let x ~ uniform(-2, 2)\nreport '9999-12-31T23:59:59' + seconds(x)",
    ] {
        assert!(!error(src).is_empty());
    }
}

#[test]
fn narrow_normal_moments_include_rare_rounding_jumps() {
    close(
        chance(
            r#"
        let t = '2027-01-01T00:00:00'
        let forecast = simulate { let x ~ normal(0, 0.1); t + x * '1ns' }
        let expected = 2 * (1 - cdf(normal(0, 1), 5))
        report abs(variance(forecast) / '1ns^2' - expected) < 1e-15
    "#,
        ),
        1.0,
    );
}

#[test]
fn rounded_support_endpoints_medians_and_aggregate_transport() {
    let out = outcome(
        r#"
        let t = '2027-01-01T00:00:00'
        let forecast = simulate { let x ~ uniform(0.5, 2.5); t + x * '1ns' }
        let pair = simulate { let x ~ uniform(-1, 1); list(t + x * '1ns', x) }
        let result ~ pair
        observe result[0] == t
        report quantile(forecast, 0) == t + '1ns'
        report quantile(forecast, 1) == t + '2ns'
        report median_low(forecast) == t + '1ns'
        report median_high(forecast) == t + '2ns'
        report median(forecast) == t + '1ns'
        report result[1] <= 0.5 and result[1] >= -0.5
    "#,
    );
    for r in out.reports {
        assert_eq!(r.distribution(), vec![(Value::Bool(true), 1.0)]);
    }
}

#[test]
fn report_mixtures_accept_matching_temporal_point_masses_only() {
    let out = outcome(
        r#"
        let t = '2027-01-01T00:00:00'
        let branch ~ bernoulli(50%)
        let x ~ uniform(-1, 1)
        report if branch { t } else { t + x * '1s' }
    "#,
    );
    assert!(!out.output.contains("unavailable"), "{}", out.output);
    assert!(
        error(
            r#"
        let t = '2027-01-01T00:00:00'
        let branch ~ bernoulli(50%)
        let x ~ uniform(-1, 1)
        report if branch { 1 } else { t + x * '1s' }
    "#
        )
        .contains("mixing")
    );
}
