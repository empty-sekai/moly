//! Integration and lifetime state from the current native SimulateParticles.
//! Native age is a percentage, advanced by (dt * 100) * inverse_lifetime.
//! Remaining seconds are a derived view, never the clock's accumulated state.
use crate::particle::buffer::RingBufferMode;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub start_lifetime: f32,
    pub age_percent: f32,
    pub inverse_lifetime: f32,
}
impl Particle {
    pub fn normalized_age(&self) -> f32 { self.age_percent * 0.01 }
    pub fn remaining_lifetime(&self) -> f32 {
        if self.inverse_lifetime == 0.0 { f32::INFINITY }
        else { (100.0 - self.age_percent) / (100.0 * self.inverse_lifetime) }
    }
    pub fn born(position: [f32; 3], velocity: [f32; 3], start_lifetime: f32) -> Self {
        Self { position, velocity, start_lifetime, age_percent: 0.0,
            inverse_lifetime: 1.0 / start_lifetime }
    }
    /// Import an observed API snapshot once. Simulation thereafter keeps native
    /// age/inverse state, avoiding repeated seconds/percentage round trips.
    pub fn with_remaining(mut self, remaining: f32) -> Self {
        self.age_percent = if self.start_lifetime == f32::INFINITY && remaining == f32::INFINITY {
            0.0
        } else { (1.0 - remaining / self.start_lifetime) * 100.0 };
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StepVerdict { Stepped, Refused }

/// Position uses persistent + animated velocity, including the module speed
/// modifier. The caller owns module order and restores persistent velocity.
pub fn integrate(p: &mut Particle, dt: f32, frame_velocity: [f32; 3]) -> StepVerdict {
    if !dt.is_finite() || dt < 0.0 || !all_finite(p.position)
        || !all_finite(p.velocity) || !all_finite(frame_velocity) {
        return StepVerdict::Refused;
    }
    p.velocity = frame_velocity;
    for i in 0..3 { p.position[i] += p.velocity[i] * dt; }
    StepVerdict::Stepped
}

/// InitialModule gravity updates persistent velocity before SimulateParticles.
/// The caller supplies gravity in simulation space.
pub fn apply_gravity(p: &mut Particle, dt: f32, gravity: [f32; 3], gravity_modifier: f32) {
    if !dt.is_finite() || dt < 0.0 || !all_finite(p.velocity) || !all_finite(gravity) { return; }
    for i in 0..3 { p.velocity[i] += gravity[i] * gravity_modifier * dt; }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LifetimeVerdict { Alive(f32), Died, PausedAtEnd, Looped(f32) }

/// Advance a protected particle's lifetime. The caller passes Disabled for
/// Loop overflow indices >= maximum. All modes preserve an already-dead age.
/// Ordinary death is strictly age > 100, after position integration. Pause
/// caps at the float immediately below 100 and does not itself freeze position.
/// Loop subtracts the authored span once, retaining overshoot.
pub fn advance_lifetime(
    p: &mut Particle, dt: f32, mode: RingBufferMode, loop_range: [f32; 2],
) -> LifetimeVerdict {
    if !dt.is_finite() || dt < 0.0 || !(p.start_lifetime > 0.0)
        || !loop_range.iter().all(|v| v.is_finite()) {
        return LifetimeVerdict::Alive(p.remaining_lifetime());
    }
    let previous = p.age_percent;
    let mut age = previous + (dt * 100.0) * p.inverse_lifetime;
    let mut looped = false;
    if mode == RingBufferMode::LoopUntilReplaced {
        let end = loop_range[1] * 100.0;
        let span = end - loop_range[0] * 100.0;
        if age >= end { age -= span; looped = true; }
    }
    let cap = f32::from_bits(if mode == RingBufferMode::PauseUntilReplaced {
        0x42c7ffff
    } else { 0x42c80001 });
    p.age_percent = if previous > 100.0 { previous } else { age.min(cap) };
    if mode == RingBufferMode::PauseUntilReplaced && age >= cap {
        LifetimeVerdict::PausedAtEnd
    } else if looped && previous <= 100.0 {
        LifetimeVerdict::Looped(p.remaining_lifetime())
    } else if p.age_percent > 100.0 {
        LifetimeVerdict::Died
    } else {
        LifetimeVerdict::Alive(p.remaining_lifetime())
    }
}

fn all_finite(v: [f32; 3]) -> bool { v.iter().all(|v| v.is_finite()) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_survives_until_age_is_strictly_greater_than_100() {
        let mut p=Particle::born([0.;3],[0.;3],1.);
        for _ in 0..4 {
            assert!(matches!(advance_lifetime(&mut p,0.25,RingBufferMode::Disabled,[0.,1.]),
                LifetimeVerdict::Alive(_)));
        }
        assert_eq!(p.age_percent,100.);
        assert_eq!(advance_lifetime(&mut p,0.25,RingBufferMode::Disabled,[0.,1.]),LifetimeVerdict::Died);
        assert_eq!(p.age_percent.to_bits(),0x42c80001);
    }
    #[test]
    fn pause_caps_below_100_but_preserves_an_explicitly_dead_age() {
        let mut p=Particle::born([0.;3],[0.;3],1.);
        advance_lifetime(&mut p,2.,RingBufferMode::PauseUntilReplaced,[0.,1.]);
        assert_eq!(p.age_percent.to_bits(),0x42c7ffff);
        p.age_percent=120.;
        advance_lifetime(&mut p,2.,RingBufferMode::PauseUntilReplaced,[0.,1.]);
        assert_eq!(p.age_percent,120.);
    }
    #[test]
    fn loop_preserves_remainder_and_subtracts_only_once() {
        let mut p=Particle::born([0.;3],[0.;3],1.);
        p.age_percent=65.;
        assert!(matches!(advance_lifetime(&mut p,0.25,RingBufferMode::LoopUntilReplaced,[0.25,0.75]),
            LifetimeVerdict::Looped(_)));
        assert_eq!(p.age_percent,40.);
        advance_lifetime(&mut p,3.,RingBufferMode::LoopUntilReplaced,[0.25,0.75]);
        assert_eq!(p.age_percent.to_bits(),0x42c80001);
    }
    #[test]
    fn infinite_lifetime_has_zero_inverse_and_does_not_age() {
        let mut p=Particle::born([0.;3],[0.;3],f32::INFINITY);
        advance_lifetime(&mut p,1.,RingBufferMode::Disabled,[0.,1.]);
        assert_eq!(p.age_percent,0.);
        assert_eq!(p.remaining_lifetime(),f32::INFINITY);
    }
    #[test]
    fn integrate_uses_supplied_velocity_and_refuses_invalid_inputs() {
        let mut p=Particle::born([0.,10.,0.],[5.;3],1.);
        assert_eq!(integrate(&mut p,0.5,[0.,-2.,0.]),StepVerdict::Stepped);
        assert_eq!(p.position,[0.,9.,0.]);
        let before=p;
        assert_eq!(integrate(&mut p,f32::NAN,[0.;3]),StepVerdict::Refused);
        assert_eq!(integrate(&mut p,-1.,[0.;3]),StepVerdict::Refused);
        assert_eq!(p,before);
    }
}
