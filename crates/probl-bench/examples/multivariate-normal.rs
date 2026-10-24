//! Numerical experiment for the M5 proposal; this is not a language builtin.
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

fn main() {
    let mut samples = 100_000;
    for arg in std::env::args().skip(1) {
        if arg == "--replay" {
            for seed in [0, 42, u32::MAX] {
                println!(
                    "{}",
                    json!({"seed":seed,"fingerprint":format!("{:016x}", replay_fingerprint(seed))})
                );
            }
            return;
        }
        if let Some(value) = arg
            .strip_prefix("--samples=")
            .and_then(|value| value.parse::<usize>().ok())
        {
            assert!(value > 0, "samples must be positive");
            samples = value;
        } else {
            panic!("expected --samples=N or --replay");
        }
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
