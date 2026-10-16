//! ARM64 single-precision arithmetic as the engine's vector kernels execute it:
//! round to nearest even, denormals kept (FPCR.FZ 0) and default NaN off
//! (FPCR.DN 0). A NaN operand propagates (a signalling NaN is quietened, the
//! first operand before the second), an invalid operation yields the default
//! NaN `0x7fc00000`. The host's own NaN rules differ (its invalid result is
//! `0xffc00000`, and it picks operands in another order), so every operation a
//! NaN can reach goes through here. Non-NaN results are the IEEE-754 binary32
//! results, which the host computes bit for bit.
//!
//! The fused step instructions (reciprocal and reciprocal square root steps)
//! round the exact value once; the estimates follow the ARMv8.0 8-bit tables.

const QNAN: u32 = 0x7fc0_0000;
const QUIET: u32 = 0x0040_0000;

#[inline]
fn quiet(a: f32) -> f32 {
    f32::from_bits(a.to_bits() | QUIET)
}

/// Propagated NaN of a two-operand operation, if any operand is NaN.
#[inline]
fn nan2(a: f32, c: f32) -> Option<f32> {
    let (an, cn) = (a.is_nan(), c.is_nan());
    if an && a.to_bits() & QUIET == 0 {
        return Some(quiet(a));
    }
    if cn && c.to_bits() & QUIET == 0 {
        return Some(quiet(c));
    }
    if an {
        return Some(a);
    }
    if cn {
        return Some(c);
    }
    None
}

#[inline]
fn invalid(r: f32) -> f32 {
    if r.is_nan() { f32::from_bits(QNAN) } else { r }
}

#[inline]
pub(crate) fn add(a: f32, c: f32) -> f32 {
    nan2(a, c).unwrap_or_else(|| invalid(a + c))
}

#[inline]
pub(crate) fn sub(a: f32, c: f32) -> f32 {
    nan2(a, c).unwrap_or_else(|| invalid(a - c))
}

#[inline]
pub(crate) fn mul(a: f32, c: f32) -> f32 {
    nan2(a, c).unwrap_or_else(|| invalid(a * c))
}

#[inline]
pub(crate) fn div(a: f32, c: f32) -> f32 {
    nan2(a, c).unwrap_or_else(|| invalid(a / c))
}

#[inline]
pub(crate) fn sqrt(a: f32) -> f32 {
    if a.is_nan() {
        return quiet(a);
    }
    if a == 0.0 {
        return a;
    }
    if a < 0.0 {
        return f32::from_bits(QNAN);
    }
    a.sqrt()
}

#[inline]
pub(crate) fn neg(a: f32) -> f32 {
    f32::from_bits(a.to_bits() ^ 0x8000_0000)
}

#[inline]
pub(crate) fn abs(a: f32) -> f32 {
    f32::from_bits(a.to_bits() & 0x7fff_ffff)
}

/// FMAX: NaN propagates; +0 unless both zeros are -0.
#[inline]
pub(crate) fn max(a: f32, c: f32) -> f32 {
    if let Some(n) = nan2(a, c) {
        return n;
    }
    if a == 0.0 && c == 0.0 {
        return f32::from_bits(a.to_bits() & c.to_bits());
    }
    if a > c { a } else { c }
}

/// FMIN: NaN propagates; -0 if either zero is -0.
#[inline]
pub(crate) fn min(a: f32, c: f32) -> f32 {
    if let Some(n) = nan2(a, c) {
        return n;
    }
    if a == 0.0 && c == 0.0 {
        return f32::from_bits(a.to_bits() | c.to_bits());
    }
    if a < c { a } else { c }
}

/// FMAXNM: a single quiet NaN yields the other operand.
#[inline]
pub(crate) fn max_nm(a: f32, c: f32) -> f32 {
    let (an, cn) = (a.is_nan(), c.is_nan());
    if an && !cn && a.to_bits() & QUIET != 0 {
        return c;
    }
    if cn && !an && c.to_bits() & QUIET != 0 {
        return a;
    }
    max(a, c)
}

/// FCVT single from double, round to nearest even; a NaN becomes the default NaN.
#[inline]
pub(crate) fn narrow(d: f64) -> f32 {
    if d.is_nan() { f32::from_bits(QNAN) } else { d as f32 }
}

/// FCVTZS to i32: toward zero, saturating, NaN to zero.
#[inline]
pub(crate) fn to_i32_toward_zero(a: f32) -> i32 {
    a as i32
}

// ---- fused steps: exact value rounded once ----

fn next_toward(r: f32, up: bool) -> f32 {
    let bits = r.to_bits();
    let positive = bits & 0x8000_0000 == 0;
    let next = if r == 0.0 {
        if up { 1 } else { 0x8000_0001 }
    } else if positive == up {
        bits + 1
    } else {
        bits - 1
    };
    f32::from_bits(next)
}

/// Round `hi + lo` (an exact two-term sum, |lo| at most half an ulp of hi in
/// binary64) to binary32 with a single round to nearest even.
fn round_pair(hi: f64, lo: f64) -> f32 {
    let r = hi as f32;
    if lo == 0.0 {
        return r;
    }
    if r.is_infinite() {
        // The only binary64 value rounding to infinity that the low term can
        // pull back is the exact midpoint above the largest finite value.
        let boundary = (f32::MAX as f64) + 2f64.powi(103);
        if hi.abs() == boundary && (lo < 0.0) == (hi > 0.0) {
            return f32::MAX.copysign(r);
        }
        return r;
    }
    let r64 = r as f64;
    if r64 == hi {
        return r;
    }
    let other = next_toward(r, r64 < hi);
    let mid = (r64 + other as f64) * 0.5;
    if hi != mid {
        return r;
    }
    // A tie in binary64 that the exact value does not share: go its way.
    let toward_up = lo > 0.0;
    let (low, high) = if r < other { (r, other) } else { (other, r) };
    if toward_up { high } else { low }
}

/// `k - a*c` (then halved when `half`), exact until the final rounding. The
/// product of two binary32 values is exact in binary64; the difference is
/// carried as an exact two-term sum.
fn fused(k: f64, a: f32, c: f32, half: bool) -> f32 {
    let p = -((a as f64) * (c as f64));
    let s = k + p;
    let bb = s - k;
    let mut err = (k - (s - bb)) + (p - bb);
    let mut s = s;
    if half {
        s *= 0.5;
        err *= 0.5;
    }
    round_pair(s, err)
}

fn step_special(a: f32, c: f32, zero_times_inf: f32) -> Option<f32> {
    if let Some(n) = nan2(neg(a), c) {
        return Some(n);
    }
    let ai = a.is_infinite();
    let ci = c.is_infinite();
    if (ai && c == 0.0) || (a == 0.0 && ci) {
        return Some(zero_times_inf);
    }
    if ai || ci {
        let negative_product = (a.is_sign_negative()) != (c.is_sign_negative());
        return Some(if negative_product { f32::INFINITY } else { f32::NEG_INFINITY });
    }
    None
}

/// FRECPS: 2 - a*c, fused.
pub(crate) fn recip_step(a: f32, c: f32) -> f32 {
    step_special(a, c, 2.0).unwrap_or_else(|| fused(2.0, a, c, false))
}

/// FRSQRTS: (3 - a*c) / 2, fused.
pub(crate) fn rsqrt_step(a: f32, c: f32) -> f32 {
    step_special(a, c, 1.5).unwrap_or_else(|| fused(3.0, a, c, true))
}

// ---- estimates ----

fn recip_estimate_table(a: u64) -> u64 {
    let a = a * 2 + 1;
    let b = (1u64 << 19) / a;
    (b + 1) / 2
}

fn rsqrt_estimate_table(a: u64) -> u64 {
    let a = if a < 256 { a * 2 + 1 } else { ((a >> 1) << 1) * 2 + 2 };
    let mut b = 512u64;
    while a * (b + 1) * (b + 1) < (1u64 << 28) {
        b += 1;
    }
    (b + 1) / 2
}

const FRAC52: u64 = (1u64 << 52) - 1;

/// FRECPE.
pub(crate) fn recip_estimate(a: f32) -> f32 {
    if a.is_nan() {
        return quiet(a);
    }
    let bits = a.to_bits();
    let sign = bits & 0x8000_0000;
    if a.is_infinite() {
        return f32::from_bits(sign);
    }
    if a == 0.0 {
        return f32::from_bits(sign | 0x7f80_0000);
    }
    if bits & 0x7fff_ffff < 0x0020_0000 {
        return f32::from_bits(sign | 0x7f80_0000);
    }
    let mut exp = ((bits >> 23) & 0xff) as i64;
    let mut frac = ((bits & 0x7f_ffff) as u64) << 29;
    if exp == 0 {
        if (frac >> 51) & 1 == 0 {
            exp = -1;
            frac = (frac << 2) & FRAC52;
        } else {
            frac = (frac << 1) & FRAC52;
        }
    }
    let scaled = (1u64 << 8) | (frac >> 44);
    let mut rexp = 253 - exp;
    let est = recip_estimate_table(scaled);
    let mut frac = (est & 0xff) << 44;
    if rexp == 0 {
        frac = (1u64 << 51) | (frac >> 1);
    } else if rexp == -1 {
        frac = (1u64 << 50) | (frac >> 2);
        rexp = 0;
    }
    f32::from_bits(sign | (((rexp as u32) & 0xff) << 23) | ((frac >> 29) as u32))
}

/// FRSQRTE.
pub(crate) fn rsqrt_estimate(a: f32) -> f32 {
    if a.is_nan() {
        return quiet(a);
    }
    let bits = a.to_bits();
    if a == 0.0 {
        return f32::from_bits((bits & 0x8000_0000) | 0x7f80_0000);
    }
    if a < 0.0 {
        return f32::from_bits(QNAN);
    }
    if a.is_infinite() {
        return 0.0;
    }
    let mut exp = ((bits >> 23) & 0xff) as i64;
    let mut frac = ((bits & 0x7f_ffff) as u64) << 29;
    if exp == 0 {
        while (frac >> 51) & 1 == 0 {
            frac = (frac << 1) & FRAC52;
            exp -= 1;
        }
        frac = (frac << 1) & FRAC52;
    }
    let scaled = if exp & 1 == 0 { (1u64 << 8) | (frac >> 44) } else { (1u64 << 7) | (frac >> 45) };
    let rexp = (380 - exp).div_euclid(2);
    let est = rsqrt_estimate_table(scaled);
    f32::from_bits((((rexp as u32) & 0xff) << 23) | (((est as u32) & 0xff) << 15))
}

/// FRECPE and two FRECPS refinements; the estimate is kept where x is zero.
pub(crate) fn recip2(x: f32) -> f32 {
    let e = recip_estimate(x);
    let r = mul(e, recip_step(x, e));
    let r = mul(r, recip_step(x, r));
    if x == 0.0 { e } else { r }
}

/// FRSQRTE and two FRSQRTS refinements (the square multiplied first); the
/// estimate is kept where x is zero.
pub(crate) fn rsqrt2(x: f32) -> f32 {
    let e = rsqrt_estimate(x);
    let r = mul(e, rsqrt_step(mul(x, e), e));
    let r = mul(r, rsqrt_step(mul(x, r), r));
    if x == 0.0 { e } else { r }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::json;

    /// Single ARM64 scalar instructions executed in an ARMv8 emulator on
    /// recorded operands (special values, subnormals, wide exponents, and
    /// operand pairs whose fused step lands on a binary32 tie): every result
    /// bit must match, NaN payloads included.
    #[test]
    #[ignore = "needs MOLY_ARMF_ROWS"]
    fn emulated_instruction_rows_match_bits() {
        let path = std::env::var("MOLY_ARMF_ROWS").expect("MOLY_ARMF_ROWS");
        let doc = json::parse(&std::fs::read(&path).expect("read rows")).expect("parse rows");
        let ops: [(&str, fn(f32, f32) -> f32); 12] = [
            ("add", add), ("sub", sub), ("mul", mul), ("div", div),
            ("sqrt", |a, _| sqrt(a)), ("max", max), ("min", min), ("maxnm", max_nm),
            ("frecpe", |a, _| recip_estimate(a)), ("frsqrte", |a, _| rsqrt_estimate(a)),
            ("frecps", recip_step), ("frsqrts", rsqrt_step),
        ];
        let mut total = 0usize;
        for (name, op) in ops {
            let rows = doc.get(name).and_then(json::Value::as_array).unwrap_or_else(|| panic!("rows for {name}"));
            assert!(!rows.is_empty(), "{name}: no rows");
            for row in rows {
                let w: Vec<u32> = row.as_array().expect("row").iter().map(|v| v.as_f64().expect("word") as u32).collect();
                let got = op(f32::from_bits(w[0]), f32::from_bits(w[1])).to_bits();
                assert_eq!(got, w[2], "{name}({:#010x}, {:#010x})", w[0], w[1]);
            }
            total += rows.len();
        }
        println!("armf replay: {total} emulated instruction rows, 0 mismatches");
    }
}
