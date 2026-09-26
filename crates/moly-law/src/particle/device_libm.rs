//! The four C library functions the engine's weighted curve segment calls on the
//! device (`logf`, `exp`, `cosf` and `atan2f`), in plain Rust, the `sincosf` of the
//! engine's Euler-angle conversion (see [`super::placement`]), and the `log2f` and
//! `exp2f` of the sphere kernels' shell preparation (see [`super::shape::Shell`]).
//!
//! The Bezier time solve of a weighted AnimationCurve segment calls them through the
//! dynamic linker, so its value is whatever the phone's libm returns. On Android that is
//! bionic's libm. Since Android 10 bionic builds `logf`, `exp` and `cosf` from Arm
//! Optimized Routines and `atan2f` (with `atanf`) from FreeBSD's msun; Android 7 to 9
//! built all four from msun. This module reproduces the Android 10 and later functions:
//!
//! * The multiply-adds the arm64 build fuses are fused here (`f64::mul_add`, one
//!   rounding on every target); the ones it leaves as a multiply and an add stay so.
//!   The msun functions fuse nothing.
//! * Every other operation is one IEEE operation in the machine code's order, and the
//!   rounding instructions are reproduced (`frinta` and `fcvtas` round half away from
//!   zero, the conversion to single precision rounds to nearest even).
//! * NaN results are NaN; their payload bits are not reproduced. No caller here reads a
//!   NaN payload: each function returns NaN for a NaN argument whatever its payload, and
//!   the solve only tests NaN-ness.
//!
//! How this is known: the AOSP arm64 emulator images for API 29, 31, 33, 35 and 36 carry
//! these functions with these table values; executed in an emulator they give the same
//! bits as this module. The API 24 and 28 images carry the msun `logf`, `exp` and
//! `cosf`; this module does not reproduce those. `sincosf`, `log2f` and `exp2f` are the
//! API 29 image's, executed in an emulator over every path of each function.
//!
//! Ported from Arm Optimized Routines (`math/logf.c`, `math/logf_data.c`, `math/exp.c`,
//! `math/exp_data.c`, `math/cosf.c`, `math/sincosf.c`, `math/sincosf.h`,
//! `math/sincosf_data.c`, `math/log2f.c`, `math/log2f_data.c`, `math/exp2f.c`,
//! `math/exp2f_data.c`),
//! Copyright (c) 2017-2018 Arm Limited, MIT licence, and from FreeBSD msun
//! (`e_atan2f.c`, `s_atanf.c`), Copyright (C) 1993 by Sun Microsystems, Inc.; see the
//! repository's third-party notices.

/// `logf(x)`.
///
/// Fused on the device: `invc * z - 1`, `ln2 * k + logc`, `A1 * r + A2`,
/// `A0 * r2 + y` and `r2 * y + (y0 + r)`.
pub fn logf(x: f32) -> f32 {
    let mut ix = x.to_bits();
    if ix.wrapping_sub(0x0080_0000) >= 0x7f80_0000 - 0x0080_0000 {
        // x below the smallest normal, inf or NaN.
        if ix.wrapping_mul(2) == 0 {
            return f32::NEG_INFINITY; // -1 / 0
        }
        if ix == 0x7f80_0000 {
            return x;
        }
        if ix & 0x8000_0000 != 0 || ix.wrapping_mul(2) >= 0xff00_0000 {
            return f32::NAN; // (x - x) / (x - x)
        }
        // A positive subnormal: normalize it.
        ix = (x * f32::from_bits(0x4b00_0000)).to_bits().wrapping_sub(23 << 23);
    }
    const OFF: u32 = 0x3f33_0000;
    let tmp = ix.wrapping_sub(OFF);
    let i = ((tmp >> 19) & 15) as usize;
    let k = (tmp as i32) >> 23;
    let iz = ix.wrapping_sub(tmp & 0xff80_0000);
    let invc = f64::from_bits(LOGF_T[2 * i]);
    let logc = f64::from_bits(LOGF_T[2 * i + 1]);
    let z = f32::from_bits(iz) as f64;
    let r = invc.mul_add(z, -1.0);
    let y0 = LOGF_LN2.mul_add(k as f64, logc);
    let r2 = r * r;
    let y = LOGF_A[1].mul_add(r, LOGF_A[2]);
    let y = LOGF_A[0].mul_add(r2, y);
    r2.mul_add(y, y0 + r) as f32
}

const LOGF_LN2: f64 = f64::from_bits(0x3fe6_2e42_fefa_39ef);
const LOGF_A: [f64; 3] = [
    f64::from_bits(0xbfd0_0ea3_48b8_8334),
    f64::from_bits(0x3fd5_575b_0be0_0b6a),
    f64::from_bits(0xbfdf_fffe_f20a_4123),
];

/// `exp(x)`.
///
/// Fused on the device: both reduction steps (`-ln2hi/N * kd + x` and
/// `kd * -ln2lo/N + r`), `C3 * r + C2`, `r2 * (C2 + C3 r) + (tail + r)`,
/// `r * C5 + C4`, `r4 * (C4 + C5 r) + ...` and `tmp * scale + scale`; in the branch
/// whose result is subnormal, `scale * tmp` and its sum are separate operations.
pub fn exp(x: f64) -> f64 {
    let ix = x.to_bits();
    let mut abstop = ((ix >> 52) & 0x7ff) as u32;
    if abstop.wrapping_sub(0x3c9) >= 0x3f {
        if (abstop.wrapping_sub(0x3c9) as i32) < 0 {
            // |x| < 2^-54: 1 + x rounds to 1.
            return 1.0;
        }
        if abstop >= 0x409 {
            if ix == f64::NEG_INFINITY.to_bits() {
                return 0.0;
            }
            if abstop == 0x7ff {
                return 1.0 + x;
            }
            return if ix >> 63 != 0 { 0.0 } else { f64::INFINITY };
        }
        // 512 <= |x| < 1024: the scale is handled below.
        abstop = 0;
    }
    let z = f64::from_bits(EXP_INVLN2N) * x;
    let kd = round_half_away(z);
    let ki = round_half_away_to_i64(z) as u64;
    let r = f64::from_bits(EXP_NEGLN2HIN).mul_add(kd, x);
    let r = kd.mul_add(f64::from_bits(EXP_NEGLN2LON), r);
    let idx = 2 * (ki & 0x7f) as usize;
    let tail = f64::from_bits(EXP_TAB[idx]);
    let sbits = EXP_TAB[idx + 1].wrapping_add(ki << 45);
    let [c2, c3, c4, c5] = EXP_C.map(f64::from_bits);
    let r2 = r * r;
    let p = c3.mul_add(r, c2);
    let q = r2.mul_add(p, tail + r);
    let s = r.mul_add(c5, c4);
    let tmp = (r2 * r2).mul_add(s, q);
    if abstop == 0 {
        return exp_special(tmp, sbits, ki);
    }
    let scale = f64::from_bits(sbits);
    tmp.mul_add(scale, scale)
}

/// The scale of `exp` for 512 <= |x| < 1024, where `2^k` alone would overflow or be
/// subnormal.
fn exp_special(tmp: f64, sbits: u64, ki: u64) -> f64 {
    if ki & 0x8000_0000 == 0 {
        let scale = f64::from_bits(sbits.wrapping_sub(1009 << 52));
        return tmp.mul_add(scale, scale) * f64::from_bits(0x7f00_0000_0000_0000);
    }
    let scale = f64::from_bits(sbits.wrapping_add(1022 << 52));
    let st = tmp * scale;
    let mut y = st + scale;
    if y < 1.0 {
        let hi = y + 1.0;
        let lo = st + (scale - y);
        let lo = lo + (y + (1.0 - hi));
        y = (hi + lo) + -1.0;
    }
    y * f64::from_bits(0x0010_0000_0000_0000)
}

const EXP_INVLN2N: u64 = 0x4067_1547_652b_82fe;
const EXP_NEGLN2HIN: u64 = 0xbf76_2e42_fefa_0000;
const EXP_NEGLN2LON: u64 = 0xbd0c_f79a_bc9e_3b3a;
const EXP_C: [u64; 4] = [0x3fdf_ffff_ffff_fdbd, 0x3fc5_5555_5555_543c, 0x3fa5_5555_cf17_2b91, 0x3f81_1111_67a4_d017];

/// arm64 `frinta`: to the nearest integer, halves away from zero.
fn round_half_away(x: f64) -> f64 {
    x.round()
}

/// arm64 `fcvtas`: to the nearest integer, halves away from zero, saturating, NaN to 0.
fn round_half_away_to_i64(x: f64) -> i64 {
    x.round() as i64
}

/// `cosf(y)`.
///
/// Fused on the device: the fast reduction `x - kd * pi/2`, and in the polynomials
/// `C0 + x2 C1`, `C3 + x2 C4`, `(C0 + x2 C1) + x4 C2`, `S2 + x2 S3` and `x + x3 S1`.
/// The last cosine term `c + x6 (C3 + x2 C4)` is fused for |y| < pi/4 only; after a
/// reduction it, and the last sine term `s + x7 (S2 + x2 S3)`, are a multiply and an
/// add.
pub fn cosf(y: f32) -> f32 {
    let ix = y.to_bits();
    let top = (ix >> 20) & 0x7ff;
    let x = y as f64;
    if top < 0x3f4 {
        // |y| < pi/4.
        if top < 0x398 {
            return 1.0; // |y| < 2^-12
        }
        let t = &SINCOSF[0];
        let x2 = x * x;
        let x4 = x2 * x2;
        let c2 = x2.mul_add(t.c[4], t.c[3]);
        let c1 = x2.mul_add(t.c[1], t.c[0]);
        let x6 = x2 * x4;
        let c = x4.mul_add(t.c[2], c1);
        return x6.mul_add(c2, c) as f32;
    }
    let (r, n, sign_n) = if top < 0x42f {
        // |y| < 120.
        let t = &SINCOSF[0];
        let z = t.hpi_inv * x;
        let n = round_half_away_to_i64(z) as i32;
        let r = (-t.hpi).mul_add(round_half_away(z), x);
        (r, n, n)
    } else if top < 0x7f8 {
        let (r, n) = reduce_large(ix);
        (r, n, n.wrapping_add((ix >> 31) as i32))
    } else {
        return f32::NAN; // (y - y) / (y - y)
    };
    let t = if sign_n & 2 == 0 { &SINCOSF[0] } else { &SINCOSF[1] };
    let x2 = r * r;
    if n & 1 == 0 {
        // The cosine polynomial of the reduced argument.
        let x4 = x2 * x2;
        let x6 = x2 * x4;
        let c2 = x2.mul_add(t.c[4], t.c[3]);
        let c1 = x2.mul_add(t.c[1], t.c[0]);
        let c = x4.mul_add(t.c[2], c1);
        (x6 * c2 + c) as f32
    } else {
        // The sine polynomial of the signed reduced argument.
        let x = SINCOSF[0].sign[(sign_n & 3) as usize] * r;
        let s1 = x2.mul_add(t.s[2], t.s[1]);
        let x3 = x * x2;
        let x7 = x2 * x3;
        let s = x3.mul_add(t.s[0], x);
        (x7 * s1 + s) as f32
    }
}

/// `sincosf(y)`: `(sin y, cos y)`.
///
/// Fused on the device, on every path: the fast reduction `x - kd * pi/2` and every
/// polynomial term, the last sine term `s + x5 (S2 + x2 S3)` and the last cosine term
/// `c + x6 (C3 + x2 C4)` included (unlike `cosf`, whose last terms after a reduction are
/// a multiply and an add). Below 2^-12 the pair is `(y, 1)`. After a reduction the
/// sine polynomial takes the signed reduced argument and an odd quadrant swaps the pair.
pub fn sincosf(y: f32) -> (f32, f32) {
    let ix = y.to_bits();
    let top = (ix >> 20) & 0x7ff;
    let x = y as f64;
    if top < 0x3f4 {
        // |y| < pi/4.
        if top < 0x398 {
            return (y, 1.0); // |y| < 2^-12
        }
        let t = &SINCOSF[0];
        let x2 = x * x;
        let x3 = x2 * x;
        let c2 = x2.mul_add(t.c[4], t.c[3]);
        let s1 = x2.mul_add(t.s[2], t.s[1]);
        let s = x3.mul_add(t.s[0], x);
        let x4 = x2 * x2;
        let x5 = x2 * x3;
        let c1 = x2.mul_add(t.c[1], t.c[0]);
        let x6 = x2 * x4;
        let c = x4.mul_add(t.c[2], c1);
        return (x5.mul_add(s1, s) as f32, x6.mul_add(c2, c) as f32);
    }
    let (r, n, sign_n) = if top < 0x42f {
        // |y| < 120.
        let t = &SINCOSF[0];
        let z = t.hpi_inv * x;
        let n = round_half_away_to_i64(z) as i32;
        let r = (-t.hpi).mul_add(round_half_away(z), x);
        (r, n, n)
    } else if top < 0x7f8 {
        let (r, n) = reduce_large(ix);
        (r, n, n.wrapping_add((ix >> 31) as i32))
    } else {
        let nan = y - y;
        return (nan, nan);
    };
    let t = if sign_n & 2 == 0 { &SINCOSF[0] } else { &SINCOSF[1] };
    let x2 = r * r;
    let xs = SINCOSF[0].sign[(sign_n & 3) as usize] * r;
    let c2 = x2.mul_add(t.c[4], t.c[3]);
    let s1 = x2.mul_add(t.s[2], t.s[1]);
    let x4 = x2 * x2;
    let c1 = x2.mul_add(t.c[1], t.c[0]);
    let x3 = xs * x2;
    let s = x3.mul_add(t.s[0], xs);
    let x6 = x2 * x4;
    let x5 = x2 * x3;
    let c = x4.mul_add(t.c[2], c1);
    let sine = x5.mul_add(s1, s) as f32;
    let cosine = x6.mul_add(c2, c) as f32;
    if n & 1 == 0 { (sine, cosine) } else { (cosine, sine) }
}

/// The large-argument reduction of `cosf` (120 <= |y| < inf): `y * 2/pi` in fixed point
/// from 96 bits of 2/pi, then the quadrant and the remainder times pi/2^63.
fn reduce_large(ix: u32) -> (f64, i32) {
    let arr = &INV_PIO4[((ix >> 26) & 15) as usize..];
    let shift = (ix >> 23) & 7;
    let xi = ((ix & 0x007f_ffff) | 0x0080_0000) << shift;
    let res0 = xi.wrapping_mul(arr[0]) as u64;
    let res1 = xi as u64 * arr[4] as u64;
    let res2 = xi as u64 * arr[8] as u64;
    let res0 = ((res2 >> 32) | (res0 << 32)).wrapping_add(res1);
    let n = res0.wrapping_add(1 << 61) >> 62;
    let res0 = res0.wrapping_sub(n << 62);
    ((res0 as i64) as f64 * f64::from_bits(0x3c19_21fb_5444_2d18), n as i32)
}

struct SinCosTable {
    sign: [f64; 4],
    hpi_inv: f64,
    hpi: f64,
    c: [f64; 5],
    s: [f64; 3],
}

const fn b(x: u64) -> f64 {
    f64::from_bits(x)
}

const SINCOSF: [SinCosTable; 2] = [
    SinCosTable {
        sign: [b(0x3ff0000000000000), b(0xbff0000000000000), b(0xbff0000000000000), b(0x3ff0000000000000)],
        hpi_inv: b(0x3fe45f306dc9c883),
        hpi: b(0x3ff921fb54442d18),
        c: [b(0x3ff0000000000000), b(0xbfdffffffd0c621c), b(0x3fa55553e1068f19), b(0xbf56c087e89a359d),
            b(0x3ef99343027bf8c3)],
        s: [b(0xbfc555545995a603), b(0x3f81107605230bc4), b(0xbf2994eb3774cf24)],
    },
    SinCosTable {
        sign: [b(0x3ff0000000000000), b(0xbff0000000000000), b(0xbff0000000000000), b(0x3ff0000000000000)],
        hpi_inv: b(0x3fe45f306dc9c883),
        hpi: b(0x3ff921fb54442d18),
        c: [b(0xbff0000000000000), b(0x3fdffffffd0c621c), b(0xbfa55553e1068f19), b(0x3f56c087e89a359d),
            b(0xbef99343027bf8c3)],
        s: [b(0xbfc555545995a603), b(0x3f81107605230bc4), b(0xbf2994eb3774cf24)],
    },
];

/// `log2f(x)`.
///
/// Fused on the device: `invc * z - 1`, `A1 * r + A2`, `A0 * r2 + y`, `A3 * r + y0`
/// and `r2 * y + p`; `logc + k` is a separate add. No errno is set (the image is built
/// without it): +-0 gives -inf, a negative or NaN argument NaN.
pub fn log2f(x: f32) -> f32 {
    let mut ix = x.to_bits();
    if ix.wrapping_sub(0x0080_0000) >= 0x7f80_0000 - 0x0080_0000 {
        // x below the smallest normal, inf or NaN.
        if ix.wrapping_mul(2) == 0 {
            return f32::NEG_INFINITY; // -1 / 0
        }
        if ix == 0x7f80_0000 {
            return x;
        }
        if ix & 0x8000_0000 != 0 || ix.wrapping_mul(2) >= 0xff00_0000 {
            return f32::NAN; // (x - x) / (x - x)
        }
        // A positive subnormal: normalize it.
        #[cfg(test)]
        let skip = super::shape::shell_arms::on("log2fSubnormalUnscaled");
        #[cfg(not(test))]
        let skip = false;
        if !skip {
            ix = (x * f32::from_bits(0x4b00_0000)).to_bits().wrapping_sub(23 << 23);
        }
    }
    const OFF: u32 = 0x3f33_0000;
    let tmp = ix.wrapping_sub(OFF);
    let i = ((tmp >> 19) & 15) as usize;
    let iz = ix.wrapping_sub(tmp & 0xff80_0000);
    let k = (tmp as i32) >> 23;
    let invc = f64::from_bits(LOG2F_T[2 * i]);
    let logc = f64::from_bits(LOG2F_T[2 * i + 1]);
    let z = f32::from_bits(iz) as f64;
    let r = invc.mul_add(z, -1.0);
    let y0 = logc + k as f64;
    let [a0, a1, a2, a3] = LOG2F_A.map(f64::from_bits);
    let y = a1.mul_add(r, a2);
    let r2 = r * r;
    let y = a0.mul_add(r2, y);
    let p = a3.mul_add(r, y0);
    r2.mul_add(y, p) as f32
}

const LOG2F_A: [u64; 4] = [0xbfd712b6f70a7e4d, 0x3fdecabf496832e0, 0xbfe715479ffae3de, 0x3ff715475f35c8b8];

/// `exp2f(x)`.
///
/// Fused on the device: `C0 * r + C1`, `r * C2 + 1` and `r2 * z + y`; the shift, the
/// reduction and the final scaling are separate operations. -inf gives +0, x at or
/// below -150 gives +0 and a positive x at or above 128 gives +inf (no errno); inf and
/// NaN give `x + x`.
pub fn exp2f(x: f32) -> f32 {
    let bits = x.to_bits();
    let abstop = (bits >> 20) & 0x7ff;
    if abstop >= 0x430 {
        // |x| >= 128, or x is NaN.
        if bits == 0xff80_0000 {
            return 0.0;
        }
        if abstop >= 0x7f8 {
            return x + x;
        }
        if x > 0.0 {
            return f32::INFINITY; // 0x1p97 * 0x1p97
        }
        if x <= -150.0 {
            return 0.0; // 0x1p-95 * 0x1p-95
        }
        #[cfg(test)]
        if super::shape::shell_arms::on("exp2fUnderflowBelow149") && x < -149.0 {
            return 0.0;
        }
    }
    let xd = x as f64;
    let shift = f64::from_bits(EXP2F_SHIFT);
    let kd = shift + xd;
    let ki = kd.to_bits();
    let kd = kd - shift;
    let r = xd - kd;
    let [c0, c1, c2] = EXP2F_C.map(f64::from_bits);
    let t = EXP2F_T[(ki & 31) as usize].wrapping_add(ki << 47);
    let z = c0.mul_add(r, c1);
    let r2 = r * r;
    let y = r.mul_add(c2, 1.0);
    let y = r2.mul_add(z, y);
    (y * f64::from_bits(t)) as f32
}

/// `0x1.8p+52 / 32`.
const EXP2F_SHIFT: u64 = 0x42e8000000000000;
const EXP2F_C: [u64; 3] = [0x3fac6af84b912394, 0x3fcebfce50fac4f3, 0x3fe62e42ff0c52d6];

/// `atan2f(y, x)` (msun: nothing fused, every constant single precision).
pub fn atan2f(y: f32, x: f32) -> f32 {
    const TINY: f32 = f32::from_bits(0x0da2_4260);
    const PI_LO: f32 = f32::from_bits(0xb3bb_bd2e);
    const PI_O_4: f32 = f32::from_bits(0x3f49_0fdb);
    const PI_O_2: f32 = f32::from_bits(0x3fc9_0fdb);
    const PI: f32 = f32::from_bits(0x4049_0fdb);
    const THREE_PI_O_4: f32 = f32::from_bits(0x4016_cbe4);
    let hy = y.to_bits();
    let iy = hy & 0x7fff_ffff;
    let hx = x.to_bits();
    let ix = hx & 0x7fff_ffff;
    if iy > 0x7f80_0000 || ix > 0x7f80_0000 {
        return f32::NAN;
    }
    if hx == 0x3f80_0000 {
        return atanf(y);
    }
    let m = ((hx >> 30) & 2) | (hy >> 31);
    if iy == 0 {
        return match m {
            2 => TINY + PI,
            3 => -PI - TINY,
            _ => y,
        };
    }
    if ix == 0x7f80_0000 {
        return if iy == 0x7f80_0000 {
            match m {
                0 => TINY + PI_O_4,
                1 => -PI_O_4 - TINY,
                2 => TINY + THREE_PI_O_4,
                _ => -THREE_PI_O_4 - TINY,
            }
        } else {
            match m {
                0 => 0.0,
                1 => -0.0,
                2 => TINY + PI,
                _ => -PI - TINY,
            }
        };
    }
    if ix == 0 || iy == 0x7f80_0000 {
        return if (hy as i32) < 0 { -PI_O_2 - TINY } else { TINY + PI_O_2 };
    }
    let k = (iy as i32).wrapping_sub(ix as i32);
    if k >= 27 << 23 {
        // |y / x| > 2^26
        let z = PI_LO * 0.5 + PI_O_2;
        return if hy >> 31 != 0 { -z } else { z };
    }
    let z = if (hx as i32) < 0 && k < -(26 << 23) { 0.0 } else { atanf((y / x).abs()) };
    match m {
        0 => z,
        1 => -z,
        2 => (PI_LO - z) + PI,
        _ => (z - PI_LO) + -PI,
    }
}

/// `atanf(x)` (msun: nothing fused).
fn atanf(x: f32) -> f32 {
    const ATANHI: [u32; 4] = [0x3eed_6338, 0x3f49_0fda, 0x3f7b_985e, 0x3fc9_0fda];
    const ATANLO: [u32; 4] = [0x31ac_3769, 0x3322_2168, 0x3314_0fb4, 0x33a2_2168];
    const AT: [u32; 5] = [0x3eaa_aaa9, 0xbe4c_ca98, 0x3e11_f50d, 0xbdda_1247, 0x3d7c_ac25];
    const HUGE: f32 = f32::from_bits(0x7149_f2ca);
    let f = f32::from_bits;
    let hx = x.to_bits();
    let ix = hx & 0x7fff_ffff;
    if ix >= 0x4c80_0000 {
        // |x| >= 2^26
        if ix > 0x7f80_0000 {
            return x + x;
        }
        return if (hx as i32) < 1 { -f(ATANHI[3]) - f(ATANLO[3]) } else { f(ATANLO[3]) + f(ATANHI[3]) };
    }
    let (x, id) = if ix < 0x3ee0_0000 {
        // |x| < 0.4375
        if x + HUGE > 1.0 && ix < 0x3980_0000 {
            return x; // |x| < 2^-12
        }
        (x, -1)
    } else {
        let x = x.abs();
        if ix < 0x3f98_0000 {
            if ix < 0x3f30_0000 {
                (((x + x) + -1.0) / (x + 2.0), 0)
            } else {
                ((x + -1.0) / (x + 1.0), 1)
            }
        } else if ix <= 0x401b_ffff {
            ((x + -1.5) / (x * 1.5 + 1.0), 2)
        } else {
            (-1.0 / x, 3)
        }
    };
    let z = x * x;
    let w = z * z;
    let s1 = z * (w * (w * f(AT[4]) + f(AT[2])) + f(AT[0]));
    let s2 = w * (w * f(AT[3]) + f(AT[1]));
    if id < 0 {
        return x - x * (s2 + s1);
    }
    let id = id as usize;
    let z = f(ATANHI[id]) - ((x * (s2 + s1) - f(ATANLO[id])) - x);
    if (hx as i32) < 0 { -z } else { z }
}

const LOGF_T: [u64; 32] = [
    0x3ff661ec79f8f3be, 0xbfd57bf7808caade,
    0x3ff571ed4aaf883d, 0xbfd2bef0a7c06ddb,
    0x3ff49539f0f010b0, 0xbfd01eae7f513a67,
    0x3ff3c995b0b80385, 0xbfcb31d8a68224e9,
    0x3ff30d190c8864a5, 0xbfc6574f0ac07758,
    0x3ff25e227b0b8ea0, 0xbfc1aa2bc79c8100,
    0x3ff1bb4a4a1a343f, 0xbfba4e76ce8c0e5e,
    0x3ff12358f08ae5ba, 0xbfb1973c5a611ccc,
    0x3ff0953f419900a7, 0xbfa252f438e10c1e,
    0x3ff0000000000000, 0x0000000000000000,
    0x3fee608cfd9a47ac, 0x3faaa5aa5df25984,
    0x3feca4b31f026aa0, 0x3fbc5e53aa362eb4,
    0x3feb2036576afce6, 0x3fc526e57720db08,
    0x3fe9c2d163a1aa2d, 0x3fcbc2860d224770,
    0x3fe886e6037841ed, 0x3fd1058bc8a07ee1,
    0x3fe767dcf5534862, 0x3fd4043057b6ee09,
];
const LOG2F_T: [u64; 32] = [
    0x3ff661ec79f8f3be, 0xbfdefec65b963019,
    0x3ff571ed4aaf883d, 0xbfdb0b6832d4fca4,
    0x3ff49539f0f010b0, 0xbfd7418b0a1fb77b,
    0x3ff3c995b0b80385, 0xbfd39de91a6dcf7b,
    0x3ff30d190c8864a5, 0xbfd01d9bf3f2b631,
    0x3ff25e227b0b8ea0, 0xbfc97c1d1b3b7af0,
    0x3ff1bb4a4a1a343f, 0xbfc2f9e393af3c9f,
    0x3ff12358f08ae5ba, 0xbfb960cbbf788d5c,
    0x3ff0953f419900a7, 0xbfaa6f9db6475fce,
    0x3ff0000000000000, 0x0000000000000000,
    0x3fee608cfd9a47ac, 0x3fb338ca9f24f53d,
    0x3feca4b31f026aa0, 0x3fc476a9543891ba,
    0x3feb2036576afce6, 0x3fce840b4ac4e4d2,
    0x3fe9c2d163a1aa2d, 0x3fd40645f0c6651c,
    0x3fe886e6037841ed, 0x3fd88e9c2c1b9ff8,
    0x3fe767dcf5534862, 0x3fdce0a44eb17bcc,
];
/// `2^(i/32)` with the low exponent bits cleared for the `ki << 47` addition.
const EXP2F_T: [u64; 32] = [
    0x3ff0000000000000, 0x3fefd9b0d3158574, 0x3fefb5586cf9890f, 0x3fef9301d0125b51,
    0x3fef72b83c7d517b, 0x3fef54873168b9aa, 0x3fef387a6e756238, 0x3fef1e9df51fdee1,
    0x3fef06fe0a31b715, 0x3feef1a7373aa9cb, 0x3feedea64c123422, 0x3feece086061892d,
    0x3feebfdad5362a27, 0x3feeb42b569d4f82, 0x3feeab07dd485429, 0x3feea47eb03a5585,
    0x3feea09e667f3bcd, 0x3fee9f75e8ec5f74, 0x3feea11473eb0187, 0x3feea589994cce13,
    0x3feeace5422aa0db, 0x3feeb737b0cdc5e5, 0x3feec49182a3f090, 0x3feed503b23e255d,
    0x3feee89f995ad3ad, 0x3feeff76f2fb5e47, 0x3fef199bdd85529c, 0x3fef3720dcef9069,
    0x3fef5818dcfba487, 0x3fef7c97337b9b5f, 0x3fefa4afa2a490da, 0x3fefd0765b6e4540,
];
const EXP_TAB: [u64; 256] = [
    0x0000000000000000, 0x3ff0000000000000,
    0x3c9b3b4f1a88bf6e, 0x3feff63da9fb3335,
    0xbc7160139cd8dc5d, 0x3fefec9a3e778061,
    0xbc905e7a108766d1, 0x3fefe315e86e7f85,
    0x3c8cd2523567f613, 0x3fefd9b0d3158574,
    0xbc8bce8023f98efa, 0x3fefd06b29ddf6de,
    0x3c60f74e61e6c861, 0x3fefc74518759bc8,
    0x3c90a3e45b33d399, 0x3fefbe3ecac6f383,
    0x3c979aa65d837b6d, 0x3fefb5586cf9890f,
    0x3c8eb51a92fdeffc, 0x3fefac922b7247f7,
    0x3c3ebe3d702f9cd1, 0x3fefa3ec32d3d1a2,
    0xbc6a033489906e0b, 0x3fef9b66affed31b,
    0xbc9556522a2fbd0e, 0x3fef9301d0125b51,
    0xbc5080ef8c4eea55, 0x3fef8abdc06c31cc,
    0xbc91c923b9d5f416, 0x3fef829aaea92de0,
    0x3c80d3e3e95c55af, 0x3fef7a98c8a58e51,
    0xbc801b15eaa59348, 0x3fef72b83c7d517b,
    0xbc8f1ff055de323d, 0x3fef6af9388c8dea,
    0x3c8b898c3f1353bf, 0x3fef635beb6fcb75,
    0xbc96d99c7611eb26, 0x3fef5be084045cd4,
    0x3c9aecf73e3a2f60, 0x3fef54873168b9aa,
    0xbc8fe782cb86389d, 0x3fef4d5022fcd91d,
    0x3c8a6f4144a6c38d, 0x3fef463b88628cd6,
    0x3c807a05b0e4047d, 0x3fef3f49917ddc96,
    0x3c968efde3a8a894, 0x3fef387a6e756238,
    0x3c875e18f274487d, 0x3fef31ce4fb2a63f,
    0x3c80472b981fe7f2, 0x3fef2b4565e27cdd,
    0xbc96b87b3f71085e, 0x3fef24dfe1f56381,
    0x3c82f7e16d09ab31, 0x3fef1e9df51fdee1,
    0xbc3d219b1a6fbffa, 0x3fef187fd0dad990,
    0x3c8b3782720c0ab4, 0x3fef1285a6e4030b,
    0x3c6e149289cecb8f, 0x3fef0cafa93e2f56,
    0x3c834d754db0abb6, 0x3fef06fe0a31b715,
    0x3c864201e2ac744c, 0x3fef0170fc4cd831,
    0x3c8fdd395dd3f84a, 0x3feefc08b26416ff,
    0xbc86a3803b8e5b04, 0x3feef6c55f929ff1,
    0xbc924aedcc4b5068, 0x3feef1a7373aa9cb,
    0xbc9907f81b512d8e, 0x3feeecae6d05d866,
    0xbc71d1e83e9436d2, 0x3feee7db34e59ff7,
    0xbc991919b3ce1b15, 0x3feee32dc313a8e5,
    0x3c859f48a72a4c6d, 0x3feedea64c123422,
    0xbc9312607a28698a, 0x3feeda4504ac801c,
    0xbc58a78f4817895b, 0x3feed60a21f72e2a,
    0xbc7c2c9b67499a1b, 0x3feed1f5d950a897,
    0x3c4363ed60c2ac11, 0x3feece086061892d,
    0x3c9666093b0664ef, 0x3feeca41ed1d0057,
    0x3c6ecce1daa10379, 0x3feec6a2b5c13cd0,
    0x3c93ff8e3f0f1230, 0x3feec32af0d7d3de,
    0x3c7690cebb7aafb0, 0x3feebfdad5362a27,
    0x3c931dbdeb54e077, 0x3feebcb299fddd0d,
    0xbc8f94340071a38e, 0x3feeb9b2769d2ca7,
    0xbc87deccdc93a349, 0x3feeb6daa2cf6642,
    0xbc78dec6bd0f385f, 0x3feeb42b569d4f82,
    0xbc861246ec7b5cf6, 0x3feeb1a4ca5d920f,
    0x3c93350518fdd78e, 0x3feeaf4736b527da,
    0x3c7b98b72f8a9b05, 0x3feead12d497c7fd,
    0x3c9063e1e21c5409, 0x3feeab07dd485429,
    0x3c34c7855019c6ea, 0x3feea9268a5946b7,
    0x3c9432e62b64c035, 0x3feea76f15ad2148,
    0xbc8ce44a6199769f, 0x3feea5e1b976dc09,
    0xbc8c33c53bef4da8, 0x3feea47eb03a5585,
    0xbc845378892be9ae, 0x3feea34634ccc320,
    0xbc93cedd78565858, 0x3feea23882552225,
    0x3c5710aa807e1964, 0x3feea155d44ca973,
    0xbc93b3efbf5e2228, 0x3feea09e667f3bcd,
    0xbc6a12ad8734b982, 0x3feea012750bdabf,
    0xbc6367efb86da9ee, 0x3fee9fb23c651a2f,
    0xbc80dc3d54e08851, 0x3fee9f7df9519484,
    0xbc781f647e5a3ecf, 0x3fee9f75e8ec5f74,
    0xbc86ee4ac08b7db0, 0x3fee9f9a48a58174,
    0xbc8619321e55e68a, 0x3fee9feb564267c9,
    0x3c909ccb5e09d4d3, 0x3feea0694fde5d3f,
    0xbc7b32dcb94da51d, 0x3feea11473eb0187,
    0x3c94ecfd5467c06b, 0x3feea1ed0130c132,
    0x3c65ebe1abd66c55, 0x3feea2f336cf4e62,
    0xbc88a1c52fb3cf42, 0x3feea427543e1a12,
    0xbc9369b6f13b3734, 0x3feea589994cce13,
    0xbc805e843a19ff1e, 0x3feea71a4623c7ad,
    0xbc94d450d872576e, 0x3feea8d99b4492ed,
    0x3c90ad675b0e8a00, 0x3feeaac7d98a6699,
    0x3c8db72fc1f0eab4, 0x3feeace5422aa0db,
    0xbc65b6609cc5e7ff, 0x3feeaf3216b5448c,
    0x3c7bf68359f35f44, 0x3feeb1ae99157736,
    0xbc93091fa71e3d83, 0x3feeb45b0b91ffc6,
    0xbc5da9b88b6c1e29, 0x3feeb737b0cdc5e5,
    0xbc6c23f97c90b959, 0x3feeba44cbc8520f,
    0xbc92434322f4f9aa, 0x3feebd829fde4e50,
    0xbc85ca6cd7668e4b, 0x3feec0f170ca07ba,
    0x3c71affc2b91ce27, 0x3feec49182a3f090,
    0x3c6dd235e10a73bb, 0x3feec86319e32323,
    0xbc87c50422622263, 0x3feecc667b5de565,
    0x3c8b1c86e3e231d5, 0x3feed09bec4a2d33,
    0xbc91bbd1d3bcbb15, 0x3feed503b23e255d,
    0x3c90cc319cee31d2, 0x3feed99e1330b358,
    0x3c8469846e735ab3, 0x3feede6b5579fdbf,
    0xbc82dfcd978e9db4, 0x3feee36bbfd3f37a,
    0x3c8c1a7792cb3387, 0x3feee89f995ad3ad,
    0xbc907b8f4ad1d9fa, 0x3feeee07298db666,
    0xbc55c3d956dcaeba, 0x3feef3a2b84f15fb,
    0xbc90a40e3da6f640, 0x3feef9728de5593a,
    0xbc68d6f438ad9334, 0x3feeff76f2fb5e47,
    0xbc91eee26b588a35, 0x3fef05b030a1064a,
    0x3c74ffd70a5fddcd, 0x3fef0c1e904bc1d2,
    0xbc91bdfbfa9298ac, 0x3fef12c25bd71e09,
    0x3c736eae30af0cb3, 0x3fef199bdd85529c,
    0x3c8ee3325c9ffd94, 0x3fef20ab5fffd07a,
    0x3c84e08fd10959ac, 0x3fef27f12e57d14b,
    0x3c63cdaf384e1a67, 0x3fef2f6d9406e7b5,
    0x3c676b2c6c921968, 0x3fef3720dcef9069,
    0xbc808a1883ccb5d2, 0x3fef3f0b555dc3fa,
    0xbc8fad5d3ffffa6f, 0x3fef472d4a07897c,
    0xbc900dae3875a949, 0x3fef4f87080d89f2,
    0x3c74a385a63d07a7, 0x3fef5818dcfba487,
    0xbc82919e2040220f, 0x3fef60e316c98398,
    0x3c8e5a50d5c192ac, 0x3fef69e603db3285,
    0x3c843a59ac016b4b, 0x3fef7321f301b460,
    0xbc82d52107b43e1f, 0x3fef7c97337b9b5f,
    0xbc892ab93b470dc9, 0x3fef864614f5a129,
    0x3c74b604603a88d3, 0x3fef902ee78b3ff6,
    0x3c83c5ec519d7271, 0x3fef9a51fbc74c83,
    0xbc8ff7128fd391f0, 0x3fefa4afa2a490da,
    0xbc8dae98e223747d, 0x3fefaf482d8e67f1,
    0x3c8ec3bc41aa2008, 0x3fefba1bee615a27,
    0x3c842b94c3a9eb32, 0x3fefc52b376bba97,
    0x3c8a64a931d185ee, 0x3fefd0765b6e4540,
    0xbc8e37bae43be3ed, 0x3fefdbfdad9cbe14,
    0x3c77893b4d91cd9d, 0x3fefe7c1819e90d8,
    0x3c5305c14160cc89, 0x3feff3c22b8f71f1,
];
const INV_PIO4: [u32; 24] = [
    0x000000a2, 0x0000a2f9, 0x00a2f983, 0xa2f9836e, 0xf9836e4e, 0x836e4e44,
    0x6e4e4415, 0x4e441529, 0x441529fc, 0x1529fc27, 0x29fc2757, 0xfc2757d1,
    0x2757d1f5, 0x57d1f534, 0xd1f534dd, 0xf534ddc0, 0x34ddc0db, 0xddc0db62,
    0xc0db6295, 0xdb629599, 0x6295993c, 0x95993c43, 0x993c4390, 0x3c439041,
];

#[cfg(test)]
mod native_tests {
    use super::*;

    fn same32(a: f32, native: u32) -> bool {
        (a.is_nan() && f32::from_bits(native).is_nan()) || a.to_bits() == native
    }

    fn same64(a: f64, native: u64) -> bool {
        (a.is_nan() && f64::from_bits(native).is_nan()) || a.to_bits() == native
    }

    // Rows of the device libm executed in an emulator: [fn, in0, in1, out_lo, out_hi]
    // with fn 0 logf, 1 cosf, 2 exp(f64(f32) / 3), 3 atan2f(y, x), 4 exp of raw f64 bits.
    // NaN results compared as a class.
    #[test]
    #[ignore = "MOLY_DEVICE_LIBM_NATIVE must identify the native device libm rows"]
    fn device_libm_matches_native() {
        let path = std::env::var_os("MOLY_DEVICE_LIBM_NATIVE").expect("MOLY_DEVICE_LIBM_NATIVE");
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(bytes.len() % 20, 0);
        let words: Vec<u32> = bytes.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
        let mut counts = [0usize; 5];
        let mut mismatches = 0;
        for row in words.chunks_exact(5) {
            let wide = |lo: u32, hi: u32| (hi as u64) << 32 | lo as u64;
            let ok = match row[0] {
                0 => same32(logf(f32::from_bits(row[1])), row[3]),
                1 => same32(cosf(f32::from_bits(row[1])), row[3]),
                2 => same64(exp(f32::from_bits(row[1]) as f64 / 3.0), wide(row[3], row[4])),
                3 => same32(atan2f(f32::from_bits(row[1]), f32::from_bits(row[2])), row[3]),
                4 => same64(exp(f64::from_bits(wide(row[1], row[2]))), wide(row[3], row[4])),
                other => panic!("function {other}"),
            };
            if !ok {
                if mismatches < 8 {
                    eprintln!("mismatch {row:08x?}");
                }
                mismatches += 1;
            }
            counts[row[0] as usize] += 1;
        }
        println!("device libm rows logf {} cosf {} exp3 {} atan2f {} exp {}, mismatches {mismatches}",
            counts[0], counts[1], counts[2], counts[3], counts[4]);
        assert_eq!(mismatches, 0);
        assert!(counts.iter().all(|&c| c >= 400_000), "{counts:?}");
    }
}
