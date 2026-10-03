//! The engine against the oracle (audit finding I4). For generated programs
//! and a corpus of hand-written ones, both must give the same evidence and
//! the same measure at every report, or both must fail.
//!
//! `PROBL_ORACLE_CASES` sets how many programs to generate (default 400) and
//! `PROBL_ORACLE_SEED` the first seed (default 1). A failure prints the seed
//! and the program; `PROBL_ORACLE_SEED=<seed> PROBL_ORACLE_CASES=1` repeats it.

use probl_engine::value::Value;
use probl_engine::{ErrorKind, Limits, Options, Outcome, Weight};
use probl_oracle::{Measure, Stop, generate};
use probl_sema::ir::Program;
use probl_syntax::{SourceFile, render_all};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Verdict {
    /// Both ran, and their measures agree.
    Agree,
    /// Both rejected the program at run time (with the oracle's reason).
    BothFailed(String),
    /// One of them ran out of its limits.
    Skipped,
}

/// The engine's configurations: default, and with merging or memoization
/// turned off, which must not change any result.
const CONFIGS: [(bool, bool); 4] = [(true, true), (false, true), (true, false), (false, false)];

fn options(merge: bool, memoize: bool) -> Options {
    Options {
        merge,
        memoize,
        // No loop is cut short: the oracle is exact, and the programs'
        // loops all end.
        epsilon: Some(0.0),
        // Enough for any program the oracle can follow; more would only make
        // slow programs slower to give up on.
        limits: Limits {
            max_worlds: 200_000,
            max_outcomes: 100_000,
            max_collection: 100_000,
            max_work: 2_000_000,
            max_iterations: 10_000,
            max_call_depth: 60,
            ..Limits::default()
        },
        ..Options::default()
    }
}

/// Run `src` through the oracle and the engine; `Err` describes how they
/// disagree.
fn check(src: &str, (merge, memoize): (bool, bool)) -> Result<Verdict, String> {
    let file = SourceFile::new("test.probl", src);
    let (program, diags) = probl_sema::compile(src);
    let Some(program) = program else {
        return Err(format!("it doesn't compile:\n{}", render_all(&diags, &file, false)));
    };
    let limits = probl_oracle::Limits {
        max_steps: 500_000,
        ..probl_oracle::Limits::default()
    };
    let oracle = probl_oracle::run_source(src, &limits);
    let mut print = |_: &str| {};
    let engine = probl_engine::run(&program, &options(merge, memoize), &mut print);
    let describe = |e: &probl_engine::RuntimeError| e.to_diagnostic().render(&file, false);
    match (oracle, engine) {
        (Err(Stop::Unsupported(what)), _) => Err(format!("the oracle doesn't support {what}")),
        (_, Err(e)) if matches!(e.kind, ErrorKind::Internal | ErrorKind::Unsupported) => {
            Err(format!("the engine failed:\n{}", describe(&e)))
        }
        (Err(Stop::TooBig(_)), _) => Ok(Verdict::Skipped),
        (_, Err(e)) if e.kind == ErrorKind::Limit => Ok(Verdict::Skipped),
        (Err(Stop::Error(why)), Err(e)) => {
            // Which error comes first can depend on the order worlds run in,
            // except for impossible evidence: it's only found once every
            // world has finished without an error.
            let impossible = |m: &str| m.contains("evidence is impossible");
            if impossible(&why) != impossible(&e.message) {
                return Err(format!(
                    "the oracle fails with \"{why}\", the engine with:\n{}",
                    describe(&e)
                ));
            }
            Ok(Verdict::BothFailed(why))
        }
        (Err(Stop::Error(why)), Ok(_)) => Err(format!("the oracle fails ({why}), the engine doesn't")),
        (Ok(_), Err(e)) => Err(format!("the engine fails, the oracle doesn't:\n{}", describe(&e))),
        (Ok(measure), Ok(outcome)) => compare(&program, &measure, &outcome).map(|()| Verdict::Agree),
    }
}

/// Weights relative to the evidence, per report, per key, per value.
type Table = BTreeMap<(u32, u32), BTreeMap<String, (f64, BTreeMap<String, f64>)>>;

fn compare(program: &Program, m: &Measure, out: &Outcome) -> Result<(), String> {
    if !out.unresolved.is_zero() {
        return Err(format!("the engine left {:e} unresolved", out.unresolved.to_f64()));
    }
    if m.observed != out.evidence.is_some() {
        return Err(format!(
            "the oracle says the program observed: {}; the engine: {}",
            m.observed,
            out.evidence.is_some()
        ));
    }
    let total = m.total.clone();
    let engine_z = out.evidence.unwrap_or(Weight::ONE);
    // Evidence can be far below what an f64 holds: compare logarithms.
    let (oracle_log, engine_log) = (log10(&total), engine_z.log10());
    if (oracle_log - engine_log).abs() > 1e-9 {
        return Err(format!("evidence: oracle 10^{oracle_log}, engine 10^{engine_log}"));
    }

    let expected: Table = m
        .reports
        .iter()
        .map(|(site, groups)| {
            let groups = groups
                .iter()
                .map(|(key, g)| {
                    let values = g.values.iter().map(|(v, w)| (v.clone(), f(&(w / &total)))).collect();
                    (key.clone(), (f(&(&g.total / &total)), values))
                })
                .collect();
            (*site, groups)
        })
        .collect();

    let mut actual: Table = BTreeMap::new();
    for (site, sink) in program.reports.iter().zip(&out.reports) {
        let mut groups = BTreeMap::new();
        for (key, acc) in &sink.groups {
            if !acc.missing.is_zero() {
                return Err(format!("the report on {:?} has missing mass", site.span));
            }
            let mut values: BTreeMap<String, f64> = BTreeMap::new();
            for (v, w) in &acc.values {
                *values.entry(canon(v)).or_default() += w.ratio(engine_z);
            }
            if !acc.facts.is_zero() {
                values.insert("true".into(), acc.yes.ratio(engine_z));
                values.insert("false".into(), acc.facts.saturating_sub(acc.yes).ratio(engine_z));
            }
            values.retain(|_, w| *w > 0.0);
            groups.insert(canon(key), (acc.total.ratio(engine_z), values));
        }
        if !groups.is_empty() {
            actual.insert((site.span.lo, site.span.hi), groups);
        }
    }

    let sites: BTreeSet<_> = expected.keys().chain(actual.keys()).collect();
    let none = BTreeMap::new();
    for site in sites {
        let (e, a) = (expected.get(site).unwrap_or(&none), actual.get(site).unwrap_or(&none));
        let keys: BTreeSet<_> = e.keys().chain(a.keys()).collect();
        for key in keys {
            let zero = (0.0, BTreeMap::new());
            let ((et, ev), (at, av)) = (e.get(key).unwrap_or(&zero), a.get(key).unwrap_or(&zero));
            let at_report = format!("report at {}..{}, key {key}", site.0, site.1);
            if !close(*et, *at) {
                return Err(format!("{at_report}: reached by {et:e} (oracle), {at:e} (engine)"));
            }
            let values: BTreeSet<_> = ev.keys().chain(av.keys()).collect();
            for v in values {
                let (x, y) = (ev.get(v).copied().unwrap_or(0.0), av.get(v).copied().unwrap_or(0.0));
                if !close(x, y) {
                    return Err(format!("{at_report}: value {v} has {x:e} (oracle), {y:e} (engine)"));
                }
            }
        }
    }
    Ok(())
}

fn f(q: &probl_oracle::Q) -> f64 {
    num_traits::ToPrimitive::to_f64(q).unwrap_or(f64::NAN)
}

/// The base-10 logarithm of a positive fraction, however small.
fn log10(q: &probl_oracle::Q) -> f64 {
    fn log10_int(n: &num_bigint::BigInt) -> f64 {
        let shift = n.bits().saturating_sub(64);
        let top = num_traits::ToPrimitive::to_f64(&(n >> shift)).unwrap();
        top.log10() + shift as f64 * std::f64::consts::LOG10_2
    }
    log10_int(q.numer()) - log10_int(q.denom())
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs()))
}

/// The engine's value in the oracle's notation (`probl_oracle::canon`).
fn canon(v: &Value) -> String {
    match v {
        Value::Unit => "()".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(x) => format!("float {x:.9e}"),
        Value::Prob(x) => format!("prob {x:.9e}"),
        Value::Str(s) => format!("{:?}", &**s),
        Value::List(items) => format!("[{}]", items.iter().map(canon).collect::<Vec<_>>().join(", ")),
        Value::Range(lo, hi) => format!("{lo}..{hi}"),
        Value::Dist(d) => {
            let mut parts: Vec<String> = d
                .outcomes
                .iter()
                .map(|(x, p)| format!("{}: {p:.9e}", canon(x)))
                .collect();
            parts.sort();
            format!("dist({})", parts.join(", "))
        }
        other => format!("{other:?}"),
    }
}

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[test]
fn generated_programs_agree() {
    let cases = env("PROBL_ORACLE_CASES", 400);
    let first = env("PROBL_ORACLE_SEED", 1);
    let mut verdicts: BTreeMap<Verdict, u64> = BTreeMap::new();
    let mut failures = Vec::new();
    for seed in first..first + cases {
        let src = generate::program(seed);
        // Each program in one configuration; the seeds take turns.
        let config = CONFIGS[(seed % 4) as usize];
        match check(&src, config) {
            Ok(v) => *verdicts.entry(v).or_default() += 1,
            Err(why) => failures.push(format!(
                "seed {seed} (merge, memoize: {config:?}): {why}\n{}",
                numbered(&src)
            )),
        }
        if failures.len() >= 3 {
            break;
        }
    }
    eprintln!("generated programs: {verdicts:#?}");
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
    // Most programs must be compared, not skipped or rejected (which says
    // something only about many programs).
    let agree = verdicts.get(&Verdict::Agree).copied().unwrap_or(0);
    assert!(
        cases < 100 || agree * 10 >= cases * 6,
        "only {agree} of {cases} programs were compared: {verdicts:#?}"
    );
}

fn numbered(src: &str) -> String {
    src.lines()
        .enumerate()
        .map(|(i, l)| format!("{:>3} | {l}", i + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Hand-written programs for each kind of case the audit lists, with the
/// probes it confirmed. Each runs in every configuration of the engine.
const CORPUS: &[(&str, &str)] = &[
    (
        "score requires a scalar probability",
        "score simulate { prob(30%) }; report true",
    ),
    (
        "probabilistic conditions",
        "let a = if 30% { true } else { false }\nlet b = if d6 > 3 { true } else { false }\nreport a and b",
    ),
    (
        "probability guards",
        "report match 1 { _ if 30% => 1, _ if 50% => 2, _ => 3 }",
    ),
    (
        "prefix draw identity",
        "let p = prob(30%)\nlet a = ~p\nlet b = ~p\nreport a and a\nreport a and b",
    ),
    ("prefix draw order", "var x = 1\nlet y = x + ~{ x = 2; d2 }\nreport y"),
    (
        "bounded prefix draw loop",
        "var n = 0\nwhile n < 3 and ~d2 != 2 { n += 1 }\nreport n",
    ),
    (
        "independent boolean recipes",
        "let d = d6 > 3\nreport d and d\nreport d or d\nreport prob(30%) and d\nreport not prob(30%)",
    ),
    (
        "score conditional literals",
        "let sick = ~bernoulli(1%)\nscore if sick { 95% } else { 8% }\nreport sick",
    ),
    (
        "score match and chance",
        "score match 1 { 1 => chance { 50% => 20%, else => 40% }, _ => 50% }\nreport true",
    ),
    (
        "observe anonymous law",
        "let p = if 50% { prob(10%) } else { prob(90%) }\nobserve ~p\nreport p",
    ),
    ("score rejects numbers", "let p = 30%\nscore p\nreport true"),
    ("observe still needs a fact", "let p = prob(30%)\nobserve p\nreport p"),
    (
        "typeof scalars and recipes",
        "let d = d6\nlet x ~ d\nreport [typeof 33%, typeof 0.33, typeof (33% + 1%), typeof d, typeof x, typeof (d > d)]",
    ),
    (
        "typeof container shapes",
        "report [typeof [], typeof [d6, d8], typeof one_of([1, \"x\"]), typeof [[1], [2]]]",
    ),
    (
        "typeof evaluates its operand once",
        "var n = 0\nlet t = typeof { n += 1; d6 }\nreport t\nreport n",
    ),
    (
        "operands that assign variables",
        r#"
var x = 1
report x + { x = 2; 0 } as "a"
report [x, { x = 3; x }] as "b"
var y ~ d3
y += { y = 10; 1 }
report y as "c"
var z = 0
report { z += 1; z } + { z += 1; z } * { z = z * 10; z } as "d"
"#,
    ),
    (
        "assignments inside conditions and arms",
        r#"
var n = 0
var hits = 0
repeat 3 {
  if ({ n += 1; n > 1 }) and hits < 2 {
    hits += chance { 30% => { n += 1; 1 }, else => 0 }
  }
}
report [n, hits] as "state"
"#,
    ),
    (
        "calls with splits and evidence",
        r#"
let g ~ d4
fn roll(a) {
  let r ~ d3
  observe r != a
  r + g
}
fn twice(a) { roll(a) + roll(a) }
var total = 0
total += twice(1)
report total as "total"
report g as "g"
"#,
    ),
    (
        "calls read the caller's current top-level values",
        r#"
var level ~ d2
fn scaled(k) { level * k }
let before = scaled(10)
level += 5
let after = scaled(10)
report [before, after] as "values"
"#,
    ),
    (
        "extracting a function changes nothing",
        r#"
fn attack(bonus) {
  let hit ~ d20 + bonus >= 12
  if hit { d6 } else { 0 }
}
var hp = 8
repeat 3 { hp -= attack(2) }
var hp2 = 8
repeat 3 {
  let hit ~ d20 + 2 >= 12
  hp2 -= if hit { d6 } else { 0 }
}
report hp as "with a function"
report hp2 as "inline"
"#,
    ),
    (
        "aliases of values, events and recipes",
        r#"
let rain ~ bernoulli(30%)
let wet = rain
report rain and wet as "same event"
let die = d6
let also = die
report die + also as "two dice"
var xs = [1, 2, 3]
var ys = xs
ys[0] = 9
report [xs[0], ys[0]] as "copies"
fn change(list) {
  var l = list
  l[1] = 7
  l
}
let zs = change(xs)
report [xs[1], zs[1]] as "passed by value"
"#,
    ),
    (
        "repeated observations",
        r#"
let coin ~ one_of([1, 2])
repeat 5 {
  observe true from bernoulli(prob(if coin == 1 { 50% } else { 90% }))
}
let flip ~ d2
observe flip from d2
observe flip == 2 or coin == 2
report coin as "coin"
report flip as "flip"
"#,
    ),
    (
        "many observations",
        r#"
repeat 400 { observe true from bernoulli(10%) }
var n = 0
repeat 8 { n += if (chance { 50% => true, else => false }) { 1 } else { 0 } }
observe n > 2
report n as "n"
"#,
    ),
    ("impossible evidence", "let x ~ d6\nobserve x > 6\nreport x\n"),
    (
        "impossible evidence in a call",
        "fn f() { observe false; 1 }\nlet x = f()\nreport x\n",
    ),
    (
        "evidence ruling out one branch",
        "let x ~ d6\nif x > 3 { observe false }\nreport x\n",
    ),
    (
        "loops that end",
        r#"
var steps = 0
var pos = 0
for i in 1..4 {
  if (chance { 30% => true, else => false }) { break }
  steps += 1
  if i == 2 { continue }
  pos += if (chance { 50% => true, else => false }) { i } else { 0 }
}
var c = 0
while c < 3 {
  c += 1
  if pos > 3 and 50% > 0 { continue }
  pos -= 1
}
loop {
  if c >= 5 { break }
  c += 1
}
report steps as "steps"
report pos as "pos"
report c as "c"
"#,
    ),
    (
        "reports in loops",
        r#"
var sum = 0
for month in 1..3 {
  sum += d2
  report sum by month as "per month"
}
repeat 2 {
  report sum by 0 as "per visit"
}
if (chance { 25% => true, else => false }) {
  report sum as "sometimes"
}
"#,
    ),
    (
        "distributions with one outcome",
        r#"
report d1 + 1 as "d1"
let s ~ simulate { 3 }
report s as "simulate"
report bernoulli(0%) as "never"
report bernoulli(100%) as "always"
let one = one_of([5])
report one * 2 as "one_of"
let p = P(d1 == 1)
report p as "P"
"#,
    ),
    (
        "simulate keeps its evidence to itself",
        r#"
let d = simulate {
  let s ~ d6
  observe s > 4
  s
}
let x ~ d
observe true from bernoulli(prob(if x == 6 { true } else { 50% }))
report x as "x"
report simulate { let t ~ d2; observe t == 2; t } as "after reports"
"#,
    ),
    (
        "a shared rate is not averaged away",
        r#"
let p ~ simulate { if (chance { 50% => true, else => false }) { 10% } else { 90% } }
let a ~ bernoulli(prob(p))
let b ~ bernoulli(prob(p))
report a and b as "both"
"#,
    ),
    (
        "the audit's posterior",
        r#"
var win = false
if (chance { 0.5% => true, else => false }) { repeat 1 { win = true } }
observe true from bernoulli(prob(if win { true } else { 0.00001 }))
report win as "win"
"#,
    ),
    (
        "match on drawn values, with guards",
        r#"
let x ~ d6
let label = match x {
  1 | 2 => 10
  n if n > 4 => n * 100
  n if 50% => n
  _ => 0
}
report label as "label"
"#,
    ),
    (
        "match on a distribution is an error",
        "let x = d6\nreport match x { 1 => 1, _ => 2 }\n",
    ),
    (
        "logic on probabilities is an error",
        "let p = 30%\nreport p and not p\n",
    ),
    ("two uncertain facts are an error", "report (d6 > 3) and (d6 > 3)\n"),
    ("drawing a probability is an error", "let x ~ 30%\nreport x\n"),
    (
        "chance weights over 100% are an error",
        "let x = chance { 60% => 1, 50% => 2 }\nreport x\n",
    ),
    (
        "a chance value needs all its weight",
        "let x = chance { 60% => 1 }\nreport x\n",
    ),
    (
        "a chance statement lets the rest through",
        "var x = 0\nchance { 60% => x = 1 }\nreport x\n",
    ),
    (
        "simulate with no evidence left is an error",
        "let d = simulate { let s ~ d2; observe s > 2; s }\nreport d\n",
    ),
    (
        "one_of mixes in distributions",
        "let x ~ one_of([d2, 10])\nreport x as \"x\"\nreport x == x as \"settled\"\nreport one_of([d2, 10]) as \"recipe\"\n",
    ),
    (
        "short circuits",
        r#"
var calls = 0
fn check(v) { v > 2 }
let a ~ d4
let b = a > 3 and { calls += 1; check(a) }
let c = a > 3 or { calls += 1; true }
report [b, c, calls] as "state"
"#,
    ),
];

#[test]
fn corpus_agrees() {
    let mut failures = Vec::new();
    let mut verdicts: BTreeMap<Verdict, u64> = BTreeMap::new();
    for (name, src) in CORPUS {
        for config in CONFIGS {
            match check(src, config) {
                Ok(v) => *verdicts.entry(v).or_default() += 1,
                Err(why) => failures.push(format!("{name} (merge, memoize: {config:?}): {why}\n{}", numbered(src))),
            }
        }
    }
    eprintln!("corpus: {verdicts:#?}");
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
    assert_eq!(
        verdicts.get(&Verdict::Skipped),
        None,
        "no corpus program may be skipped"
    );
}
