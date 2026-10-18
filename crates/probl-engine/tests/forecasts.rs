//! A forecast that reports its state at every step of a horizon, as the
//! retirement example does (docs/semantics.md, sections 9 and 14). Every run
//! reports every year, so each year's summary is over all the runs, the
//! depleted ones included.

mod common;

use common::*;
use probl_engine::report::GroupResult;

const EXAMPLE: &str = include_str!("../../../examples/21_retirement.probl");

/// The example, with each `(from, to)` replaced; each `from` must be there.
fn example(changes: &[(&str, &str)]) -> String {
    let mut source = EXAMPLE.to_string();
    for (from, to) in changes {
        assert!(source.contains(from), "the example has no {from:?}");
        source = source.replace(from, to);
    }
    source
}

/// A report's groups, after checking that it has one for each year, in order.
fn yearly(groups: &[GroupResult]) -> &[GroupResult] {
    assert_eq!(groups.len(), 101);
    for (year, g) in groups.iter().enumerate() {
        assert_eq!(g.key.as_f64(), Some(year as f64));
    }
    groups
}

fn chance_of(g: &GroupResult) -> f64 {
    g.fact.expect("a fact").point.expect("a probability")
}

#[test]
fn fixed_rates_follow_the_recurrence() {
    // Literal rates, rather than normal distributions with no spread.
    let fixed = [
        ("let r ~ normal(4%, 18%)", "let r = 4%"),
        ("let q ~ normal(2%, 1.5%)", "let q = 2%"),
    ];
    let mut expected = vec![1_000_000.0];
    for year in 1..=100 {
        let before: f64 = expected[year - 1];
        expected.push((before * 1.04 / 1.02 - 30_000.0).max(0.0));
    }
    let depleted = expected.iter().position(|&b| b == 0.0).unwrap();
    assert_eq!(depleted, 55);
    let sampled = example(&[fixed[0], fixed[1], ("runs: 5_000", "runs: 20")]);
    let enumerated = example(&[fixed[0], fixed[1], ("@mode sample(runs: 5_000, seed: 42)\n", "")]);
    for source in [sampled, enumerated] {
        let out = outcome(&source);
        // The balance, and the baseline, which follows the same rates now.
        for report in [0, 2] {
            for (year, g) in yearly(&out.results[report].groups).iter().enumerate() {
                let [(value, p)] = g.distribution.as_slice() else {
                    panic!("year {year}: {:?}", g.distribution);
                };
                close(*p, 1.0);
                let value = value.as_f64().unwrap();
                assert!(
                    (value - expected[year]).abs() <= 1e-6 * expected[year],
                    "year {year}: {value}, expected {}",
                    expected[year]
                );
            }
        }
        let groups = yearly(&out.results[1].groups);
        assert_eq!(out.results[0].groups[0].distribution[0].0.as_f64(), Some(1_000_000.0));
        assert!(out.results[0].groups[depleted - 1].distribution[0].0.as_f64().unwrap() > 0.0);
        for (year, g) in groups.iter().enumerate() {
            close(chance_of(g), if year < depleted { 0.0 } else { 1.0 });
        }
    }
}

#[test]
fn every_run_reports_every_year_and_zero_stays_zero() {
    let source = example(&[
        ("runs: 5_000", "runs: 1_000"),
        ("  if year > 0 {", "  let was_depleted = balance == 0\n  if year > 0 {"),
        (
            "  report baseline by year as \"constant-rate baseline\"\n",
            "  report baseline by year as \"constant-rate baseline\"\n  report was_depleted and balance > 0 by year as \"revived\"\n",
        ),
    ]);
    let out = outcome(&source);
    assert_eq!(out.results.len(), 4);
    for report in &out.results {
        for g in yearly(&report.groups) {
            assert_eq!(g.support.expect("sampled").contributing_runs, 1_000);
        }
    }
    for g in &out.results[3].groups {
        close(chance_of(g), 0.0);
    }
    let depleted: Vec<f64> = out.results[1].groups.iter().map(chance_of).collect();
    assert_eq!(depleted[0], 0.0);
    assert!(depleted.windows(2).all(|w| w[0] <= w[1]), "{depleted:?}");
    assert!(depleted[30] > 0.3 && depleted[100] > 0.8, "{depleted:?}");
    // The depleted runs' zeros are in the balance's summaries, as many as
    // the depletion probability says.
    for (year, g) in out.results[0].groups.iter().enumerate() {
        let zero: f64 = g
            .distribution
            .iter()
            .filter(|(v, _)| v.as_f64() == Some(0.0))
            .map(|(_, p)| p)
            .sum();
        close(zero, depleted[year]);
    }
}

#[test]
fn depletion_can_be_certain_or_impossible() {
    // No first year grows $1M into $10M: every run is empty after year 1.
    let always = outcome(&example(&[
        ("runs: 5_000", "runs: 200"),
        ("let withdrawal = 30_000", "let withdrawal = 10_000_000"),
    ]));
    // Without withdrawals, clamped rates keep growth, and the balance, positive.
    let never = outcome(&example(&[
        ("runs: 5_000", "runs: 200"),
        ("let withdrawal = 30_000", "let withdrawal = 0"),
    ]));
    for (out, chance) in [(always, 1.0), (never, 0.0)] {
        for (year, g) in yearly(&out.results[1].groups).iter().enumerate() {
            let p = if year == 0 { 0.0 } else { chance };
            let fact = g.fact.unwrap();
            assert_eq!(fact.point, Some(p), "year {year}");
            let sampling = fact.sampling.unwrap();
            assert_eq!(sampling.support.contributing_runs, 200);
            let (lo, hi) = sampling.wilson.expect("a Wilson interval");
            assert!(lo <= p && p <= hi && hi - lo > 0.0, "year {year}: {lo}–{hi}");
        }
    }
}

/// An independent simulation of the example's model in NumPy 2.5.1, seed
/// 20261018 (docs/use-cases.md), over 1,000,000 trials: for each of `YEARS`,
/// the 5%, 25%, 50%, 75% and 95% balances, then the share depleted.
const YEARS: [usize; 6] = [1, 10, 20, 30, 50, 100];
const REFERENCE: [[f64; 6]; 6] = [
    [698168.0, 869926.0, 989422.0, 1109226.0, 1281998.0, 0.0],
    [177218.0, 445427.0, 741081.0, 1165721.0, 2104472.0, 0.002340],
    [0.0, 109593.0, 440120.0, 1027625.0, 2688169.0, 0.152765],
    [0.0, 0.0, 125224.0, 793873.0, 3160749.0, 0.415189],
    [0.0, 0.0, 0.0, 192905.0, 3855335.0, 0.704561],
    [0.0, 0.0, 0.0, 0.0, 4316681.0, 0.882887],
];
/// Their standard deviations over 400 such simulations of the example's
/// 5,000 trials.
const SPREAD: [[f64; 6]; 6] = [
    [5556.0, 3552.0, 3155.0, 3408.0, 5492.0, 0.0],
    [6143.0, 7339.0, 9207.0, 15119.0, 36952.0, 0.000688],
    [0.0, 7000.0, 11496.0, 21858.0, 72898.0, 0.004948],
    [0.0, 0.0, 12112.0, 27960.0, 119939.0, 0.007241],
    [0.0, 0.0, 0.0, 30798.0, 208776.0, 0.006528],
    [0.0, 0.0, 0.0, 0.0, 504870.0, 0.004426],
];

#[test]
fn the_example_agrees_with_an_independent_simulation() {
    let out = outcome(EXAMPLE);
    for ((year, reference), spread) in YEARS.into_iter().zip(REFERENCE).zip(SPREAD) {
        let balance = out.results[0].groups[year].numeric.as_ref().unwrap();
        let mut ours: Vec<f64> = [0.05, 0.25, 0.5, 0.75, 0.95]
            .map(|q| balance.quantile(q).unwrap().point.unwrap())
            .to_vec();
        ours.push(chance_of(&out.results[1].groups[year]));
        let names = ["5%", "25%", "median", "75%", "95%", "depleted"];
        for (i, x) in ours.into_iter().enumerate() {
            // Zero, with no spread, where most runs are depleted: exactly.
            assert!(
                (x - reference[i]).abs() <= 5.0 * spread[i],
                "year {year}, {}: {x}, independently {} ± {}",
                names[i],
                reference[i],
                spread[i]
            );
        }
    }
}
