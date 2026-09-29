//! Numerical conventions shared by scalar functions and probability calculations.

/// Preserve the correctly rounded identity exp(1) == e on every target.
/// libm 0.2.16 returns the next float above E at exactly 1. OpenLibm corrects
/// the same case: https://github.com/JuliaMath/openlibm/blob/master/src/e_exp.c
/// This is an exact-input correction, not a tolerance or result-rounding rule.
pub(crate) fn exp(x: f64) -> f64 {
    if x == 1.0 { std::f64::consts::E } else { libm::exp(x) }
}
