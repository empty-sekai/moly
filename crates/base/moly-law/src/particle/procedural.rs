//! The engine's procedural update of a particle system: `Update` with the
//! procedural flag, which the first Play of a looping prewarm system takes
//! when the system supports it (the per-frame update never sets the flag).
//!
//! Update1Incremental runs its ordinary slice loop, clock and emission, but
//! instead of simulating it ages every emit replay by the slice and records
//! the slice's emission as replays (`StartParticlesProcedural`, [`record`]).
//! `UpdateProcedural` then regenerates the particles from the replays: one
//! `InitialModule::GenerateProcedural` per replay
//! ([`InitialLaw::generate_group`] per four-lane group), the particle
//! arrays padded to four, `ShapeModule::Start` over all of them (the
//! caller's), each particle's replay time and age, the start speed evaluated
//! at that time with the particle's own hashed draw and the closed-form
//! position and velocity under gravity, the kill of every particle whose age
//! word is past 100, and `RotationModule::UpdateProcedural`.
//!
//! [`Storage`] follows the particle arrays slot by slot, the slots past the
//! live count included, as the regeneration leaves them.
use super::curve::{CurveSampler, CurveTime};
use super::initial::{InitialLaw, ProceduralGroupInput, ProceduralLane, Refused as InitialRefused};
use super::random::ParticleRandom;
use super::seed_owner::ModuleRandom;
use super::value::MinMaxCurve;

/// One emit replay, as `StartParticlesProcedural` appends it: the system
/// time of its slice, the time it has been alive (the burst record starts
/// at the slice, the continuous one at zero; every later slice adds its
/// own), the emission offset and the spacing times the slice, the particle
/// count and the continuous count (zero for the burst record).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmitReplay {
    pub time: f32,
    pub alive_time: f32,
    pub offset: f32,
    pub gap: f32,
    pub count: i32,
    pub continuous: u64,
}

/// Update1Incremental with the procedural flag, after each slice's Tick:
/// every replay's alive time grows by the slice.
pub fn age(replays: &mut [EmitReplay], dt: f32) {
    for replay in replays {
        replay.alive_time = dt + replay.alive_time;
    }
}

/// `StartParticlesProcedural(time, dt, continuous, amount)` with the
/// emission state's spacing and offset after the slice's EmitOverTime. The
/// capacity is the Initial maximum, doubled in a ring mode, less every
/// replay's count; the kept amount splits into a first record of the
/// non-continuous particles (its count also takes the continuous count
/// whenever the dropped part covers it, as the engine computes it, which
/// then exceeds the capacity) and a record of the continuous particles kept.
#[allow(clippy::too_many_arguments)]
pub fn record(replays: &mut Vec<EmitReplay>, time: f32, dt: f32, spacing: f32, offset: f32, continuous: u64,
    amount: u64, max_particles: i32, ring: bool) {
    if amount == 0 {
        return;
    }
    let sum = replays.iter().fold(0_i32, |sum, replay| replay.count.wrapping_add(sum));
    let requested = amount.wrapping_add(sum as i64 as u64);
    let cap = (max_particles as i64 as u64) << u32::from(ring);
    let kept = (if cap < requested { cap } else { requested }) as u32 as i32;
    let emitted = kept.wrapping_sub(sum);
    if emitted <= 0 {
        return;
    }
    let emitted = emitted as u32 as u64;
    let dropped = amount.wrapping_sub(emitted);
    let first_extra = if dropped < continuous { 0 } else { continuous };
    let kept_continuous = if continuous < dropped { 0 } else { continuous - dropped };
    let gap = spacing * dt;
    let first = emitted.wrapping_sub(kept_continuous).wrapping_add(first_extra);
    if first != 0 {
        replays.push(EmitReplay { time, alive_time: dt, offset, gap, count: first as u32 as i32, continuous: 0 });
    }
    if dropped < continuous {
        replays.push(EmitReplay { time, alive_time: 0.0, offset, gap, count: kept_continuous as u32 as i32,
            continuous: kept_continuous });
    }
}

/// One slot of the particle arrays, in source coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub animated: [f32; 3],
    pub size: [f32; 3],
    pub rotation: [f32; 3],
    pub age_percent: f32,
    pub inverse_lifetime: f32,
    /// Not an engine array: the floored lifetime the runtime keeps beside.
    pub lifetime: f32,
    pub seed: u32,
    pub color: [u8; 4],
    pub axis: [f32; 3],
    /// Every array of the slot was written by this regeneration (a generated
    /// lane, or a kill's copy of one). A slot only the padding reached keeps
    /// the seed and colour it held, which this model does not know.
    pub written: bool,
    /// The age and inverse lifetime words are known: a written slot, or one
    /// the padding copied them into (unless a later step read its unknown
    /// seed or colour into them).
    pub age_known: bool,
}

/// The particle arrays as the regeneration writes them: `slots[..live]` are
/// the particles, the rest the storage past them that some step wrote.
#[derive(Clone, Debug, Default)]
pub struct Storage {
    pub slots: Vec<Slot>,
    pub live: usize,
}

/// The emitter inputs of one regeneration besides the replays.
#[derive(Clone, Copy, Debug)]
pub struct Emitter<'a> {
    pub initial: &'a InitialLaw,
    pub speed: &'a MinMaxCurve,
    pub duration: f32,
    /// The Initial gravity times the modifier, in the simulation space.
    pub gravity: [f32; 3],
    pub storage_size_3d: bool,
    pub storage_rotation_3d: bool,
    /// The arrays carry the axis-of-rotation channel.
    pub storage_axis: bool,
    /// The simulation-space emitter matrix's translation and its normalized
    /// third column: every generated lane's position and direction.
    pub translation: [f32; 3],
    pub direction: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Initial(InitialRefused),
    Speed(&'static str),
    /// A RotationModule curve mode whose integral this law does not carry.
    RotationCurve,
    /// A lifetime reciprocal outside the positive normal range.
    Reciprocal,
}

/// GenerateProcedural's direction: the emitter matrix's third column over its
/// length, `(x*x + y*y) + (z*z + 0)`, by FRSQRTE and two FRSQRTS refinements;
/// zero at or below a tiny length (NaN included).
pub fn direction(column: [f32; 3]) -> [f32; 3] {
    let square = (column[0] * column[0] + column[1] * column[1]) + (column[2] * column[2] + 0.0);
    if !(square > f32::from_bits(0x0da2_4260)) {
        return [0.0; 3];
    }
    let r = super::shape::native_rsqrt(square);
    column.map(|c| c * r)
}

/// `InitialModule::GenerateProcedural` over every replay, in order: each
/// emitted group writes four slots at the running count, which grows by the
/// smaller of four and the replay's count still to emit (lowered by four per
/// emitted group only). Returns the first particle index of each replay and
/// the count after the last. Then `array_resize` and
/// `PadParticleDataToSIMDBoundary`: the slots up to the next multiple of four
/// take the last particle's position, velocity, animated velocity, sizes,
/// rotations, age and inverse lifetime.
pub fn generate(storage: &mut Storage, replays: &[EmitReplay], emitter: &Emitter, random: &mut ModuleRandom)
    -> Result<Vec<usize>, Refused> {
    let mut count = 0_usize;
    let mut offsets = Vec::with_capacity(replays.len() + 1);
    for replay in replays {
        offsets.push(count);
        let mut left = replay.count;
        if left == 0 {
            continue;
        }
        let input = |first_lane| ProceduralGroupInput {
            curve_time: replay.time / emitter.duration,
            alive_time: replay.alive_time,
            offset: replay.offset,
            gap: replay.gap,
            continuous: replay.continuous as f32,
            first_lane,
            storage_size_3d: emitter.storage_size_3d,
            storage_rotation_3d: emitter.storage_rotation_3d,
        };
        let mut group = 0_i64;
        while group < replay.count as i64 {
            if let Some(lanes) = emitter.initial.generate_group(random, input(group as f32)).map_err(Refused::Initial)? {
                write_group(storage, count, &lanes, emitter);
                let step = left as i64 as u64;
                count += if step < 4 { step as usize } else { 4 };
                left = left.wrapping_sub(4);
            }
            group += 4;
        }
    }
    offsets.push(count);
    storage.live = count;
    pad(storage, emitter);
    Ok(offsets)
}

fn write_group(storage: &mut Storage, at: usize, lanes: &[ProceduralLane; 4], emitter: &Emitter) {
    if storage.slots.len() < at + 4 {
        storage.slots.resize(at + 4, EMPTY);
    }
    for (index, lane) in lanes.iter().enumerate() {
        let slot = &mut storage.slots[at + index];
        slot.position = emitter.translation;
        slot.velocity = emitter.direction;
        slot.animated = [0.0; 3];
        slot.age_percent = lane.age_percent;
        slot.inverse_lifetime = lane.inverse_lifetime;
        slot.lifetime = lane.lifetime;
        slot.seed = lane.seed;
        slot.size[0] = lane.size[0].unwrap_or(0.0);
        if emitter.storage_size_3d {
            slot.size[1] = lane.size[1].unwrap_or(0.0);
            slot.size[2] = lane.size[2].unwrap_or(0.0);
        }
        slot.rotation[2] = lane.rotation[2].unwrap_or(0.0);
        if emitter.storage_rotation_3d {
            slot.rotation[0] = lane.rotation[0].unwrap_or(0.0);
            slot.rotation[1] = lane.rotation[1].unwrap_or(0.0);
        }
        slot.color = lane.color;
        if emitter.storage_axis {
            slot.axis = [0.0, 0.0, 1.0];
        }
        slot.written = true;
        slot.age_known = true;
    }
}

const EMPTY: Slot = Slot {
    position: [0.0; 3], velocity: [0.0; 3], animated: [0.0; 3], size: [0.0; 3], rotation: [0.0; 3],
    age_percent: 0.0, inverse_lifetime: 0.0, lifetime: 0.0, seed: 0, color: [0; 4], axis: [0.0; 3], written: false,
    age_known: false,
};

fn pad(storage: &mut Storage, emitter: &Emitter) {
    let count = storage.live;
    let end = (count + 3) & !3;
    if count == 0 || count >= end {
        return;
    }
    if storage.slots.len() < end {
        storage.slots.resize(end, EMPTY);
    }
    let last = storage.slots[count - 1];
    for slot in &mut storage.slots[count..end] {
        slot.position = last.position;
        slot.velocity = last.velocity;
        slot.animated = last.animated;
        slot.size[0] = last.size[0];
        if emitter.storage_size_3d {
            slot.size[1] = last.size[1];
            slot.size[2] = last.size[2];
        }
        slot.rotation[2] = last.rotation[2];
        if emitter.storage_rotation_3d {
            slot.rotation[0] = last.rotation[0];
            slot.rotation[1] = last.rotation[1];
        }
        slot.age_percent = last.age_percent;
        slot.inverse_lifetime = last.inverse_lifetime;
        slot.age_known = last.age_known;
    }
}

/// Each particle's replay time (the replay time times the duration
/// reciprocal) and age (the replay's alive time, plus `index * gap` while
/// the index, the replay offset then +1 per particle, is below the
/// continuous count), the slots up to the next multiple of four copying the
/// last; then a replay whose first particle index equals the particle count
/// is removed, the last replay taking its place.
pub fn times(replays: &mut Vec<EmitReplay>, offsets: &[usize], count: usize, duration: f32) -> (Vec<f32>, Vec<f32>) {
    let end = (count + 3) & !3;
    let mut normalized = vec![0.0_f32; end.max(count)];
    let mut ages = vec![0.0_f32; end.max(count)];
    let inverse = 1.0 / duration;
    let original = replays.len();
    let mut slot = 0_usize;
    let mut index = 0_usize;
    while index < original {
        let (from, to) = (offsets[index], offsets[index + 1]);
        if from < to {
            let replay = replays[slot];
            let continuous = replay.continuous as f32;
            let mut lane = replay.offset;
            for p in from..to {
                normalized[p] = inverse * replay.time;
                let extra = if lane < continuous { lane * replay.gap } else { 0.0 };
                ages[p] = replay.alive_time + extra;
                lane += 1.0;
            }
        }
        if from == count {
            let last = replays.len() - 1;
            replays[slot] = replays[last];
            replays.truncate(last);
        } else {
            slot += 1;
        }
        index += 1;
    }
    if count > 0 {
        for p in count..end {
            normalized[p] = normalized[count - 1];
            ages[p] = ages[count - 1];
        }
    }
    (normalized, ages)
}

/// The start speed at each particle's replay time with its hashed draw, then
/// `p += (g * age) * (age * 0.5) + age * (speed * v)` and
/// `v = g * age + speed * v` per axis, over every slot of the four-lane
/// groups below the count.
pub fn motion(storage: &mut Storage, normalized: &[f32], ages: &[f32], emitter: &Emitter) -> Result<(), Refused> {
    let count = storage.live;
    if count == 0 {
        return Ok(());
    }
    let speed = CurveSampler::new(emitter.speed, CurveTime::Normalized).map_err(Refused::Speed)?;
    let end = (count + 3) & !3;
    let g = emitter.gravity;
    for p in 0..end {
        let slot = &mut storage.slots[p];
        let s = speed.evaluate(normalized[p], ParticleRandom::sample(slot.seed, 0x96aa_4de3));
        let age = ages[p];
        let half = age * 0.5;
        for axis in 0..3 {
            let ga = g[axis] * age;
            let sv = s * slot.velocity[axis];
            slot.position[axis] = (ga * half + age * sv) + slot.position[axis];
            slot.velocity[axis] = ga + sv;
        }
    }
    Ok(())
}

/// The kill pass: four lanes at a time, lanes 3 to 0, every lane below the
/// count whose age word is past 100 takes the last particle's slot and the
/// count drops; a group with a kill is tested again.
pub fn kill(storage: &mut Storage) -> usize {
    let mut killed = 0;
    let mut group = 0_usize;
    while group < storage.live {
        let dead: Vec<usize> = (0..4).map(|lane| group + lane)
            .filter(|&p| p < storage.live && storage.slots[p].age_percent > 100.0).collect();
        if dead.is_empty() {
            group += 4;
            continue;
        }
        for &p in dead.iter().rev() {
            let last = storage.live - 1;
            storage.slots[p] = storage.slots[last];
            storage.live = last;
            killed += 1;
        }
    }
    killed
}

/// The RotationModule curves `UpdateProcedural` reads: Z, and X and Y with
/// separate axes.
#[derive(Clone, Copy, Debug)]
pub struct Rotation<'a> {
    pub separate_axes: bool,
    pub curves: [&'a MinMaxCurve; 3],
    /// The Initial module's randomize-rotation-direction threshold.
    pub randomize_direction: f32,
}

/// `RotationModule::UpdateProcedural`: per axis, every slot of the four-lane
/// groups below the count adds the lifetime (the reciprocal of the inverse
/// lifetime, the bare estimate for zero) times the signed integral of the
/// angular velocity over the normalized age `max(age * 0.01, 0)`: the
/// constant times it, or the two constants' interpolation at the particle's
/// hashed draw times it. The sign is the start rotation's (the particle
/// seed's hash above the threshold keeps it).
pub fn rotate(storage: &mut Storage, rotation: &Rotation) -> Result<(), Refused> {
    let count = storage.live;
    if count == 0 {
        return Ok(());
    }
    let end = (count + 3) & !3;
    let first = if rotation.separate_axes { 0 } else { 2 };
    for axis in first..3 {
        let (min, max) = match *rotation.curves[axis] {
            MinMaxCurve::Constant(value) => (None, value),
            MinMaxCurve::TwoConstants { min, max } => (Some(min), max),
            _ => return Err(Refused::RotationCurve),
        };
        for p in 0..end {
            let slot = &mut storage.slots[p];
            let lifetime = rotation_lifetime(slot.inverse_lifetime).ok_or(Refused::Reciprocal)?;
            let normalized = {
                let v = slot.age_percent * f32::from_bits(0x3c23_d70a);
                if v.is_nan() { v } else if v > 0.0 { v } else { 0.0 }
            };
            let rate = match min {
                None => normalized * max,
                Some(min) => {
                    let spread = max - min;
                    normalized * (min + ParticleRandom::sample(slot.seed, 0x6aed_452e) * spread)
                }
            };
            let signed = if ParticleRandom::sample(slot.seed, 0xff2b_b1a4) > rotation.randomize_direction { rate } else { -rate };
            slot.rotation[axis] = slot.rotation[axis] + lifetime * signed;
        }
    }
    Ok(())
}

/// FRECPE of the inverse lifetime refined twice (the bare estimate, +inf,
/// for a zero); `None` outside the positive normal range and zero.
fn rotation_lifetime(inverse: f32) -> Option<f32> {
    if inverse == 0.0 && inverse.is_sign_positive() {
        return Some(f32::INFINITY);
    }
    if !(inverse.is_normal() && inverse > 0.0) {
        return None;
    }
    Some(super::initial::procedural_reciprocal(inverse))
}

impl Storage {
    /// The slots past the live count whose age and inverse lifetime this
    /// regeneration determined, up to the first it did not. These are the
    /// words the storage-following laws read past the count (a curve time and
    /// a lifetime advance); a slot only the padding reached has them from the
    /// last particle, while its position depends on its unknown seed.
    pub fn known_tail(&self) -> &[Slot] {
        let tail = self.slots.get(self.live..).unwrap_or(&[]);
        let end = tail.iter().position(|slot| !slot.age_known).unwrap_or(tail.len());
        &tail[..end]
    }
}
