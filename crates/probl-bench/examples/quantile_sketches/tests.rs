use super::*;
use crate::fixtures::{exact_quantile, generate, rank_error};

fn answers(sketch: &Sketch) -> Vec<f64> {
    let view = sketch.snapshot();
    (0..=1000).map(|i| view.quantile(i as f64 / 1000.0).unwrap()).collect()
}

fn make(kind: Kind, k: usize, values: &[f64]) -> Sketch {
    let mut sketch = Sketch::new(kind, k, 42, 0);
    for (id, &value) in values.iter().enumerate() {
        sketch.add(value, id as u64);
    }
    sketch
}

#[test]
fn exact_before_compaction_and_inverse_cdf_convention() {
    let values = [-10.0, -10.0, -2.0, -0.0, 0.0, 1.0, 4.0, f64::MAX];
    for kind in [Kind::Compactor, Kind::Reservoir] {
        let sketch = make(kind, 128, &values);
        for (i, value) in answers(&sketch).into_iter().enumerate() {
            assert_eq!(value, exact_quantile(&values, i as f64 / 1000.0));
        }
        // q=0.5 is the inverse CDF, not an interpolated midpoint.
        assert_eq!(make(kind, 128, &[2.0, 8.0]).snapshot().quantile(0.5), Some(2.0));
    }
}

#[test]
fn empty_invalid_quantiles_and_singletons() {
    for kind in [Kind::Compactor, Kind::Reservoir] {
        assert_eq!(make(kind, 128, &[]).snapshot().quantile(0.5), None);
        for value in [-f64::MAX, -1.0, 0.0, f64::from_bits(1), f64::MAX] {
            let sketch = make(kind, 128, &[value]);
            assert!(answers(&sketch).iter().all(|&v| v == value));
            for q in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
                assert_eq!(sketch.snapshot().quantile(q), None);
            }
        }
    }
}

#[test]
fn conservation_and_retention_through_uneven_merges() {
    let mut all = Sketch::new(Kind::Compactor, 128, 42, u64::MAX);
    let mut n = 0;
    for batch in 0..150 {
        let mut part = Sketch::new(Kind::Compactor, 128, 42, batch);
        for i in 0..(batch * 37 % 2000 + 1) {
            part.add((n + i) as f64, n + i);
        }
        n += part.count;
        all.merge(part);
        let Core::Compactor(core) = &all.core else {
            unreachable!()
        };
        let weight: u64 = core
            .levels
            .iter()
            .enumerate()
            .map(|(level, v)| (v.len() as u64) << level)
            .sum();
        assert_eq!(weight + all.zeros, n);
        assert_eq!(all.count, n);
        assert!(core.size < core.limit);
        assert!(core.limit <= 3 * core.k + 9 * 64);
        assert_eq!(core.size, core.levels.iter().map(Vec::len).sum::<usize>());
        assert!(core.levels.iter().map(Vec::capacity).sum::<usize>() <= 4 * core.limit);
    }
}

#[test]
fn repeated_values_extremes_and_zero_atom_stay_finite_and_monotone() {
    for fixture in ["all_zero", "zero_atom", "repeated", "extremes", "normal"] {
        let values = generate(fixture, 30_000);
        let mut sorted = values.clone();
        sorted.sort_unstable_by(f64::total_cmp);
        for kind in [Kind::Compactor, Kind::Reservoir] {
            let sketch = make(kind, 128, &values);
            assert_eq!(sketch.zeros, values.iter().filter(|&&v| v == 0.0).count() as u64);
            let answers = answers(&sketch);
            assert!(answers.windows(2).all(|w| w[0] <= w[1]));
            assert_eq!(answers[0], sorted[0]);
            assert_eq!(answers[1000], sorted[sorted.len() - 1]);
            for (i, answer) in answers.into_iter().enumerate() {
                let q = i as f64 / 1000.0;
                assert!(answer.is_finite());
                assert!(sorted.binary_search_by(|v| v.total_cmp(&answer)).is_ok());
                if exact_quantile(&sorted, q) == 0.0 {
                    assert_eq!(answer, 0.0);
                }
            }
            if fixture == "all_zero" {
                assert_eq!(sketch.retained(), 0);
            }
        }
    }
}

#[test]
fn exact_zero_boundaries_even_when_a_rare_sign_is_lost() {
    // Force the query fallback without depending on a particular random seed.
    // The exact sign counts and endpoints must survive a sample missing a side.
    for kind in [Kind::Compactor, Kind::Reservoir] {
        let mut sketch = make(kind, 8, &[-9.0, 0.0, 0.0, 1.0, 2.0]);
        match &mut sketch.core {
            Core::Compactor(c) => c.levels.iter_mut().for_each(|v| v.retain(|&x| x > 0.0)),
            Core::Reservoir(r) => r.items.retain(|i| i.value > 0.0),
        }
        let view = sketch.snapshot();
        assert_eq!(view.quantile(0.2), Some(-9.0));
        assert_eq!(view.quantile(0.200_001), Some(0.0));
        assert_eq!(view.quantile(0.6), Some(0.0));
        assert!(view.quantile(0.600_001).unwrap() > 0.0);
    }
}

#[test]
fn tie_aware_rank_metric_does_not_penalize_correct_atoms() {
    let sorted = [0.0, 0.0, 0.0, 5.0];
    assert_eq!(rank_error(&sorted, 0.5, 0.0), 0.0);
    assert_eq!(rank_error(&sorted, 0.75, 0.0), 0.0);
    assert_eq!(rank_error(&sorted, 0.5, 5.0), 0.25);
    assert!((rank_error(&sorted, 0.9, 0.0) - 0.15).abs() < 1e-15);
}

#[test]
fn reservoir_is_merge_order_independent_with_global_observation_ids() {
    let values = generate("normal", 10_009);
    let stream = make(Kind::Reservoir, 128, &values);
    let mut forward = Sketch::new(Kind::Reservoir, 128, 42, 100);
    let mut reverse = forward.clone();
    let mut parts = Vec::new();
    for (batch, values) in values.chunks(1000).enumerate() {
        let mut part = Sketch::new(Kind::Reservoir, 128, 42, batch as u64);
        for (i, &value) in values.iter().enumerate() {
            part.add(value, (batch * 1000 + i) as u64);
        }
        forward.merge(part.clone());
        parts.push(part);
    }
    for part in parts.into_iter().rev() {
        reverse.merge(part);
    }
    assert_eq!(answers(&stream), answers(&forward));
    assert_eq!(answers(&stream), answers(&reverse));
}

fn parallel(values: &[f64], kind: Kind, workers: usize) -> Sketch {
    let mut result = Sketch::new(kind, 128, 42, u64::MAX);
    std::thread::scope(|scope| {
        let batches: Vec<_> = values.chunks(1000).enumerate().collect();
        for wave in batches.chunks(workers) {
            let tasks: Vec<_> = wave
                .iter()
                .map(|&(batch, values)| {
                    scope.spawn(move || {
                        let mut part = Sketch::new(kind, 128, 42, batch as u64);
                        for (i, &value) in values.iter().enumerate() {
                            part.add(value, (batch * 1000 + i) as u64);
                        }
                        part
                    })
                })
                .collect();
            // Completion order is immaterial; merge in original batch order.
            for task in tasks {
                result.merge(task.join().unwrap());
            }
        }
    });
    result
}

#[test]
fn fixed_batches_replay_across_worker_counts() {
    for fixture in ["normal", "zero_atom"] {
        let values = generate(fixture, 20_009);
        for kind in [Kind::Compactor, Kind::Reservoir] {
            let one = parallel(&values, kind, 1);
            assert_eq!(answers(&one), answers(&parallel(&values, kind, 4)));
            assert_eq!(answers(&one), answers(&parallel(&values, kind, 7)));
        }
    }
}

#[test]
fn compactor_accuracy_regression_on_ordered_and_heavy_tailed_populations() {
    // A regression threshold for these fixed fixtures/seeds, not a guarantee.
    for fixture in ["ascending", "descending", "lognormal", "zero_atom"] {
        let values = generate(fixture, 20_000);
        let mut sorted = values.clone();
        sorted.sort_unstable_by(f64::total_cmp);
        for seed in 0..8 {
            let (sketch, _) = crate::build(&values, Kind::Compactor, 128, seed, crate::Layout::Batches, false);
            for (i, value) in answers(&sketch).into_iter().enumerate() {
                assert!(rank_error(&sorted, i as f64 / 1000.0, value) < 0.04);
            }
        }
    }
}
