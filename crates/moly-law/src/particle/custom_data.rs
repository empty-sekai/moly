//! CustomDataModule writes persistent channels in the pre-simulation batch.
//! Each channel has its own salted particle stream and independent curve path.
//!
//! `CustomDataModule::Update` runs component by component, and for each
//! component over the call's range in four-lane groups from its start while
//! the group starts before the end, so the last group also evaluates the
//! lanes past the end. A curve lane the reader leaves unoptimized is evaluated
//! lane by lane through `AnimationCurveTpl::Evaluate` with no cache argument,
//! which reads and rewrites the cache the curve object carries
//! ([`CurveCache`]); a two-curve lane evaluates the maximum curve's four lanes,
//! then the minimum curve's. Every channel keeps its curve objects' caches
//! here, and each evaluation carries them as the engine does, so every value
//! follows the evaluations before it on the same curve object.
use super::{
    curve::{curve_time_fmax, CurveCache, CurveSampler, CurveTime, EngineCurve},
    random::ParticleRandom,
    schema::CustomDataParams,
    slot_tail::{SlotTail, TailRefused},
    step::Particle,
    RingBufferMode,
};

/// One component of a vector stream.
#[derive(Clone, Debug)]
enum Channel {
    /// Constants and the reader's polynomial: no evaluator cache is read.
    Direct(CurveSampler),
    /// `Evaluate` on the component's own curve objects, each with its cache,
    /// times the multiplier; with a minimum curve, the lerp by the lane's draw.
    Engine { multiplier: f32, max: (EngineCurve, CurveCache), min: Option<(EngineCurve, CurveCache)> },
}

/// One `Evaluate` call on a component's curve object: the time, the value
/// before the multiplier, and the cache it left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Evaluation {
    pub stream: usize,
    pub channel: usize,
    /// The minimum curve of a two-curve lane (the maximum is evaluated first).
    pub min: bool,
    pub time: f32,
    pub value: f32,
    pub cache: CurveCache,
}

#[derive(Clone, Debug)]
pub struct CustomData {
    streams: [Option<Vec<Channel>>; 2],
    /// The storage past the live count, for a runtime that reproduces the
    /// evaluations of the lanes past its calls' ends.
    storage: Option<SlotTail>,
    /// Every curve evaluation in call order, when set.
    pub trace: Option<Vec<Evaluation>>,
    /// Why the storage stopped following the engine's; then no hook acts.
    refused: Option<&'static str>,
}

impl CustomData {
    /// Every component follows the engine's curve dispatch on the particle's
    /// normalized age; a lane outside the transcribed evaluator is refused. A
    /// repeat wrap past the last key is admitted only where the cache history
    /// cannot change a result ([`CurveSampler::new`]): this form is for a host
    /// that does not evaluate the lanes past its calls' ends.
    pub fn from_params(params: &CustomDataParams) -> Result<Self, &'static str> {
        Self::build(params, None)
    }

    /// The same for a host that calls [`CustomData::update`] for its live
    /// lanes in the engine's slot order, one engine call at a time, and reports
    /// the calls' ends and its storage operations through
    /// [`CustomData::end_existing_call`], [`CustomData::kill`],
    /// [`CustomData::birth`] and [`CustomData::clear`]: the lanes past each
    /// call's end are evaluated from the slots they read, so every curve
    /// object's cache follows the engine's and no repeat lane needs the
    /// history-independence certificate. `capacity` is the slots known to lie
    /// in the storage Play reserved ([`crate::particle::slot_tail::RESERVED_SLOTS`]).
    pub fn with_storage(params: &CustomDataParams, capacity: usize) -> Result<Self, &'static str> {
        Self::build(params, Some(capacity))
    }

    fn build(params: &CustomDataParams, storage: Option<usize>) -> Result<Self, &'static str> {
        let mut streams: [Option<Vec<Channel>>; 2] = [None, None];
        for (stream, slot) in streams.iter_mut().zip([params.custom1.as_ref(), params.custom2.as_ref()]) {
            let Some(slot) = slot else { continue; };
            assert_eq!(slot.component_count, slot.components.len());
            assert!(slot.component_count <= 4);
            *stream = Some(slot.components.iter().map(|curve| {
                if storage.is_none() {
                    CurveSampler::new(curve, CurveTime::Normalized)?;
                }
                Ok(match CurveSampler::build(curve, None, true)? {
                    CurveSampler::CurveEngine { multiplier, curve } =>
                        Channel::Engine { multiplier, max: (curve, CurveCache::INVALID), min: None },
                    CurveSampler::TwoCurvesEngine { multiplier, min, max } => Channel::Engine {
                        multiplier, max: (max, CurveCache::INVALID), min: Some((min, CurveCache::INVALID)) },
                    direct => Channel::Direct(direct),
                })
            }).collect::<Result<Vec<_>, &'static str>>()?);
        }
        Ok(Self { streams, storage: storage.map(SlotTail::new), trace: None, refused: None })
    }

    /// Whether this law evaluates the lanes past its calls' ends
    /// ([`CustomData::with_storage`]).
    pub fn tracks_storage(&self) -> bool {
        self.storage.is_some()
    }

    /// The slots past the live count this law carries.
    pub fn storage(&self) -> Option<&SlotTail> {
        self.storage.as_ref()
    }

    /// Every curve object's cache, stream by stream and component by component,
    /// the maximum curve before the minimum.
    pub fn caches(&self) -> Vec<CurveCache> {
        let mut out = Vec::new();
        for channels in self.streams.iter().flatten() {
            for channel in channels {
                if let Channel::Engine { max, min, .. } = channel {
                    out.push(max.1);
                    if let Some(min) = min { out.push(min.1); }
                }
            }
        }
        out
    }

    /// One live lane: every component of both streams at the particle's stored
    /// age, `t = fmax(age * 0.01, +0)` (a NaN age stays NaN). Disabled streams
    /// and components outside the authored count remain intact. The renderer
    /// reads this stored result; it does not evaluate a newer age.
    pub fn update(&mut self, seed: u32, age_percent: f32, values: &mut [[f32; 4]; 2]) {
        self.lane(seed, age_percent, Some(values));
    }

    fn lane(&mut self, seed: u32, age_percent: f32, mut values: Option<&mut [[f32; 4]; 2]>) {
        let time = curve_time_fmax(age_percent);
        let trace = &mut self.trace;
        for (stream, channels) in self.streams.iter_mut().enumerate() {
            let Some(channels) = channels else { continue; };
            for (index, channel) in channels.iter_mut().enumerate() {
                let salt = (0x73a7_f7bb_u32 | ((stream as u32) << 2)).wrapping_add(index as u32);
                let value = match channel {
                    Channel::Direct(sampler) => {
                        if values.is_none() { continue; }
                        sampler.evaluate(time, ParticleRandom::sample(seed, salt))
                    }
                    Channel::Engine { multiplier, max, min } => {
                        let mut side = |curve: &mut (EngineCurve, CurveCache), is_min: bool| {
                            let value = curve.0.evaluate_cached(time, &mut curve.1);
                            if let Some(trace) = trace.as_mut() {
                                trace.push(Evaluation { stream, channel: index, min: is_min, time, value, cache: curve.1 });
                            }
                            value * *multiplier
                        };
                        let hi = side(max, false);
                        match min {
                            None => hi,
                            Some(min) => {
                                let lo = side(min, true);
                                (hi - lo) * ParticleRandom::sample(seed, salt) + lo
                            }
                        }
                    }
                };
                if let Some(values) = values.as_mut() {
                    values[stream][index] = value;
                }
            }
        }
    }

    /// Why the storage stopped following the engine's, if it did. The
    /// caller must then stop simulating the system: the curve objects' caches
    /// are no longer known.
    pub fn refused(&self) -> Option<&'static str> {
        self.refused
    }

    fn follow(&mut self, step: impl FnOnce(&mut SlotTail) -> Result<(), TailRefused>) {
        if self.refused.is_some() {
            return;
        }
        let Some(storage) = self.storage.as_mut() else { return; };
        if let Err(refused) = step(storage) {
            self.refused = Some(match refused {
                TailRefused::Unwritten =>
                    "customData curve cache: a four-lane group reads a slot no storage operation wrote",
                TailRefused::Order =>
                    "customData curve cache: the runtime's storage operations are not the ones the slot model follows",
            });
        }
    }

    /// The end of a call over the live particles `[0, count)` with the slice
    /// `dt`: the lanes of the last four-lane group past `count` are evaluated
    /// from the slots they read (their values go to slots nothing reads), then
    /// SimulateParticles advances those slots' ages as it advances the live
    /// ones'. A law without storage does nothing.
    pub fn end_existing_call(&mut self, count: usize, dt: f32, mode: RingBufferMode, loop_range: [f32; 2]) {
        let mut padding = Vec::new();
        self.follow(|storage| {
            padding = storage.padding(count)?;
            Ok(())
        });
        if self.refused.is_some() {
            return;
        }
        for slot in &padding {
            self.lane(0, slot.age_percent, None);
        }
        self.follow(|storage| {
            storage.advance_padding(count, dt, mode, loop_range);
            Ok(())
        });
    }

    /// A live lane the runtime did not evaluate: the engine evaluates every
    /// lane of the call, so a law with storage stops following it.
    pub fn skipped_lane(&mut self) {
        if self.storage.is_some() && self.refused.is_none() {
            self.refused = Some("customData curve cache: a live lane skipped its evaluation");
        }
    }

    /// See [`SlotTail::kill`].
    pub fn kill(&mut self, before: &[Particle], removed: &[usize], after: &[Particle]) {
        self.follow(|storage| storage.kill(before, removed, after));
    }

    /// See [`SlotTail::birth`].
    pub fn birth(&mut self, old: usize, lanes: &[Particle], live: usize, after: &[Particle]) {
        self.follow(|storage| storage.birth(old, lanes, live, after));
    }

    /// See [`SlotTail::clear`].
    pub fn clear(&mut self, live: &[Particle]) {
        self.follow(|storage| storage.clear(live));
    }
}

#[cfg(test)]
mod native_cache_tests {
    use super::*;
    use crate::particle::json::{parse, Value};
    use crate::particle::schema::CustomDataSlot;
    use crate::particle::{Curve, CurveKey, MinMaxCurve};

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

    // CustomDataModule::Update calls on one module over several calls, against
    // the native curve objects: every Evaluate call on each curve object (its
    // time, value and the cache it left) in order, and every live lane's
    // output. The law evaluates each call's lanes in slot order, the lanes of
    // its last four-lane group past the end included, carrying its caches from
    // call to call. The rows whose value from a reset cache differs from the
    // carried one must exist and be matched.
    #[test]
    #[ignore = "MOLY_CURVE_CACHE_MODULE_NATIVE must identify the native CustomDataModule cache receipt"]
    fn module_calls_match_native_curve_object_caches() {
        let path = std::env::var_os("MOLY_CURVE_CACHE_MODULE_NATIVE").expect("MOLY_CURVE_CACHE_MODULE_NATIVE");
        let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(get(&receipt, "librarySha256").as_str(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
        let (mut cases, mut calls, mut evaluations, mut outputs, mut carried, mut padding) = (0, 0, 0, 0, 0, 0);
        let (mut mismatches, mut bad) = (Vec::new(), 0);
        for case in get(&receipt, "moduleCases").as_array().unwrap() {
            let index = word(get(case, "case"));
            let counts: Vec<Option<usize>> = get(case, "streams").as_array().unwrap().iter()
                .map(|v| v.as_f64().map(|n| n as usize)).collect();
            let mut slots: [Option<CustomDataSlot>; 2] = [None, None];
            for (stream, count) in counts.iter().enumerate() {
                if let Some(count) = count {
                    slots[stream] = Some(CustomDataSlot { component_count: *count, components: Vec::new() });
                }
            }
            for c in get(case, "components").as_array().unwrap() {
                let stream = word(get(c, "stream")) as usize;
                slots[stream].as_mut().unwrap().components.push(component(get(c, "image")));
            }
            let [custom1, custom2] = slots;
            let params = CustomDataParams { custom1, custom2 };
            let mut law = CustomData::with_storage(&params, crate::particle::slot_tail::RESERVED_SLOTS)
                .unwrap_or_else(|reason| panic!("case {index}: {reason}"));
            law.trace = Some(Vec::new());
            let mut values = [[0.0_f32; 4]; 2];
            for call in get(case, "calls").as_array().unwrap() {
                calls += 1;
                let (from, to) = (word(get(call, "from")) as usize, word(get(call, "to")) as usize);
                let (ages, seeds) = (words(get(call, "ages")), words(get(call, "seeds")));
                let native = get(call, "outputs").as_array().unwrap();
                // The groups of four from the call's start while before its end.
                for lane in 0..(to - from).next_multiple_of(4) {
                    law.update(seeds[lane], f32::from_bits(ages[lane]), &mut values);
                    if from + lane >= to {
                        padding += 1;
                        continue;
                    }
                    for (stream, count) in counts.iter().enumerate() {
                        for component in 0..count.unwrap_or(0) {
                            let expected = word(&native[stream].as_array().unwrap()[component].as_array().unwrap()[lane]);
                            outputs += 1;
                            let ok = same(values[stream][component].to_bits(), expected);
                            bad += usize::from(!ok);
                            if !ok && mismatches.len() < 12 {
                                mismatches.push(format!(
                                    "case {index} call {calls} lane {} output {stream}.{component}: ours {:#x} native {expected:#x}",
                                    from + lane, values[stream][component].to_bits()));
                            }
                        }
                    }
                }
            }
            // Per curve object, in evaluation order.
            let ours = law.trace.take().unwrap();
            let native = get(case, "evaluations").as_array().unwrap();
            type Object = (usize, usize, bool);
            let mut by_object: std::collections::BTreeMap<Object, (Vec<&Evaluation>, Vec<&Value>)> = Default::default();
            for e in &ours {
                by_object.entry((e.stream, e.channel, e.min)).or_default().0.push(e);
            }
            for e in native {
                let min = get(e, "side").as_str() == Some("min");
                by_object.entry((word(get(e, "stream")) as usize, word(get(e, "component")) as usize, min))
                    .or_default().1.push(e);
            }
            for (object, (ours, native)) in &by_object {
                if ours.len() != native.len() {
                    bad += 1;
                    if mismatches.len() < 12 {
                        mismatches.push(format!("case {index} object {object:?}: {} evaluations, native {}",
                            ours.len(), native.len()));
                    }
                    continue;
                }
                for (n, (o, e)) in ours.iter().zip(native.iter()).enumerate() {
                    evaluations += 1;
                    let (t, value, fresh) = (word(get(e, "t")), word(get(e, "value")), word(get(e, "fresh")));
                    if !same(value, fresh) {
                        carried += 1;
                    }
                    let after = words(get(e, "after"));
                    let cache_ok = o.cache.words().iter().zip(&after).all(|(a, b)| same(*a, *b));
                    let ok = same(o.time.to_bits(), t) && same(o.value.to_bits(), value) && cache_ok;
                    bad += usize::from(!ok);
                    if !ok && mismatches.len() < 12 {
                        mismatches.push(format!(
                            "case {index} object {object:?} evaluation {n}: t {:#x}/{t:#x} value {:#x}/{value:#x} (fresh {fresh:#x}) cache {:x?}/{after:x?}",
                            o.time.to_bits(), o.value.to_bits(), o.cache.words()));
                    }
                }
            }
            cases += 1;
        }
        println!("module replay: {cases} cases, {calls} calls, {evaluations} evaluations ({carried} whose reset-cache value differs), {outputs} outputs, {padding} lanes past the call ends, {bad} mismatches");
        assert!(bad == 0 && mismatches.is_empty(), "{mismatches:#?}");
        assert!(cases > 0 && carried > 0 && padding > 0, "cases {cases} carried {carried} padding {padding}");
    }
}
