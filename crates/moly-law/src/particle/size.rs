//! Current particle dimensions derived from immutable birth dimensions.
//!
//! Two forms. [`SizeOverLifetime::from_params`] evaluates the size where a
//! consumer asks for it, as from a reset evaluator cache, and so admits a
//! curve lane only where the cache history cannot change a result (the
//! history-independence certificate). [`SizeOverLifetime::with_storage`] is
//! `SizeModule::Update` in the engine's own calls: the host runs
//! [`SizeOverLifetime::pass`] exactly where the engine calls the module,
//! reports its storage operations, and reads the stored current size, as the
//! renderer does; every curve object then carries its cache as the engine's
//! does and no lane needs the certificate.
use super::buffer::RingBufferMode;
use super::curve::{arm_fmax, curve_time_fmax, normalized_age, CurveCache, CurveSampler, CurveTime, EngineCurve};
use super::random::ParticleRandom;
use super::schema::SizeOverLifetimeParams;
use super::slot_tail::{SlotTail, TailRefused};
use super::step::Particle;

/// The salt of the particle's size random.
const SIZE_SALT: u32 = 0x8d2c_8431;

#[derive(Clone, Debug)]
pub struct SizeOverLifetime {
    axes: [CurveSampler; 3],
    /// The engine's calls, for the form that follows them.
    calls: Option<Box<SizeCalls>>,
}

impl SizeOverLifetime {
    /// Each axis follows the engine's curve dispatch on the particle's
    /// normalized age; a lane outside the transcribed evaluator is refused.
    pub fn from_params(params: &SizeOverLifetimeParams) -> Result<Self, &'static str> {
        let [x, y, z] = axis_curves(params);
        let axis = |curve| CurveSampler::new(curve, CurveTime::Normalized);
        Ok(Self { axes: [axis(x)?, axis(y)?, axis(z)?], calls: None })
    }

    /// The form that follows the engine's calls ([`SizeCalls`]). `uses_3d` is
    /// whether the particle arrays carry three sizes (a 3D start size or the
    /// module's separate axes), which makes every call run three axes;
    /// `capacity` is the slots known to lie in the storage Play reserved
    /// ([`crate::particle::slot_tail::RESERVED_SLOTS`]).
    pub fn with_storage(params: &SizeOverLifetimeParams, uses_3d: bool, capacity: usize)
        -> Result<Self, &'static str> {
        let [x, y, z] = axis_curves(params);
        let fresh = |curve| CurveSampler::build(curve, None, true);
        let axes = [fresh(x)?, fresh(y)?, fresh(z)?];
        let objects = if params.separate_axes { vec![x, y, z] } else { vec![x] };
        let objects = objects.into_iter().map(|curve| Ok(match CurveSampler::build(curve, None, true)? {
            CurveSampler::CurveEngine { multiplier, curve } =>
                Object::Engine { multiplier, max: (curve, CurveCache::INVALID), min: None },
            CurveSampler::TwoCurvesEngine { multiplier, min, max } => Object::Engine {
                multiplier, max: (max, CurveCache::INVALID), min: Some((min, CurveCache::INVALID)) },
            direct => Object::Direct(direct),
        })).collect::<Result<Vec<_>, &'static str>>()?;
        let calls = SizeCalls {
            objects,
            separate: params.separate_axes,
            axis_count: if uses_3d { 3 } else { 1 },
            storage: SlotTail::new(capacity),
            stored: Vec::new(),
            trace: None,
            refused: None,
        };
        Ok(Self { axes, calls: Some(Box::new(calls)) })
    }

    /// Every axis uses the same particle-local random factor. The nonnegative
    /// clamp applies to the curve factor, before multiplication by birth size.
    /// For the form that follows the engine's calls this is the value as from
    /// reset caches, for a consumer outside those calls; the renderer's value
    /// is [`SizeOverLifetime::stored`].
    pub fn evaluate(&self, birth_size: [f32; 3], seed: u32, age_percent: f32) -> [f32; 3] {
        let t = normalized_age(age_percent);
        let random = ParticleRandom::sample(seed, SIZE_SALT);
        std::array::from_fn(|axis| birth_size[axis] * self.axes[axis].evaluate(t, random).max(0.0))
    }

    /// The engine's calls, when this law follows them.
    pub fn calls(&self) -> Option<&SizeCalls> {
        self.calls.as_deref()
    }

    pub fn calls_mut(&mut self) -> Option<&mut SizeCalls> {
        self.calls.as_deref_mut()
    }

    /// The current size the last call stored in slot `index` (the three
    /// sizes, the first repeated without 3D sizes), when this law follows the
    /// engine's calls.
    pub fn stored(&self, index: usize) -> Option<[f32; 3]> {
        self.calls.as_ref().and_then(|calls| calls.stored.get(index).copied())
    }
}

fn axis_curves(params: &SizeOverLifetimeParams) -> [&super::MinMaxCurve; 3] {
    if params.separate_axes {
        [&params.curve,
            params.y.as_ref().expect("typed separate-axis size Y"),
            params.z.as_ref().expect("typed separate-axis size Z")]
    } else {
        [&params.curve; 3]
    }
}

/// One curve object group of the module: the x curve (every axis without
/// separate axes), or one per axis.
#[derive(Clone, Debug)]
enum Object {
    /// Constants and the reader's polynomial: no evaluator cache is read.
    Direct(CurveSampler),
    /// `Evaluate` with a null cache on the curve objects, each carrying its
    /// own cache, times the multiplier; with a minimum curve, the lerp by the
    /// lane's size random.
    Engine { multiplier: f32, max: (EngineCurve, CurveCache), min: Option<(EngineCurve, CurveCache)> },
}

/// One `Evaluate` call on a curve object: the axis of the call, the object
/// (the curve group and whether it is the minimum curve), the time, the value
/// before the multiplier and the cache it left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeEvaluation {
    pub axis: usize,
    pub object: usize,
    pub min: bool,
    pub time: f32,
    pub value: f32,
    pub cache: CurveCache,
}

/// `SizeModule::Update(particles, from, to)` over this system's calls.
///
/// A call runs every axis in turn (three when the particle arrays carry
/// three sizes, else one), each through its curve group (the axis's own with
/// separate axes, else the x curve). For an axis it walks four-lane groups
/// from `from` while the group starts before `to`, so the last group also
/// evaluates the lanes past `to`; lane by lane the time is
/// `fmax(age * 0.01, +0)`, the maximum curve's `Evaluate` with no cache
/// argument (the object's own cache) for the four lanes, then the minimum
/// curve's, the lerp `lo + r * (hi - lo)` by the size random, and
/// `start * fmax(v, +0)` is stored. The lanes past the live count hold what
/// the storage operations left ([`SlotTail`]); their values go to slots
/// nothing reads, and only their evaluations' caches matter.
#[derive(Clone, Debug)]
pub struct SizeCalls {
    objects: Vec<Object>,
    separate: bool,
    axis_count: usize,
    storage: SlotTail,
    stored: Vec<[f32; 3]>,
    /// Every curve evaluation in call order, when set.
    pub trace: Option<Vec<SizeEvaluation>>,
    refused: Option<&'static str>,
}

/// One live lane of a call: its age percent, seed and start size.
#[derive(Clone, Copy, Debug)]
pub struct SizeLane {
    pub age_percent: f32,
    pub seed: u32,
    pub start: [f32; 3],
}

impl SizeCalls {
    /// Why the storage stopped following the engine's, if it did; the host
    /// must then stop simulating the system (the caches are no longer known).
    pub fn refused(&self) -> Option<&'static str> {
        self.refused
    }

    /// Every curve object's cache, group by group, the maximum curve before
    /// the minimum.
    pub fn caches(&self) -> Vec<CurveCache> {
        let mut out = Vec::new();
        for object in &self.objects {
            if let Object::Engine { max, min, .. } = object {
                out.push(max.1);
                if let Some(min) = min { out.push(min.1); }
            }
        }
        out
    }

    /// The slots past the live count this law carries.
    pub fn storage(&self) -> &SlotTail {
        &self.storage
    }

    fn follow(&mut self, step: impl FnOnce(&mut SlotTail) -> Result<(), TailRefused>) {
        if self.refused.is_some() {
            return;
        }
        if let Err(refused) = step(&mut self.storage) {
            self.refused = Some(match refused {
                TailRefused::Unwritten =>
                    "sizeOverLifetime curve cache: a four-lane group reads a slot no storage operation wrote",
                TailRefused::Order =>
                    "sizeOverLifetime curve cache: the runtime's storage operations are not the ones the slot model follows",
            });
        }
    }

    /// One call over `[from, to)` of the live lanes `live` (the whole live
    /// storage, `to <= live.len()`), whose count the storage operations left.
    /// The stored sizes of `[from, to)` are rewritten; a call from 0 to the
    /// count rewrites them all.
    pub fn pass(&mut self, live: &[SizeLane], from: usize, to: usize) {
        if self.refused.is_some() || from >= to {
            return;
        }
        if to > live.len() || live.len() != self.storage.live() {
            self.refused = Some("sizeOverLifetime curve cache: the runtime's storage operations are not the ones the slot model follows");
            return;
        }
        let end = from + (to - from).next_multiple_of(4);
        let past = end.saturating_sub(live.len());
        let padding: Vec<f32> = (0..past).filter_map(|k| self.storage.slot(k).map(|slot| slot.age_percent)).collect();
        if padding.len() != past {
            self.refused = Some("sizeOverLifetime curve cache: a four-lane group reads a slot no storage operation wrote");
            return;
        }
        self.call(live, &padding, from, to);
    }

    /// The module body of one call: `padding` holds the ages of the lanes of
    /// the last four-lane group past the live lanes.
    fn call(&mut self, live: &[SizeLane], padding: &[f32], from: usize, to: usize) {
        let end = from + (to - from).next_multiple_of(4);
        self.stored.resize(live.len(), [f32::NAN; 3]);
        for axis in 0..self.axis_count {
            let group = if self.separate { axis } else { 0 };
            for slot in from..end {
                let (age, lane) = match live.get(slot) {
                    Some(lane) => (lane.age_percent, Some(lane)),
                    None => (padding[slot - live.len()], None),
                };
                let t = curve_time_fmax(age);
                let random = lane.map_or(0.0, |lane| ParticleRandom::sample(lane.seed, SIZE_SALT));
                let trace = &mut self.trace;
                let v = match &mut self.objects[group] {
                    Object::Direct(sampler) => {
                        if lane.is_none() { continue; }
                        sampler.evaluate(t, random)
                    }
                    Object::Engine { multiplier, max, min } => {
                        let mut side = |curve: &mut (EngineCurve, CurveCache), is_min: bool| {
                            let value = curve.0.evaluate_cached(t, &mut curve.1);
                            if let Some(trace) = trace.as_mut() {
                                trace.push(SizeEvaluation { axis, object: group, min: is_min, time: t, value, cache: curve.1 });
                            }
                            value * *multiplier
                        };
                        let hi = side(max, false);
                        match min {
                            None => hi,
                            Some(min) => {
                                let lo = side(min, true);
                                lo + random * (hi - lo)
                            }
                        }
                    }
                };
                if let Some(lane) = lane {
                    self.stored[slot][axis] = lane.start[axis] * arm_fmax(v, 0.0);
                }
            }
        }
        if self.axis_count == 1 {
            for slot in from..to {
                let x = self.stored[slot][0];
                self.stored[slot] = [x; 3];
            }
        }
    }

    /// SimulateParticles over a call's range `[0, count)`: the lanes of the
    /// last four-lane group past `count` advance their ages as the live ones
    /// do.
    pub fn end_existing_call(&mut self, count: usize, dt: f32, mode: RingBufferMode, loop_range: [f32; 2]) {
        self.follow(|storage| {
            storage.padding(count)?;
            storage.advance_padding(count, dt, mode, loop_range);
            Ok(())
        });
    }

    /// See [`SlotTail::kill`]; the stored sizes move as the slots do.
    pub fn kill(&mut self, before: &[Particle], removed: &[usize], after: &[Particle]) {
        for &index in removed {
            if index < self.stored.len() {
                self.stored.swap_remove(index);
            }
        }
        self.follow(|storage| storage.kill(before, removed, after));
    }

    /// See [`SlotTail::birth`]. The newborns' stored sizes are unknown until
    /// the next call writes them.
    pub fn birth(&mut self, old: usize, lanes: &[Particle], live: usize, after: &[Particle]) {
        self.stored.truncate(old);
        self.follow(|storage| storage.birth(old, lanes, live, after));
    }

    /// See [`SlotTail::clear`].
    pub fn clear(&mut self, live: &[Particle]) {
        self.stored.clear();
        self.follow(|storage| storage.clear(live));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::MinMaxCurve;

    #[test]
    fn size_axes_share_a_factor_but_not_their_authored_ranges() {
        let law = SizeOverLifetime::from_params(&SizeOverLifetimeParams {
            separate_axes: true,
            curve: MinMaxCurve::TwoConstants { min: 0.0, max: 1.0 },
            y: Some(MinMaxCurve::TwoConstants { min: 0.0, max: 2.0 }),
            z: Some(MinMaxCurve::TwoConstants { min: 0.0, max: 4.0 }),
        }).unwrap();
        let size = law.evaluate([1.0; 3], 17, 45.0);
        assert!(size[0] > 0.0 && size[0] < 1.0);
        assert_eq!(size[1], size[0] * 2.0);
        assert_eq!(size[2], size[0] * 4.0);
        assert_eq!(size, law.evaluate([1.0; 3], 17, 0.0));
    }

    #[test]
    fn negative_factor_is_not_a_mirrored_particle() {
        let law = SizeOverLifetime::from_params(&SizeOverLifetimeParams {
            separate_axes: false, curve: MinMaxCurve::Constant(-2.0), y: None, z: None,
        }).unwrap();
        assert_eq!(law.evaluate([1.0, 2.0, 3.0], 17, 0.0), [0.0; 3]);
    }
}

#[cfg(test)]
mod native_calls_tests {
    use super::*;
    use crate::particle::json::{parse, Value};
    use crate::particle::{Curve, CurveKey, MinMaxCurve};
    use std::collections::BTreeMap;

    fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
        value.get(key).unwrap_or_else(|| panic!("{key}"))
    }
    fn word(value: &Value) -> u32 {
        value.as_f64().expect("word") as u32
    }
    fn words(value: &Value) -> Vec<u32> {
        value.as_array().expect("words").iter().map(word).collect()
    }
    /// Bits, except that a NaN matches any NaN (the sign and payload of a
    /// generated NaN are the host's).
    fn same(ours: u32, native: u32) -> bool {
        ours == native || (f32::from_bits(ours).is_nan() && f32::from_bits(native).is_nan())
    }
    fn side(image: &Value) -> Curve {
        let keys = get(image, "keys").as_array().unwrap().iter().map(|key| {
            let k = words(key);
            CurveKey { time: f32::from_bits(k[0]), value: f32::from_bits(k[1]), in_slope: f32::from_bits(k[2]),
                out_slope: f32::from_bits(k[3]), weighted_mode: k[4] as u8, in_weight: f32::from_bits(k[5]),
                out_weight: f32::from_bits(k[6]) }
        }).collect();
        Curve { multiplier: 1.0, keys, pre_wrap: Some(word(get(image, "pre"))), post_wrap: Some(word(get(image, "post"))) }
    }
    fn component(image: &Value) -> MinMaxCurve {
        let bits = |key: &str| f32::from_bits(word(get(image, key)));
        match word(get(image, "mode")) {
            0 => MinMaxCurve::Constant(bits("bits")),
            3 => MinMaxCurve::TwoConstants { min: bits("minBits"), max: bits("maxBits") },
            1 => MinMaxCurve::Curve { multiplier: bits("multiplierBits"), max: side(get(image, "max")) },
            2 => MinMaxCurve::TwoCurves { multiplier: bits("multiplierBits"), min: side(get(image, "min")),
                max: side(get(image, "max")) },
            mode => panic!("mode {mode}"),
        }
    }

    struct Case {
        index: u32,
        params: SizeOverLifetimeParams,
        uses_3d: bool,
        calls: Vec<Call>,
        /// Per curve object (curve field, minimum side): (t, value, fresh, cache after).
        native: BTreeMap<(usize, bool), Vec<(u32, u32, u32, Vec<u32>)>>,
    }

    struct Call {
        from: usize,
        to: usize,
        ages: Vec<u32>,
        seeds: Vec<u32>,
        starts: [Vec<u32>; 3],
        outputs: [Vec<u32>; 3],
    }

    fn case(value: &Value) -> Case {
        let curves: Vec<MinMaxCurve> = get(value, "curves").as_array().unwrap().iter().map(component).collect();
        let separate = word(get(value, "separateAxes")) == 1;
        let params = SizeOverLifetimeParams { separate_axes: separate, curve: curves[0].clone(),
            y: separate.then(|| curves[1].clone()), z: separate.then(|| curves[2].clone()) };
        let calls = get(value, "calls").as_array().unwrap().iter().map(|call| {
            let triple = |key: &str| -> [Vec<u32>; 3] {
                let rows = get(call, key).as_array().unwrap();
                std::array::from_fn(|axis| words(&rows[axis]))
            };
            Call { from: word(get(call, "from")) as usize, to: word(get(call, "to")) as usize,
                ages: words(get(call, "ages")), seeds: words(get(call, "seeds")), starts: triple("starts"),
                outputs: triple("outputs") }
        }).collect();
        let mut native: BTreeMap<(usize, bool), Vec<(u32, u32, u32, Vec<u32>)>> = BTreeMap::new();
        for e in get(value, "evaluations").as_array().unwrap() {
            let key = (word(get(e, "curve")) as usize, get(e, "side").as_str() == Some("min"));
            native.entry(key).or_default().push((word(get(e, "t")), word(get(e, "value")), word(get(e, "fresh")),
                words(get(e, "after"))));
        }
        Case { index: word(get(value, "case")), params, uses_3d: word(get(value, "flag7d4")) == 1, calls, native }
    }

    /// The live lanes of a call as the law reads them (slots below its start
    /// are never read) and the ages of its last group's lanes past its end.
    fn lanes(call: &Call) -> (Vec<SizeLane>, Vec<f32>) {
        let unused = SizeLane { age_percent: f32::NAN, seed: 0, start: [f32::NAN; 3] };
        let mut live = vec![unused; call.to];
        for k in 0..call.to - call.from {
            live[call.from + k] = SizeLane { age_percent: f32::from_bits(call.ages[k]), seed: call.seeds[k],
                start: std::array::from_fn(|axis| f32::from_bits(call.starts[axis][k])) };
        }
        let padding = call.ages[call.to - call.from..].iter().map(|&age| f32::from_bits(age)).collect();
        (live, padding)
    }

    /// The other readings of the module this receipt must tell apart, each
    /// evaluated on the curve objects directly.
    #[derive(Clone, Copy, Debug)]
    enum Mutant {
        /// Every call starts from reset caches.
        FreshPerCall,
        /// Every evaluation starts from a reset cache.
        ResetPerEvaluation,
        /// The lanes of the last group past the call's end are not evaluated.
        PaddingSkipped,
        /// Lanes outer, axes inner.
        AxisInner,
        /// The minimum curve reads and writes the maximum curve's cache.
        MinSharesMaxCache,
    }

    fn mutant_trace(case: &Case, mutant: Mutant) -> BTreeMap<(usize, bool), Vec<(u32, u32, Vec<u32>)>> {
        let [x, y, z] = axis_curves(&case.params);
        let groups: Vec<&MinMaxCurve> = if case.params.separate_axes { vec![x, y, z] } else { vec![x] };
        let mut objects: Vec<Option<(EngineCurve, CurveCache, Option<(EngineCurve, CurveCache)>)>> = groups.iter()
            .map(|curve| match CurveSampler::build(curve, None, true).unwrap() {
                CurveSampler::CurveEngine { curve, .. } => Some((curve, CurveCache::INVALID, None)),
                CurveSampler::TwoCurvesEngine { min, max, .. } =>
                    Some((max, CurveCache::INVALID, Some((min, CurveCache::INVALID)))),
                _ => None,
            }).collect();
        let axes = if case.uses_3d { 3 } else { 1 };
        let mut out: BTreeMap<(usize, bool), Vec<(u32, u32, Vec<u32>)>> = BTreeMap::new();
        for call in &case.calls {
            if matches!(mutant, Mutant::FreshPerCall) {
                for object in objects.iter_mut().flatten() {
                    object.1 = CurveCache::INVALID;
                    if let Some(min) = object.2.as_mut() { min.1 = CurveCache::INVALID; }
                }
            }
            let span = if matches!(mutant, Mutant::PaddingSkipped) { call.to - call.from }
                else { (call.to - call.from).next_multiple_of(4) };
            let order: Vec<(usize, usize)> = if matches!(mutant, Mutant::AxisInner) {
                (0..span).flat_map(|k| (0..axes).map(move |axis| (axis, k))).collect()
            } else {
                (0..axes).flat_map(|axis| (0..span).map(move |k| (axis, k))).collect()
            };
            for (axis, k) in order {
                let group = if case.params.separate_axes { axis } else { 0 };
                let Some(object) = objects[group].as_mut() else { continue; };
                let t = curve_time_fmax(f32::from_bits(call.ages[k]));
                let eval = |curve: &EngineCurve, cache: &mut CurveCache| {
                    if matches!(mutant, Mutant::ResetPerEvaluation) { *cache = CurveCache::INVALID; }
                    let value = curve.evaluate_cached(t, cache);
                    (t.to_bits(), value.to_bits(), cache.words().to_vec())
                };
                let row = eval(&object.0, &mut object.1);
                out.entry((group, false)).or_default().push(row);
                if let Some((min, cache)) = object.2.as_mut() {
                    let row = if matches!(mutant, Mutant::MinSharesMaxCache) { eval(min, &mut object.1) }
                        else { eval(min, cache) };
                    out.entry((group, true)).or_default().push(row);
                }
            }
        }
        out
    }

    // SizeModule::Update calls on one module over several calls, against the
    // native curve objects: every live lane's stored size on every axis the
    // call runs, and every Evaluate call on each curve object (its time, value
    // and the cache it left) in order, the lanes of each call's last four-lane
    // group past its end included. The law runs each call through the module
    // body the runtime drives, carrying the curve objects' caches from call
    // to call; the rows whose reset-cache value differs from the carried one
    // must exist and be matched. The other readings (fresh caches per call or
    // per evaluation, the padding lanes skipped, the axes inner, one cache for
    // both sides of a two-curve object) must each disagree with the receipt.
    #[test]
    #[ignore = "MOLY_SIZE_CALLS_NATIVE must identify the native SizeModule call-sequence receipt"]
    fn module_calls_match_native_size_rows() {
        let path = std::env::var_os("MOLY_SIZE_CALLS_NATIVE").expect("MOLY_SIZE_CALLS_NATIVE");
        let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(get(&receipt, "librarySha256").as_str(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
        let cases: Vec<Case> = get(&receipt, "cases").as_array().unwrap().iter().map(case).collect();
        let (mut calls, mut outputs, mut evaluations, mut carried, mut padding, mut shared_axes) = (0, 0, 0, 0, 0, 0);
        let (mut mismatches, mut bad) = (Vec::new(), 0usize);
        for case in &cases {
            let mut law = SizeOverLifetime::with_storage(&case.params, case.uses_3d, crate::particle::slot_tail::RESERVED_SLOTS)
                .unwrap_or_else(|reason| panic!("case {}: {reason}", case.index));
            let module = law.calls_mut().unwrap();
            module.trace = Some(Vec::new());
            shared_axes += usize::from(case.uses_3d && !case.params.separate_axes);
            for call in &case.calls {
                calls += 1;
                let (live, pad) = lanes(call);
                padding += pad.len();
                module.call(&live, &pad, call.from, call.to);
                for k in 0..call.to - call.from {
                    for axis in 0..module.axis_count {
                        let (ours, expected) = (module.stored[call.from + k][axis].to_bits(), call.outputs[axis][k]);
                        outputs += 1;
                        let ok = same(ours, expected);
                        bad += usize::from(!ok);
                        if !ok && mismatches.len() < 12 {
                            mismatches.push(format!("case {} call {calls} lane {} axis {axis}: ours {ours:#x} native {expected:#x}",
                                case.index, call.from + k));
                        }
                    }
                }
            }
            let mut ours: BTreeMap<(usize, bool), Vec<&SizeEvaluation>> = BTreeMap::new();
            let trace = module.trace.take().unwrap();
            for e in &trace {
                ours.entry((e.object, e.min)).or_default().push(e);
            }
            let keys: std::collections::BTreeSet<(usize, bool)> = ours.keys().chain(case.native.keys()).copied().collect();
            for key in keys {
                let (o, n) = (ours.get(&key).map_or(&[][..], |v| &v[..]), case.native.get(&key).map_or(&[][..], |v| &v[..]));
                if o.len() != n.len() {
                    bad += 1;
                    if mismatches.len() < 12 {
                        mismatches.push(format!("case {} object {key:?}: {} evaluations, native {}", case.index, o.len(), n.len()));
                    }
                    continue;
                }
                for (i, (o, (t, value, fresh, after))) in o.iter().zip(n).enumerate() {
                    evaluations += 1;
                    carried += usize::from(!same(*value, *fresh));
                    let ok = same(o.time.to_bits(), *t) && same(o.value.to_bits(), *value)
                        && o.cache.words().iter().zip(after).all(|(a, b)| same(*a, *b));
                    bad += usize::from(!ok);
                    if !ok && mismatches.len() < 12 {
                        mismatches.push(format!("case {} object {key:?} evaluation {i}: t {:#x}/{t:#x} value {:#x}/{value:#x} cache {:x?}/{after:x?}",
                            case.index, o.time.to_bits(), o.value.to_bits(), o.cache.words()));
                    }
                }
            }
        }
        let mut red = Vec::new();
        for mutant in [Mutant::FreshPerCall, Mutant::ResetPerEvaluation, Mutant::PaddingSkipped, Mutant::AxisInner,
            Mutant::MinSharesMaxCache] {
            let mut differing = 0usize;
            for case in &cases {
                let trace = mutant_trace(case, mutant);
                let keys: std::collections::BTreeSet<(usize, bool)> = trace.keys().chain(case.native.keys()).copied().collect();
                for key in keys {
                    let (o, n) = (trace.get(&key).map_or(&[][..], |v| &v[..]), case.native.get(&key).map_or(&[][..], |v| &v[..]));
                    differing += o.len().abs_diff(n.len());
                    differing += o.iter().zip(n).filter(|((t, value, cache), (nt, nvalue, _, after))|
                        !(same(*t, *nt) && same(*value, *nvalue) && cache.iter().zip(after).all(|(a, b)| same(*a, *b)))).count();
                }
            }
            red.push(format!("{mutant:?} {differing}"));
            assert!(differing > 0, "mutant {mutant:?} agrees with the receipt");
        }
        eprintln!("size module replay: {} cases ({shared_axes} with three axes on one curve group), {calls} calls, \
            {outputs} stored sizes, {evaluations} evaluations ({carried} whose reset-cache value differs), \
            {padding} lanes past the call ends, {bad} mismatches; mutants red: {}", cases.len(), red.join(", "));
        assert!(bad == 0 && mismatches.is_empty(), "first mismatches: {mismatches:#?}");
        assert!(calls > 0 && evaluations > 0 && carried > 0 && padding > 0);
    }
}
