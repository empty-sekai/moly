//! Current JP InitialModule::Start, one dispatched four-lane group.
//!
//! Current libunity 937c6d28...75badd9, entry 0xd53e2c. The caller owns
//! capacity, aligned birth storage and packing. Every nonempty group consumes
//! all four random lanes, including discarded tail lanes. This law excludes
//! Shape, StartVelocity, module updates and the game seed/lifecycle owner.
//!
//! Admission is deliberately bounded to constant/two-constant scalar curves,
//! finite constant colour and an autonomous initial context. Inherited initial
//! multipliers/offsets and generic curve preparation need their own evidence.
//! Nonzero randomize-rotation-direction remains unqualified. The native entry
//! uses the included ARM FRECPE/FRECPS law; host division is not a bit-equivalent
//! default. An explicit callback variant remains available for diagnostics.

use super::color::initial_rgba8;
use super::curve::CurveSampler;
use super::random::ParticleRandom;
use super::schema::StartParams;
use super::seed_owner::ModuleRandom;
use super::value::{MinMaxCurve, MinMaxGradient};

pub const MIN_LIFETIME: f32 = f32::from_bits(0x3727_c5ac);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitialField {
    Lifetime,
    SizeX,
    SizeY,
    SizeZ,
    RotationZ,
    RotationX,
    RotationY,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    InvalidLaneCount,
    InvalidTiming,
    InvalidRotationDirection,
    UnverifiedRotationDirection,
    MissingAxis(InitialField),
    UnsupportedCurve(InitialField),
    InvalidCurve(InitialField),
    UnsupportedColor,
    InvalidColor,
    UnverifiedInheritedContext,
    ReciprocalUnavailable,
}

/// Autonomous corresponds to the native neutral initial context: lifetime
/// and size multipliers 1, rotation offsets 0, colour multiplier white, and
/// curve-time override +Infinity. The inherited path is a typed refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitialContext {
    Autonomous,
    Inherited,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InitialGroupInput {
    /// Number of accepted particles in this group, 1..=4. Zero accepted
    /// particles must not dispatch Initial at all.
    pub active_lanes: usize,
    /// Storage capability and authored flags are distinct native gates.
    pub storage_size_3d: bool,
    pub storage_rotation_3d: bool,
    /// Kept independently for the later birth integration phase; Initial
    /// does not evaluate its curves at the birth fraction.
    pub birth_fraction: [f32; 4],
    /// Native x4 input, one curve evaluation time per SIMD lane.
    pub curve_time: [f32; 4],
    pub context: InitialContext,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InitialLane {
    pub seed: u32,
    /// Effective lifetime after the native minimum, before reciprocal.
    pub lifetime: f32,
    pub inverse_lifetime: f32,
    /// XYZ. None means native storage did not expose/write that axis.
    pub size: [Option<f32>; 3],
    /// XYZ, although RNG/evaluation order is Z, then X/Y when enabled.
    pub rotation: [Option<f32>; 3],
    pub color: [u8; 4],
    pub birth_fraction: f32,
    pub curve_time: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InitialGroup {
    pub active_lanes: usize,
    /// Includes all four native writes. Only the first active_lanes become
    /// live particles; unused lanes must not become a cached random stream.
    pub lanes: [InitialLane; 4],
    pub random_after: ModuleRandom,
}

#[derive(Clone, Debug)]
pub struct InitialLaw {
    source: StartParams,
    randomize_rotation_direction: f32,
}

impl InitialLaw {
    /// Keep authored controls until the actual storage flags are known.
    /// Unsupported dormant axes do not reject a scalar-storage invocation.
    /// Only the source-qualified zero randomize-direction control is admitted.
    pub fn from_params(
        source: &StartParams,
        randomize_rotation_direction: f32,
    ) -> Result<Self, Refused> {
        if !randomize_rotation_direction.is_finite()
            || !(0.0..=1.0).contains(&randomize_rotation_direction)
        {
            return Err(Refused::InvalidRotationDirection);
        }
        if randomize_rotation_direction != 0.0 {
            return Err(Refused::UnverifiedRotationDirection);
        }
        Ok(Self {
            source: source.clone(),
            randomize_rotation_direction,
        })
    }

    /// Use the current ARM reciprocal law directly. Explicit callback access
    /// below is for diagnostics or a separately qualified numeric backend.
    pub fn start_group_native(
        &self,
        random: &mut ModuleRandom,
        input: InitialGroupInput,
    ) -> Result<InitialGroup, Refused> {
        self.start_group(random, input, initial_reciprocal)
    }

    /// Refusals never commit any of the caller's RNG words. The reciprocal
    /// callback is supplied separately so this kernel cannot accidentally
    /// claim native inverse-lifetime bits from host `1.0 / lifetime`.
    pub fn start_group(
        &self,
        random: &mut ModuleRandom,
        input: InitialGroupInput,
        mut reciprocal: impl FnMut(f32) -> Option<f32>,
    ) -> Result<InitialGroup, Refused> {
        if !(1..=4).contains(&input.active_lanes) {
            return Err(Refused::InvalidLaneCount);
        }
        if input.context != InitialContext::Autonomous {
            return Err(Refused::UnverifiedInheritedContext);
        }
        if input
            .birth_fraction
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || input.curve_time.iter().any(|v| !v.is_finite())
        {
            return Err(Refused::InvalidTiming);
        }
        let source = &self.source;
        let lifetime = prepare(&source.lifetime, InitialField::Lifetime)?;
        let sx = prepare(&source.size, InitialField::SizeX)?;
        let rz = prepare(&source.rotation, InitialField::RotationZ)?;
        let size_yz = if input.storage_size_3d && source.size3d {
            Some([
                prepare_axis(source.size_y.as_ref(), InitialField::SizeY)?,
                prepare_axis(source.size_z.as_ref(), InitialField::SizeZ)?,
            ])
        } else {
            None
        };
        let rotation_xy = if input.storage_rotation_3d && source.rotation3d {
            Some([
                prepare_axis(source.rotation_x.as_ref(), InitialField::RotationX)?,
                prepare_axis(source.rotation_y.as_ref(), InitialField::RotationY)?,
            ])
        } else {
            None
        };
        match &source.color {
            MinMaxGradient::Color(rgba) if rgba.iter().all(|v| v.is_finite()) => {}
            MinMaxGradient::Color(_) => return Err(Refused::InvalidColor),
            _ => return Err(Refused::UnsupportedColor),
        }

        let mut next = *random;
        let seeds = next.next4_u32();
        let life = evaluate4(&lifetime, &mut next, input.curve_time).map(|v| v.max(MIN_LIFETIME));
        let size_x = evaluate4(&sx, &mut next, input.curve_time).map(nonnegative);
        let sizes_yz = size_yz.as_ref().map(|axes| {
            [
                evaluate4(&axes[0], &mut next, input.curve_time).map(nonnegative),
                evaluate4(&axes[1], &mut next, input.curve_time).map(nonnegative),
            ]
        });
        // Current body 0xd54300..0xd543b0, particle-seed hash independent of
        // RandN. Even the admitted threshold zero uses strict >: a zero hash
        // sample selects -1. Nonzero thresholds remain gated by construction.
        let signs = seeds.map(|seed| {
            if ParticleRandom::sample(seed, 0xff2b_b1a4) > self.randomize_rotation_direction {
                1.0
            } else {
                -1.0
            }
        });
        let rotation_z = evaluate4(&rz, &mut next, input.curve_time);
        let rotations_xy = rotation_xy.as_ref().map(|axes| {
            [
                evaluate4(&axes[0], &mut next, input.curve_time),
                evaluate4(&axes[1], &mut next, input.curve_time),
            ]
        });
        let color_draw = next.next4_u32().map(unit);
        let mut inverses = [0.0; 4];
        for lane in 0..4 {
            let inverse = reciprocal(life[lane]).ok_or(Refused::ReciprocalUnavailable)?;
            if !inverse.is_finite()
                || inverse < 0.0
                || (life[lane].is_finite() && inverse <= 0.0)
                || (life[lane] == f32::INFINITY && inverse.to_bits() != 0)
            {
                return Err(Refused::ReciprocalUnavailable);
            }
            inverses[lane] = inverse;
        }
        let lanes = std::array::from_fn(|lane| {
            let mut size = [Some(size_x[lane]), None, None];
            if input.storage_size_3d {
                size[1] = Some(sizes_yz.as_ref().map_or(size_x[lane], |v| v[0][lane]));
                size[2] = Some(sizes_yz.as_ref().map_or(size_x[lane], |v| v[1][lane]));
            }
            // Preserve native fadd +0 before sign multiplication, including
            // signed-zero behavior. Scalar-authored X/Y are written +0.
            let mut rotation = [None, None, Some((0.0 + rotation_z[lane]) * signs[lane])];
            if input.storage_rotation_3d {
                rotation[0] = Some(
                    rotations_xy
                        .as_ref()
                        .map_or(0.0, |v| (0.0 + v[0][lane]) * signs[lane]),
                );
                rotation[1] = Some(
                    rotations_xy
                        .as_ref()
                        .map_or(0.0, |v| (0.0 + v[1][lane]) * signs[lane]),
                );
            }
            InitialLane {
                seed: seeds[lane],
                lifetime: life[lane],
                inverse_lifetime: inverses[lane],
                size,
                rotation,
                color: initial_rgba8(&source.color, input.curve_time[lane], color_draw[lane]),
                birth_fraction: input.birth_fraction[lane],
                curve_time: input.curve_time[lane],
            }
        });
        *random = next;
        Ok(InitialGroup {
            active_lanes: input.active_lanes,
            lanes,
            random_after: next,
        })
    }
}

fn prepare_axis(curve: Option<&MinMaxCurve>, field: InitialField) -> Result<CurveSampler, Refused> {
    prepare(curve.ok_or(Refused::MissingAxis(field))?, field)
}

fn prepare(curve: &MinMaxCurve, field: InitialField) -> Result<CurveSampler, Refused> {
    let valid = match curve {
        MinMaxCurve::Constant(value) => {
            value.is_finite() || (field == InitialField::Lifetime && *value == f32::INFINITY)
        }
        MinMaxCurve::TwoConstants { min, max } => {
            min.is_finite() && max.is_finite() && (max - min).is_finite()
        }
        _ => return Err(Refused::UnsupportedCurve(field)),
    };
    if !valid {
        return Err(Refused::InvalidCurve(field));
    }
    Ok(CurveSampler::with_baking(curve, false))
}

fn evaluate4(curve: &CurveSampler, random: &mut ModuleRandom, time: [f32; 4]) -> [f32; 4] {
    let draw = random.next4_u32();
    std::array::from_fn(|lane| curve.evaluate(time[lane], unit(draw[lane])))
}

// Same low-23-bit conversion as random.rs; next4 supplies the already advanced
// RandN word, so ParticleRandom::sample would advance the wrong generator.
fn unit(word: u32) -> f32 {
    ((word & 0x007f_ffff) as f32) * f32::from_bits(0x3400_0001)
}

fn nonnegative(value: f32) -> f32 {
    // ARM FMAX with +0 produces +0 for either signed zero.
    if value > 0.0 { value } else { 0.0 }
}

// Current JP 6.8.1 InitialModule::Start at 0xd54104..0xd54124.
// Contract: FPCR round-to-nearest ties-to-even, FZ=0, input already clamped by
// Initial's FMAX to at least f32::from_bits(0x3727c5ac), or positive infinity.
// NaN, negative infinity and values below the source clamp are refused.
pub fn initial_reciprocal(lifetime: f32) -> Option<f32> {
    if lifetime.is_nan() || lifetime < f32::from_bits(0x3727_c5ac) {
        return None;
    }
    if lifetime == f32::INFINITY {
        // ARM FRECPE(+inf)=+0; FRECPS(+inf,+0)=2, so both steps retain +0.
        return Some(0.0);
    }
    let bits = lifetime.to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32;
    let index = 256 + ((bits & 0x007f_ffff) >> 15);
    // Exact 8-bit ARM reciprocal estimate. Integer divisions are floors.
    let estimate = (((1_u32 << 19) / (2 * index + 1)) + 1) / 2;
    let result_exponent = 253 - exponent;
    let estimate_bits = if result_exponent > 0 {
        ((result_exponent as u32) << 23) | ((estimate - 256) << 15)
    } else {
        // Exponents 253/254 produce a subnormal estimate. Its discarded bits
        // are all zero; this shift is exact and does not require rounding.
        estimate << (14 + result_exponent) as u32
    };
    let r0 = f32::from_bits(estimate_bits);
    // The product is evaluated exactly in binary64 (the source significands have
    // at most 24 bits each); one cast rounds the complete subtraction like the
    // source FRECPS operation. This is a fused-rounding transcription, not a
    // host reciprocal or an f32 multiply/subtract sequence.
    // Casting each exact product likewise preserves FMUL's f32 rounding and
    // gradual underflow. No host reciprocal or target FMA is used.
    let c0 = (2.0_f64 - (lifetime as f64) * (r0 as f64)) as f32;
    let r1 = ((r0 as f64) * (c0 as f64)) as f32;
    let c1 = (2.0_f64 - (lifetime as f64) * (r1 as f64)) as f32;
    Some(((r1 as f64) * (c1 as f64)) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::value::{MinMaxCurve, MinMaxGradient};

    fn source(lifetime: MinMaxCurve) -> StartParams {
        StartParams {
            lifetime,
            speed: MinMaxCurve::Constant(0.0),
            size: MinMaxCurve::TwoConstants { min: 1.0, max: 2.0 },
            size_y: Some(MinMaxCurve::Constant(3.0)),
            size_z: Some(MinMaxCurve::Constant(4.0)),
            size3d: true,
            rotation: MinMaxCurve::Constant(5.0),
            rotation_x: Some(MinMaxCurve::Constant(6.0)),
            rotation_y: Some(MinMaxCurve::Constant(7.0)),
            rotation3d: true,
            color: MinMaxGradient::Color([0.5, 0.25, 0.0, 1.0]),
            gravity_modifier: MinMaxCurve::Constant(0.0),
        }
    }

    fn input() -> InitialGroupInput {
        InitialGroupInput {
            active_lanes: 1,
            storage_size_3d: true,
            storage_rotation_3d: true,
            birth_fraction: [0.1, 0.2, 0.3, 0.4],
            curve_time: [0.0; 4],
            context: InitialContext::Autonomous,
        }
    }

    #[test]
    fn four_lane_tail_advances_all_draws_and_keeps_birth_inputs_separate() {
        let law = InitialLaw::from_params(&source(MinMaxCurve::Constant(2.0)), 0.0).unwrap();
        let mut random = ModuleRandom::from_owner_seed(17);
        let before = random;
        let group = law
            .start_group(&mut random, input(), |_| Some(0.5))
            .unwrap();
        assert_eq!(group.active_lanes, 1);
        assert_eq!(group.lanes[0].birth_fraction, 0.1);
        assert_eq!(group.lanes[3].curve_time, 0.0);
        assert_eq!(group.lanes[0].lifetime, 2.0);
        assert_eq!(group.lanes[0].inverse_lifetime, 0.5);
        assert_eq!(group.lanes[0].color, [128, 64, 0, 255]);
        assert_ne!(
            before, random,
            "one accepted lane still consumes four SIMD lanes"
        );
        assert_eq!(group.random_after, random);
        assert_eq!(
            random.words,
            [
                [833674532, 3974046291, 2808572772, 4147015473],
                [4199102174, 1693126925, 3594424202, 1334261316],
                [3475656697, 3373801961, 2383906904, 3274531779],
                [1044344810, 566636008, 2525843728, 3354318437],
            ]
        );
    }

    #[test]
    fn storage_gates_do_not_consume_dormant_axis_draws() {
        let law = InitialLaw::from_params(&source(MinMaxCurve::Constant(2.0)), 0.0).unwrap();
        let mut scalar = ModuleRandom::from_owner_seed(17);
        let mut three_d = scalar;
        let mut a = input();
        a.storage_size_3d = false;
        a.storage_rotation_3d = false;
        let b = input();
        let ga = law.start_group(&mut scalar, a, |_| Some(0.5)).unwrap();
        let gb = law.start_group(&mut three_d, b, |_| Some(0.5)).unwrap();
        assert_eq!(ga.lanes[0].size, [Some(1.0802940130233765), None, None]);
        assert_eq!(
            gb.lanes[0].size,
            [Some(1.0802940130233765), Some(3.0), Some(4.0)]
        );
        assert_ne!(scalar, three_d);
    }

    #[test]
    fn infinity_lifetime_uses_minimum_and_reciprocal_callback_without_division() {
        let law =
            InitialLaw::from_params(&source(MinMaxCurve::Constant(f32::INFINITY)), 0.0).unwrap();
        let mut random = ModuleRandom::from_owner_seed(17);
        let group = law
            .start_group(&mut random, input(), |value| {
                assert_eq!(value, f32::INFINITY);
                Some(0.0)
            })
            .unwrap();
        assert_eq!(group.lanes[0].lifetime, f32::INFINITY);
        assert_eq!(group.lanes[0].inverse_lifetime, 0.0);
    }

    #[test]
    fn native_lifetime_minimum_and_reciprocal_are_not_the_old_point_zero_one() {
        for value in [-1.0, -0.0, 0.0, f32::from_bits(1), MIN_LIFETIME] {
            let law = InitialLaw::from_params(&source(MinMaxCurve::Constant(value)), 0.0).unwrap();
            let mut random = ModuleRandom::from_owner_seed(17);
            let group = law.start_group_native(&mut random, input()).unwrap();
            assert_eq!(group.lanes[0].lifetime.to_bits(), 0x3727_c5ac);
            assert_eq!(group.lanes[0].inverse_lifetime, 100_000.0);
        }
        assert_eq!(initial_reciprocal(120.0).unwrap().to_bits(), 0x3c08_8888);
        assert_eq!(initial_reciprocal(f32::INFINITY).unwrap().to_bits(), 0);
        for value in [0.0, -1.0, f32::NAN, f32::NEG_INFINITY] {
            assert!(initial_reciprocal(value).is_none());
        }
    }

    #[test]
    fn scalar_authoring_with_three_dimensional_storage_has_explicit_fallback_writes() {
        let mut params = source(MinMaxCurve::Constant(2.0));
        params.size3d = false;
        params.rotation3d = false;
        params.size_y = None;
        params.size_z = None;
        params.rotation_x = None;
        params.rotation_y = None;
        let law = InitialLaw::from_params(&params, 0.0).unwrap();
        let mut random = ModuleRandom::from_owner_seed(17);
        let group = law.start_group_native(&mut random, input()).unwrap();
        for lane in group.lanes {
            assert_eq!(lane.size, [lane.size[0]; 3]);
            assert_eq!(lane.rotation[0], Some(0.0));
            assert_eq!(lane.rotation[1], Some(0.0));
        }
        assert_eq!(
            random.words[3],
            [2647491883, 2574134017, 3134474944, 2513576617]
        );
    }

    #[test]
    fn invalid_tail_or_failed_reciprocal_does_not_commit_any_random_words() {
        let law = InitialLaw::from_params(&source(MinMaxCurve::Constant(2.0)), 0.0).unwrap();
        let mut random = ModuleRandom::from_owner_seed(17);
        let before = random;
        for count in [0, 5] {
            let mut invalid = input();
            invalid.active_lanes = count;
            assert_eq!(
                law.start_group_native(&mut random, invalid),
                Err(Refused::InvalidLaneCount)
            );
            assert_eq!(random, before);
        }
        let mut calls = 0;
        assert_eq!(
            law.start_group(&mut random, input(), |_| {
                calls += 1;
                if calls == 3 { None } else { Some(0.5) }
            }),
            Err(Refused::ReciprocalUnavailable)
        );
        assert_eq!(random, before);
        assert!(InitialLaw::from_params(&source(MinMaxCurve::Constant(2.0)), 0.5).is_err());
    }

    #[test]
    fn unsupported_curve_and_context_refuse_without_rng_commit() {
        let mut invalid = source(MinMaxCurve::Curve {
            multiplier: 1.0,
            max: crate::particle::value::Curve {
                multiplier: 1.0,
                keys: Vec::new(),
            },
        });
        let law = InitialLaw::from_params(&invalid, 0.0).unwrap();
        let mut random = ModuleRandom::from_owner_seed(17);
        let before = random;
        assert!(matches!(
            law.start_group(&mut random, input(), |_| Some(0.5)),
            Err(Refused::UnsupportedCurve(InitialField::Lifetime))
        ));
        assert_eq!(before, random);
        invalid = source(MinMaxCurve::Constant(2.0));
        let law = InitialLaw::from_params(&invalid, 0.0).unwrap();
        let mut rejected = input();
        rejected.context = InitialContext::Inherited;
        assert!(matches!(
            law.start_group(&mut random, rejected, |_| Some(0.5)),
            Err(Refused::UnverifiedInheritedContext)
        ));
        assert_eq!(before, random);
    }

    use crate::particle::json::{self, Value};

    fn read_evidence(name: &str) -> Value {
        let directory = std::env::var_os("MOLY_INITIAL_EVIDENCE_DIR").map(std::path::PathBuf::from)
            .expect("supply external current native evidence directory");
        json::parse(&std::fs::read(directory.join(name)).expect("native receipt path")).unwrap()
    }

    fn at<'a>(value: &'a Value, key: &str) -> &'a Value {
        value.get(key).unwrap()
    }
    fn array(value: &Value) -> &[Value] {
        value.as_array().unwrap()
    }
    fn number(value: &Value) -> f32 {
        value.as_f64().unwrap() as f32
    }
    fn boolean(value: &Value, key: &str) -> bool {
        at(value, key).as_bool().unwrap()
    }
    fn words(value: &Value) -> ModuleRandom {
        let list = array(value);
        assert_eq!(list.len(), 16);
        ModuleRandom {
            words: std::array::from_fn(|word| {
                std::array::from_fn(|lane| list[word * 4 + lane].as_f64().unwrap() as u32)
            }),
        }
    }

    fn probe_source(size3d: bool, rotation3d: bool, constant: bool) -> StartParams {
        let curve = |min, max| {
            if constant {
                MinMaxCurve::Constant(max)
            } else {
                MinMaxCurve::TwoConstants { min, max }
            }
        };
        let mut result = source(curve(2.0, 4.0));
        result.size3d = size3d;
        result.rotation3d = rotation3d;
        result.size = curve(1.0, 2.0);
        result.size_y = Some(curve(3.0, 5.0));
        result.size_z = Some(curve(7.0, 11.0));
        result.rotation = curve(0.5, 1.0);
        result.rotation_x = Some(curve(0.125, 0.25));
        result.rotation_y = Some(curve(0.25, 0.5));
        result.color = MinMaxGradient::Color([1.0; 4]);
        result
    }

    fn replay_call(law: &InitialLaw, call: &Value) -> usize {
        let mut random = words(at(call, "beforeWords"));
        let count = (number(at(call, "end")) - number(at(call, "start"))) as usize;
        let observed = at(call, "storageSnapshot");
        assert_eq!(
            number(at(observed, "count")) as usize,
            count.div_ceil(4) * 4
        );
        for offset in (0..count).step_by(4) {
            let group = law
                .start_group_native(
                    &mut random,
                    InitialGroupInput {
                        active_lanes: (count - offset).min(4),
                        storage_size_3d: boolean(call, "storageSize3D"),
                        storage_rotation_3d: boolean(call, "storageRotation3D"),
                        birth_fraction: [0.25; 4],
                        curve_time: [0.25; 4],
                        context: InitialContext::Autonomous,
                    },
                )
                .unwrap();
            for (lane, actual) in group.lanes.iter().enumerate() {
                let index = offset + lane;
                assert_eq!(
                    actual.seed,
                    array(at(observed, "seed"))[index].as_f64().unwrap() as u32
                );
                assert_eq!(
                    actual.inverse_lifetime.to_bits(),
                    number(&array(at(observed, "inverseLifetime"))[index]).to_bits()
                );
                for (name, values) in [("size", actual.size), ("rotation", actual.rotation)] {
                    for (axis, value) in values.into_iter().enumerate() {
                        if let Some(value) = value {
                            assert_eq!(
                                value.to_bits(),
                                number(&array(&array(at(observed, name))[axis])[index]).to_bits(),
                                "{name} axis {axis} lane {index}"
                            );
                        }
                    }
                }
                let expected = array(&array(at(observed, "color"))[index]);
                assert_eq!(
                    actual.color,
                    std::array::from_fn(|axis| number(&expected[axis]) as u8)
                );
            }
        }
        assert_eq!(random, words(at(call, "afterWords")));
        count.div_ceil(4) * 4
    }

    /// Existing current native bodies, all storage lanes and all 16 words.
    /// No generated vectors, no whole-runtime or source-owner admission claim.
    #[test]
    #[ignore]
    fn replay_current_native_initial_groups_and_reciprocal_bits() {
        let receipt = read_evidence("initial-rng-native.json");
        let mut calls = 0;
        let mut lanes = 0;
        for row in array(at(&receipt, "directRows")) {
            let params = probe_source(
                boolean(row, "authoredSize3D"),
                boolean(row, "authoredRotation3D"),
                boolean(row, "constant"),
            );
            let law = InitialLaw::from_params(&params, 0.0).unwrap();
            for call in array(at(row, "batches")) {
                lanes += replay_call(&law, call);
                calls += 1;
            }
        }
        assert_eq!(calls, 224);
        let mut pipeline_calls = 0;
        for row in array(at(&receipt, "pipelineRows")) {
            let params = probe_source(
                boolean(row, "authoredSize3D"),
                boolean(row, "authoredRotation3D"),
                false,
            );
            let law = InitialLaw::from_params(&params, 0.0).unwrap();
            for batch in array(at(row, "batches")) {
                let actual_calls = array(at(batch, "initialCalls"));
                if actual_calls.is_empty() {
                    assert_eq!(
                        words(at(batch, "beforeWords")),
                        words(at(batch, "afterWords"))
                    );
                }
                for call in actual_calls {
                    lanes += replay_call(&law, call);
                    pipeline_calls += 1;
                }
            }
        }
        assert_eq!(pipeline_calls, 12);
        assert_eq!(lanes, 1072);

        let reciprocal = read_evidence("initial-reciprocal-native.json");
        let rows = array(at(&reciprocal, "rows"));
        assert_eq!(rows.len(), 3262);
        for row in rows {
            let row = array(row);
            let input_bits = row[1].as_f64().unwrap() as u32;
            let result_bits = row[6].as_f64().unwrap() as u32;
            assert_eq!(
                initial_reciprocal(f32::from_bits(input_bits))
                    .unwrap()
                    .to_bits(),
                result_bits,
                "lifetime {input_bits:08x}"
            );
        }
        for row in array(at(&reciprocal, "fullInitial")) {
            let clamped = f32::from_bits(at(row, "clampedBits").as_f64().unwrap() as u32);
            let result = initial_reciprocal(clamped).unwrap().to_bits();
            for lane in array(at(row, "inverseLifetimeBits")) {
                assert_eq!(result, lane.as_f64().unwrap() as u32);
            }
        }
    }
}
