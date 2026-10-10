//! Several continuous draws together when enumerating: events of
//! independent draws, and sums of normal draws, whose covariance comes from
//! hierarchical draws and normal observations (docs/semantics.md, section
//! 13).
mod common;
use common::*;
use probl_engine::continuous::{Family, Mixture};
use probl_engine::report::analytic_mixture;
use probl_engine::{Options, Outcome};
use probl_sema::ir::Mode;

fn marginal(out: &Outcome, i: usize) -> Mixture {
    analytic_mixture(&out.reports[i].distribution()).expect("a continuous report")
}

fn normal(mean: f64, sd: f64) -> Family {
    Family::normal(mean, sd).unwrap()
}

/// The standard normal CDF.
fn phi(z: f64) -> f64 {
    normal(0.0, 1.0).cdf(z)
}

/// A report's marginal is normal with this mean and standard deviation.
#[track_caller]
fn is_normal(out: &Outcome, i: usize, mean: f64, sd: f64) {
    let m = marginal(out, i);
    let family = normal(mean, sd);
    close(m.mean(), mean);
    close(m.variance(), sd * sd);
    for q in [0.05, 0.5, 0.95] {
        assert!((m.quantile(q) - family.quantile(q)).abs() < 1e-9, "quantile {q}");
    }
}

#[track_caller]
fn chance_of(out: &Outcome, i: usize, expected: f64) {
    close(out.reports[i].chance().expect("a probability"), expected);
}

const TWO: &str = "let x ~ normal(0, 1)\nlet y ~ normal(0, 1)\n";

#[test]
fn events_of_independent_draws_combine() {
    let o = outcome(
        "let x ~ normal(0, 1)\nlet y ~ normal(0, 1)\nlet z ~ uniform(0, 1)\n\
         report x > 0 and y > 1\nreport x > 0 or y > 1\nreport not (x > 0 and y > 1)\n\
         report (x > 0) == (y > 1)\nreport x > 0 and y > 1 and z < 0.5\n\
         report (x > 0 and y > 1) or (y < -1 and z < 0.5)\nreport x > 0 and (x < 1 or y > 1)",
    );
    let (px, py, pz) = (0.5, 1.0 - phi(1.0), 0.5);
    chance_of(&o, 0, px * py);
    chance_of(&o, 1, px + py - px * py);
    chance_of(&o, 2, 1.0 - px * py);
    chance_of(&o, 3, px * py + (1.0 - px) * (1.0 - py));
    chance_of(&o, 4, px * py * pz);
    // Disjoint, since y can't be both above 1 and below -1.
    chance_of(&o, 5, px * py + phi(-1.0) * pz);
    // Both conditions on x stay one condition on x.
    chance_of(&o, 6, (phi(1.0) - 0.5) + (1.0 - phi(1.0)) * py);
}

#[test]
fn observing_an_event_of_several_draws_restricts_each() {
    // The worlds where x > 0, and where x ≤ 0 and y > 0.
    let o = outcome(&format!(
        "{TWO}observe x > 0 or y > 0\nreport x\nreport x > 0\nreport x > 0 and y > 0"
    ));
    close(o.evidence.unwrap().to_f64(), 0.75);
    let half = (2.0 / std::f64::consts::PI).sqrt();
    close(marginal(&o, 0).mean(), (0.5 * half - 0.25 * half) / 0.75);
    chance_of(&o, 1, 2.0 / 3.0);
    chance_of(&o, 2, 1.0 / 3.0);
    // `if` and `report … by` split the same way as a variable does.
    let by_event = output(&format!("{TWO}report x by x > 0 or y > 0"));
    let by_variable = output(&format!(
        "{TWO}let g = if x > 0 or y > 0 {{ true }} else {{ false }}\nreport x by g"
    ));
    // The same groups, under a different heading.
    let rows = |s: &str| s.lines().skip(4).map(|l| l.trim().to_string()).collect::<Vec<_>>();
    assert_eq!(rows(&by_event), rows(&by_variable));
    let o = outcome(&format!("{TWO}if x > 0 and y > 0 {{ report 1 }}"));
    close(o.results[0].reach.as_ref().unwrap().share, 0.25);
}

#[test]
fn sums_of_normal_draws_are_normal() {
    let o = outcome(
        "let x ~ normal(1, 2)\nlet y ~ normal(-1, 0.5)\n\
         report x + y\nreport x - 2 * y + 3\nreport (x + y) - y\nreport x + y > 1\n\
         report abs(x - y)\nreport exp(x + y)",
    );
    let spread = 4.25f64.sqrt();
    is_normal(&o, 0, 0.0, spread);
    is_normal(&o, 1, 6.0, 5f64.sqrt());
    is_normal(&o, 2, 1.0, 2.0);
    chance_of(&o, 3, 1.0 - phi(1.0 / spread));
    // A folded normal, and a lognormal.
    let m: f64 = 2.0;
    let folded = spread * half_normal_mean() * (-m * m / (2.0 * 4.25)).exp() + m * (1.0 - 2.0 * phi(-m / spread));
    close(marginal(&o, 4).mean(), folded);
    close(marginal(&o, 5).mean(), (4.25f64 / 2.0).exp());
    // What's left of `(x + y) - y` is x itself, which a condition restricts.
    let o = outcome(&format!("{TWO}let d = (x + y) - y\nobserve d > 0\nreport x\nreport y"));
    close(marginal(&o, 0).mean(), half_normal_mean());
    is_normal(&o, 1, 0.0, 1.0);
}

fn half_normal_mean() -> f64 {
    (2.0 / std::f64::consts::PI).sqrt()
}

#[test]
fn hierarchical_draws_carry_their_covariance() {
    // Var(y) = 2 and Cov(mu, y) = 1.
    let o =
        outcome("let mu ~ normal(0, 1)\nlet y ~ normal(mu, 1)\nreport y\nreport y - mu\nreport y + mu\nreport y > mu");
    is_normal(&o, 0, 0.0, 2f64.sqrt());
    is_normal(&o, 1, 0.0, 1.0);
    is_normal(&o, 2, 0.0, 5f64.sqrt());
    chance_of(&o, 3, 0.5);
    let o = outcome(
        "let a ~ normal(1, 1)\nlet b ~ normal(2, 1)\nlet y ~ normal(a + 2 * b, 0.5)\nreport y\nreport y - a - 2 * b",
    );
    is_normal(&o, 0, 5.0, 5.25f64.sqrt());
    is_normal(&o, 1, 0.0, 0.5);
    // Each call draws afresh.
    let src = "fn f(m) {\n let y ~ normal(m, 1)\n return y\n}\nlet mu ~ normal(0, 1)\nlet a = f(mu)\nlet b = f(mu)\nreport a - b\nreport a - mu";
    for memoize in [true, false] {
        let o = exec(
            src,
            &Options {
                memoize,
                ..Options::default()
            },
        )
        .unwrap();
        is_normal(&o, 0, 0.0, 2f64.sqrt());
        is_normal(&o, 1, 0.0, 1.0);
    }
}

#[test]
fn normal_observations_of_sums_update_jointly() {
    // mu ~ N(0, 1), y ~ N(mu, 1), and 1.5 ~ N(y, 0.5): 1.5 has variance
    // 2.25, and covariances 1 with mu and 2 with y.
    let o = outcome(
        "let mu ~ normal(0, 1)\nlet y ~ normal(mu, 1)\nobserve 1.5 from normal(y, 0.5)\nreport mu\nreport y\nreport y - mu",
    );
    close(o.evidence.unwrap().to_f64(), normal(0.0, 1.5).pdf(1.5));
    assert!(o.densities);
    is_normal(&o, 0, 1.5 / 2.25, (1.0 - 1.0 / 2.25f64).sqrt());
    is_normal(&o, 1, 2.0 * 1.5 / 2.25, (2.0 - 4.0 / 2.25f64).sqrt());
    is_normal(&o, 2, 1.5 / 2.25, (1.0 - 1.0 / 2.25f64).sqrt());
}

/// A straight line through three points, with priors normal(0, 10) on its
/// intercept and slope, and noise 0.5: the posterior and the evidence as a
/// whole, against observing the points one by one, in either order.
#[test]
fn a_regression_agrees_with_the_batch_formulas() {
    let points = [(1.0, 1.1), (2.0, 2.9), (3.0, 5.2)];
    let (prior, noise) = (100.0, 0.25);
    // Posterior precision 1/prior + XᵀX/noise, and mean P⁻¹ Xᵀy / noise.
    let (mut p, mut xy) = ([[1.0 / prior, 0.0], [0.0, 1.0 / prior]], [0.0, 0.0]);
    for (t, v) in points {
        let row = [1.0, t];
        for i in 0..2 {
            xy[i] += row[i] * v / noise;
            for j in 0..2 {
                p[i][j] += row[i] * row[j] / noise;
            }
        }
    }
    let det = p[0][0] * p[1][1] - p[0][1] * p[1][0];
    let cov = [[p[1][1] / det, -p[0][1] / det], [-p[1][0] / det, p[0][0] / det]];
    let mean = [
        cov[0][0] * xy[0] + cov[0][1] * xy[1],
        cov[1][0] * xy[0] + cov[1][1] * xy[1],
    ];
    // The points together are normal with covariance prior X Xᵀ + noise I.
    let mut k = [[0.0; 3]; 3];
    for (i, (ti, _)) in points.iter().enumerate() {
        for (j, (tj, _)) in points.iter().enumerate() {
            k[i][j] = prior * (1.0 + ti * tj) + if i == j { noise } else { 0.0 };
        }
    }
    let ln_evidence = ln_normal_density(&points.map(|(_, v)| v), k);

    let observe = |order: &[usize]| {
        let lines: String = order
            .iter()
            .map(|&i| format!("observe {} from normal(a + b * {}, 0.5)\n", points[i].1, points[i].0))
            .collect();
        outcome(&format!(
            "let a ~ normal(0, 10)\nlet b ~ normal(0, 10)\n{lines}report a\nreport b\nreport a + 4 * b"
        ))
    };
    for o in [observe(&[0, 1, 2]), observe(&[2, 0, 1])] {
        is_normal(&o, 0, mean[0], cov[0][0].sqrt());
        is_normal(&o, 1, mean[1], cov[1][1].sqrt());
        let var = cov[0][0] + 16.0 * cov[1][1] + 8.0 * cov[0][1];
        is_normal(&o, 2, mean[0] + 4.0 * mean[1], var.sqrt());
        close(o.evidence.unwrap().ln(), ln_evidence);
    }
}

/// The logarithm of the density of `y` under a normal with mean 0 and
/// covariance `k`, by Gaussian elimination.
fn ln_normal_density(y: &[f64; 3], mut k: [[f64; 3]; 3]) -> f64 {
    let mut b = *y;
    let mut ln_det = 0.0;
    for i in 0..3 {
        ln_det += k[i][i].ln();
        for r in i + 1..3 {
            let f = k[r][i] / k[i][i];
            let pivot = k[i];
            for (x, p) in k[r].iter_mut().zip(pivot).skip(i) {
                *x -= f * p;
            }
            b[r] -= f * b[i];
        }
    }
    let mut x = [0.0; 3];
    for i in (0..3).rev() {
        x[i] = (b[i] - (i + 1..3).map(|c| k[i][c] * x[c]).sum::<f64>()) / k[i][i];
    }
    let quad: f64 = y.iter().zip(&x).map(|(a, b)| a * b).sum();
    -0.5 * (3.0 * (2.0 * std::f64::consts::PI).ln() + ln_det + quad)
}

#[test]
fn an_affine_mean_of_one_draw_is_its_conjugate_update() {
    // 1 ~ N(2 mu, 1) says what 0.5 ~ N(mu, 0.5) does, with half the density,
    // restricted or not.
    for condition in ["", "observe mu > 0\n"] {
        let a = outcome(&format!(
            "let mu ~ normal(0, 1)\n{condition}observe 1 from normal(2 * mu, 1)\nreport mu"
        ));
        let b = outcome(&format!(
            "let mu ~ normal(0, 1)\n{condition}observe 0.5 from normal(mu, 0.5)\nreport mu"
        ));
        let (ma, mb) = (marginal(&a, 0), marginal(&b, 0));
        close(ma.mean(), mb.mean());
        close(ma.variance(), mb.variance());
        close(a.evidence.unwrap().ln(), b.evidence.unwrap().ln() - 2f64.ln());
    }
    // A sum's observation sees a draw's earlier conjugate update.
    let o = outcome(&format!("{TWO}let s = x + y\nobserve 1 from normal(x, 1)\nreport s"));
    is_normal(&o, 0, 0.5, 1.5f64.sqrt());
}

#[test]
fn an_event_made_before_an_observation_reads_the_posterior() {
    // After 1 ~ N(x + y, 1), x is N(1/3, √(2/3)).
    let o = outcome(&format!(
        "{TWO}let e = x > 0\nobserve 1 from normal(x + y, 1)\nreport e\nreport x"
    ));
    chance_of(&o, 0, phi((1.0 / 3.0) / (2.0f64 / 3.0).sqrt()));
    is_normal(&o, 1, 1.0 / 3.0, (2.0f64 / 3.0).sqrt());
}

#[test]
fn sampling_agrees() {
    let src = "let mu ~ normal(0, 1)\nlet y ~ normal(mu, 1)\nobserve 1.5 from normal(y, 0.5)\nlet z ~ normal(0, 1)\nlet w ~ normal(0, 1)\nobserve z > 0 or w > 1\nreport mu\nreport y - mu\nreport z";
    let exact = outcome(src);
    let sampled = exec(
        src,
        &Options {
            mode: Some(Mode::Sample { runs: 100_000, seed: 3 }),
            ..Options::default()
        },
    )
    .unwrap();
    for i in 0..3 {
        let (a, b) = (exact.reports[i].distribution(), sampled.reports[i].distribution());
        let mean = |d: &[(probl_engine::value::Value, f64)]| match analytic_mixture(d) {
            Some(m) => m.mean(),
            None => d.iter().map(|(v, p)| v.as_f64().unwrap() * p).sum(),
        };
        assert!(
            (mean(&a) - mean(&b)).abs() < 0.02,
            "report {i}: {} against {}",
            mean(&a),
            mean(&b)
        );
    }
}

#[test]
fn what_sums_and_events_dont_cover_is_unsupported() {
    for (tail, what) in [
        ("report x > 0 and x + y > 0", "depend on each other"),
        ("observe x + y > 0\nreport x", "several normal draws together"),
        ("if x - y > 0 { report 1 }", "several normal draws together"),
        ("report x by x + y > 0", "several normal draws together"),
        (
            "let e = x > 0 and y > 0\nobserve 1 from normal(x + y, 1)\nreport e",
            "depend on each other",
        ),
        ("report x * y", "different continuous draws"),
        ("observe x > 0\nreport x + y", "different continuous draws"),
        ("let z ~ uniform(0, 1)\nreport x + z", "different continuous draws"),
        ("let z ~ normal(abs(x), 1)\nreport z", "whose mean is this outcome"),
        (
            "observe 1 from normal(x * x, 1)\nreport x",
            "whose mean is this outcome",
        ),
        (
            "fn see(v) { observe v from normal(x + y, 1) }\nsee(1)\nreport x",
            "inside a function",
        ),
        (
            "let k = round(clamp(x + y, 0, 2))\nreport k",
            "rounded outcome of several",
        ),
    ] {
        let e = error(&format!("{TWO}{tail}"));
        assert!(e.contains(what), "{tail}: {e}");
    }
    // Reported rather than assigned, a rounded sum has its values.
    let d = distribution(&format!("{TWO}report round(clamp(x + y, 0, 2))"));
    let spread = 2f64.sqrt();
    let p0 = phi(0.5 / spread);
    let p2 = 1.0 - phi(1.5 / spread);
    close(d[0].1, p0);
    close(d[2].1, p2);
}
