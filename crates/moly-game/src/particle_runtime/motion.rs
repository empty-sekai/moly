//! Space conversion and appearance inputs shared by particle consumers.
use super::Side;
use bevy::prelude::*;
use moly_law::particle::Particle;
use moly_law::particle::schema::SimulationSpace;
use moly_law::particle::velocity::{VelocityOverLifetime, animated_velocity};

/// Consume the native age-percent word directly. Normalizing and multiplying
/// back by 100 rounds it again before SizeOverLifetime performs normalization.
pub(super) fn size_at_age_percent(
    system: &super::Runtime, side: &Side, age_percent: f32,
) -> [f32; 3] {
    system.size_law.as_ref().map_or(side.size,
        |law| law.evaluate(side.size, side.seed, age_percent))
}

/// Force and linear velocity share the source space transform, including the
/// emitter scale for world-space modules. Directions are never normalized.
pub(super) fn module_vector(
    value: [f32; 3], in_world_space: bool, simulation: SimulationSpace,
    owner: &GlobalTransform,
) -> Vec3 {
    let value = crate::particle_geometry::reflect(Vec3::from_array(value));
    let affine = owner.affine();
    match (simulation == SimulationSpace::World, in_world_space) {
        (false, false) => value,
        (true, false) => affine.transform_vector3(value),
        (true, true) => value * owner.to_scale_rotation_translation().0,
        (false, true) => affine.inverse().transform_vector3(value * owner.to_scale_rotation_translation().0),
    }
}

pub(super) fn velocity_at_age(
    params: &VelocityOverLifetime,
    particle: &Particle,
    side: &Side,
    batch_seed: u32,
    simulation: SimulationSpace,
    owner: &GlobalTransform,
    age: f32,
    dt: f32,
) -> ([f32; 3], f32) {
    let reflect = crate::particle_geometry::reflect;
    let affine = owner.affine();
    let world = simulation == SimulationSpace::World;
    let sampled = params.sample(side.seed, batch_seed, age * 100.0);
    let linear = module_vector(sampled.linear, params.in_world_space, simulation, owner);
    let modifier = sampled.speed_modifier;
    let position = Vec3::from_array(particle.position);
    let local = if world { affine.inverse().transform_point3(position) } else { position };
    // Orbital motion deliberately does not branch on params.in_world_space.
    let orbit = sampled.orbital;
    let delta = reflect(Vec3::from_array(orbit.displacement(reflect(local).to_array(), dt, modifier)));
    let delta = if world { affine.transform_vector3(delta) } else { delta };
    let orbital = Vec3::from_array(animated_velocity(delta.to_array(), dt, modifier));
    ((linear + orbital).to_array(), modifier)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moly_law::particle::{Curve, CurveKey, MinMaxCurve};
    use moly_law::particle::schema::{SizeOverLifetimeParams, VelocityOverLifetimeParams};

    fn params() -> VelocityOverLifetimeParams {
        VelocityOverLifetimeParams {
            x: MinMaxCurve::Constant(0.0), y: MinMaxCurve::Constant(0.0), z: MinMaxCurve::Constant(0.0),
            speed_modifier: MinMaxCurve::Constant(1.0), in_world_space: false,
            orbital: [MinMaxCurve::Constant(0.0), MinMaxCurve::Constant(1.0), MinMaxCurve::Constant(0.0)],
            orbital_offset: std::array::from_fn(|_| MinMaxCurve::Constant(0.0)),
            radial: MinMaxCurve::Constant(0.0),
        }
    }

    fn side() -> Side {
        Side { rand: 0.5, seed: 17, rot: [0.0; 3], size: [1.0; 3], gravity: 0.0,
            colour: [1.0; 4], total_velocity: [0.0; 3], custom_data: [[0.0; 4]; 2],
            emit_carry: [0.0; 2], animated: [0.0; 3] }
    }

    #[test]
    fn source_snow_size_keeps_native_age_percent_for_draw() {
        // Current JP 6.8.1 snow, first normal frame after 12s prewarm,
        // particle 66. Native pool +0x300 is 0x3b3840b0; the old age
        // round trip produced 0x3b38418a. Keep the authored Size curve.
        let keys = [(0.0, 1.0, 0.3333333432674408),
            (0.9457467198371887, 1.0, 0.0), (1.0, 0.0, 0.0)]
            .map(|(time, value, weight)| CurveKey {
                time, value, in_slope: 0.0, out_slope: 0.0,
                weighted_mode: 0, in_weight: weight, out_weight: weight,
            });
        let params = SizeOverLifetimeParams {
            separate_axes: false,
            curve: MinMaxCurve::Curve {
                multiplier: 1.0,
                max: Curve { keys: keys.to_vec(), multiplier: 1.0 },
            },
            y: None, z: None,
        };
        let mut system = super::super::test_support::runtime();
        system.size_law = Some(moly_law::particle::size::SizeOverLifetime::from_params(&params));
        system.pool[0].age_percent = f32::from_bits(0x42c6_c23d);
        system.side[0].size = [f32::from_bits(0x3d9e_c5e4); 3];
        system.side[0].seed = 1_790_689_260;
        let expected = 0x3b38_40b0;
        assert_eq!(size_at_age_percent(&system, &system.side[0],
            system.pool[0].age_percent).map(f32::to_bits), [expected; 3]);
        let quads = super::super::build_quads(&system, &GlobalTransform::IDENTITY);
        assert_eq!(quads.len(), 1);
        assert_eq!(quads[0].size.to_array().map(f32::to_bits), [expected; 2]);
    }

    #[test]
    fn orbital_space_does_not_follow_the_linear_space_switch() {
        let owner = GlobalTransform::from(Transform {
            translation: Vec3::new(-3.0, 4.0, 5.0),
            rotation: Quat::from_rotation_y(-45.0_f32.to_radians()),
            ..default()
        });
        let particle = Particle::born([-2.0, 1.0, 3.0], [0.0; 3], 10.0);
        let mut params = params();
        for simulation in [SimulationSpace::Local, SimulationSpace::World] {
            params.in_world_space = false;
            let local = velocity_at_age(&VelocityOverLifetime::from_params(&params), &particle, &side(), 17, simulation, &owner, 0.0, 0.01);
            params.in_world_space = true;
            let world = velocity_at_age(&VelocityOverLifetime::from_params(&params), &particle, &side(), 17, simulation, &owner, 0.0, 0.01);
            assert_eq!(local, world);
            let expected = if simulation == SimulationSpace::World {
                [1.9949831, 0.0, 1.0099609]
            } else { [-2.9899597, 0.0, -2.014947] };
            for axis in 0..3 { assert!((local.0[axis] - expected[axis]).abs() < 0.0002); }
        }
    }

    #[test]
    fn linear_motion_crosses_the_reflected_basis_once() {
        let mut params = params();
        params.orbital = std::array::from_fn(|_| MinMaxCurve::Constant(0.0));
        params.x = MinMaxCurve::Constant(1.0);
        params.y = MinMaxCurve::Constant(2.0);
        params.z = MinMaxCurve::Constant(3.0);
        let particle = Particle::born([0.0; 3], [0.0; 3], 10.0);
        let (linear, modifier) = velocity_at_age(&VelocityOverLifetime::from_params(&params), &particle, &side(), 17, SimulationSpace::Local,
            &GlobalTransform::IDENTITY, 0.0, 0.01);
        assert_eq!(linear, [-1.0, 2.0, 3.0]);
        assert_eq!(modifier, 1.0);
    }
}
