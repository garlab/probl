//! Numerical experiment for `multivariate_normal`; this is not the builtin itself.
//! cargo run --release -p probl-bench --example multivariate-normal -- --samples=100000
//! --replay prints fingerprints also exported by the wasm32 build.
#[path = "multivariate_normal/factor.rs"]
mod factor;
#[path = "multivariate_normal/fixtures.rs"]
mod fixtures;
#[cfg(test)]
#[path = "multivariate_normal/tests.rs"]
mod tests;

use factor::Factor;
use probl_engine::continuous::Rng;
use probl_engine::dist::Budget;
use probl_engine::multivariate::{Cache, Normal, Resources};
use serde_json::json;
use std::hint::black_box;
use std::time::Instant;

fn fingerprint(h: u64, value: f64) -> u64 {
    (h ^ value.to_bits()).wrapping_mul(0x100000001b3)
}

// This pure entry point avoids platform clocks and I/O when run in WASM.
#[unsafe(no_mangle)]
pub extern "C" fn replay_fingerprint(seed: u32) -> u64 {
    let mut h = 0xcbf29ce484222325;
    for fixture in fixtures::fixtures() {
        let factor = Factor::new(&fixture.covariance).expect("valid fixture");
        h = fingerprint(h, factor.rank as f64);
        for &coefficient in &factor.lower {
            h = fingerprint(h, coefficient);
        }
        h = fingerprint(h, factor.normalized_covariance(0, 0));
        let means = vec![0.0; factor.dimension];
        for batch in 0..3 {
            let mut rng = Rng::stream(u64::from(seed), batch);
            for _ in 0..1000 {
                for value in factor.sample(&means, &mut rng).expect("finite sample") {
                    h = fingerprint(h, value);
                }
            }
        }
    }
    h
}

#[unsafe(no_mangle)]
pub extern "C" fn engine_replay_fingerprint(seed: u32) -> u64 {
    let mut h = 0xcbf29ce484222325;
    let mut budget = Budget::unlimited();
    for fixture in fixtures::fixtures() {
        let means = vec![0.0; fixture.covariance.len()];
        let normal = Normal::new(&means, &fixture.covariance, &mut budget).unwrap();
        let factor = normal.factor();
        h = fingerprint(h, factor.rank() as f64);
        for &coefficient in factor.coefficients() {
            h = fingerprint(h, coefficient);
        }
        // Match the prototype's +0 accumulator, including an empty rank-zero dot.
        let mut first_variance = 0.0;
        for value in &factor.coefficients()[..factor.rank()] {
            first_variance += value * value;
        }
        h = fingerprint(h, first_variance);
        for batch in 0..3 {
            let mut rng = Rng::stream(u64::from(seed), batch);
            for _ in 0..1000 {
                for value in normal.sample(&mut rng, &mut budget).unwrap() {
                    h = fingerprint(h, value);
                }
            }
        }
    }
    assert_eq!(budget.multivariate.used_bytes(), 0);
    h
}

fn benchmark_engine(samples: usize) {
    for fixture in fixtures::fixtures() {
        let mut budget = Budget::unlimited();
        let means = vec![0.0; fixture.covariance.len()];
        let normal = Normal::new(&means, &fixture.covariance, &mut budget).unwrap();
        let retained = budget.multivariate.used_bytes();
        let start = Instant::now();
        for _ in 0..100 {
            black_box(Normal::new(&means, &fixture.covariance, &mut budget).unwrap());
        }
        let construction_ns = start.elapsed().as_nanos() / 100;
        let mut rng = Rng::new(42);
        let start = Instant::now();
        for _ in 0..samples {
            black_box(normal.sample(&mut rng, &mut budget).unwrap());
        }
        let sample_ns = start.elapsed().as_nanos() / samples as u128;
        let mut cache = Cache::new(1, &budget);
        drop(cache.normal((0, 0), &means, &fixture.covariance, &mut budget).unwrap());
        let work_before = budget.work_left;
        let start = Instant::now();
        for _ in 0..1000 {
            black_box(cache.normal((0, 0), &means, &fixture.covariance, &mut budget).unwrap());
        }
        let cached_ns = start.elapsed().as_nanos() / 1000;
        let cached_work = (work_before - budget.work_left) / 1000;
        println!(
            "{}",
            json!({"engine":fixture.name,"dimension":means.len(),"rank":normal.factor().rank(),
            "construction_ns":construction_ns,"sample_ns":sample_ns,"cached_constructor_ns":cached_ns,
            "cached_constructor_work":cached_work,"retained_accounted_bytes":retained,
            "overlapping_peak_accounted_bytes":budget.multivariate.peak_bytes()})
        );
        drop((normal, cache));
        assert_eq!(budget.multivariate.used_bytes(), 0);
    }
    for workers in [1, 4] {
        for changing in [false, true] {
            for steps in [100, 1000] {
                cache_workload(workers, steps, changing);
            }
        }
    }
}

fn cache_workload(workers: usize, steps: usize, changing: bool) {
    let budget = Budget {
        multivariate: Resources::new(64, 16 * 1024 * 1024),
        ..Budget::unlimited()
    };
    let start = Instant::now();
    std::thread::scope(|scope| {
        for worker in 0..workers {
            let mut budget = budget.clone();
            scope.spawn(move || {
                let mut covariance: Vec<Vec<f64>> = (0..64)
                    .map(|i| (0..64).map(|j| if i == j { 1.0 } else { 0.25 }).collect())
                    .collect();
                let mut cache = Cache::new(2, &budget);
                let mut retained = [None, None];
                let mut rng = Rng::stream(42, worker as u64);
                for step in 0..steps {
                    if changing {
                        covariance[0][0] = 1.0 + ((step / 2) % 2) as f64 * 0.25;
                    }
                    let normal = cache
                        .normal((0, step % 2), &[0.0; 64], &covariance, &mut budget)
                        .unwrap();
                    black_box(normal.sample(&mut rng, &mut budget).unwrap());
                    retained[step % 2] = Some(normal);
                }
            });
        }
    });
    println!(
        "{}",
        json!({"cache_workload":"two_sites_64d","workers":workers,"steps_per_worker":steps,
        "changing_covariance":changing,"elapsed_ms":start.elapsed().as_secs_f64()*1000.0,
        "peak_accounted_bytes":budget.multivariate.peak_bytes(),"after_drop_bytes":budget.multivariate.used_bytes()})
    );
    assert_eq!(budget.multivariate.used_bytes(), 0);
}

fn main() {
    let mut samples = 100_000;
    let mut engine = false;
    for arg in std::env::args().skip(1) {
        if arg == "--replay" {
            for seed in [0, 42, u32::MAX] {
                println!(
                    "{}",
                    json!({"seed":seed,"fingerprint":format!("{:016x}", replay_fingerprint(seed)),
                        "engine_fingerprint":format!("{:016x}", engine_replay_fingerprint(seed))})
                );
            }
            return;
        }
        if arg == "--engine" {
            engine = true;
            continue;
        }
        if let Some(value) = arg
            .strip_prefix("--samples=")
            .and_then(|value| value.parse::<usize>().ok())
        {
            assert!(value > 0, "samples must be positive");
            samples = value;
        } else {
            panic!("expected --samples=N, --engine or --replay");
        }
    }
    if engine {
        benchmark_engine(samples);
        return;
    }
    for fixture in fixtures::fixtures() {
        let factor = Factor::new(&fixture.covariance).expect("valid fixture");
        let start = Instant::now();
        for _ in 0..100 {
            black_box(Factor::new(black_box(&fixture.covariance)).unwrap());
        }
        let construction_ns = start.elapsed().as_nanos() / 100;
        let means = vec![0.0; factor.dimension];
        let mut rng = Rng::new(42);
        let start = Instant::now();
        for _ in 0..samples {
            black_box(factor.sample(black_box(&means), &mut rng).unwrap());
        }
        let sample_ns = start.elapsed().as_nanos() / samples as u128;
        println!(
            "{}",
            json!({
                "fixture":fixture.name,"dimension":factor.dimension,"rank":factor.rank,
                "construction_ns":construction_ns,"sample_ns":sample_ns,"samples":samples,
                "max_normalized_residual":factor.residual,"tolerance":factor.tolerance,
                "retained_numeric_bytes":8*(factor.lower.capacity()+factor.scales.capacity()),
            })
        );
    }
    // ABBA comparison, including the returned Vec in both paths. This compares
    // the numerical kernels, not Probl's interpreter or constructor cache.
    let factor = Factor::new(&[vec![4.0, 3.0], vec![3.0, 9.0]]).unwrap();
    for method in ["factor", "shared", "shared", "factor"] {
        let mut rng = Rng::new(42);
        let start = Instant::now();
        for _ in 0..samples {
            let value = if method == "factor" {
                factor.sample(black_box(&[0.0, 0.0]), &mut rng).unwrap()
            } else {
                let a = rng.normal();
                let b = rng.normal();
                vec![2.0 * a, 1.5 * a + libm::sqrt(6.75) * b]
            };
            black_box(value);
        }
        println!(
            "{}",
            json!({"comparison":"two_variables","method":method,"sample_ns":start.elapsed().as_nanos()/samples as u128})
        );
    }
}
