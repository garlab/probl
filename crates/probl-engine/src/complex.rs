//! Finite complex scalars. These are data, never probability weights.
//!
//! Keeping the numerical kernel independent of Value and the interpreter lets
//! a future quantum register use dense arrays of the same scalars.

use crate::error::{OpError, OpResult};
use std::f64::consts::{FRAC_PI_2, LN_2, LN_10, SQRT_2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Complex {
    re: f64,
    im: f64,
}

impl Complex {
    pub fn new(re: f64, im: f64) -> OpResult<Self> {
        if !re.is_finite() || !im.is_finite() {
            return Err(OpError::new("complex components and results must be finite"));
        }
        // Equality and world merging identify signed zeros. Branch cuts use
        // the side selected by positive zero in each input component.
        Ok(Self {
            re: if re == 0.0 { 0.0 } else { re },
            im: if im == 0.0 { 0.0 } else { im },
        })
    }

    pub fn re(self) -> f64 {
        self.re
    }

    pub fn im(self) -> f64 {
        self.im
    }

    pub fn conjugate(self) -> Self {
        Self {
            im: if self.im == 0.0 { 0.0 } else { -self.im },
            ..self
        }
    }

    pub fn negated(self) -> Self {
        Self {
            re: if self.re == 0.0 { 0.0 } else { -self.re },
            im: if self.im == 0.0 { 0.0 } else { -self.im },
        }
    }

    pub fn plus(self, other: Self) -> OpResult<Self> {
        Self::new(self.re + other.re, self.im + other.im)
    }

    pub fn minus(self, other: Self) -> OpResult<Self> {
        Self::new(self.re - other.re, self.im - other.im)
    }

    pub fn times(self, other: Self) -> OpResult<Self> {
        let (re, er) = products(self.re, other.re, -self.im, other.im);
        let (im, ei) = products(self.re, other.im, self.im, other.re);
        Self::new(libm::scalbn(re, er), libm::scalbn(im, ei))
    }

    pub fn divided_by(self, other: Self) -> OpResult<Self> {
        if other.re == 0.0 && other.im == 0.0 {
            return Err(OpError::new("division by zero"));
        }
        let (den, ed) = products(other.re, other.re, other.im, other.im);
        let (re, er) = products(self.re, other.re, self.im, other.im);
        let (im, ei) = products(self.im, other.re, -self.re, other.im);
        Self::new(libm::scalbn(re / den, er - ed), libm::scalbn(im / den, ei - ed))
    }

    pub fn powi(self, exponent: i64) -> OpResult<Self> {
        let one = Self { re: 1.0, im: 0.0 };
        let mut base = if exponent < 0 { one.divided_by(self)? } else { self };
        let mut n = exponent.unsigned_abs();
        let mut result = one;
        // At most 64 iterations, even for i64::MIN.
        while n != 0 {
            if n & 1 != 0 {
                result = result.times(base)?;
            }
            n >>= 1;
            if n != 0 {
                base = base.times(base)?;
            }
        }
        Ok(result)
    }

    pub fn pow_integer(self, exponent: &probl_number::Integer) -> OpResult<Self> {
        if let Some(n) = exponent.to_i64() {
            return self.powi(n);
        }
        let one = Self { re: 1.0, im: 0.0 };
        let mut base = if exponent.is_negative() {
            one.divided_by(self)?
        } else {
            self
        };
        let mut result = one;
        for i in 0..exponent.bits() {
            if exponent.magnitude_bit(i) {
                result = result.times(base)?;
            }
            if i + 1 < exponent.bits() {
                base = base.times(base)?;
            }
        }
        Ok(result)
    }

    pub fn abs(self) -> f64 {
        libm::hypot(self.re, self.im)
    }

    pub fn abs2(self) -> f64 {
        let (m, e) = products(self.re, self.re, self.im, self.im);
        libm::scalbn(m, e)
    }

    pub fn arg(self) -> f64 {
        libm::atan2(self.im, self.re)
    }

    /// Principal square root: nonnegative real part, and +i on the negative
    /// real axis. Scale before taking the norm so even MAX + MAX*i works.
    pub fn sqrt(self) -> OpResult<Self> {
        let m = self.re.abs().max(self.im.abs());
        if m == 0.0 {
            return Ok(self);
        }
        let x = self.re / m;
        let y = self.im / m;
        let t = libm::sqrt(m) * libm::sqrt((libm::hypot(x, y) + x.abs()) / 2.0);
        if self.re >= 0.0 {
            Self::new(t, self.im / (2.0 * t))
        } else {
            Self::new(self.im.abs() / (2.0 * t), t.copysign(self.im))
        }
    }

    /// Principal cube root; unlike the real cbrt, the negative axis has phase pi/3.
    pub fn cbrt(self) -> OpResult<Self> {
        if self.im == 0.0 && self.re >= 0.0 {
            return Self::new(libm::cbrt(self.re), 0.0);
        }
        let r = crate::math::exp(log_hypot(self.re, self.im) / 3.0);
        let theta = self.arg() / 3.0;
        Self::new(r * libm::cos(theta), r * libm::sin(theta))
    }

    pub fn ln(self) -> OpResult<Self> {
        if self.re == 0.0 && self.im == 0.0 {
            return Err(OpError::new("`ln` isn't defined for complex zero"));
        }
        Self::new(log_hypot(self.re, self.im), self.arg())
    }

    pub fn log2(self) -> OpResult<Self> {
        let z = self.ln()?;
        Self::new(z.re / LN_2, z.im / LN_2)
    }

    pub fn log10(self) -> OpResult<Self> {
        let z = self.ln()?;
        Self::new(z.re / LN_10, z.im / LN_10)
    }

    pub fn log1p(self) -> OpResult<Self> {
        let x = self.re;
        let y = self.im;
        if x.abs() < 0.5 && y.abs() < 0.5 {
            // Keep the low bits that forming 1 + z would discard.
            Self::new(0.5 * libm::log1p(x * (2.0 + x) + y * y), libm::atan2(y, 1.0 + x))
        } else {
            Self::new(1.0 + x, y)?.ln()
        }
    }

    pub fn exp(self) -> OpResult<Self> {
        Self::new(
            exp_times(self.re, libm::cos(self.im)),
            exp_times(self.re, libm::sin(self.im)),
        )
    }

    pub fn exp2(self) -> OpResult<Self> {
        let theta = self.im * LN_2;
        let component = |factor| {
            if factor == 0.0 {
                0.0
            } else if self.re > 1000.0 {
                (libm::exp2(1000.0) * factor) * libm::exp2(self.re - 1000.0)
            } else {
                libm::exp2(self.re) * factor
            }
        };
        Self::new(component(libm::cos(theta)), component(libm::sin(theta)))
    }

    pub fn expm1(self) -> OpResult<Self> {
        if self.re.abs() < 0.5 && self.im.abs() < 0.5 {
            let s = libm::sin(self.im / 2.0);
            Self::new(
                libm::expm1(self.re) * libm::cos(self.im) - 2.0 * s * s,
                crate::math::exp(self.re) * libm::sin(self.im),
            )
        } else {
            let z = self.exp()?;
            Self::new(z.re - 1.0, z.im)
        }
    }

    pub fn sin(self) -> OpResult<Self> {
        Self::new(
            cosh_times(self.im, libm::sin(self.re)),
            sinh_times(self.im, libm::cos(self.re)),
        )
    }

    pub fn cos(self) -> OpResult<Self> {
        Self::new(
            cosh_times(self.im, libm::cos(self.re)),
            -sinh_times(self.im, libm::sin(self.re)),
        )
    }

    pub fn tan(self) -> OpResult<Self> {
        let s = libm::sin(self.re);
        let c = libm::cos(self.re);
        if self.im.abs() > 20.0 {
            // Divide the double-angle formula by exp(2*|y|). Taking sin/cos
            // of x first also avoids overflow when forming the angle 2*x.
            let t = crate::math::exp(-2.0 * self.im.abs());
            let den = 1.0 + 2.0 * (c * c - s * s) * t + t * t;
            Self::new(4.0 * s * c * t / den, ((1.0 - t * t) / den).copysign(self.im))
        } else {
            let sh = libm::sinh(self.im);
            let den = c * c + sh * sh;
            Self::new(s * c / den, sh * libm::cosh(self.im) / den)
        }
    }

    pub fn sinh(self) -> OpResult<Self> {
        Self::new(
            sinh_times(self.re, libm::cos(self.im)),
            cosh_times(self.re, libm::sin(self.im)),
        )
    }

    pub fn cosh(self) -> OpResult<Self> {
        Self::new(
            cosh_times(self.re, libm::cos(self.im)),
            sinh_times(self.re, libm::sin(self.im)),
        )
    }

    pub fn tanh(self) -> OpResult<Self> {
        let z = Self {
            re: self.im,
            im: self.re,
        }
        .tan()?;
        Self::new(z.im, z.re)
    }

    pub fn asin(self) -> OpResult<Self> {
        self.inverse_sin_cos(false)
    }

    pub fn acos(self) -> OpResult<Self> {
        self.inverse_sin_cos(true)
    }

    fn inverse_sin_cos(self, cosine: bool) -> OpResult<Self> {
        let x = self.re.abs();
        let y = self.im.abs();
        let (d, im) = if x.max(y) > 1e150 {
            // The omitted terms are O(1/|z|^2); avoid squaring huge values.
            (y, log_hypot(x, y) + LN_2)
        } else {
            let r = libm::hypot(x + 1.0, y);
            let s = libm::hypot(x - 1.0, y);
            let a = r / 2.0 + s / 2.0;
            // a = max(x, 1) + correction. Rationalize the hypot differences
            // and compute sqrt(correction) directly, preserving tiny y even
            // when y*y underflows or a rounds to 1 (including near z = +/-1).
            let correction_root = if y == 0.0 {
                0.0
            } else {
                libm::hypot(y / libm::sqrt(r + x + 1.0), y / libm::sqrt(s + (x - 1.0).abs())) / SQRT_2
            };
            let amx_root = libm::hypot(libm::sqrt((1.0 - x).max(0.0)), correction_root);
            let am1_root = libm::hypot(libm::sqrt((x - 1.0).max(0.0)), correction_root);
            (libm::sqrt(a + x) * amx_root, 2.0 * libm::asinh(am1_root / SQRT_2))
        };
        if cosine {
            Self::new(libm::atan2(d, self.re), -im.copysign(self.im))
        } else {
            Self::new(libm::atan2(self.re, d), im.copysign(self.im))
        }
    }

    pub fn asinh(self) -> OpResult<Self> {
        // Keep signs during internal rotations; only public results canonicalize
        // zero. In particular, do not switch the side of an inverse's cut.
        let z = Self {
            re: -self.im,
            im: self.re,
        }
        .asin()?;
        Self::new(z.im, -z.re)
    }

    pub fn acosh(self) -> OpResult<Self> {
        let z = self.acos()?;
        Self::new(z.im.abs(), z.re.copysign(self.im))
    }

    pub fn atanh(self) -> OpResult<Self> {
        let x = self.re.abs();
        let y = self.im;
        if x == 1.0 && y == 0.0 {
            return Err(OpError::new("`atanh` isn't defined at complex +1 or -1"));
        }
        let m = x.max(y.abs());
        if m > 1e150 {
            let rx = x / m;
            let ry = y / m;
            return Self::new(
                ((rx / (rx * rx + ry * ry)) / m).copysign(self.re),
                FRAC_PI_2.copysign(y),
            );
        }
        let h = libm::hypot(1.0 - x, y);
        let q = (4.0 * x / h) / h;
        let re = if q.is_finite() {
            0.25 * libm::log1p(q)
        } else {
            0.5 * (log_hypot(1.0 + x, y) - log_hypot(1.0 - x, y))
        };
        Self::new(
            re.copysign(self.re),
            0.5 * libm::atan2(2.0 * y, (1.0 - x) * (1.0 + x) - y * y),
        )
    }

    pub fn atan(self) -> OpResult<Self> {
        let z = Self {
            re: -self.im,
            im: self.re,
        }
        .atanh()?;
        Self::new(z.im, -z.re)
    }
}

/// ln(hypot(x,y)) without overflowing the norm or losing tiny offsets from 1.
fn log_hypot(x: f64, y: f64) -> f64 {
    let m = x.abs().max(y.abs());
    let n = x.abs().min(y.abs());
    if (0.5..1.5).contains(&m) {
        0.5 * libm::log1p((m - 1.0) * (m + 1.0) + n * n)
    } else if m == 0.0 {
        f64::NEG_INFINITY
    } else {
        let r = n / m;
        libm::log(m) + 0.5 * libm::log1p(r * r)
    }
}

/// exp(x)*factor, with |factor| <= 1. The modulus can overflow even when
/// both final components fit, e.g. exp(710 + pi/4*i).
fn exp_times(x: f64, factor: f64) -> f64 {
    if factor == 0.0 {
        0.0
    } else if x > 700.0 {
        (crate::math::exp(700.0) * factor) * crate::math::exp(x - 700.0)
    } else {
        crate::math::exp(x) * factor
    }
}

fn cosh_times(x: f64, factor: f64) -> f64 {
    if x.abs() > 20.0 {
        exp_times(x.abs() - LN_2, factor)
    } else {
        libm::cosh(x) * factor
    }
}

fn sinh_times(x: f64, factor: f64) -> f64 {
    if x.abs() > 20.0 {
        exp_times(x.abs() - LN_2, factor) * x.signum()
    } else {
        libm::sinh(x) * factor
    }
}

/// a*b + c*d as mantissa * 2^exponent. Separate exponents avoid overflowing
/// or underflowing intermediate products, notably the denominator of division.
/// Scale each product separately so disparate components (1e308 + 1e-308 i)
/// survive multiplication by one.
fn products(a: f64, b: f64, c: f64, d: f64) -> (f64, i32) {
    let product = |x, y| {
        let (x, ex) = libm::frexp(x);
        let (y, ey) = libm::frexp(y);
        (x * y, ex + ey)
    };
    let (x, ex) = product(a, b);
    let (y, ey) = product(c, d);
    if x == 0.0 {
        return (y, ey);
    }
    if y == 0.0 {
        return (x, ex);
    }
    let e = ex.max(ey);
    (libm::scalbn(x, ex - e) + libm::scalbn(y, ey - e), e)
}
