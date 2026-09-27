//! The single-precision `powf` of the device's C library, in plain Rust.
//!
//! The engine's colour-space conversions (`Mathf.GammaToLinearSpace` and
//! `LinearToGammaSpace`, and through them every environment colour blend) call the C
//! library's `powf` on the phone. On Android that is bionic's libm. Since Android 10
//! bionic builds `powf` from Arm Optimized Routines (the algorithm of glibc's generic
//! `powf` since 2.27); Android 7 to 9 built FreeBSD's `e_powf.c`. This module reproduces the
//! Android 10 and later function, bit for bit, on every target:
//!
//! * The browser build must not reach for the platform `powf`: for wasm32 that is the
//!   `libm` crate's port of FreeBSD's `e_powf.c`, which differs from the device on about
//!   7% of the bases checked below (4% to 10% per exponent).
//! * The Android build compiles the C source with `-ffp-contract=fast`, and the arm64
//!   machine code fuses nine multiply-adds (listed in [`log2_inline`] and
//!   [`exp2_inline`]). Those are fused here too, with an exact integer multiply-add
//!   ([`fma`]), because a host `mul_add` may itself be a library call.
//! * NaN results carry the bits the arm64 instructions produce ([`nan_sum`],
//!   [`quiet`], and the default NaN of an invalid operation), not whatever the host's
//!   arithmetic gives.
//!
//! How this is known: the AOSP emulator system images for API 29, 31, 33, 35 and 36 (all
//! arm64) carry `powf` with these table values and nine fused multiply-adds. Their `powf`,
//! executed in an emulator, gives the same bits as this function on native and on wasm32
//! builds for every base the colour conversions can pass (checked as every base in [0, 1]
//! for the exponents 2.4 and 0.41666669 and every base in [1, inf] for 2.2 and 0.45454547,
//! against the API 29 and 36 images) and on a random sample of the whole domain, NaN
//! payloads included (all five images agree on it). The API 24 and 28 images carry the
//! FreeBSD function instead; this module does not reproduce them.
//!
//! Ported from Arm Optimized Routines (`math/powf.c`, `math/powf_log2_data.c`,
//! `math/exp2f_data.c`), Copyright (c) 1999-2018 Arm Limited, MIT licence; see the
//! repository's third-party notices.

/// `x` raised to `y`, as the Android 10+ bionic libm computes it.
pub fn powf(x: f32, y: f32) -> f32 {
    let mut sign_bias = 0u32;
    let mut ix = x.to_bits();
    let iy = y.to_bits();
    if ix.wrapping_sub(0x0080_0000) >= 0x7f80_0000 - 0x0080_0000 || zeroinfnan(iy) {
        // x below the smallest normal, inf or NaN, or y zero, inf or NaN.
        if zeroinfnan(iy) {
            if iy.wrapping_mul(2) == 0 {
                return if is_signaling(ix) {
                    nan_sum(ix, iy)
                } else {
                    1.0
                };
            }
            if ix == 0x3f80_0000 {
                return if is_signaling(iy) {
                    nan_sum(ix, iy)
                } else {
                    1.0
                };
            }
            if ix.wrapping_mul(2) > 2 * 0x7f80_0000 || iy.wrapping_mul(2) > 2 * 0x7f80_0000 {
                return nan_sum(ix, iy);
            }
            if ix.wrapping_mul(2) == 2 * 0x3f80_0000 {
                return 1.0;
            }
            if (ix.wrapping_mul(2) < 2 * 0x3f80_0000) == (iy & 0x8000_0000 == 0) {
                return 0.0; // |x| < 1 and y = inf, or |x| > 1 and y = -inf
            }
            return y * y;
        }
        if zeroinfnan(ix) {
            // x is zero, inf or NaN; y is finite and non-zero.
            let mut x2 = if is_nan(ix) {
                quiet(ix)
            } else {
                (x * x).to_bits()
            };
            if ix & 0x8000_0000 != 0 && checkint(iy) == 1 {
                x2 ^= 0x8000_0000;
            }
            // 1 / x2 for negative y; a NaN x2 passes through (already quiet).
            return if iy & 0x8000_0000 == 0 || is_nan(x2) {
                f32::from_bits(x2)
            } else {
                1.0 / f32::from_bits(x2)
            };
        }
        // x and y are non-zero finite.
        if ix & 0x8000_0000 != 0 {
            match checkint(iy) {
                0 => return f32::from_bits(DEFAULT_NAN), // (x - x) / (x - x)
                1 => sign_bias = SIGN_BIAS,
                _ => {}
            }
            ix &= 0x7fff_ffff;
        }
        if ix < 0x0080_0000 {
            // Normalise a subnormal x so that its exponent field goes negative.
            ix = (x * f32::from_bits(0x4b00_0000)).to_bits() & 0x7fff_ffff;
            ix = ix.wrapping_sub(23 << 23);
        }
    }
    let logx = log2_inline(ix);
    let ylogx = y as f64 * logx;
    if (ylogx.to_bits() >> 47 & 0xffff) >= (126.0 * POWF_SCALE).to_bits() >> 47 {
        // |y * log2(x)| >= 126
        if ylogx > f64::from_bits(0x40af_ffff_ffd1_d571) {
            return xflow(sign_bias, f32::from_bits(0x7000_0000)); // overflow: 0x1p97 squared
        }
        if ylogx <= -150.0 * POWF_SCALE {
            return xflow(sign_bias, f32::from_bits(0x1000_0000)); // underflow: 0x1p-95 squared
        }
    }
    exp2_inline(ylogx, sign_bias)
}

/// `log2(x) * 32` for the bits of a positive x (a subnormal already rescaled).
///
/// Fused on the device: `r = invc * z - 1`, `A0 * r + A1`, `A2 * r + A3`,
/// `A4 * r + y0`, `p * r2 + q` and `y * r4 + q`; `logc + k`, `r * r` and `r2 * r2` are not.
fn log2_inline(ix: u32) -> f64 {
    const OFF: u32 = 0x3f33_0000;
    let tmp = ix.wrapping_sub(OFF);
    let i = ((tmp >> (23 - 4)) % 16) as usize;
    let top = tmp & 0xff80_0000;
    let iz = ix.wrapping_sub(top);
    let k = (top as i32) >> (23 - 5);
    let (invc, logc) = (f64::from_bits(LOG2_TAB[i].0), f64::from_bits(LOG2_TAB[i].1));
    let z = f32::from_bits(iz) as f64;
    let r = fma(z, invc, -1.0);
    let y0 = logc + k as f64;
    let a = LOG2_POLY.map(f64::from_bits);
    let r2 = r * r;
    let y = fma(a[0], r, a[1]);
    let p = fma(a[2], r, a[3]);
    let r4 = r2 * r2;
    let q = fma(a[4], r, y0);
    let q = fma(p, r2, q);
    fma(y, r4, q)
}

/// `2^(xd / 32)`, with the sign bit set when `sign_bias` asks for a negative result.
///
/// The device rounds `xd` half away from zero (`frinta`/`fcvtas`), then fuses
/// `C0 * r + C1`, `C2 * r + 1` and `z * r2 + y`; `r * r` and the final `y * s` are not.
fn exp2_inline(xd: f64, sign_bias: u32) -> f32 {
    let kd = round_half_away(xd);
    let ki = kd as i64 as u64;
    let r = xd - kd;
    let c = EXP2_POLY_SCALED.map(f64::from_bits);
    let t = EXP2_TAB[(ki % 32) as usize];
    let ski = (ki as u32).wrapping_add(sign_bias);
    let s = f64::from_bits(t.wrapping_add((ski as u64) << (52 - 5)));
    let z = fma(c[0], r, c[1]);
    let r2 = r * r;
    let y = fma(c[2], r, 1.0);
    let y = fma(z, r2, y);
    (y * s) as f32
}

/// `x` rounded to an integer, ties away from zero, keeping the sign of zero; valid for
/// |x| < 2^52, which is all [`exp2_inline`] sees.
fn round_half_away(x: f64) -> f64 {
    let t = x as i64 as f64; // toward zero, exact here
    let f = x - t; // exact
    let r = if f >= 0.5 {
        t + 1.0
    } else if f <= -0.5 {
        t - 1.0
    } else {
        t
    };
    f64::from_bits(r.to_bits() | (x.to_bits() & SIGN64))
}

/// The overflow and underflow results: `(sign ? -v : v) * v`.
fn xflow(sign_bias: u32, v: f32) -> f32 {
    let signed = if sign_bias == 0 { v } else { -v };
    signed * v
}

/// 0 if not an integer, 1 if an odd integer, 2 if an even integer (the bits of a non-zero
/// finite y).
fn checkint(iy: u32) -> u32 {
    let e = iy >> 23 & 0xff;
    if e < 0x7f {
        return 0;
    }
    if e > 0x7f + 23 {
        return 2;
    }
    if iy & ((1 << (0x7f + 23 - e)) - 1) != 0 {
        return 0;
    }
    if iy & (1 << (0x7f + 23 - e)) != 0 {
        return 1;
    }
    2
}

fn zeroinfnan(i: u32) -> bool {
    i.wrapping_mul(2).wrapping_sub(1) >= 2 * 0x7f80_0000 - 1
}

fn is_nan(i: u32) -> bool {
    i & 0x7fff_ffff > 0x7f80_0000
}

fn is_signaling(i: u32) -> bool {
    (i ^ 0x0040_0000).wrapping_mul(2) > 2 * 0x7fc0_0000
}

fn quiet(i: u32) -> u32 {
    i | 0x0040_0000
}

/// `x + y` when at least one of them is a NaN, as the arm64 `fadd` picks the result
/// (default-NaN mode off): a signalling NaN in the first operand, then one in the second,
/// then a quiet NaN in the first, then one in the second; the chosen NaN is made quiet.
fn nan_sum(a: u32, b: u32) -> f32 {
    let pick = if is_signaling(a) {
        a
    } else if is_signaling(b) {
        b
    } else if is_nan(a) {
        a
    } else {
        b
    };
    f32::from_bits(quiet(pick))
}

/// `a * b + c` rounded once (to nearest, ties to even) in binary64, computed with integer
/// arithmetic so that no platform library or fused instruction is involved.
fn fma(a: f64, b: f64, c: f64) -> f64 {
    if a.is_nan() || b.is_nan() || c.is_nan() || a.is_infinite() || b.is_infinite() {
        return a * b + c; // NaN, or an infinite exact product: the unfused form agrees
    }
    if c.is_infinite() {
        return c; // the exact product is finite even where `a * b` would overflow
    }
    if a == 0.0 || b == 0.0 {
        return a * b + c; // the product is an exact zero: one rounding either way
    }
    if c == 0.0 {
        return a * b; // one rounding of a non-zero exact product; its sign survives
    }
    let (sa, ea, ma) = unpack(a);
    let (sb, eb, mb) = unpack(b);
    let (sc, ec, mc) = unpack(c);
    // Both terms as integers with their top bit at bit 125.
    let p = ma as u128 * mb as u128; // in [2^104, 2^106)
    let shift = 125 - (127 - p.leading_zeros() as i32);
    let (pm, pe, ps) = (p << shift, ea + eb - shift, sa ^ sb);
    let (cm, ce, cs) = ((mc as u128) << 73, ec - 73, sc);
    let p_bigger = pe > ce || (pe == ce && pm >= cm);
    let ((bm, be, bs), (lm, le, _)) = if p_bigger {
        ((pm, pe, ps), (cm, ce, cs))
    } else {
        ((cm, ce, cs), (pm, pe, ps))
    };
    let d = (be - le) as u32;
    // The smaller term aligned to the bigger one; bits shifted out leave a sticky 1.
    let lm = if d == 0 {
        lm
    } else if d >= 127 {
        1
    } else {
        (lm >> d) | u128::from(lm & ((1u128 << d) - 1) != 0)
    };
    let sum = if ps == cs { bm + lm } else { bm - lm };
    if sum == 0 {
        return 0.0;
    }
    // Round `sum * 2^be` to 53 bits.
    let top = 127 - sum.leading_zeros() as i32;
    let mut e = be + top - 52;
    let mut s = top - 52;
    if e < -1074 {
        s += -1074 - e;
        e = -1074;
    }
    let m = if s <= 0 {
        (sum << (-s) as u32) as u64
    } else if s >= 128 {
        0
    } else {
        let q = sum >> s;
        let rem = sum & ((1u128 << s) - 1);
        let half = 1u128 << (s - 1);
        (q + u128::from(rem > half || (rem == half && q & 1 == 1))) as u64
    };
    let bits = (((e + 1074) as u64) << 52)
        .wrapping_add(m)
        .min(0x7ff0_0000_0000_0000);
    f64::from_bits(bits | if bs { SIGN64 } else { 0 })
}

/// Sign, exponent and 53-bit integer significand (top bit set) of a finite non-zero value.
fn unpack(x: f64) -> (bool, i32, u64) {
    let bits = x.to_bits();
    let field = (bits >> 52 & 0x7ff) as i32;
    let frac = bits & ((1 << 52) - 1);
    let (e, m) = if field == 0 {
        let lz = frac.leading_zeros() as i32 - 11;
        (-1074 - lz, frac << lz)
    } else {
        (field - 1075, frac | 1 << 52)
    };
    (bits & SIGN64 != 0, e, m)
}

const SIGN64: u64 = 1 << 63;
const DEFAULT_NAN: u32 = 0x7fc0_0000;
const SIGN_BIAS: u32 = 1 << (5 + 11);
const POWF_SCALE: f64 = 32.0;

/// `(1/c, log2(c) * 32)` per subinterval of the reduced argument.
const LOG2_TAB: [(u64, u64); 16] = [
    (0x3ff6_61ec_79f8_f3be, 0xc02e_fec6_5b96_3019),
    (0x3ff5_71ed_4aaf_883d, 0xc02b_0b68_32d4_fca4),
    (0x3ff4_9539_f0f0_10b0, 0xc027_418b_0a1f_b77b),
    (0x3ff3_c995_b0b8_0385, 0xc023_9de9_1a6d_cf7b),
    (0x3ff3_0d19_0c88_64a5, 0xc020_1d9b_f3f2_b631),
    (0x3ff2_5e22_7b0b_8ea0, 0xc019_7c1d_1b3b_7af0),
    (0x3ff1_bb4a_4a1a_343f, 0xc012_f9e3_93af_3c9f),
    (0x3ff1_2358_f08a_e5ba, 0xc009_60cb_bf78_8d5c),
    (0x3ff0_953f_4199_00a7, 0xbffa_6f9d_b647_5fce),
    (0x3ff0_0000_0000_0000, 0x0000_0000_0000_0000),
    (0x3fee_608c_fd9a_47ac, 0x4003_38ca_9f24_f53d),
    (0x3fec_a4b3_1f02_6aa0, 0x4014_76a9_5438_91ba),
    (0x3feb_2036_576a_fce6, 0x401e_840b_4ac4_e4d2),
    (0x3fe9_c2d1_63a1_aa2d, 0x4024_0645_f0c6_651c),
    (0x3fe8_86e6_0378_41ed, 0x4028_8e9c_2c1b_9ff8),
    (0x3fe7_67dc_f553_4862, 0x402c_e0a4_4eb1_7bcc),
];

/// The log2 polynomial, times 32.
const LOG2_POLY: [u64; 5] = [
    0x4022_7616_c949_6e0b,
    0xc027_1969_a075_c67a,
    0x402e_c70a_6ca7_badd,
    0xc037_1547_48be_f6c8,
    0x4047_1547_652a_b82b,
];

/// `bits(2^(i/32)) - (i << 47)`.
const EXP2_TAB: [u64; 32] = [
    0x3ff0_0000_0000_0000,
    0x3fef_d9b0_d315_8574,
    0x3fef_b558_6cf9_890f,
    0x3fef_9301_d012_5b51,
    0x3fef_72b8_3c7d_517b,
    0x3fef_5487_3168_b9aa,
    0x3fef_387a_6e75_6238,
    0x3fef_1e9d_f51f_dee1,
    0x3fef_06fe_0a31_b715,
    0x3fee_f1a7_373a_a9cb,
    0x3fee_dea6_4c12_3422,
    0x3fee_ce08_6061_892d,
    0x3fee_bfda_d536_2a27,
    0x3fee_b42b_569d_4f82,
    0x3fee_ab07_dd48_5429,
    0x3fee_a47e_b03a_5585,
    0x3fee_a09e_667f_3bcd,
    0x3fee_9f75_e8ec_5f74,
    0x3fee_a114_73eb_0187,
    0x3fee_a589_994c_ce13,
    0x3fee_ace5_422a_a0db,
    0x3fee_b737_b0cd_c5e5,
    0x3fee_c491_82a3_f090,
    0x3fee_d503_b23e_255d,
    0x3fee_e89f_995a_d3ad,
    0x3fee_ff76_f2fb_5e47,
    0x3fef_199b_dd85_529c,
    0x3fef_3720_dcef_9069,
    0x3fef_5818_dcfb_a487,
    0x3fef_7c97_337b_9b5f,
    0x3fef_a4af_a2a4_90da,
    0x3fef_d076_5b6e_4540,
];

/// The exp2 polynomial for an argument scaled by 32.
const EXP2_POLY_SCALED: [u64; 3] = [
    0x3ebc_6af8_4b91_2394,
    0x3f2e_bfce_50fa_c4f3,
    0x3f96_2e42_ff0c_52d6,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn env_path(name: &str) -> std::path::PathBuf {
        std::env::var_os(name)
            .unwrap_or_else(|| panic!("{name} must name the oracle output"))
            .into()
    }

    fn read_u32s(path: &std::path::Path) -> Vec<u32> {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(bytes.len() % 4, 0, "{}", path.display());
        bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect()
    }

    /// Against the device libm executed in an emulator. `MOLY_POWF_ORACLE` names a
    /// directory with `ranges.txt` (lines `name y_bits x_first_bits count file`, the file
    /// holding the device's result for every x from `x_first` on) and optionally
    /// `pairs.txt` (lines `name file_in file_out`, inputs as `x, y` bit pairs). Every
    /// result must match bit for bit, NaN payloads included.
    #[test]
    #[ignore = "MOLY_POWF_ORACLE must name a directory of device powf outputs"]
    fn powf_matches_the_device_libm() {
        let dir = env_path("MOLY_POWF_ORACLE");
        let threads = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(6);
        let mut total_bad = 0u64;
        let ranges = std::fs::read_to_string(dir.join("ranges.txt")).unwrap_or_default();
        for line in ranges.lines().filter(|l| !l.trim().is_empty()) {
            let f: Vec<&str> = line.split_whitespace().collect();
            let y = f32::from_bits(u32::from_str_radix(f[1], 16).unwrap());
            let x0 = u32::from_str_radix(f[2], 16).unwrap();
            let count: usize = f[3].parse().unwrap();
            let oracle = read_u32s(&dir.join(f[4]));
            assert_eq!(oracle.len(), count, "{line}");
            let chunk = count.div_ceil(threads);
            let (bad, nan_only) = std::thread::scope(|scope| {
                let jobs: Vec<_> = oracle
                    .chunks(chunk)
                    .enumerate()
                    .map(|(j, part)| {
                        scope.spawn(move || {
                            let mut bad = (0u64, 0u64, None);
                            for (i, &want) in part.iter().enumerate() {
                                let x = f32::from_bits(x0.wrapping_add((j * chunk + i) as u32));
                                let got = powf(x, y).to_bits();
                                if got != want {
                                    bad.0 += 1;
                                    bad.1 += u64::from(is_nan(got) && is_nan(want));
                                    bad.2.get_or_insert((x.to_bits(), got, want));
                                }
                            }
                            bad
                        })
                    })
                    .collect();
                let mut sum = (0u64, 0u64);
                for job in jobs {
                    let (b, n, first) = job.join().unwrap();
                    if let Some((x, got, want)) = first {
                        eprintln!(
                            "{}: first mismatch x {x:#010x}: ours {got:#010x}, device {want:#010x}",
                            f[0]
                        );
                    }
                    sum = (sum.0 + b, sum.1 + n);
                }
                sum
            });
            eprintln!("{}: y {:#010x}, {count} bases from {x0:#010x}: {bad} mismatches ({nan_only} NaN-payload only)", f[0], y.to_bits());
            total_bad += bad;
        }
        let pairs = std::fs::read_to_string(dir.join("pairs.txt")).unwrap_or_default();
        for line in pairs.lines().filter(|l| !l.trim().is_empty()) {
            let f: Vec<&str> = line.split_whitespace().collect();
            let input = read_u32s(&dir.join(f[1]));
            let oracle = read_u32s(&dir.join(f[2]));
            assert_eq!(input.len(), 2 * oracle.len(), "{line}");
            let (mut bad, mut nan_only, mut nan_rows) = (0u64, 0u64, 0u64);
            for (xy, &want) in input.chunks_exact(2).zip(&oracle) {
                let got = powf(f32::from_bits(xy[0]), f32::from_bits(xy[1])).to_bits();
                nan_rows += u64::from(is_nan(want));
                if got != want {
                    if bad == 0 {
                        eprintln!("{}: first mismatch x {:#010x} y {:#010x}: ours {got:#010x}, device {want:#010x}", f[0], xy[0], xy[1]);
                    }
                    bad += 1;
                    nan_only += u64::from(is_nan(got) && is_nan(want));
                }
            }
            eprintln!("{}: {} pairs ({nan_rows} NaN results): {bad} mismatches ({nan_only} NaN-payload only)", f[0], oracle.len());
            total_bad += bad;
        }
        assert!(
            !ranges.trim().is_empty() || !pairs.trim().is_empty(),
            "no oracle tables in {}",
            dir.display()
        );
        assert_eq!(total_bad, 0, "powf differs from the device libm");
    }

    /// The integer multiply-add against the host's fused multiply-add (IEEE 754 fixes its
    /// result, so any correct implementation is an oracle), on random operands of every
    /// exponent and on near-cancelling ones. `MOLY_FMA_SAMPLES` sets the sample count.
    #[test]
    #[ignore = "MOLY_FMA_SAMPLES sets how many random multiply-adds to compare"]
    fn fma_matches_the_host_fused_multiply_add() {
        let n: u64 = std::env::var("MOLY_FMA_SAMPLES")
            .expect("MOLY_FMA_SAMPLES")
            .parse()
            .unwrap();
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut bad = 0u64;
        for i in 0..n {
            let (a, b) = (f64::from_bits(next()), f64::from_bits(next()));
            let c = match i % 4 {
                0 => f64::from_bits(next()),
                // near cancellation: c close to -a*b
                1 => -(a * b) * (1.0 + (next() % 64) as f64 * f64::EPSILON),
                2 => -(a * b),
                // the operand ranges powf feeds it
                _ => {
                    let r = f64::from_bits(0x3fd0_0000_0000_0000 | next() >> 12) - 0.3;
                    let got = fma(r, a.abs().min(2.0), 1.0);
                    let want = r.mul_add(a.abs().min(2.0), 1.0);
                    bad += u64::from(
                        got.to_bits() != want.to_bits() && !(got.is_nan() && want.is_nan()),
                    );
                    continue;
                }
            };
            let (got, want) = (fma(a, b, c), a.mul_add(b, c));
            if got.to_bits() != want.to_bits() && !(got.is_nan() && want.is_nan()) {
                if bad < 5 {
                    eprintln!(
                        "fma({a:e}, {b:e}, {c:e}): ours {:#018x}, host {:#018x}",
                        got.to_bits(),
                        want.to_bits()
                    );
                }
                bad += 1;
            }
        }
        eprintln!("compared {n} multiply-adds: {bad} mismatches");
        assert_eq!(bad, 0);
    }
}
