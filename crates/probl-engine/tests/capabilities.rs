//! What enumeration and sampling can run, as an executable inventory
//! (docs/design/enumeration-support.md). Each program names the inventory
//! item that would change it, or is a control: one that works in both
//! modes, or one the language rejects on purpose.
//!
//! Building a capability changes some expectations from `Unsupported` to
//! `Works`: update them here, with the tests the capability itself needs.
//! `PROBL_CAPABILITIES=print cargo test -p probl-engine --test capabilities`
//! prints what every program does now.

use probl_engine::{ErrorKind, FailureMode, Limits, Options, RuntimeError, run};
use probl_sema::ir::Mode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    Works,
    /// A capability that isn't built: an `Unsupported` error.
    Unsupported,
    /// A resource limit, at the corpus's small limits.
    Limit,
    /// A rule of the language, or a fault in the model: a `Language` error.
    Rejected,
}

use Expect::*;

struct Case {
    /// The inventory item, or `control` or `invalid`.
    item: &'static str,
    program: &'static str,
    enumerate: Expect,
    sample: Expect,
}

const fn case(item: &'static str, program: &'static str, enumerate: Expect, sample: Expect) -> Case {
    Case {
        item,
        program,
        enumerate,
        sample,
    }
}

const CASES: &[Case] = &[
    // Controls that work in both modes.
    case(
        "control",
        "let x ~ uniform(0, 2)\nobserve x > 1\nreport x + 1",
        Works,
        Works,
    ),
    case(
        "control",
        "let x ~ uniform(-1, 1)\nreport if x < 0 { -x } else { x }",
        Works,
        Works,
    ),
    case("control", "let x ~ uniform(0, 2)\nreport [x][0]", Works, Works),
    case(
        "control",
        "let x ~ uniform(0, 2)\nreport [x, x + 1].reduce((a, b) -> a + b, 0)",
        Works,
        Works,
    ),
    case("control", "let x ~ uniform(0, 2)\nreport len([x, x])", Works, Works),
    case(
        "control",
        "let x ~ normal(0, 1)\nlet y ~ normal(0, 1)\nreport if x > 0 { if y > 0 { true } else { false } } else { false }",
        Works,
        Works,
    ),
    case(
        "control",
        "let x ~ uniform(0, 2)\nlet group = if x > 1 { true } else { false }\nreport x by group",
        Works,
        Works,
    ),
    case(
        "control",
        "let k ~ one_of([0, 1])\nlet c ~ if k == 0 { normal(0, 1) } else { uniform(0, 1) }\nreport c",
        Works,
        Works,
    ),
    case(
        "control",
        "fn f(x) { return 2 * x + 1 }\nlet x ~ normal(0, 1)\nreport f(x) > 1",
        Works,
        Works,
    ),
    case("control", "let x ~ uniform(0, 1)\nreport typeof x", Works, Works),
    case("control", "let n ~ poisson(3)\nreport n", Works, Works),
    case(
        "control",
        "var n = 0\nwhile ~d6 != 6 { n += 1 }\nreport n",
        Works,
        Works,
    ),
    // Programs the language rejects in both modes, on purpose.
    case("invalid", "let x ~ uniform(0, 2)\nreport mean(x)", Rejected, Rejected),
    case("invalid", "report support(normal(0, 1))", Rejected, Rejected),
    case("invalid", "report [1, 2].map(v -> ~d6)", Rejected, Rejected),
    // A: density observations over finite worlds.
    case(
        "A",
        "let mu ~ one_of([0, 1])\nobserve 0 from normal(mu, 1)\nreport mu == 0",
        Works,
        Works,
    ),
    case("A", "observe 0 from normal(0, 0.1)\nreport true", Works, Works),
    // B: collection built-ins on analytic values.
    case("B", "let x ~ uniform(0, 2)\nreport [x].get(0)", Works, Works),
    case("B", "let x ~ uniform(0, 2)\nreport sum([x, x + 1])", Works, Works),
    case("B", "let x ~ uniform(0, 2)\nreport mean([x, x + 1])", Works, Works),
    case("B", "let x ~ uniform(0, 2)\nreport reverse([x, 1])[0]", Works, Works),
    // C: piecewise affine math.
    case("C", "let x ~ uniform(-1, 1)\nreport abs(x)", Works, Works),
    case("C", "let x ~ uniform(0, 2)\nreport max(x, 0.5)", Works, Works),
    case("C", "let x ~ uniform(0, 2)\nreport min(x, 1)", Works, Works),
    case("C", "let x ~ uniform(-1, 2)\nreport clamp(x, 0, 1)", Works, Works),
    // D: rounding into finite bins.
    case("D", "let x ~ uniform(0, 3)\nreport floor(x)", Works, Works),
    case("D", "let x ~ uniform(0, 3)\nreport round(x)", Works, Works),
    // Not yet: infinitely many bins, and decimal places.
    case("D", "let x ~ normal(0, 1)\nreport floor(x)", Unsupported, Works),
    case("D", "let x ~ uniform(0, 3)\nreport round(x, 1)", Unsupported, Works),
    // E: nonlinear transforms of one latent.
    case("E", "let x ~ uniform(-1, 1)\nreport x * x", Works, Works),
    case("E", "let x ~ normal(0, 1)\nreport exp(x)", Works, Works),
    case("E", "let x ~ uniform(1, 2)\nreport sqrt(x)", Works, Works),
    // Not yet: trigonometry, and a cube.
    case("E", "let x ~ uniform(0, 1)\nreport sin(x)", Unsupported, Works),
    case("E", "let x ~ uniform(0, 1)\nreport x * x * x", Unsupported, Works),
    // F: Boolean combinations of independent analytic events.
    case(
        "F",
        "let x ~ normal(0, 1)\nlet y ~ normal(0, 1)\nreport x > 0 and y > 0",
        Unsupported,
        Works,
    ),
    // G: conjugate updates in enumeration.
    case(
        "G",
        "let p ~ beta(2, 3)\nobserve 3 from binomial(5, p)\nreport p",
        Works,
        Works,
    ),
    case(
        "G",
        "let r ~ gamma(2, 1)\nobserve 3 from poisson(r)\nreport r",
        Works,
        Works,
    ),
    // H: a continuous draw as a probability.
    case(
        "H",
        "let p ~ beta(2, 3)\nlet hit = if p { true } else { false }\nreport hit",
        Works,
        Works,
    ),
    case("H", "let p ~ beta(2, 3)\nlet b ~ bernoulli(p)\nreport b", Works, Works),
    case("H", "let p ~ uniform(0, 1)\nscore p\nreport p", Works, Works),
    // Not yet: an affine mean in a conjugate likelihood, and a drawn weight
    // in `chance`.
    case(
        "G",
        "let mu ~ normal(0, 1)\nobserve 1 from normal(2 * mu, 1)\nreport mu",
        Unsupported,
        Works,
    ),
    case(
        "H",
        "let p ~ beta(2, 3)\nlet c = chance { p => 1, else => 0 }\nreport c",
        Unsupported,
        Works,
    ),
    // I: joint continuous values.
    case(
        "I",
        "let x ~ normal(0, 1)\nlet y ~ normal(0, 1)\nreport x + y",
        Unsupported,
        Works,
    ),
    case(
        "I",
        "let mu ~ normal(0, 1)\nlet y ~ normal(mu, 1)\nreport y",
        Unsupported,
        Works,
    ),
    // K: continuous `simulate`.
    case(
        "K",
        "let d = simulate { let x ~ normal(0, 1)\n x > 0 }\nreport d",
        Unsupported,
        Unsupported,
    ),
    case(
        "K",
        "let x ~ uniform(0, 2)\nlet d = simulate { x + 1 }\nreport d",
        Unsupported,
        Works,
    ),
    // L: aggregates containing analytic values.
    case(
        "L",
        "let x ~ uniform(0, 2)\nreport { a: x, b: x + 1 }",
        Unsupported,
        Works,
    ),
    // M: collection operations that compare analytic values.
    case("M", "let x ~ uniform(0, 2)\nreport sort([x, 1])[0]", Unsupported, Works),
    case(
        "M",
        "let x ~ uniform(0, 2)\nreport [x, 1] == [1, 1]",
        Unsupported,
        Works,
    ),
    // N: grouping and text.
    case("N", "let x ~ uniform(0, 2)\nreport x by x > 1", Works, Works),
    case("N", "let x ~ uniform(0, 2)\nreport str(x)", Unsupported, Works),
    // O: broad count supports and queries about count laws.
    case("O", "let n ~ geometric(0.000000000001)\nreport n", Limit, Works),
    case(
        "O",
        "let d = geometric(0.000000000001)\nlet n ~ d\nreport n",
        Limit,
        Works,
    ),
    case("O", "report pmf(geometric(0.5), 1)", Works, Works),
    case("O", "report cdf(poisson(3), 2)", Works, Works),
    // Not yet: the CDF of a law too broad to list, other than a geometric.
    case("O", "report median(poisson(1000000000000))", Unsupported, Unsupported),
    // P: recursion with output.
    case(
        "P",
        "fn f() {\n print(1)\n if 50% { 1 } else { f() }\n}\nreport f()",
        Rejected,
        Works,
    ),
    // R: faults on part of an analytic domain. Sampling meets the fault.
    case("R", "let x ~ uniform(-1, 1)\nreport sqrt(x)", Unsupported, Rejected),
    case(
        "R",
        "let x ~ uniform(-1, 1)\nreport try { sqrt(x) } catch DomainError { 0 }",
        Unsupported,
        Works,
    ),
    case("N", "let x ~ uniform(0, 2)\nreport true by x", Unsupported, Works),
    case("N", "let x ~ uniform(0, 2)\nprint(x)\nreport true", Unsupported, Works),
    // Recipes that aren't drawn, and built-ins that aren't built: not
    // sampling features either.
    case("recipes", "report normal(0, 1) * 2", Unsupported, Unsupported),
    case(
        "recipes",
        "report mixture([normal(0, 1), normal(5, 1)])",
        Unsupported,
        Unsupported,
    ),
    case(
        "recipes",
        "report truncate(normal(0, 1), 0, 1)",
        Unsupported,
        Unsupported,
    ),
    case("recipes", "report bins(normal(0, 1), [0, 1])", Unsupported, Unsupported),
];

fn options(mode: Mode) -> Options {
    Options {
        mode: Some(mode),
        on_error: Some(FailureMode::Total),
        limits: Limits {
            max_work: 2_000_000,
            ..Limits::default()
        },
        ..Options::default()
    }
}

/// What the program does in this mode, and its error if any.
fn outcome(program: &str, mode: Mode) -> (Expect, Option<RuntimeError>) {
    let (compiled, diagnostics) = probl_sema::compile(program);
    let compiled = compiled.unwrap_or_else(|| panic!("{program}\ndoesn't compile: {diagnostics:?}"));
    match run(&compiled, &options(mode), &mut |_| {}) {
        Ok(_) => (Works, None),
        Err(e) => {
            let got = match e.kind {
                ErrorKind::Unsupported => Unsupported,
                ErrorKind::Limit => Limit,
                ErrorKind::Language => Rejected,
                ErrorKind::Internal => panic!("{program}\nis an internal error: {e:?}"),
            };
            (got, Some(e))
        }
    }
}

/// Whether the error tells the user to sample: in its message or its help.
/// Notes give context, such as `simulate` enumerating "even in sample mode".
fn suggests_sampling(e: &RuntimeError) -> bool {
    let advice = format!("{} {}", e.message, e.help.as_deref().unwrap_or(""));
    ["@mode sample", "--mode sample", "sample mode", "sample the model"]
        .iter()
        .any(|s| advice.contains(s))
}

#[test]
fn the_inventory_runs_as_listed() {
    let print = std::env::var("PROBL_CAPABILITIES").is_ok_and(|v| v == "print");
    let mut wrong = Vec::new();
    for c in CASES {
        let (enumerated, enumeration_error) = outcome(c.program, Mode::Enumerate);
        let (sampled, _) = outcome(c.program, Mode::Sample { runs: 200, seed: 7 });
        let program = c.program.replace('\n', "; ");
        if print {
            let why = enumeration_error.as_ref().map(|e| e.message.as_str()).unwrap_or("");
            println!("{:8} {enumerated:12?} {sampled:12?} {program}  [{why}]", c.item);
        }
        if (enumerated, sampled) != (c.enumerate, c.sample) {
            wrong.push(format!(
                "{} `{program}`: expected {:?}/{:?}, got {enumerated:?}/{sampled:?}",
                c.item, c.enumerate, c.sample
            ));
        }
        // Advice to sample only where sampling runs the model.
        if let Some(e) = enumeration_error.filter(suggests_sampling) {
            if matches!(sampled, Unsupported | Limit) {
                wrong.push(format!(
                    "{} `{program}`: suggests sampling, which fails too: {e:?}",
                    c.item
                ));
            }
        }
        match c.item {
            "control" if (c.enumerate, c.sample) != (Works, Works) => {
                wrong.push(format!("control `{program}` must work in both modes"));
            }
            "invalid" if (c.enumerate, c.sample) != (Rejected, Rejected) => {
                wrong.push(format!("invalid `{program}` must be rejected in both modes"));
            }
            _ => {}
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}
