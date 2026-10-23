//! Reproducible candidate evaluation, not a report collection policy.
//! cargo run --release -p probl-bench --example quantile-sketches -- --n=100000 --seeds=32
//! Writes JSONL to stdout; see docs/quantile-sketches.md for interpretation.
#[path = "quantile_sketches/candidates.rs"]
mod candidates;
#[path = "quantile_sketches/fixtures.rs"]
mod fixtures;
#[path = "quantile_sketches/heap.rs"]
mod heap;

use candidates::{Kind, Sketch};
use serde_json::json;
use std::hint::black_box;
use std::time::{Duration, Instant};

#[global_allocator]
static ALLOCATOR: heap::Counting = heap::Counting;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    Stream,
    Batches,
}
impl Layout {
    fn name(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::Batches => "batches",
        }
    }
}

#[derive(Default)]
struct Timing {
    update: Duration,
    merge: Duration,
}

fn build(values: &[f64], kind: Kind, capacity: usize, seed: u64, layout: Layout, timed: bool) -> (Sketch, Timing) {
    let mut result = Sketch::new(kind, capacity, seed, u64::MAX);
    let mut timing = Timing::default();
    if layout == Layout::Stream {
        let start = timed.then(Instant::now);
        for (i, &value) in values.iter().enumerate() {
            result.add(value, i as u64);
        }
        if let Some(start) = start {
            timing.update = start.elapsed();
        }
    } else {
        // At most eight pending batch sketches, then combine in batch order.
        // This measures bounded overlapping storage, not engine worker overhead.
        for (wave, values) in values.chunks(8000).enumerate() {
            let start = timed.then(Instant::now);
            let mut pending = Vec::new();
            for (batch, values) in values.chunks(1000).enumerate() {
                let index = wave * 8 + batch;
                let mut part = Sketch::new(kind, capacity, seed, index as u64);
                for (i, &value) in values.iter().enumerate() {
                    part.add(value, (index * 1000 + i) as u64);
                }
                pending.push(part);
            }
            if let Some(start) = start {
                timing.update += start.elapsed();
            }
            let start = timed.then(Instant::now);
            for part in pending {
                result.merge(part);
            }
            if let Some(start) = start {
                timing.merge += start.elapsed();
            }
        }
    }
    (result, timing)
}

fn levels() -> Vec<f64> {
    let mut levels: Vec<_> = (0..=1000).map(|i| i as f64 / 1000.0).collect();
    levels.extend([0.0001, 0.9999]);
    levels.sort_unstable_by(f64::total_cmp);
    levels
}

fn fingerprint(values: impl IntoIterator<Item = f64>) -> u64 {
    values.into_iter().fold(0_u64, |h, v| candidates::mix(h ^ v.to_bits()))
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_unstable_by(f64::total_cmp);
    values[values.len() / 2]
}

const QUANTILES: &[f64] = &[
    0.0, 0.0001, 0.001, 0.01, 0.05, 0.25, 0.5, 0.75, 0.95, 0.99, 0.999, 0.9999, 1.0,
];

fn evaluate(name: &str, values: &[f64], sorted: &[f64], kind: Kind, capacity: usize, seeds: usize, layout: Layout) {
    let levels = levels();
    let zero_count = values.iter().filter(|&&v| v == 0.0).count() as u64;
    let negative_count = values.iter().filter(|&&v| v < 0.0).count() as u64;
    let exact: Vec<_> = QUANTILES.iter().map(|&q| fixtures::exact_quantile(sorted, q)).collect();
    let mut errors = Vec::new();
    let mut update_times = Vec::new();
    let mut merge_times = Vec::new();
    let mut query_times = Vec::new();
    let mut quantile_errors = vec![0.0_f64; QUANTILES.len()];
    let mut value_errors = vec![0.0_f64; QUANTILES.len()];
    let mut value_error_overflow = vec![false; QUANTILES.len()];
    let mut replay = Vec::new();
    let mut first_quantiles = Vec::new();
    let (mut retained_items, mut retained_heap, mut peak_heap) = (0, 0, 0);
    for seed in 0..seeds as u64 {
        let (sketch, timing) = build(values, kind, capacity, seed, layout, true);
        assert_eq!(
            (sketch.count, sketch.zeros, sketch.negative),
            (values.len() as u64, zero_count, negative_count)
        );
        retained_items = retained_items.max(sketch.retained());
        update_times.push(timing.update.as_secs_f64());
        merge_times.push(timing.merge.as_secs_f64());
        let start = Instant::now();
        let view = sketch.snapshot();
        let answers: Vec<_> = levels.iter().map(|&q| view.quantile(q).unwrap()).collect();
        query_times.push(start.elapsed().as_secs_f64());
        assert!(answers.windows(2).all(|w| w[0] <= w[1]));
        assert!(answers.iter().all(|v| v.is_finite()));
        errors.push(
            levels
                .iter()
                .zip(&answers)
                .map(|(&q, &v)| fixtures::rank_error(sorted, q, v))
                .fold(0.0, f64::max),
        );
        for (&q, &answer) in levels.iter().zip(&answers) {
            if fixtures::exact_quantile(sorted, q) == 0.0 {
                assert_eq!(answer, 0.0, "zero atom changed at q={q}");
            }
        }
        for (i, &q) in QUANTILES.iter().enumerate() {
            let answer = view.quantile(q).unwrap();
            if seed == 0 {
                first_quantiles.push(answer);
            }
            quantile_errors[i] = quantile_errors[i].max(fixtures::rank_error(sorted, q, answer));
            if exact[i] != 0.0 {
                let relative = (answer / exact[i] - 1.0).abs();
                if relative.is_finite() {
                    value_errors[i] = value_errors[i].max(relative);
                } else {
                    value_error_overflow[i] = true;
                }
            }
        }
        let signature = fingerprint(answers);
        replay.push(format!("{signature:016x}"));
        // Rebuild with allocation accounting, comparing every query bitwise.
        // No fixtures, exact sorted samples or JSON buffers are allocated here.
        let (retained, peak) = heap::measure(|| {
            let (sketch, _) = build(values, kind, capacity, seed, layout, false);
            let retained = heap::live();
            let view = sketch.snapshot();
            let repeated = fingerprint(levels.iter().map(|&q| view.quantile(q).unwrap()));
            assert_eq!(black_box(repeated), signature, "seeded replay changed");
            retained
        });
        retained_heap = retained_heap.max(retained);
        peak_heap = peak_heap.max(peak);
    }
    let quantiles: Vec<_> = QUANTILES
        .iter()
        .enumerate()
        .map(|(i, q)| {
            json!({
                "q": q,
                "exact": exact[i],
                "seed_0_estimate": first_quantiles[i],
                "max_rank_error": quantile_errors[i],
                "max_relative_value_error": (exact[i] != 0.0 && !value_error_overflow[i]).then_some(value_errors[i]),
                "relative_value_error_overflow": value_error_overflow[i],
            })
        })
        .collect();
    println!(
        "{}",
        json!({
            "version": 1,
            "fixture": name,
            "fixture_fingerprint": format!("{:016x}", fingerprint(values.iter().copied())),
            "n": values.len(),
            "zeros": zero_count,
            "negative": negative_count,
            "candidate": kind.name(),
            "capacity": capacity,
            "layout": layout.name(),
            "batch_size": 1000,
            "pending_batches": 8,
            "seeds": seeds,
            "query_count": levels.len(),
            "max_rank_error": errors.iter().copied().fold(0.0, f64::max),
            "max_rank_error_by_seed": errors,
            "max_retained_items": retained_items,
            "max_retained_heap_bytes": retained_heap,
            "max_peak_heap_bytes": peak_heap,
            "median_update_seconds": median(&mut update_times),
            "median_merge_seconds": median(&mut merge_times),
            "median_query_seconds": median(&mut query_times),
            "query_fingerprints": replay,
            "quantiles": quantiles,
        })
    );
}

fn main() {
    if let Err(error) = run() {
        eprintln!("quantile-sketches: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let mut n = 100_000;
    let mut seeds = 32;
    let mut sizes = vec![128, 256, 512];
    let mut names: Vec<_> = fixtures::NAMES.iter().map(|s| s.to_string()).collect();
    let mut layouts = vec![Layout::Stream, Layout::Batches];
    for arg in std::env::args().skip(1) {
        if arg == "--help" {
            println!(
                "quantile-sketches [--n=100000] [--seeds=32] [--sizes=128,256,512] [--fixtures=uniform,retirement_30] [--layouts=stream,batches]\nWrites one JSONL record per fixture/candidate/capacity/layout. See docs/quantile-sketches.md."
            );
            return Ok(());
        }
        let (key, value) = arg
            .split_once('=')
            .ok_or_else(|| format!("expected --key=value, got {arg}"))?;
        let number = || value.parse::<usize>().map_err(|_| format!("invalid {key}: {value}"));
        match key {
            "--n" => n = number()?,
            "--seeds" => seeds = number()?,
            "--sizes" => {
                sizes = value
                    .split(',')
                    .map(|v| v.parse().map_err(|_| format!("invalid capacity: {v}")))
                    .collect::<Result<_, _>>()?
            }
            "--fixtures" => names = value.split(',').map(str::to_string).collect(),
            "--layouts" => {
                layouts = value
                    .split(',')
                    .map(|v| match v {
                        "stream" => Ok(Layout::Stream),
                        "batches" => Ok(Layout::Batches),
                        _ => Err(format!("invalid layout: {v}")),
                    })
                    .collect::<Result<_, _>>()?
            }
            _ => return Err(format!("unknown option {key}")),
        }
    }
    if !(1..=10_000_000).contains(&n) || !(1..=1024).contains(&seeds) || sizes.iter().any(|s| !(8..=65_536).contains(s))
    {
        return Err("require n in 1..=10000000, seeds in 1..=1024 and capacities in 8..=65536".into());
    }
    if let Some(name) = names.iter().find(|s| !fixtures::NAMES.contains(&s.as_str())) {
        return Err(format!(
            "unknown fixture {name}; choose from {}",
            fixtures::NAMES.join(",")
        ));
    }
    for name in names {
        eprintln!("{name}: {n} values, {seeds} sketch seeds");
        let values = fixtures::generate(&name, n);
        let mut sorted = values.clone();
        sorted.sort_unstable_by(f64::total_cmp);
        for &capacity in &sizes {
            for &layout in &layouts {
                for kind in [Kind::Compactor, Kind::Reservoir] {
                    evaluate(&name, &values, &sorted, kind, capacity, seeds, layout);
                }
            }
        }
    }
    Ok(())
}
