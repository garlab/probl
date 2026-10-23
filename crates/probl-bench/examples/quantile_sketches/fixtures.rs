use super::candidates::Random;

pub const NAMES: &[&str] = &[
    "uniform",
    "ascending",
    "descending",
    "normal",
    "lognormal",
    "pareto",
    "all_zero",
    "zero_atom",
    "repeated",
    "extremes",
    "retirement_30",
    "retirement_100",
];

fn normal(random: &mut Random) -> f64 {
    libm::sqrt(-2.0 * libm::log(random.unit())) * libm::cos(std::f64::consts::TAU * random.unit())
}

pub fn generate(name: &str, count: usize) -> Vec<f64> {
    let mut random = Random::new(0x4649_5854_5552_4501);
    let mut values = Vec::with_capacity(count);
    for i in 0..count {
        let value = match name {
            "uniform" | "ascending" | "descending" => random.unit(),
            "normal" => normal(&mut random),
            "lognormal" => libm::exp(3.0 * normal(&mut random)),
            "pareto" => libm::pow(random.unit(), -1.0 / 1.1),
            "all_zero" => 0.0,
            "zero_atom" => {
                if i % 10 == 0 {
                    libm::exp(3.0 * normal(&mut random))
                } else {
                    0.0
                }
            }
            "repeated" => [-3.0, -1.0, 0.0, 1.0, 7.0][(random.next() % 5) as usize],
            "extremes" => {
                let values = [
                    -f64::MAX,
                    -1e300,
                    -1e-300,
                    -f64::from_bits(1),
                    0.0,
                    f64::from_bits(1),
                    1e-300,
                    1e300,
                    f64::MAX,
                ];
                values[(random.next() % values.len() as u64) as usize]
            }
            "retirement_30" | "retirement_100" => {
                // Independent reference population with the example's recurrence
                // and assumptions, NOT Probl's random stream or model execution.
                let years = if name == "retirement_30" { 30 } else { 100 };
                let mut balance = 1_000_000.0;
                for _ in 0..years {
                    let r = 0.04 + 0.18 * normal(&mut random);
                    let q = 0.02 + 0.015 * normal(&mut random);
                    balance = (balance * (1.0 + r.max(-0.99)) / (1.0 + q.max(-0.99)) - 30_000.0).max(0.0);
                }
                balance
            }
            _ => panic!("unknown fixture {name}"),
        };
        assert!(value.is_finite());
        values.push(value);
    }
    if name == "ascending" || name == "descending" {
        values.sort_unstable_by(f64::total_cmp);
        if name == "descending" {
            values.reverse();
        }
    }
    values
}

/// Distance to the entire valid rank interval of a returned value. Measuring
/// only F(x) would wrongly penalize correct answers inside repeated-value atoms.
pub fn rank_error(sorted: &[f64], q: f64, value: f64) -> f64 {
    let lower = sorted.partition_point(|&v| v < value) as f64 / sorted.len() as f64;
    let upper = sorted.partition_point(|&v| v <= value) as f64 / sorted.len() as f64;
    (lower - q).max(q - upper).max(0.0)
}

pub fn exact_quantile(sorted: &[f64], q: f64) -> f64 {
    let index = ((q * sorted.len() as f64).ceil() as usize).saturating_sub(1);
    sorted[index.min(sorted.len() - 1)]
}
