use super::*;
use crate::ErrorKind;
use std::sync::atomic::{AtomicBool, Ordering};

fn budget(bytes: usize) -> Budget {
    Budget {
        multivariate: Resources::new(64, bytes),
        ..Budget::unlimited()
    }
}

#[test]
fn language_conversion_is_charged_and_released_on_every_failure() {
    let args = [
        Value::list(vec![Value::Float(0.0); 2]),
        Value::list(
            covariance()
                .into_iter()
                .map(|row| Value::list(row.into_iter().map(Value::Float).collect()))
                .collect(),
        ),
    ];
    let mut reference = budget(usize::MAX);
    let mut cache = Cache::new(1, &reference);
    let initial = reference.work_left;
    drop(cache.normal_values((0, 0), &args, &mut reference).unwrap());
    let work = initial - reference.work_left;
    let peak = reference.multivariate.peak_bytes();
    drop(cache);
    assert_eq!(reference.multivariate.used_bytes(), 0);
    for bytes in 0..peak {
        let mut budget = budget(bytes);
        let mut cache = Cache::new(1, &budget);
        assert_eq!(
            cache.normal_values((0, 0), &args, &mut budget).unwrap_err().kind,
            ErrorKind::Limit
        );
        drop(cache);
        assert_eq!(budget.multivariate.used_bytes(), 0, "bytes {bytes}");
    }
    for units in 0..work {
        let mut budget = budget(usize::MAX);
        budget.work_left = units;
        let mut cache = Cache::new(1, &budget);
        assert_eq!(
            cache.normal_values((0, 0), &args, &mut budget).unwrap_err().kind,
            ErrorKind::Limit
        );
        drop(cache);
        assert_eq!(budget.multivariate.used_bytes(), 0, "work {units}");
    }
    // Shape/dimension checks precede conversion or temporary allocation.
    let mut budget = budget(0);
    let mut cache = Cache::new(1, &budget);
    let bad = [
        Value::list(vec![Value::Bool(true); 2]),
        Value::list(vec![Value::list(vec![]); 2]),
    ];
    assert_eq!(
        cache.normal_values((0, 0), &bad, &mut budget).unwrap_err().fault,
        Some(Fault::DomainError)
    );
    assert_eq!(budget.multivariate.peak_bytes(), 0);
}

fn covariance() -> Vec<Vec<f64>> {
    vec![vec![4.0, 3.0], vec![3.0, 9.0]]
}

#[test]
fn recipe_identity_ignores_shared_resource_mutation_and_factor_replacement() {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    fn hash(value: &Value) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }
    let mut budget = budget(usize::MAX);
    let mut cache = Cache::new(1, &budget);
    let first = Value::MultivariateNormal(cache.normal((0, 0), &[-0.0, 0.0], &covariance(), &mut budget).unwrap());
    let original_hash = hash(&first);
    let other = Value::MultivariateNormal(
        cache
            .normal((0, 0), &[0.0, 1.0], &[vec![2.0, 0.0], vec![0.0, 1.0]], &mut budget)
            .unwrap(),
    );
    cache.clear();
    let equivalent = Value::MultivariateNormal(Normal::new(&[0.0; 2], &covariance(), &mut budget).unwrap());
    assert_eq!(first, equivalent);
    assert_eq!(first.cmp(&equivalent), std::cmp::Ordering::Equal);
    assert_eq!(hash(&equivalent), original_hash);
    assert_ne!(first, other);
    drop(other);
    assert_eq!(hash(&first), original_hash);
    drop((first, equivalent));
    assert_eq!(budget.multivariate.used_bytes(), 0);
}

#[test]
fn every_construction_memory_failure_releases_its_reservations() {
    let mut reference = budget(usize::MAX);
    let normal = Normal::new(&[0.0; 2], &covariance(), &mut reference).unwrap();
    let peak = reference.multivariate.peak_bytes();
    drop(normal);
    assert_eq!(reference.multivariate.used_bytes(), 0);
    for bytes in 0..peak {
        let mut budget = budget(bytes);
        let error = Normal::new(&[0.0; 2], &covariance(), &mut budget).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Limit);
        assert!(error.message.contains("max_multivariate_bytes"));
        assert_eq!(budget.multivariate.used_bytes(), 0, "allowance {bytes}");
    }
}

#[test]
fn work_failure_at_every_construction_step_leaves_no_storage() {
    let mut reference = budget(usize::MAX);
    let initial = reference.work_left;
    drop(Normal::new(&[0.0; 2], &covariance(), &mut reference).unwrap());
    let needed = initial - reference.work_left;
    for work in 0..needed {
        let mut budget = budget(usize::MAX);
        budget.work_left = work;
        assert_eq!(
            Normal::new(&[0.0; 2], &covariance(), &mut budget).unwrap_err().kind,
            ErrorKind::Limit
        );
        assert_eq!(budget.multivariate.used_bytes(), 0, "work allowance {work}");
    }
}

#[test]
fn shape_dimension_and_domain_errors_are_bounded_and_explicit() {
    let mut budget = budget(usize::MAX);
    for (means, covariance, message) in [
        (vec![], vec![], "nonempty"),
        (vec![0.0], covariance(), "dimensions differ"),
        (vec![0.0; 2], vec![vec![1.0], vec![0.0; 2]], "row 0"),
        (vec![f64::NAN; 2], covariance(), "mean at index 0"),
        (vec![0.0; 2], vec![vec![1.0, f64::INFINITY], vec![0.0, 1.0]], "[0, 1]"),
        (vec![0.0], vec![vec![-f64::MIN_POSITIVE]], "nonnegative"),
        (
            vec![0.0; 2],
            vec![vec![0.0, 1e-300], vec![1e-300, 1.0]],
            "zero variance",
        ),
        (vec![0.0; 2], vec![vec![1.0, 0.2], vec![0.1, 1.0]], "symmetric"),
        (
            vec![0.0; 3],
            vec![vec![1.0, 1.0, 1.0], vec![1.0, 1.0, -1.0], vec![1.0, -1.0, 1.0]],
            "positive semidefinite",
        ),
    ] {
        let error = Normal::new(&means, &covariance, &mut budget).unwrap_err();
        assert_eq!(error.fault, Some(Fault::DomainError));
        assert!(error.message.contains(message), "{error:?}");
        assert_eq!(budget.multivariate.used_bytes(), 0);
    }
    // Reject size before inspecting or copying the oversized rows.
    let error = Normal::new(&[], &vec![vec![]; 65], &mut budget).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Limit);
    budget.multivariate = Resources::new(1, usize::MAX);
    assert!(
        Normal::new(&[0.0; 2], &covariance(), &mut budget)
            .unwrap_err()
            .message
            .contains("dimension")
    );
    budget.multivariate = Resources::new(usize::MAX, usize::MAX);
    assert!(
        Normal::new(&[], &vec![vec![]; 65], &mut budget).is_err(),
        "host cannot raise hard cap"
    );
    budget.max_collection = 1;
    assert!(
        Normal::new(&[0.0; 2], &covariance(), &mut budget)
            .unwrap_err()
            .message
            .contains("collection")
    );
}

#[test]
fn cache_reuses_only_the_factor_and_keeps_evicted_recipes_charged() {
    let mut budget = budget(usize::MAX);
    let mut cache = Cache::new(1, &budget);
    let first = cache.normal((0, 0), &[0.0; 2], &covariance(), &mut budget).unwrap();
    let retained = budget.multivariate.used_bytes();
    let hit = cache.normal((0, 0), &[10.0, 20.0], &covariance(), &mut budget).unwrap();
    assert!(Arc::ptr_eq(first.factor(), hit.factor()));
    assert_eq!(hit.means(), &[10.0, 20.0]);
    drop(hit);
    assert_eq!(budget.multivariate.used_bytes(), retained);
    let replacement = cache
        .normal((0, 0), &[0.0; 2], &[vec![4.0, 0.0], vec![0.0, 9.0]], &mut budget)
        .unwrap();
    assert!(!Arc::ptr_eq(first.factor(), replacement.factor()));
    cache.clear();
    assert!(budget.multivariate.used_bytes() > retained);
    drop(replacement);
    assert!(budget.multivariate.used_bytes() > 0, "old recipe still owns its factor");
    let clone = first.clone();
    drop(first);
    assert!(budget.multivariate.used_bytes() > 0);
    drop(clone);
    assert_eq!(budget.multivariate.used_bytes(), 0);
}

#[test]
fn cache_failure_and_hits_obey_memory_work_and_cancellation() {
    let mut budget = budget(usize::MAX);
    let flag = Arc::new(AtomicBool::new(false));
    budget.cancel = Some(flag.clone());
    let mut cache = Cache::new(4, &budget);
    let first = cache.normal((1, 2), &[0.0; 2], &covariance(), &mut budget).unwrap();
    let retained = budget.multivariate.used_bytes();
    let hold = budget.multivariate.reserve(usize::MAX - retained).unwrap();
    assert!(
        cache.normal((1, 2), &[0.0; 2], &covariance(), &mut budget).is_err(),
        "hit still allocates means"
    );
    assert!(
        cache
            .normal((1, 2), &[0.0; 2], &[vec![1.0, 0.0], vec![0.0, 1.0]], &mut budget)
            .is_err()
    );
    drop(hold);
    assert_eq!(budget.multivariate.used_bytes(), retained);
    assert!(Arc::ptr_eq(
        first.factor(),
        cache
            .normal((1, 2), &[0.0; 2], &covariance(), &mut budget)
            .unwrap()
            .factor()
    ));
    budget.work_left = 0;
    assert!(cache.normal((1, 2), &[0.0; 2], &covariance(), &mut budget).is_err());
    budget.work_left = u64::MAX;
    flag.store(true, Ordering::Relaxed);
    assert!(
        cache
            .normal((1, 2), &[0.0; 2], &covariance(), &mut budget)
            .unwrap_err()
            .message
            .contains("cancelled")
    );
    assert_eq!(budget.multivariate.used_bytes(), retained);
    drop(first);
    cache.clear();
    assert_eq!(budget.multivariate.used_bytes(), 0);
}

#[test]
fn cache_growth_and_entry_limit_release_all_buffers() {
    let mut budget = budget(usize::MAX);
    let mut cache = Cache::new(3, &budget);
    for site in [4, 1, 3] {
        drop(cache.normal((0, site), &[0.0; 2], &covariance(), &mut budget).unwrap());
    }
    assert_eq!(cache.entries.iter().map(|e| e.site.1).collect::<Vec<_>>(), [1, 3, 4]);
    let before = budget.multivariate.used_bytes();
    drop(cache.normal((0, 2), &[0.0; 2], &covariance(), &mut budget).unwrap());
    assert_eq!(cache.entries.len(), 1);
    assert!(budget.multivariate.used_bytes() < before);
    cache.clear();
    assert_eq!(budget.multivariate.used_bytes(), 0);
    let mut disabled = Cache::new(0, &budget);
    drop(disabled.normal((0, 0), &[0.0; 2], &covariance(), &mut budget).unwrap());
    assert_eq!(budget.multivariate.used_bytes(), 0);
}

#[test]
fn failed_cache_growth_preserves_existing_entries_and_reservations() {
    let mut reference = budget(usize::MAX);
    let mut cache = Cache::new(2, &reference);
    let first = cache.normal((0, 0), &[0.0; 2], &covariance(), &mut reference).unwrap();
    let retained = reference.multivariate.used_bytes();
    let second = cache.normal((0, 1), &[0.0; 2], &covariance(), &mut reference).unwrap();
    let peak = reference.multivariate.peak_bytes();
    drop((first, second, cache));
    for limit in retained..peak {
        let mut budget = budget(limit);
        let mut cache = Cache::new(2, &budget);
        let first = cache.normal((0, 0), &[0.0; 2], &covariance(), &mut budget).unwrap();
        assert_eq!(
            cache
                .normal((0, 1), &[0.0; 2], &covariance(), &mut budget)
                .unwrap_err()
                .kind,
            ErrorKind::Limit
        );
        assert_eq!(budget.multivariate.used_bytes(), retained);
        assert_eq!(cache.entries.len(), 1);
        assert!(Arc::ptr_eq(first.factor(), &cache.entries[0].factor));
        drop((first, cache));
        assert_eq!(budget.multivariate.used_bytes(), 0);
    }
}

#[test]
fn failed_sample_preflight_does_not_consume_rng_or_leak_scratch() {
    let mut budget = budget(usize::MAX);
    let normal = Normal::new(&[0.0; 2], &covariance(), &mut budget).unwrap();
    let retained = budget.multivariate.used_bytes();
    let mut rng = Rng::new(42);
    let mut untouched = Rng::new(42);
    let hold = budget.multivariate.reserve(usize::MAX - retained).unwrap();
    assert!(normal.sample(&mut rng, &mut budget).is_err());
    drop(hold);
    budget.work_left = 0;
    assert!(normal.sample(&mut rng, &mut budget).is_err());
    budget.work_left = u64::MAX;
    budget.cancel = Some(Arc::new(AtomicBool::new(true)));
    assert!(
        normal
            .sample(&mut rng, &mut budget)
            .unwrap_err()
            .message
            .contains("cancelled")
    );
    budget.cancel = None;
    assert_eq!(rng.next_u64(), untouched.next_u64());
    assert_eq!(budget.multivariate.used_bytes(), retained);
    normal.sample(&mut rng, &mut budget).unwrap();
    assert_eq!(budget.multivariate.used_bytes(), retained);
    drop(normal);
    assert_eq!(budget.multivariate.used_bytes(), 0);
}

#[test]
fn concurrent_workers_share_one_live_allowance() {
    let mut reference = budget(usize::MAX);
    let normal = Normal::new(&[0.0; 2], &covariance(), &mut reference).unwrap();
    let allowance = reference.multivariate.peak_bytes();
    drop(normal);
    let budget = budget(allowance);
    let resources = budget.multivariate.clone();
    let barrier = std::sync::Barrier::new(4);
    let successes = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let mut worker = budget.clone();
            let barrier = &barrier;
            let successes = &successes;
            scope.spawn(move || {
                let normal = Normal::new(&[0.0; 2], &covariance(), &mut worker);
                if normal.is_ok() {
                    successes.fetch_add(1, Ordering::Relaxed);
                }
                barrier.wait(); // Hold successful reservations until every worker finishes.
                drop(normal);
            });
        }
    });
    assert!(successes.load(Ordering::Relaxed) <= 1);
    assert!(resources.peak_bytes() <= allowance);
    assert_eq!(resources.used_bytes(), 0);
}

#[test]
fn recipes_and_caches_cannot_escape_their_execution_accounting() {
    let mut first = budget(usize::MAX);
    let normal = Normal::new(&[0.0; 2], &covariance(), &mut first).unwrap();
    let mut cache = Cache::new(1, &first);
    let mut other = budget(usize::MAX);
    assert_eq!(
        normal.sample(&mut Rng::new(1), &mut other).unwrap_err().kind,
        ErrorKind::Internal
    );
    assert_eq!(
        cache
            .normal((0, 0), &[0.0; 2], &covariance(), &mut other)
            .unwrap_err()
            .kind,
        ErrorKind::Internal
    );
}

#[test]
fn cached_batch_draws_replay_across_worker_counts() {
    fn batch(index: usize, cache: &mut Cache, budget: &mut Budget) -> Vec<Vec<f64>> {
        cache.clear();
        let mut rng = Rng::stream(42, index as u64);
        (0..100)
            .map(|i| {
                let normal = cache.normal((1, 0), &[i as f64, 0.0], &covariance(), budget).unwrap();
                normal.sample(&mut rng, budget).unwrap()
            })
            .collect()
    }
    let mut budget = budget(1024 * 1024);
    let mut cache = Cache::new(2, &budget);
    let expected: Vec<_> = (0..7).map(|i| batch(i, &mut cache, &mut budget)).collect();
    cache.clear();
    for workers in [2, 4, 7] {
        let mut actual = std::thread::scope(|scope| {
            (0..workers)
                .map(|worker| {
                    let mut budget = budget.clone();
                    scope.spawn(move || {
                        let mut cache = Cache::new(2, &budget);
                        (worker..7)
                            .step_by(workers)
                            .map(|i| (i, batch(i, &mut cache, &mut budget)))
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .flat_map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });
        actual.sort_by_key(|(i, _)| *i);
        assert_eq!(actual.into_iter().map(|(_, draws)| draws).collect::<Vec<_>>(), expected);
        assert_eq!(budget.multivariate.used_bytes(), 0);
    }
}
