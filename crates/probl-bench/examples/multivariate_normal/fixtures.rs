pub struct Fixture {
    pub name: &'static str,
    pub covariance: Vec<Vec<f64>>,
}

pub fn gram(rows: &[Vec<f64>]) -> Vec<Vec<f64>> {
    rows.iter()
        .map(|a| rows.iter().map(|b| a.iter().zip(b).map(|(x, y)| x * y).sum()).collect())
        .collect()
}

pub fn fixtures() -> Vec<Fixture> {
    let mut fixtures = vec![
        Fixture {
            name: "diagonal",
            covariance: vec![vec![4.0, 0.0], vec![0.0, 9.0]],
        },
        Fixture {
            name: "correlated",
            covariance: vec![vec![4.0, 3.0], vec![3.0, 9.0]],
        },
        Fixture {
            name: "perfect_positive",
            covariance: vec![vec![4.0, 6.0], vec![6.0, 9.0]],
        },
        Fixture {
            name: "perfect_negative",
            covariance: vec![vec![4.0, -6.0], vec![-6.0, 9.0]],
        },
        Fixture {
            name: "zero_variance",
            covariance: vec![vec![0.0, 0.0], vec![0.0, 9.0]],
        },
        Fixture {
            name: "all_zero",
            covariance: vec![vec![0.0; 3]; 3],
        },
        Fixture {
            name: "mixed_scales",
            covariance: vec![vec![1e300, 0.5], vec![0.5, 1e-300]],
        },
        Fixture {
            name: "subnormal",
            covariance: vec![vec![f64::from_bits(1)]],
        },
        Fixture {
            name: "near_singular",
            covariance: vec![vec![1.0, 1.0 - 1e-14], vec![1.0 - 1e-14, 1.0]],
        },
    ];
    for (name, n, rank) in [
        ("gram_8_rank3", 8, 3),
        ("gram_32_rank5", 32, 5),
        ("gram_64_rank8", 64, 8),
    ] {
        let rows: Vec<Vec<f64>> = (0..n)
            .map(|i| {
                (0..rank)
                    .map(|j| ((i * 13 + j * 7 + i * j * 3) % 23) as f64 - 11.0)
                    .collect()
            })
            .collect();
        fixtures.push(Fixture {
            name,
            covariance: gram(&rows),
        });
    }
    fixtures.push(Fixture {
        name: "full_rank_64",
        covariance: (0..64)
            .map(|i| (0..64).map(|j| if i == j { 1.0 } else { 0.25 }).collect())
            .collect(),
    });
    fixtures
}
