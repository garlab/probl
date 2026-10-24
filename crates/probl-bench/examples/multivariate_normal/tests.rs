use super::{Factor, factor::Error, fixtures, replay_fingerprint};
use probl_engine::continuous::Rng;

#[test]
fn singular_and_mixed_scale_fixtures_reconstruct() {
    for fixture in fixtures::fixtures() {
        let factor = Factor::new(&fixture.covariance).unwrap_or_else(|e| panic!("{}: {e:?}", fixture.name));
        assert!(factor.residual <= factor.tolerance, "{}", fixture.name);
        for i in 0..factor.dimension {
            for j in 0..factor.dimension {
                let a = factor.scales[i];
                let b = factor.scales[j];
                let expected = if a == 0.0 || b == 0.0 {
                    0.0
                } else {
                    fixture.covariance[i][j] / a.max(b) / a.min(b)
                };
                assert!((factor.normalized_covariance(i, j) - expected).abs() <= factor.tolerance);
            }
        }
        if fixture.name == "mixed_scales" {
            assert_eq!(factor.rank, 2, "a small absolute variance must not disappear");
        }
    }
}

#[test]
fn invalid_covariances_fail_explicitly() {
    for (covariance, error) in [
        (vec![], Error::Shape),
        (vec![vec![1.0, 0.0]], Error::Shape),
        (vec![vec![1.0; 65]; 65], Error::Dimension),
        (vec![vec![f64::NAN]], Error::NonFinite),
        (vec![vec![f64::INFINITY]], Error::NonFinite),
        (vec![vec![-f64::MIN_POSITIVE]], Error::NegativeVariance),
        (
            vec![vec![0.0, 1e-300], vec![1e-300, 1.0]],
            Error::ZeroVarianceCovariance,
        ),
        (vec![vec![1.0, 0.2], vec![0.1, 1.0]], Error::Asymmetric),
        (vec![vec![1.0, 1.01], vec![1.01, 1.0]], Error::NotPositiveSemidefinite),
        (
            vec![vec![1e-300, 1e300], vec![1e300, 1e-300]],
            Error::NotPositiveSemidefinite,
        ),
        // Pairwise correlations all look valid. The remaining diagonal is zero
        // after the first pivot, but the residual off-diagonal is -2.
        (
            vec![vec![1.0, 1.0, 1.0], vec![1.0, 1.0, -1.0], vec![1.0, -1.0, 1.0]],
            Error::NotPositiveSemidefinite,
        ),
        (
            vec![vec![1.0, 0.9, 0.9], vec![0.9, 1.0, -0.9], vec![0.9, -0.9, 1.0]],
            Error::NotPositiveSemidefinite,
        ),
    ] {
        assert_eq!(Factor::new(&covariance).unwrap_err(), error);
    }
}

#[test]
fn tolerance_is_dimensionless_and_bounds_the_reconstruction() {
    let tolerance = 128.0 * f64::EPSILON;
    for correlation in [1.0 - tolerance / 4.0, 1.0 + tolerance / 4.0] {
        let factor = Factor::new(&[vec![1.0, correlation], vec![correlation, 1.0]]).unwrap();
        assert_eq!(factor.rank, 1);
        assert!(factor.residual <= tolerance);
    }
    let factor = Factor::new(&[vec![1.0, 0.5], vec![0.5 + tolerance / 2.0, 1.0]]).unwrap();
    assert!((factor.normalized_covariance(0, 1) - (0.5 + tolerance / 4.0)).abs() < 1e-15);
    assert!(Factor::new(&[vec![1.0, 0.5], vec![0.5 + tolerance * 2.0, 1.0]]).is_err());
}

#[test]
fn independent_gram_matrices_preserve_every_covariance_entry() {
    let mut rng = Rng::new(19);
    for n in [1_usize, 2, 3, 8, 16, 32, 64] {
        for rank in [1, n.div_ceil(2), n] {
            for _ in 0..4 {
                let rows: Vec<Vec<_>> = (0..n).map(|_| (0..rank).map(|_| rng.normal()).collect()).collect();
                let covariance = fixtures::gram(&rows);
                let factor = Factor::new(&covariance).unwrap();
                assert!(factor.rank <= rank);
                for (i, row) in covariance.iter().enumerate() {
                    for (j, &entry) in row.iter().enumerate() {
                        let expected = entry / factor.scales[i] / factor.scales[j];
                        assert!((factor.normalized_covariance(i, j) - expected).abs() < factor.tolerance);
                    }
                }
            }
        }
    }
}

#[test]
fn joint_draws_preserve_perfect_dependence_and_zero_variance() {
    let factor = Factor::new(&[vec![4.0, -6.0, 0.0], vec![-6.0, 9.0, 0.0], vec![0.0; 3]]).unwrap();
    assert_eq!(factor.rank, 1);
    let mut rng = Rng::new(42);
    for _ in 0..1000 {
        let value = factor.sample(&[0.0, 0.0, 7.0], &mut rng).unwrap();
        assert!((value[1] + 1.5 * value[0]).abs() < 1e-14);
        assert_eq!(value[2], 7.0);
    }
    let factor = Factor::new(&vec![vec![0.0; 2]; 2]).unwrap();
    let mut untouched = Rng::new(5);
    let mut rng = Rng::new(5);
    assert_eq!(factor.sample(&[1.0, 2.0], &mut rng).unwrap(), vec![1.0, 2.0]);
    assert_eq!(rng.next_u64(), untouched.next_u64(), "zero rank consumes no randomness");
}

#[test]
fn empirical_moments_and_repeated_draw_independence() {
    let covariance = vec![vec![4.0, 3.0], vec![3.0, 9.0]];
    let factor = Factor::new(&covariance).unwrap();
    let n = 100_000;
    for seed in [0, 42, 7919] {
        let mut rng = Rng::new(seed);
        let mut mean = [0.0; 4];
        let mut products = [[0.0; 4]; 4];
        for k in 1..=n {
            let a = factor.sample(&[2.0, -3.0], &mut rng).unwrap();
            let b = factor.sample(&[2.0, -3.0], &mut rng).unwrap();
            let x = [a[0], a[1], b[0], b[1]];
            let delta = std::array::from_fn::<_, 4, _>(|i| x[i] - mean[i]);
            for i in 0..4 {
                mean[i] += delta[i] / k as f64;
            }
            for i in 0..4 {
                for j in 0..4 {
                    products[i][j] += delta[i] * (x[j] - mean[j]);
                }
            }
        }
        for i in 0..4 {
            let variance = covariance[i % 2][i % 2];
            let wanted_mean = if i % 2 == 0 { 2.0 } else { -3.0 };
            assert!((mean[i] - wanted_mean).abs() < 7.0 * libm::sqrt(variance / n as f64));
            for j in 0..4 {
                let expected = if i / 2 == j / 2 { covariance[i % 2][j % 2] } else { 0.0 };
                let error =
                    7.0 * libm::sqrt((variance * covariance[j % 2][j % 2] + expected * expected) / (n - 1) as f64);
                assert!((products[i][j] / (n - 1) as f64 - expected).abs() < error);
            }
        }
    }
}

#[test]
fn invalid_means_do_not_consume_rng() {
    let factor = Factor::new(&[vec![1.0]]).unwrap();
    let mut rng = Rng::new(42);
    let mut untouched = Rng::new(42);
    assert_eq!(factor.sample(&[], &mut rng), Err(Error::Shape));
    assert_eq!(factor.sample(&[f64::NAN], &mut rng), Err(Error::NonFinite));
    assert_eq!(rng.next_u64(), untouched.next_u64());
}

#[test]
fn batch_streams_replay_across_worker_counts() {
    let batch = |index| {
        let factor = Factor::new(&[vec![4.0, 3.0], vec![3.0, 9.0]]).unwrap();
        let mut rng = Rng::stream(42, index);
        (0..1000)
            .map(|_| factor.sample(&[0.0, 0.0], &mut rng).unwrap())
            .collect::<Vec<_>>()
    };
    let sequential: Vec<_> = (0..7).map(batch).collect();
    for workers in [2, 4, 7] {
        let mut parallel = Vec::new();
        for start in (0..7).step_by(workers) {
            parallel.extend(std::thread::scope(|scope| {
                (start..(start + workers).min(7))
                    .map(|i| scope.spawn(move || batch(i as u64)))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|handle| handle.join().unwrap())
                    .collect::<Vec<_>>()
            }));
        }
        assert_eq!(parallel, sequential);
    }
    assert_eq!(replay_fingerprint(42), replay_fingerprint(42));
    assert_ne!(replay_fingerprint(0), replay_fingerprint(42));
}
