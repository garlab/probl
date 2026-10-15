//! Primality and factorization, charging their work as they go.
//!
//! Integers below 2⁶⁴ use machine arithmetic: trial division by the primes
//! below 1000, Miller–Rabin with bases proven to recognize every composite
//! below 2⁶⁴, and Pollard's rho method with Brent's cycle detection. Larger
//! integers take the same steps on bigints, and are checked for perfect
//! powers, which rho can't split. Beyond 3.3 × 10²⁴, where no set of
//! Miller–Rabin bases is proven, a strong Lucas test completes the
//! Baillie–PSW test: no composite is known to pass it, though none is proven
//! not to.

use crate::Integer;
use num_bigint::{BigInt, BigUint};
use num_traits::{One, ToPrimitive, Zero};

/// Spends units of work: one for each machine-word multiplication or
/// division, and the product of the operands' word counts for a bigint one.
pub type Work<'a, E> = &'a mut dyn FnMut(u64) -> Result<(), E>;

/// Whether `n` is prime. Negative numbers, 0 and 1 aren't.
pub fn is_prime<E>(n: &Integer, work: Work<E>) -> Result<bool, E> {
    if n.is_negative() {
        return Ok(false);
    }
    if let Some(n) = n.to_u64() {
        if n < 2 {
            return Ok(false);
        }
        for p in SMALL_PRIMES.iter().map(|&p| u64::from(p)) {
            if p * p > n {
                return Ok(true);
            }
            work(1)?;
            if n % p == 0 {
                return Ok(n == p);
            }
        }
        return miller_rabin(n, work);
    }
    let n = n.big();
    let n = n.magnitude();
    for p in SMALL_PRIMES {
        work(words(n))?;
        if (n % p).is_zero() {
            return Ok(false);
        }
    }
    probable_prime(n, work)
}

/// Whether no prime divides `n` more than once. 0 is divisible by every
/// square; the sign doesn't matter.
pub fn is_squarefree<E>(n: &Integer, work: Work<E>) -> Result<bool, E> {
    if n.is_zero() {
        return Ok(false);
    }
    let mut f = Factorizer::new(work, true);
    f.factor(n)?;
    Ok(!f.square && f.finish().iter().all(|&(_, k)| k == 1))
}

/// The primes dividing the magnitude of `n`, which mustn't be 0, in
/// increasing order, each with its multiplicity.
pub fn factorize<E>(n: &Integer, work: Work<E>) -> Result<Vec<(Integer, u32)>, E> {
    debug_assert!(!n.is_zero(), "0 has no factorization");
    let mut f = Factorizer::new(work, false);
    f.factor(n)?;
    Ok(f.finish())
}

/// The divisors of the number with this factorization, in increasing order.
/// There's one for each choice of exponents, so the caller should check
/// their count first.
pub fn divisors(factors: &[(Integer, u32)]) -> Vec<Integer> {
    let mut divisors = vec![Integer::ONE];
    for (p, k) in factors {
        let smaller = divisors.len();
        let mut power = Integer::ONE;
        for _ in 0..*k {
            power = power.mul(p).expect("a divisor is no larger than its multiple");
            for i in 0..smaller {
                let d = divisors[i]
                    .mul(&power)
                    .expect("a divisor is no larger than its multiple");
                divisors.push(d);
            }
        }
    }
    divisors.sort_unstable();
    divisors
}

/// The smallest prime above `n`, or `None` if it's past the integer size
/// ceiling.
pub fn next_prime<E>(n: &Integer, work: Work<E>) -> Result<Option<Integer>, E> {
    if *n < 2 {
        return Ok(Some(2.into()));
    }
    let Ok(mut candidate) = n.add(&Integer::from(if n.is_odd() { 2 } else { 1 })) else {
        return Ok(None);
    };
    loop {
        work(1)?;
        if is_prime(&candidate, work)? {
            return Ok(Some(candidate));
        }
        match candidate.add(&Integer::from(2)) {
            Ok(next) => candidate = next,
            Err(_) => return Ok(None),
        }
    }
}

/// The largest prime below `n`, or `None` if `n` is 2 or less.
pub fn prev_prime<E>(n: &Integer, work: Work<E>) -> Result<Option<Integer>, E> {
    if *n <= 3 {
        return Ok((*n == 3).then(|| 2.into()));
    }
    let step = |n: &Integer, by: i64| n.sub(&by.into()).expect("a smaller positive integer");
    let mut candidate = step(n, if n.is_odd() { 2 } else { 1 });
    // The search ends at 3 at the latest.
    loop {
        work(1)?;
        if is_prime(&candidate, work)? {
            return Ok(Some(candidate));
        }
        candidate = step(&candidate, 2);
    }
}

/// The primes from `lo` to `hi`, both included, in increasing order, found
/// as they're asked for. A range at least as wide as √hi is sieved a segment
/// at a time, with the primes up to √hi; a narrower one, or one beyond the
/// sieve's reach, is searched by testing each odd number.
pub struct Primes {
    /// The next number to look at, or `None` once past `hi`.
    next: Option<Integer>,
    hi: Integer,
    sieve: bool,
    /// The primes up to √hi, once the sieve needs them.
    base: Vec<u32>,
    /// Primes found by the sieve and not yet asked for.
    found: std::vec::IntoIter<u64>,
}

/// The largest √hi the sieve takes base primes up to: their sieve takes
/// 16 MB.
const MAX_BASE: u64 = 1 << 24;

/// Numbers sieved at a time.
const SEGMENT: u64 = 1 << 16;

impl Primes {
    pub fn new(lo: &Integer, hi: &Integer) -> Self {
        let lo = if *lo < 2 { 2.into() } else { lo.clone() };
        let sieve = match (lo.to_u64(), hi.to_u64()) {
            (Some(lo), Some(hi)) if lo <= hi => {
                let root = hi.isqrt();
                root <= MAX_BASE && root <= hi - lo + 1
            }
            _ => false,
        };
        Primes {
            next: (lo <= *hi).then_some(lo),
            hi: hi.clone(),
            sieve,
            base: Vec::new(),
            found: Vec::new().into_iter(),
        }
    }

    /// The next prime in the range, or `None` after the last.
    pub fn next<E>(&mut self, work: Work<E>) -> Result<Option<Integer>, E> {
        loop {
            if let Some(p) = self.found.next() {
                return Ok(Some(p.into()));
            }
            let Some(n) = self.next.take() else {
                return Ok(None);
            };
            if self.sieve {
                self.sieve_segment(&n, work)?;
                continue;
            }
            // Past 2, only odd numbers can be prime.
            let step = Integer::from(if n == 2 || !n.is_odd() { 1 } else { 2 });
            self.next = n.add(&step).ok().filter(|next| *next <= self.hi);
            work(1)?;
            if (n.is_odd() || n == 2) && is_prime(&n, work)? {
                return Ok(Some(n));
            }
        }
    }

    /// Sieves the segment starting at `lo`, and moves past it.
    fn sieve_segment<E>(&mut self, lo: &Integer, work: Work<E>) -> Result<(), E> {
        let lo = lo.to_u64().expect("a sieved range fits in a word");
        let hi = self.hi.to_u64().expect("a sieved range fits in a word");
        if self.base.is_empty() {
            let root = hi.isqrt();
            work(root.max(1))?;
            self.base = sieve_up_to(root);
        }
        let end = hi.min(lo + SEGMENT - 1);
        work(end - lo + 1)?;
        let mut composite = vec![false; (end - lo + 1) as usize];
        let mut marks = 0;
        for p in self.base.iter().map(|&p| u64::from(p)) {
            if p * p > end {
                break;
            }
            let mut multiple = (p * p).max(lo.div_ceil(p) * p);
            while multiple <= end {
                composite[(multiple - lo) as usize] = true;
                multiple += p;
                marks += 1;
            }
        }
        work(marks)?;
        let found: Vec<u64> = (lo..=end).filter(|&n| !composite[(n - lo) as usize]).collect();
        self.found = found.into_iter();
        self.next = (end < hi).then(|| (end + 1).into());
        Ok(())
    }
}

/// The primes up to `n`, at most [`MAX_BASE`], by the sieve of Eratosthenes.
fn sieve_up_to(n: u64) -> Vec<u32> {
    let mut composite = vec![false; n as usize + 1];
    let mut primes = Vec::new();
    for i in 2..=n {
        if !composite[i as usize] {
            primes.push(i as u32);
            let mut multiple = i * i;
            while multiple <= n {
                composite[multiple as usize] = true;
                multiple += i;
            }
        }
    }
    primes
}

const SMALL_PRIMES: [u32; 168] = small_primes();

/// A number below this with no prime factor below 1000 is prime: the
/// smallest composite without one is 1009².
const TRIAL_BOUND: u64 = 1009 * 1009;

const fn small_primes() -> [u32; 168] {
    let mut primes = [0; 168];
    let (mut count, mut n) = (0, 2);
    while n < 1000 {
        let mut i = 0;
        while i < count && primes[i] * primes[i] <= n && n % primes[i] != 0 {
            i += 1;
        }
        if i == count || primes[i] * primes[i] > n {
            primes[count] = n;
            count += 1;
        }
        n += 1;
    }
    primes
}

/// The first primes, as Miller–Rabin bases.
const BASES: [u32; 13] = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41];

/// The smallest composite that passes Miller–Rabin with each count of the
/// first bases; below it, that many bases recognize every composite
/// (Jaeschke 1993; Jiang and Deng 2014; Sorenson and Webster 2015).
const PROVEN: [(u128, usize); 10] = [
    (2_047, 1),
    (1_373_653, 2),
    (25_326_001, 3),
    (3_215_031_751, 4),
    (2_152_302_898_747, 5),
    (3_474_749_660_383, 6),
    (341_550_071_728_321, 7),
    (3_825_123_056_546_413_051, 9),
    (318_665_857_834_031_151_167_461, 12),
    (3_317_044_064_679_887_385_961_981, 13),
];

/// How many of the first bases decide `n`, if any set of them is proven to.
fn proven_bases(n: u128) -> Option<usize> {
    PROVEN.iter().find(|&&(bound, _)| n < bound).map(|&(_, count)| count)
}

struct Factorizer<'a, E> {
    work: Work<'a, E>,
    primes: Vec<(Integer, u32)>,
    /// Stop at the first prime known to divide more than once, which is
    /// enough for `is_squarefree`.
    squarefree_only: bool,
    square: bool,
}

impl<'a, E> Factorizer<'a, E> {
    fn new(work: Work<'a, E>, squarefree_only: bool) -> Self {
        Factorizer {
            work,
            primes: Vec::new(),
            squarefree_only,
            square: false,
        }
    }

    fn done(&self) -> bool {
        self.squarefree_only && self.square
    }

    fn push(&mut self, p: Integer, k: u32) {
        self.square |= k > 1;
        self.primes.push((p, k));
    }

    /// The primes in increasing order, the same primes found apart merged.
    fn finish(mut self) -> Vec<(Integer, u32)> {
        self.primes.sort_unstable();
        let mut merged: Vec<(Integer, u32)> = Vec::with_capacity(self.primes.len());
        for (p, k) in self.primes {
            match merged.last_mut() {
                Some((q, j)) if *q == p => *j += k,
                _ => merged.push((p, k)),
            }
        }
        merged
    }

    fn factor(&mut self, n: &Integer) -> Result<(), E> {
        let n = n.abs();
        if let Some(n) = n.to_u64() {
            return self.factor_small(n);
        }
        let mut n = n.big().magnitude().clone();
        for p in SMALL_PRIMES {
            if self.done() {
                return Ok(());
            }
            if let Some(n) = n.to_u64() {
                return self.factor_small(n);
            }
            (self.work)(words(&n))?;
            if (&n % p).is_zero() {
                let mut k = 0;
                while (&n % p).is_zero() {
                    (self.work)(words(&n))?;
                    n /= p;
                    k += 1;
                }
                self.push(p.into(), k);
            }
        }
        self.split_big(n, 1)
    }

    fn factor_small(&mut self, mut n: u64) -> Result<(), E> {
        for p in SMALL_PRIMES.iter().map(|&p| u64::from(p)) {
            if p * p > n || self.done() {
                break;
            }
            (self.work)(1)?;
            if n % p == 0 {
                let mut k = 0;
                while n % p == 0 {
                    n /= p;
                    k += 1;
                }
                (self.work)(u64::from(k))?;
                self.push(p.into(), k);
            }
        }
        self.split_small(n, 1)
    }

    /// Factors `n`, which has no prime factor below 1000, each `k` times.
    fn split_small(&mut self, n: u64, k: u32) -> Result<(), E> {
        if n == 1 || self.done() {
            return Ok(());
        }
        if n < TRIAL_BOUND || miller_rabin(n, self.work)? {
            self.push(n.into(), k);
            return Ok(());
        }
        let d = rho_small(n, self.work)?;
        self.split_small(d, k)?;
        self.split_small(n / d, k)
    }

    /// Factors `n`, which has no prime factor below 1000, each `k` times.
    fn split_big(&mut self, n: BigUint, k: u32) -> Result<(), E> {
        if let Some(n) = n.to_u64() {
            return self.split_small(n, k);
        }
        if self.done() {
            return Ok(());
        }
        if probable_prime(&n, self.work)? {
            let p = Integer::from_big(BigInt::from(n)).expect("a factor is no larger than its multiple");
            self.push(p, k);
            return Ok(());
        }
        if let Some((root, e)) = perfect_power(&n, self.work)? {
            self.square = true;
            return self.split_big(root, k * e);
        }
        let d = rho_big(&n, self.work)?;
        let rest = &n / &d;
        self.split_big(d, k)?;
        self.split_big(rest, k)
    }
}

// ── Machine words ─────────────────────────────────────────────────────────

fn mul_mod(a: u64, b: u64, m: u64) -> u64 {
    (u128::from(a) * u128::from(b) % u128::from(m)) as u64
}

pub(crate) fn pow_mod(mut base: u64, mut exponent: u64, m: u64) -> u64 {
    let mut result = 1 % m;
    base %= m;
    while exponent > 0 {
        if exponent & 1 == 1 {
            result = mul_mod(result, base, m);
        }
        base = mul_mod(base, base, m);
        exponent >>= 1;
    }
    result
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Whether `n` passes Miller–Rabin to base `a`, as every prime does.
fn strong_probable_prime(n: u64, a: u64) -> bool {
    let s = (n - 1).trailing_zeros();
    let mut x = pow_mod(a, (n - 1) >> s, n);
    if x == 1 || x == n - 1 {
        return true;
    }
    for _ in 1..s {
        x = mul_mod(x, x, n);
        if x == n - 1 {
            return true;
        }
    }
    false
}

/// Whether odd `n`, above 1000, is prime: exact, with the bases proven for it.
fn miller_rabin<E>(n: u64, work: Work<E>) -> Result<bool, E> {
    let count = proven_bases(u128::from(n)).expect("2⁶⁴ is below a proven bound");
    for a in &BASES[..count] {
        work(2 * u64::from(64 - n.leading_zeros()))?;
        if !strong_probable_prime(n, u64::from(*a)) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Products of differences taken at once by rho, before one gcd.
const BATCH: u64 = 128;

/// A proper divisor of odd composite `n`, by Pollard's rho method with
/// Brent's cycle detection.
fn rho_small<E>(n: u64, work: Work<E>) -> Result<u64, E> {
    let mut c = 0;
    loop {
        c += 1;
        let f = |x: u64| ((u128::from(x) * u128::from(x) + c) % u128::from(n)) as u64;
        let (mut x, mut y, mut ys, mut q, mut g, mut r) = (2, 2, 2, 1, 1, 1);
        while g == 1 {
            x = y;
            work(r)?;
            for _ in 0..r {
                y = f(y);
            }
            let mut k = 0;
            while k < r && g == 1 {
                ys = y;
                let steps = BATCH.min(r - k);
                work(2 * steps + 1)?;
                for _ in 0..steps {
                    y = f(y);
                    q = mul_mod(q, x.abs_diff(y), n);
                }
                g = gcd(q, n);
                k += steps;
            }
            r *= 2;
        }
        if g == n {
            // The batch overshot: retrace it a step at a time.
            loop {
                work(3)?;
                ys = f(ys);
                g = gcd(x.abs_diff(ys), n);
                if g > 1 {
                    break;
                }
            }
        }
        if g != n {
            return Ok(g);
        }
    }
}

// ── Bigints ───────────────────────────────────────────────────────────────

fn words(n: &BigUint) -> u64 {
    n.bits().div_ceil(64).max(1)
}

/// Whether odd `n`, above 2⁶⁴ and without a prime factor below 1000, is
/// prime: Miller–Rabin to the first 13 bases, which is exact below
/// 3.3 × 10²⁴, and above it a strong Lucas test too.
fn probable_prime<E>(n: &BigUint, work: Work<E>) -> Result<bool, E> {
    let w = words(n);
    let n_minus_1 = n - 1u32;
    let s = n_minus_1.trailing_zeros().expect("n is above 1");
    let d = &n_minus_1 >> s;
    'bases: for a in BASES {
        work(2 * n.bits() * w * w)?;
        let mut x = BigUint::from(a).modpow(&d, n);
        if x.is_one() || x == n_minus_1 {
            continue;
        }
        for _ in 1..s {
            x = &x * &x % n;
            if x == n_minus_1 {
                continue 'bases;
            }
        }
        return Ok(false);
    }
    if n.to_u128().and_then(proven_bases).is_some() {
        return Ok(true);
    }
    strong_lucas(n, work)
}

/// The strong Lucas probable-prime test, with Selfridge's parameters: P = 1
/// and the first D of 5, −7, 9, −11, … whose Jacobi symbol (D/n) is −1.
/// `n` is odd and above every D tried.
fn strong_lucas<E>(n: &BigUint, work: Work<E>) -> Result<bool, E> {
    let w = words(n);
    work(4 * w * w)?;
    let root = n.sqrt();
    if &root * &root == *n {
        // No D would do: (D/n) is never −1 for a square.
        return Ok(false);
    }
    let mut d: i64 = 5;
    loop {
        work(w)?;
        match jacobi(d, n) {
            -1 => break,
            0 => return Ok(false),
            _ => d = if d > 0 { -(d + 2) } else { -d + 2 },
        }
    }
    let modulo = |v: i64| {
        let m = BigUint::from(v.unsigned_abs()) % n;
        if v < 0 && !m.is_zero() { n - m } else { m }
    };
    let (big_d, big_q) = (modulo(d), modulo((1 - d) / 4));
    let half = |x: BigUint| if x.bit(0) { (x + n) >> 1 } else { x >> 1 };
    // V(2j) = V(j)² − 2Qʲ, the same for any Qʲ.
    let double = |v: &BigUint, qj: &BigUint| (v * v + n * 2u32 - ((qj << 1) % n)) % n;
    // n + 1 = m · 2ˢ with m odd; walk m's bits from the top for U(m), V(m), Qᵐ.
    let n_plus_1 = n + 1u32;
    let s = n_plus_1.trailing_zeros().expect("n + 1 is positive");
    let m = &n_plus_1 >> s;
    let (mut u, mut v, mut qj) = (BigUint::one(), BigUint::one(), big_q.clone());
    for i in (0..m.bits() - 1).rev() {
        work(8 * w * w)?;
        u = &u * &v % n;
        v = double(&v, &qj);
        qj = &qj * &qj % n;
        if m.bit(i) {
            // U(j + 1) = (U + V)/2 and V(j + 1) = (DU + V)/2, with P = 1.
            (u, v) = (half((&u + &v) % n), half((&big_d * &u + &v) % n));
            qj = &qj * &big_q % n;
        }
    }
    if u.is_zero() || v.is_zero() {
        return Ok(true);
    }
    for _ in 1..s {
        work(4 * w * w)?;
        v = double(&v, &qj);
        if v.is_zero() {
            return Ok(true);
        }
        qj = &qj * &qj % n;
    }
    Ok(false)
}

/// The Jacobi symbol (a/n), for odd `a` and odd positive `n`.
fn jacobi(a: i64, n: &BigUint) -> i64 {
    let n_mod_4 = n.iter_u32_digits().next().unwrap_or(0) & 3;
    let mut sign = 1;
    // (−1/n) is −1 when n is 3 mod 4.
    if a < 0 && n_mod_4 == 3 {
        sign = -sign;
    }
    // Reciprocity: (a/n) = (n/a), unless both are 3 mod 4.
    let a = a.unsigned_abs();
    if a & 3 == 3 && n_mod_4 == 3 {
        sign = -sign;
    }
    let (mut x, mut m) = ((n % a).to_u64().expect("below a"), a);
    while x != 0 {
        while x % 2 == 0 {
            x /= 2;
            if matches!(m % 8, 3 | 5) {
                sign = -sign;
            }
        }
        (x, m) = (m, x);
        if x & 3 == 3 && m & 3 == 3 {
            sign = -sign;
        }
        x %= m;
    }
    if m == 1 { sign } else { 0 }
}

/// A root and a prime exponent, if `n` is a perfect power. `n` has no prime
/// factor below 1000, so the exponent is at most its bit count over 9.97.
fn perfect_power<E>(n: &BigUint, work: Work<E>) -> Result<Option<(BigUint, u32)>, E> {
    let w = words(n);
    for e in SMALL_PRIMES.into_iter().take_while(|&e| u64::from(e) <= n.bits() / 9) {
        work(8 * w * w)?;
        let root = n.nth_root(e);
        if root.pow(e) == *n {
            return Ok(Some((root, e)));
        }
    }
    Ok(None)
}

fn abs_diff(a: &BigUint, b: &BigUint) -> BigUint {
    if a > b { a - b } else { b - a }
}

fn gcd_big<E>(mut a: BigUint, mut b: BigUint, work: Work<E>) -> Result<BigUint, E> {
    let w = words(&a).max(words(&b));
    while !b.is_zero() {
        work(w * w)?;
        let r = &a % &b;
        (a, b) = (b, r);
    }
    Ok(a)
}

/// [`rho_small`] on bigints.
fn rho_big<E>(n: &BigUint, work: Work<E>) -> Result<BigUint, E> {
    let w = words(n);
    let mut c = 0u32;
    loop {
        c += 1;
        let f = |x: &BigUint| (x * x + c) % n;
        let mut y = BigUint::from(2u32);
        let (mut x, mut ys, mut q, mut g, mut r) = (y.clone(), y.clone(), BigUint::one(), BigUint::one(), 1);
        while g.is_one() {
            x.clone_from(&y);
            work(r * 2 * w * w)?;
            for _ in 0..r {
                y = f(&y);
            }
            let mut k = 0;
            while k < r && g.is_one() {
                ys.clone_from(&y);
                let steps = BATCH.min(r - k);
                work(steps * 4 * w * w)?;
                for _ in 0..steps {
                    y = f(&y);
                    q = q * abs_diff(&x, &y) % n;
                }
                g = gcd_big(q.clone(), n.clone(), work)?;
                k += steps;
            }
            r *= 2;
        }
        if g == *n {
            loop {
                work(4 * w * w)?;
                ys = f(&ys);
                g = gcd_big(abs_diff(&x, &ys), n.clone(), work)?;
                if !g.is_one() {
                    break;
                }
            }
        }
        if g != *n {
            return Ok(g);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;

    fn free(_: u64) -> Result<(), Infallible> {
        Ok(())
    }

    fn int(s: &str) -> Integer {
        s.parse().unwrap()
    }

    fn prime(n: &Integer) -> bool {
        is_prime(n, &mut free).unwrap()
    }

    fn factors(n: &Integer) -> Vec<(Integer, u32)> {
        factorize(n, &mut free).unwrap()
    }

    fn squarefree(n: &Integer) -> bool {
        is_squarefree(n, &mut free).unwrap()
    }

    fn sieve(limit: usize) -> Vec<bool> {
        let mut prime = vec![true; limit];
        prime[0] = false;
        prime[1] = false;
        for i in 2..limit {
            if prime[i] {
                for j in (i * i..limit).step_by(i) {
                    prime[j] = false;
                }
            }
        }
        prime
    }

    #[test]
    fn small_primes_are_the_primes_below_1000() {
        let primes = sieve(1000);
        let expected: Vec<u32> = (0..1000).filter(|&n| primes[n as usize]).collect();
        assert_eq!(SMALL_PRIMES.to_vec(), expected);
    }

    #[test]
    fn primality_matches_a_sieve() {
        let primes = sieve(300_000);
        for (n, &expected) in primes.iter().enumerate() {
            assert_eq!(prime(&Integer::from(n)), expected, "{n}");
        }
        for n in [-1, -2, -7, i64::MIN] {
            assert!(!prime(&n.into()), "{n}");
        }
    }

    #[test]
    fn miller_rabin_bases_suffice_up_to_their_bounds() {
        // Each bound passes the bases before it and isn't prime, so it needs
        // one base more; the test must catch every one of them.
        for (bound, count) in PROVEN {
            let n = Integer::from(bound);
            assert!(!prime(&n), "{bound}");
            if let Ok(n) = u64::try_from(bound) {
                assert!(
                    BASES[..count].iter().all(|&a| strong_probable_prime(n, a.into())),
                    "{bound}"
                );
            }
            let product: Integer = factors(&n)
                .iter()
                .fold(Integer::ONE, |acc, (p, k)| acc.mul(&p.pow(*k).unwrap()).unwrap());
            assert_eq!(product, n);
        }
        for n in [
            "2305843009213693951",
            "18446744073709551557",
            "170141183460469231731687303715884105727",
        ] {
            assert!(prime(&int(n)), "{n}");
        }
        // The largest 64-bit prime squared, and the first above 2⁶⁴.
        assert!(!prime(&int("340282366920938461286658806734041124249")));
        assert!(prime(&int("18446744073709551629")));
    }

    #[test]
    fn strong_lucas_passes_primes_and_only_the_known_pseudoprimes() {
        // OEIS A217255, the strong Lucas pseudoprimes with Selfridge's
        // parameters, below 100,000.
        let pseudoprimes = [
            5459, 5777, 10877, 16109, 18971, 22499, 24569, 25199, 40309, 58519, 75077, 97439,
        ];
        let primes = sieve(100_000);
        for n in (101..100_000u32).step_by(2) {
            let passes = strong_lucas(&BigUint::from(n), &mut free).unwrap();
            assert_eq!(passes, primes[n as usize] || pseudoprimes.contains(&n), "{n}");
        }
    }

    #[test]
    fn factorizations_multiply_back_to_their_numbers() {
        for n in 1..20_000u32 {
            let found = factors(&n.into());
            assert!(found.windows(2).all(|w| w[0].0 < w[1].0), "{n}");
            assert!(found.iter().all(|(p, _)| prime(p)), "{n}");
            let product = found.iter().fold(1, |acc, (p, k)| acc * p.to_u64().unwrap().pow(*k));
            assert_eq!(product, u64::from(n));
        }
        assert_eq!(factors(&Integer::ONE), vec![]);
        assert_eq!(factors(&(-12).into()), vec![(2.into(), 2), (3.into(), 1)]);
        for (n, expected) in [
            // Euler's factor of the sixth Fermat number, and Cole's of 2⁶⁷ − 1.
            ("18446744073709551617", vec![("274177", 1), ("67280421310721", 1)]),
            ("147573952589676412927", vec![("193707721", 1), ("761838257287", 1)]),
            // Squares and cubes of primes too large for rho to split.
            (
                "5316911983139663487003542222693990401",
                vec![("2305843009213693951", 2)],
            ),
            (
                "510423550381407695195061911147652317181",
                vec![("3", 1), ("170141183460469231731687303715884105727", 1)],
            ),
            (
                "6277101735386680777106801733124266500526464379673737431189",
                vec![("18446744073709551629", 3)],
            ),
            ("18446744073709551616", vec![("2", 64)]),
            ("4611686014132420609", vec![("2147483647", 2)]),
        ] {
            let expected: Vec<_> = expected.into_iter().map(|(p, k)| (int(p), k)).collect();
            assert_eq!(factors(&int(n)), expected, "{n}");
        }
    }

    #[test]
    fn squarefree_numbers_have_no_repeated_prime() {
        for n in 1..20_000i64 {
            let expected = !(2..=n).take_while(|d| d * d <= n).any(|d| n % (d * d) == 0);
            assert_eq!(squarefree(&n.into()), expected, "{n}");
            assert_eq!(squarefree(&(-n).into()), expected, "{n}");
        }
        assert!(!squarefree(&Integer::ZERO));
        assert!(squarefree(&int("170141183460469231731687303715884105727")));
        assert!(squarefree(&int("147573952589676412927")));
        assert!(!squarefree(&int("5316911983139663487003542222693990401")));
        assert!(!squarefree(&int(
            "6277101735386680777106801733124266500526464379673737431189"
        )));
    }

    #[test]
    fn divisors_are_every_number_that_divides() {
        for n in 1..3000u64 {
            let expected: Vec<Integer> = (1..=n).filter(|d| n % d == 0).map(Integer::from).collect();
            assert_eq!(divisors(&factors(&n.into())), expected, "{n}");
        }
        let m61 = int("2305843009213693951");
        let square = m61.mul(&m61).unwrap();
        assert_eq!(divisors(&factors(&square)), vec![Integer::ONE, m61, square]);
    }

    fn primes_between(lo: &Integer, hi: &Integer, sieve: Option<bool>) -> Vec<Integer> {
        let mut primes = Primes::new(lo, hi);
        if let Some(sieve) = sieve {
            primes.sieve = sieve;
        }
        let mut found = Vec::new();
        while let Some(p) = primes.next(&mut free).unwrap() {
            found.push(p);
        }
        found
    }

    #[test]
    fn sieved_and_tested_ranges_list_the_same_primes() {
        let primes = sieve(400_000);
        for (lo, hi) in [
            (-5, 30),
            (0, 1),
            (2, 2),
            (3, 2),
            (24, 28),
            (0, 300_000),
            (65_530, 65_540),
            (131_071, 131_072),
            (399_000, 399_999),
            (399_900, 399_999),
        ] {
            let expected: Vec<Integer> = (lo.max(0)..=hi)
                .filter(|&n| primes[n as usize])
                .map(Integer::from)
                .collect();
            for sieve in [None, Some(true), Some(false)] {
                if sieve == Some(true) && lo > hi {
                    continue;
                }
                assert_eq!(
                    primes_between(&lo.into(), &hi.into(), sieve),
                    expected,
                    "{lo}..{hi} {sieve:?}"
                );
            }
        }
        let primes = |lo: &str, hi: &str| primes_between(&int(lo), &int(hi), None);
        let ints = |xs: &[&str]| xs.iter().map(|x| int(x)).collect::<Vec<_>>();
        assert_eq!(
            primes("18446744073709551516", "18446744073709551715"),
            ints(&[
                "18446744073709551521",
                "18446744073709551533",
                "18446744073709551557",
                "18446744073709551629",
                "18446744073709551653",
                "18446744073709551667",
                "18446744073709551697",
                "18446744073709551709",
            ])
        );
        assert_eq!(
            primes("100000000000000000000", "100000000000000000299"),
            ints(&[
                "100000000000000000039",
                "100000000000000000129",
                "100000000000000000151",
                "100000000000000000193",
                "100000000000000000207",
            ])
        );
    }

    #[test]
    fn next_and_previous_primes_skip_to_the_nearest() {
        let primes = sieve(20_000);
        for n in -5..19_000i64 {
            let next = (n + 1..).find(|&m| m >= 0 && primes[m as usize]).unwrap();
            assert_eq!(next_prime(&n.into(), &mut free).unwrap(), Some(next.into()), "{n}");
            let previous = (0..n.max(0)).rev().find(|&m| primes[m as usize]).map(Integer::from);
            assert_eq!(prev_prime(&n.into(), &mut free).unwrap(), previous, "{n}");
        }
        let (below, above) = (int("18446744073709551557"), int("18446744073709551629"));
        assert_eq!(next_prime(&below, &mut free).unwrap(), Some(above.clone()));
        assert_eq!(prev_prime(&above, &mut free).unwrap(), Some(below.clone()));
        assert_eq!(
            prev_prime(&int("18446744073709551616"), &mut free).unwrap(),
            Some(below)
        );
        let m127 = int("170141183460469231731687303715884105727");
        let before = m127.sub(&Integer::ONE).unwrap();
        assert_eq!(next_prime(&before, &mut free).unwrap(), Some(m127));
    }

    #[test]
    fn work_is_charged_and_stops_the_search() {
        let mut spent = 0;
        let mut budget = |units| {
            spent += units;
            if spent > 100 { Err(()) } else { Ok(()) }
        };
        assert_eq!(is_prime(&Integer::from(1_000_000_007), &mut budget), Err(()));
        let mut spent = 0;
        let mut count = |units| {
            spent += units;
            Ok::<_, Infallible>(())
        };
        factorize(&int("147573952589676412927"), &mut count).unwrap();
        assert!(spent > 1000, "{spent}");
    }
}
