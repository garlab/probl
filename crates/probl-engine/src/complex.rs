//! Finite complex scalars. These are data, never probability weights.
//!
//! Keeping the numerical kernel independent of Value and the interpreter lets
//! a future quantum register use dense arrays of the same scalars.

use crate::error::{OpError, OpResult};

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
        // Equality and world merging identify signed zeros. Phase and any
        // future branch cuts must consequently use one canonical side.
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
