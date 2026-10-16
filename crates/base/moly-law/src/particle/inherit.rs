//! The inherited block of a sub-emitter command: what RecordEmit samples from
//! the parent particle at the event, and what the child's InitialModule start
//! does with it at the command's delivery.
//!
//! RecordEmit writes the neutral block ([`crate::particle::sub_emission`]'s
//! thirteen words) and, only when the edge's properties word is not zero and
//! the event issues its commands, copies the particle out of the parent's
//! arrays at the event (so for a death edge inside the kill, before the slot
//! is overwritten) and lets the properties bits overwrite parts of it:
//!
//! - colour (1): the particle colour through the parent's colour modules;
//! - size (2): the particle size through the parent's size modules (below);
//! - rotation (4): the particle rotation and its axis of rotation;
//! - lifetime (8): the particle's remaining lifetime in seconds;
//! - duration (16): the event's current normalized time.
//!
//! The child copies the block into its InitialModule before its start and
//! restores the neutral block after it. There the size and the lifetime
//! multiply the child's own start values, the rotation is added to the
//! child's start rotation, the colour multiplies each colour channel, and a
//! finite duration replaces the time every start curve reads. None of it
//! reaches the child's emission clock.
//!
//! Only the size bit is transcribed: it is the only bit an edge with a child
//! sets in the weather effects. The other bits, and the parent size-module
//! configurations the size path does not compose, are refused by name.

use crate::particle::armf as a;
use crate::particle::curve::{CurveSampler, CurveTime};
use crate::particle::schema::SizeOverLifetimeParams;
use crate::particle::sub_emission::neutral_inherited;
use crate::particle::value::MinMaxCurve;

pub const INHERIT_COLOR: u32 = 1;
pub const INHERIT_SIZE: u32 = 2;
pub const INHERIT_ROTATION: u32 = 4;
pub const INHERIT_LIFETIME: u32 = 8;
pub const INHERIT_DURATION: u32 = 16;

/// `-0.01` as RecordEmit's age-to-remaining-lifetime multiply holds it.
const MINUS_PERCENT: f32 = f32::from_bits(0xbc23_d70a);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// An inherit bit this port does not transcribe, by name.
    Bit(&'static str),
    /// A properties word with bits outside the five inherit flags.
    UnknownBits,
    /// A configuration of the parent's SizeModule the inherited size does not
    /// compose, by name.
    ParentSize(&'static str),
    /// A child command whose block carries a non-neutral word the child side
    /// does not transcribe, by name.
    Block(&'static str),
}

/// The parent's SizeModule as the inherited size reads it (its
/// `UpdateSingle` over three axes, from the particle the event copied).
#[derive(Clone, Debug)]
enum SizeFactor {
    /// The parent has no enabled SizeModule.
    Absent,
    /// A constant: every axis times the constant floored at +0.
    Constant(f32),
    /// A curve through `AnimationCurveTpl::Evaluate` times its multiplier.
    Curve(CurveSampler),
}

/// The size bit of one edge: the parent's size module and whether the
/// parent's particle arrays store three size axes.
#[derive(Clone, Debug)]
pub struct InheritSize {
    factor: SizeFactor,
    size_3d: bool,
}

/// The parent particle the event copies, as its arrays hold it at the event:
/// its size (x only when the arrays store one axis), its age percent and its
/// inverse lifetime.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InheritParent {
    pub size: [f32; 3],
    pub age_percent: f32,
    pub inverse_lifetime: f32,
}

impl InheritSize {
    /// The inherit law of an edge's properties word. `Ok(None)` for zero.
    /// `size_over_lifetime` is the parent's enabled SizeModule, `size_3d`
    /// whether the parent's arrays store three size axes (start size 3D, or a
    /// size module with separate axes). The parent admission refuses a system
    /// with a SizeBySpeed module (no consumer), which the size path would read
    /// after the SizeModule.
    pub fn from_parent(properties: u32, size_over_lifetime: Option<&SizeOverLifetimeParams>, size_3d: bool)
        -> Result<Option<Self>, Refused> {
        if properties == 0 {
            return Ok(None);
        }
        if properties & !31 != 0 {
            return Err(Refused::UnknownBits);
        }
        for (bit, name) in [(INHERIT_COLOR, "color"), (INHERIT_ROTATION, "rotation"),
            (INHERIT_LIFETIME, "lifetime"), (INHERIT_DURATION, "duration")] {
            if properties & bit != 0 {
                return Err(Refused::Bit(name));
            }
        }
        let factor = match size_over_lifetime {
            None => SizeFactor::Absent,
            Some(size) => {
                if size.separate_axes {
                    return Err(Refused::ParentSize("separate axes"));
                }
                match &size.curve {
                    MinMaxCurve::Constant(value) => SizeFactor::Constant(*value),
                    MinMaxCurve::TwoConstants { .. } | MinMaxCurve::TwoCurves { .. } => {
                        return Err(Refused::ParentSize(
                            "two constants or two curves: the particle copy's random word is never written"));
                    }
                    curve @ MinMaxCurve::Curve { .. } => {
                        if CurveSampler::engine_optimized(curve) {
                            return Err(Refused::ParentSize("curve on the optimized polynomial"));
                        }
                        // The copy's normalized age has no upper bound.
                        SizeFactor::Curve(CurveSampler::new(curve, CurveTime::Unbounded).map_err(Refused::ParentSize)?)
                    }
                }
            }
        };
        Ok(Some(Self { factor, size_3d }))
    }

    /// The inherited block of one event: the neutral words with the parent
    /// particle's seed last, the three size words replaced.
    ///
    /// The copy holds the size (x on all three axes when one is stored), the
    /// start lifetime `1 / inverse` and the remaining lifetime
    /// `start * (age * -0.01 + 1)`. The SizeModule multiplies each axis by
    /// its factor floored at +0 with the maximum-number rule: a constant, or
    /// the curve at the copy's normalized age `(start - remaining) / start`
    /// (+0 when the start lifetime is zero), itself floored at +0 the same way
    /// and not clamped above. With one stored axis, y and z then take x.
    pub fn block(&self, parent: &InheritParent, seed: u32) -> [u32; 13] {
        let mut size = if self.size_3d { parent.size } else { [parent.size[0]; 3] };
        let factor = match &self.factor {
            SizeFactor::Absent => None,
            SizeFactor::Constant(value) => Some(*value),
            SizeFactor::Curve(curve) => Some(curve.evaluate(copy_time(parent), 0.0)),
        };
        if let Some(factor) = factor {
            let factor = a::max_nm(factor, 0.0);
            size = size.map(|axis| a::mul(axis, factor));
        }
        if !self.size_3d {
            size = [size[0]; 3];
        }
        let mut block = neutral_inherited(seed);
        for axis in 0..3 {
            block[1 + axis] = size[axis].to_bits();
        }
        block
    }
}

/// The copy's normalized age as the SizeModule's single-particle update
/// computes it from the start and remaining lifetime.
fn copy_time(parent: &InheritParent) -> f32 {
    let start = a::div(1.0, parent.inverse_lifetime);
    let remaining = a::mul(start, a::add(a::mul(parent.age_percent, MINUS_PERCENT), 1.0));
    let t = if start == 0.0 { 0.0 } else { a::div(a::sub(start, remaining), start) };
    a::max_nm(t, 0.0)
}

/// The inherited size a child command carries, read on the child side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChildInherit {
    size: [f32; 3],
}

impl ChildInherit {
    /// The block of a command (its first twelve words; the thirteenth is the
    /// parent particle's seed, which only a trail reads). Every word but the
    /// three size words must be neutral; a non-neutral one is refused by the
    /// bit that writes it.
    pub fn from_words(words: &[u32; 13]) -> Result<Self, Refused> {
        let neutral = neutral_inherited(0);
        if words[0] != neutral[0] {
            return Err(Refused::Block("color"));
        }
        if words[4..10] != neutral[4..10] {
            return Err(Refused::Block("rotation"));
        }
        if words[10] != neutral[10] {
            return Err(Refused::Block("lifetime"));
        }
        if words[11] != neutral[11] {
            return Err(Refused::Block("duration"));
        }
        Ok(Self { size: std::array::from_fn(|axis| f32::from_bits(words[1 + axis])) })
    }

    /// InitialModule's start size: each stored axis is the inherited axis
    /// times the child's own start size of that axis, already floored at +0.
    pub fn start_size(&self, own: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|axis| a::mul(self.size[axis], own[axis]))
    }

    /// Whether every size word is the neutral 1.
    pub fn is_neutral(&self) -> bool {
        self.size.iter().all(|s| s.to_bits() == 1.0_f32.to_bits())
    }
}

#[cfg(test)]
mod tests;
