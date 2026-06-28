//! Fixed-point math for the Onsnes posterior.
//!
//! Every value is an `i128` in 1e12 fixed-point (so `SCALE` == 1.0) unless a
//! comment says otherwise. The exp / log2 routines are deliberately small
//! polynomial approximations — accurate enough to drive a 256-bin posterior,
//! but NOT IEEE-grade. See the README for the compute-budget caveat.

/// 1.0 in fixed-point (1e12).
pub const SCALE: i128 = 1_000_000_000_000;

/// log2(e) * 1e12.
pub const LOG2E: i128 = 1_442_695_040_889;

/// e^-1 * 1e12.
pub const E_INV: i128 = 367_879_441_171;

/// (a * b) in fixed-point.
#[inline]
pub fn mul_fp(a: i128, b: i128) -> i128 {
    a.saturating_mul(b) / SCALE
}

/// (a / b) in fixed-point.
#[inline]
pub fn div_fp(a: i128, b: i128) -> i128 {
    if b == 0 {
        0
    } else {
        a.saturating_mul(SCALE) / b
    }
}

/// e^-x for x >= 0, returned in fixed-point (0, SCALE].
pub fn exp_neg_fp(x: i128) -> i128 {
    if x <= 0 {
        return SCALE;
    }
    if x > 50 * SCALE {
        return 0; // underflows to ~0
    }
    // split into integer + fractional part: e^-x = e^-n * e^-f
    let n = x / SCALE;
    let f = x - n * SCALE; // [0, SCALE)

    // e^-f via Taylor on [0, 1):
    // 1 - f + f^2/2 - f^3/6 + f^4/24 - f^5/120 + f^6/720
    let f2 = mul_fp(f, f);
    let f3 = mul_fp(f2, f);
    let f4 = mul_fp(f3, f);
    let f5 = mul_fp(f4, f);
    let f6 = mul_fp(f5, f);
    let mut ef = SCALE - f + f2 / 2 - f3 / 6 + f4 / 24 - f5 / 120 + f6 / 720;
    if ef < 0 {
        ef = 0;
    }

    // multiply by e^-1, n times
    let mut result = ef;
    let mut k = n;
    while k > 0 {
        result = mul_fp(result, E_INV);
        k -= 1;
    }
    result
}

/// Gaussian likelihood exp( -(x - mu)^2 / (2 * sigma^2) ), fixed-point.
pub fn gaussian_fp(x: i128, mu: i128, sigma: i128) -> i128 {
    let d = x - mu;
    let d2 = mul_fp(d, d);
    let two_sigma2 = mul_fp(sigma, sigma) * 2;
    let expo = div_fp(d2, two_sigma2); // >= 0
    exp_neg_fp(expo)
}

/// log2(value / SCALE) in fixed-point. `value` must be > 0; result may be
/// negative (it is, for value < SCALE, i.e. probabilities < 1).
pub fn log2_fp(value: i128) -> i128 {
    if value <= 0 {
        return -64 * SCALE;
    }
    // normalise value into [SCALE, 2*SCALE) tracking the exponent e
    let mut v = value;
    let mut e: i128 = 0;
    while v < SCALE {
        v *= 2;
        e -= 1;
    }
    while v >= 2 * SCALE {
        v /= 2;
        e += 1;
    }
    // m = v / SCALE in [1, 2). ln(m) = 2 * atanh(z), z = (m-1)/(m+1), |z| < 1/3
    let num = v - SCALE;
    let den = v + SCALE;
    let z = div_fp(num, den);
    let z2 = mul_fp(z, z);
    // 2 * (z + z^3/3 + z^5/5 + z^7/7)
    let mut term = z;
    let mut sum = z;
    term = mul_fp(term, z2);
    sum += term / 3;
    term = mul_fp(term, z2);
    sum += term / 5;
    term = mul_fp(term, z2);
    sum += term / 7;
    let ln_m = sum * 2;
    let log2_m = mul_fp(ln_m, LOG2E);
    e * SCALE + log2_m
}

/// Shannon entropy H = -sum p_i * log2(p_i) over a posterior whose entries are
/// fixed-point fractions summing to SCALE. Result is bits in fixed-point.
pub fn entropy_fp(p: &[i128]) -> i128 {
    let mut h: i128 = 0;
    for &prob in p.iter() {
        if prob <= 0 {
            continue;
        }
        let lg = log2_fp(prob); // <= 0 for prob <= SCALE
        h -= mul_fp(prob, lg); // -p*log2(p) >= 0
    }
    h
}

/// Rescale a posterior in place so its entries sum to SCALE. Falls back to a
/// uniform distribution if the mass collapses to zero.
pub fn renormalise(p: &mut [i128]) {
    let mut sum: i128 = 0;
    for &v in p.iter() {
        sum += v;
    }
    if sum <= 0 {
        let u = SCALE / p.len() as i128;
        for v in p.iter_mut() {
            *v = u;
        }
        return;
    }
    for v in p.iter_mut() {
        *v = v.saturating_mul(SCALE) / sum;
    }
}

/// Index of the maximum-a-posteriori bin.
pub fn argmax(p: &[i128]) -> u16 {
    let mut idx = 0usize;
    let mut best = i128::MIN;
    for (i, &v) in p.iter().enumerate() {
        if v > best {
            best = v;
            idx = i;
        }
    }
    idx as u16
}

/// Centre price of bin `i` under a linear mapping over [lo, hi].
pub fn bin_to_price_fp(i: usize, bins: usize, lo: i128, hi: i128) -> i128 {
    if bins <= 1 {
        return lo;
    }
    lo + (hi - lo) * (i as i128) / ((bins as i128) - 1)
}

/// base^exp in fixed-point via exponentiation by squaring (exp may be negative).
pub fn pow_fp(base: i128, exp: i32) -> i128 {
    let mut result = SCALE;
    let mut b = base;
    let mut e = exp.unsigned_abs();
    while e > 0 {
        if e & 1 == 1 {
            result = mul_fp(result, b);
        }
        b = mul_fp(b, b);
        e >>= 1;
    }
    if exp < 0 {
        div_fp(SCALE, result)
    } else {
        result
    }
}
