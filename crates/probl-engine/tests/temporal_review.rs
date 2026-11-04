mod common;
use common::*;
use probl_engine::{Options, value::Value};
use probl_sema::ir::Mode;

fn phi(x: f64) -> f64 {
    0.5 * libm::erfc(-x / std::f64::consts::SQRT_2)
}

#[test]
fn unbounded_rounding_keeps_integer_types_probabilities_and_observations() {
    for (name, mass, below) in [
        ("floor", phi(1.0) - 0.5, phi(1.0)),
        ("ceil", phi(1.0) - 0.5, 0.5),
        ("trunc", phi(1.0) - phi(-1.0), phi(1.0)),
        ("round", phi(0.5) - phi(-0.5), phi(0.5)),
    ] {
        let out = outcome(&format!(
            "let x~normal(0,1); let n: int={name}(x); report n==0; report n<=0; report typeof n"
        ));
        close(out.reports[0].chance().unwrap(), mass);
        close(out.reports[1].chance().unwrap(), below);
        assert_eq!(out.reports[2].distribution(), vec![(Value::str("int"), 1.0)]);
        close(
            chance(&format!("let x~normal(0,1); observe {name}(x)==0; report {name}(x)==0")),
            1.0,
        );
    }
    close(chance("let x~normal(0,1); observe round(x)==-1; report x<0"), 1.0);
    close(chance("let x~normal(0,1); report round(max(x,0.5))==1"), phi(1.5));
    let out = outcome("let x~uniform(0,1000000); let n:int=floor(x); report n<500000");
    close(out.reports[0].chance().unwrap(), 0.5);
}

#[test]
fn rounded_day_forecasts_work_in_both_modes_but_fractional_days_need_a_choice() {
    let source = r#"
        let start='2027-01-31'
        let x~normal(30,5)
        let finish: date=start+days(round(x))
        report finish
        report finish < '2027-03-02'
        report typeof finish=="date"
        report date(datetime(finish))==finish
        report finish+1==start+days(round(x))+days(1)
        report start+round(x)==finish
        report 1+finish==finish+1
        let morning=datetime(finish)+'9h 30m'
        report date(morning)==finish
        report morning.hour()==9 and morning.minute()==30
    "#;
    for mode in [Mode::Enumerate, Mode::Sample { runs: 1000, seed: 14 }] {
        let out = exec_raw(
            source,
            &Options {
                mode: Some(mode.clone()),
                ..Options::default()
            },
        )
        .unwrap();
        assert!(out.output.contains("median 2027-03-"), "{}", out.output);
        assert!(!out.output.contains("unavailable"));
        assert!((out.reports[1].chance().unwrap() - phi(-0.1)).abs() < 0.05);
        for report in &out.reports[2..] {
            assert_eq!(report.distribution(), vec![(Value::Bool(true), 1.0)]);
        }
        let error = exec_raw(
            "let x~normal(30,5); report '2027-01-31'+x*'1d'",
            &Options {
                mode: Some(mode),
                ..Options::default()
            },
        )
        .unwrap_err();
        assert!(error.message.contains("whole number of 24-hour days"));
        assert!(error.help.unwrap().contains("days(round(x))"));
    }
}

#[test]
fn date_and_clock_projections_preserve_midnight_and_conditioning() {
    let base = "let t='2027-01-31T23:00:00'; let x~uniform(-1,2); let arrival=t+x*'1h';";
    close(chance(&format!("{base} report date(arrival)=='2027-01-31'")), 2.0 / 3.0);
    close(chance(&format!("{base} report arrival.hour()==23")), 1.0 / 3.0);
    close(
        chance(&format!(
            "{base} observe date(arrival)=='2027-01-31'; report arrival.hour()==23"
        )),
        0.5,
    );
    close(chance(&format!("{base} observe arrival.hour()==0; report x>=1")), 1.0);
    let edge = "let t='2027-01-31T23:59:59.999999999'; let x~normal(0,1); let arrival=t+max(x,0.5)*'1ns';";
    close(chance(&format!("{edge} report date(arrival)=='2027-01-31'")), phi(0.5));
    close(chance(&format!("{edge} report arrival.hour()==23")), phi(0.5));
    let fine = "let t='9999-01-31T23:59:59.999999999'; let x~normal(0,1); let arrival=t+x*'1ns';";
    close(chance(&format!("{fine} report arrival.second()==0")), 1.0 - phi(0.5));
    close(
        chance(&format!("{fine} report arrival.nanosecond()==0")),
        phi(1.5) - phi(0.5),
    );
}

#[test]
fn datetime_interval_queries_and_samples_keep_the_calendar_type() {
    let source = r#"
        let a='2027-01-31T09:00:00.000000001'
        let b=a+'1h'
        let flat:dist[datetime]=uniform(a,b)
        let estimate:dist[datetime]=a to b
        report mean(flat)==a+'30m'
        report quantile(estimate,5%)==a
        report quantile(estimate,95%)==b
        report cdf(flat,a+'30m') > 49.9999% and cdf(flat,a+'30m') < 50.0001%
        let t~flat
        report typeof t=="datetime" and t>=a and t<=b
        report date(t)=='2027-01-31'
    "#;
    for mode in [Mode::Enumerate, Mode::Sample { runs: 200, seed: 31 }] {
        let out = exec_raw(
            source,
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
    assert!(error("report uniform('2027-01-31T10:00:00','2027-01-31T09:00:00')").contains("lower endpoint"));
}

#[test]
fn duration_migration_and_analytic_limit_messages_are_actionable() {
    for src in [
        "report ('2027-01-05'-'2027-01-01')>3",
        "report [1,2]['2027-01-02'-'2027-01-01']",
    ] {
        let e = error(src);
        assert!(e.contains("divide by '1d'") && e.contains("div '1d'"), "{e}");
    }
    for op in ["div", "mod"] {
        let e = error(&format!("let x~normal(30,5); report minutes(x) {op} '1m'"));
        assert!(e.contains("analytic durations") && e.contains("sample(runs:"), "{e}");
        assert!(!e.contains("incompatible time dimensions"));
    }
    let e = error("let x~uniform(0,1); let y~uniform(0,1); report x*'1m'+y*'1m'");
    assert!(
        e.contains("except to add or subtract normal") && e.contains("sample(runs:"),
        "{e}"
    );
    let e = error("let x~normal(30,5); let y~normal(10,2); report round(x)*'1m'+round(y)*'1m'");
    assert!(
        e.contains("sum the numeric delays") && e.contains("sample(runs:"),
        "{e}"
    );
    close(
        chance("let x~normal(30,5); let y~normal(10,2); report x*'1m'+y*'1m' < '40m'"),
        0.5,
    );
    let e = error("let x~uniform(-1,1); report '0001-01-01T00:00:00'+x*'1h'");
    assert!(e.contains("supported calendar") && !e.contains("yet"), "{e}");
}
