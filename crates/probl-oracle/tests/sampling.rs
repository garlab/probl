//! Sampling against enumeration (audit, step 4): for generated programs,
//! every estimate that sampling prints must be within six standard errors of
//! the exact answer, and the standard errors must be calibrated: across all
//! the comparisons, estimates more than four standard errors off must be
//! rare (docs/semantics.md, section 14).
//!
//! `PROBL_SAMPLING_CASES` sets how many programs to generate (default 150),
//! `PROBL_SAMPLING_SEED` the first seed and `PROBL_SAMPLING_RUNS` the runs per
//! program (default 2,000).

use probl_engine::value::Value;
use probl_engine::{ErrorKind, Limits, Options, Outcome, Weight};
use probl_oracle::generate;
use probl_sema::ir::{Mode, Program, ReportKind};

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn options(mode: Option<Mode>) -> Options {
    Options {
        // Exact: no loop is cut short (the generated loops all end).
        epsilon: Some(0.0),
        mode,
        limits: Limits {
            max_worlds: 200_000,
            max_outcomes: 100_000,
            max_collection: 100_000,
            max_work: 20_000_000,
            max_iterations: 10_000,
            max_call_depth: 60,
            ..Limits::default()
        },
        ..Options::default()
    }
}

/// How far the estimates are from the exact values, in standard errors.
#[derive(Default)]
struct Scores {
    z: Vec<f64>,
    compared: u64,
}

impl Scores {
    /// An estimate `est` of `exact` with standard error `se`, from runs worth
    /// `n` equally weighted ones; `spread` is the exact standard deviation of
    /// one run's value, which bounds the error when the sample happens to
    /// show none.
    fn check(&mut self, what: &str, exact: f64, est: f64, se: f64, spread: f64, n: f64) -> Result<(), String> {
        self.compared += 1;
        let expected = spread / n.max(1.0).sqrt();
        let bound = 6.0 * se.max(expected) + 1e-9;
        if (est - exact).abs() > bound {
            return Err(format!(
                "{what}: estimate {est} ± {se}, exact {exact} (more than {bound} apart, from runs worth {n:.0})"
            ));
        }
        if se > 1e-12 && n >= 100.0 {
            self.z.push((est - exact) / se);
        }
        Ok(())
    }
}

fn numeric(dist: &[(Value, f64)]) -> Option<(f64, f64)> {
    let mut mean = 0.0;
    let mut second = 0.0;
    for (v, p) in dist {
        let x = match v {
            Value::Int(_) | Value::Float(_) | Value::Prob(_) => v.as_f64()?,
            _ => return None,
        };
        mean += x * p;
        second += x * x * p;
    }
    Some((mean, (second - mean * mean).max(0.0).sqrt()))
}

fn compare(program: &Program, exact: &Outcome, sampled: &Outcome, scores: &mut Scores) -> Result<(), String> {
    let info = sampled.sample.as_ref().expect("sampled");
    let z_exact = exact.evidence.unwrap_or(Weight::ONE);
    for (i, (site, (e, s))) in program
        .reports
        .iter()
        .zip(exact.reports.iter().zip(&sampled.reports))
        .enumerate()
    {
        let at = format!("report {} (`{}`)", i + 1, site.label);
        for key in s.groups.keys() {
            if !e.groups.contains_key(key) {
                return Err(format!(
                    "{at}: sampling reached the key {key}, which enumeration never does"
                ));
            }
        }
        // The share of the weight that reaches a report without `by`.
        if site.kind == ReportKind::Once && !e.groups.is_empty() {
            let reach = e.reach().ratio(z_exact);
            let est = s.reached.ratio(info.weight);
            let spread = (reach * (1.0 - reach)).max(0.0).sqrt();
            scores.check(
                &format!("{at}: reach"),
                reach,
                est,
                spread / info.effective.sqrt(),
                spread,
                info.effective,
            )?;
        }
        for (key, ea) in &e.groups {
            let Some(sa) = s.groups.get(key) else {
                // A key reached rarely may not be reached by any run.
                let share = ea.total.ratio(z_exact);
                if share * info.effective > 30.0 {
                    return Err(format!(
                        "{at}: key {key}, reached by {share} of the weight, was never sampled"
                    ));
                }
                continue;
            };
            let n = sa.effective();
            let at = format!("{at}, key {key}");
            if ea.is_event() && sa.is_event() {
                let p = ea.chance();
                scores.check(&at, p, sa.chance(), sa.chance_se(), (p * (1.0 - p)).sqrt(), n)?;
                continue;
            }
            let (ed, sd) = (ea.distribution(), sa.distribution());
            for (v, _) in &sd {
                if !ed.iter().any(|(x, _)| x == v) {
                    return Err(format!("{at}: sampling reported {v}, which enumeration never does"));
                }
            }
            if let (Some((mean, spread)), Some(_)) = (numeric(&ed), numeric(&sd)) {
                let (est, se) = sa.mean_se();
                scores.check(&format!("{at}: mean"), mean, est, se, spread, n)?;
                continue;
            }
            for (v, p) in &ed {
                let est = sd.iter().find(|(x, _)| x == v).map_or(0.0, |(_, q)| *q);
                let se = sa.value_se(v, est);
                scores.check(&format!("{at}: value {v}"), *p, est, se, (p * (1.0 - p)).sqrt(), n)?;
            }
        }
    }
    Ok(())
}

#[test]
fn sampling_agrees_with_enumeration() {
    let cases = env("PROBL_SAMPLING_CASES", 150);
    let first = env("PROBL_SAMPLING_SEED", 1);
    let runs = env("PROBL_SAMPLING_RUNS", 2_000);
    let mut scores = Scores::default();
    let (mut checked, mut skipped) = (0, 0);
    let mut failures = Vec::new();
    for seed in first..first + cases {
        let src = generate::program(seed);
        let (program, _) = probl_sema::compile(&src);
        let program = program.unwrap_or_else(|| panic!("seed {seed} doesn't compile:\n{src}"));
        let mut print = |_: &str| {};
        let Ok(exact) = probl_engine::run(&program, &options(None), &mut print) else {
            skipped += 1;
            continue;
        };
        let mode = Mode::Sample { runs, seed };
        match probl_engine::run(&program, &options(Some(mode)), &mut print) {
            Ok(sampled) => {
                checked += 1;
                if let Err(why) = compare(&program, &exact, &sampled, &mut scores) {
                    failures.push(format!("seed {seed}: {why}\n{src}"));
                }
            }
            Err(e) if e.kind == ErrorKind::Limit => skipped += 1,
            // Evidence too rare to be hit by any run.
            Err(e)
                if e.message.contains("every run was ruled out")
                    && exact.evidence.is_some_and(|z| z.to_f64() * (runs as f64) < 20.0) =>
            {
                skipped += 1
            }
            Err(e) => failures.push(format!(
                "seed {seed}: sampling fails, enumeration doesn't: {}\n{src}",
                e.message
            )),
        }
        if failures.len() >= 3 {
            break;
        }
    }
    let far = scores.z.iter().filter(|z| z.abs() > 4.0).count();
    let mean_square = scores.z.iter().map(|z| z * z).sum::<f64>() / scores.z.len().max(1) as f64;
    eprintln!(
        "{checked} programs checked, {skipped} skipped; {} estimates compared; {} z-scores, mean z² {mean_square:.2}, {far} beyond 4",
        scores.compared,
        scores.z.len()
    );
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
    // Calibration: with honest standard errors, |z| > 4 happens 0.006% of the
    // time, and the mean of z² is about 1.
    assert!(
        far * 100 <= scores.z.len().max(100),
        "{far} of {} estimates are more than 4 standard errors off",
        scores.z.len()
    );
    assert!(
        scores.z.len() < 100 || (0.5..2.0).contains(&mean_square),
        "mean z² is {mean_square}: the standard errors are off"
    );
}
