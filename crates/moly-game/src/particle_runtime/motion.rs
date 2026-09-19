//! Space conversion and appearance inputs shared by particle consumers.
use super::Side;
use bevy::prelude::*;
use moly_law::particle::Particle;
use moly_law::particle::schema::SimulationSpace;
use moly_law::particle::velocity::{VelocityOverLifetime, animated_velocity};

pub(super) fn size_at_age(system: &super::Runtime, side: &Side, age: f32) -> [f32; 3] {
    system.size_law.as_ref().map_or(side.size,
        |law| law.evaluate(side.size, side.seed, age * 100.0))
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
    let linear = reflect(Vec3::from_array(sampled.linear));
    // The linear module's world-space value retains the emitter scale. Its
    // local-space value uses the complete emitter basis when simulated in world.
    let linear = match (world, params.in_world_space) {
        (false, false) => linear,
        (true, false) => affine.transform_vector3(linear),
        (true, true) => linear * owner.to_scale_rotation_translation().0,
        (false, true) => affine.inverse().transform_vector3(linear * owner.to_scale_rotation_translation().0),
    };
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
    use moly_law::particle::MinMaxCurve;
    use moly_law::particle::schema::VelocityOverLifetimeParams;

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
            colour: [1.0; 4], total_velocity: [0.0; 3] }
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
