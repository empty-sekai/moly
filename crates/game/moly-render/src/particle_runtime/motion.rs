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

/// The size the renderer reads for live particle `index`. A size law that
/// follows the engine's SizeModule calls stored it in the last call, which
/// every update that births or ages a particle ends in; otherwise the law is
/// evaluated at the age.
pub(super) fn current_size(system: &super::Runtime, index: usize, age_percent: f32) -> [f32; 3] {
    let side = &system.side[index];
    match system.size_law.as_ref() {
        Some(law) if law.calls().is_some() => law.stored(index)
            .expect("a size law that follows the engine's calls stored every live slot in the update's last call"),
        _ => size_at_age_percent(system, side, age_percent),
    }
}

/// Why a Velocity or Force module in world space in a Custom simulation space
/// is not run: GetTransformationMatrix tests only whether the space is World,
/// so it reads the world-to-local the owner update composes from the custom
/// transform, which the root update does not have.
pub(super) const CUSTOM_WORLD_MODULE: &str =
    "Velocity or Force in world space in a Custom simulation space: the custom transform's world-to-local is not composed here";

/// Whether the system has a Velocity or Force module in world space in a
/// Custom simulation space ([`CUSTOM_WORLD_MODULE`]).
pub(super) fn custom_world_module(system: &super::Runtime) -> bool {
    system.emitter.simulation_space == SimulationSpace::Custom
        && (system.velocity_law.as_ref().is_some_and(|law| law.in_world_space)
            || system.force_law.as_ref().is_some_and(|law| law.in_world_space))
}

const UNIT_COLUMNS: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// The columns ForceModule and VelocityModule take their linear vector
/// through, in source axes, as GetTransformationMatrix builds them once per
/// module call from the owner words. It tests only whether the simulation
/// space is World. A World system with the module not in world space takes
/// the first three columns of the local-to-world; a system in another space
/// with the module in world space takes the first three columns of the
/// world-to-local, each times its axis of the emitter scale; a World system
/// with the module in world space takes the unit columns times the emitter
/// scale; any other system takes the unit columns. The local-to-world is the
/// owner the root update reads, and the world-to-local is the engine's general
/// 3D inverse of those words (all zero below its determinant threshold, which
/// the engine keeps using).
pub(super) fn transformation_columns(system: &super::Runtime, in_world_space: bool, owner: &GlobalTransform)
    -> Result<[[f32; 3]; 3], &'static str> {
    let world = system.emitter.simulation_space == SimulationSpace::World;
    if !world && in_world_space && system.emitter.simulation_space != SimulationSpace::Local {
        return Err(CUSTOM_WORLD_MODULE);
    }
    let local_to_world = super::birth::source_owner_matrix(owner);
    Ok(columns(world, in_world_space, &local_to_world, emitter_scale(system, owner)))
}

/// [`transformation_columns`] from source-axis owner words.
fn columns(world: bool, in_world_space: bool, local_to_world: &[f32; 16], emitter_scale: [f32; 3])
    -> [[f32; 3]; 3] {
    let column = |m: &[f32; 16], c: usize| [m[4 * c], m[4 * c + 1], m[4 * c + 2]];
    match (world, in_world_space) {
        (true, false) => std::array::from_fn(|c| column(local_to_world, c)),
        (false, true) => {
            let (world_to_local, _) = moly_law::particle::owner::invert_general_3d(local_to_world);
            std::array::from_fn(|c| column(&world_to_local, c).map(|v| v * emitter_scale[c]))
        }
        (true, true) => std::array::from_fn(|c| UNIT_COLUMNS[c].map(|v| v * emitter_scale[c])),
        (false, false) => UNIT_COLUMNS,
    }
}

/// The emitter scale the owner update stores for GetTransformationMatrix: the
/// node's own local scale in Local scaling and one in Shape scaling. In
/// Hierarchy scaling it is the lossy global scale of the node's chain, which
/// the root update does not compose; it reads its owner's decomposed scale,
/// the scale the renderer applies in that mode, as it does for the legacy
/// billboard, which carries no scaling mode.
fn emitter_scale(system: &super::Runtime, owner: &GlobalTransform) -> [f32; 3] {
    use crate::particle_geometry::Scaling;
    match system.geometry.shape_evidence().map(|evidence| evidence.scaling) {
        Some(Scaling::Local { scale, .. }) => scale.to_array(),
        Some(Scaling::Shape { .. }) => [1.0; 3],
        Some(Scaling::Hierarchy) | None => owner.to_scale_rotation_translation().0.to_array(),
    }
}

/// `M * v` as both modules add it, axis `a`:
/// `v.x * c0[a] + (v.y * c1[a] + v.z * c2[a])` over source-axis columns of
/// [`transformation_columns`], reflected once into runtime axes. Module values
/// are source vectors; directions are never normalized.
pub(super) fn module_vector(value: [f32; 3], columns: &[[f32; 3]; 3]) -> Vec3 {
    let [x, y, z] = value;
    let c = columns;
    crate::particle_geometry::reflect(Vec3::from_array(
        std::array::from_fn(|a| x * c[0][a] + (y * c[1][a] + z * c[2][a]))))
}

/// VelocityOverLifetime's animated velocity of one particle and its speed
/// modifier: the linear vector through `linear_columns` (the module's
/// [`transformation_columns`]) plus the orbital velocity.
pub(super) fn velocity_at_age(
    params: &VelocityOverLifetime,
    particle: &Particle,
    side: &Side,
    batch_seed: u32,
    simulation: SimulationSpace,
    owner: &GlobalTransform,
    linear_columns: &[[f32; 3]; 3],
    age_percent: f32,
    dt: f32,
    newborn: bool,
) -> ([f32; 3], f32) {
    let reflect = crate::particle_geometry::reflect;
    let affine = owner.affine();
    let world = simulation == SimulationSpace::World;
    let sampled = params.sample(side.seed, batch_seed, age_percent);
    let linear = module_vector(sampled.linear, linear_columns);
    let modifier = sampled.speed_modifier;
    let position = Vec3::from_array(particle.position);
    let local = if world { affine.inverse().transform_point3(position) } else { position };
    // Orbital motion deliberately does not branch on params.in_world_space.
    let orbit = sampled.orbital;
    let delta = reflect(Vec3::from_array(orbit.displacement(reflect(local).to_array(), dt, modifier)));
    let delta = if world { affine.transform_vector3(delta) } else { delta };
    let orbital = Vec3::from_array(animated_velocity(delta.to_array(), dt, modifier));
    let orbital = if newborn && super::child::arms::on("orbitalSkipsBirths") { Vec3::ZERO } else { orbital };
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
            emit_carry: [0.0; 2], animated: [0.0; 3], current_size: 0.0, axis: [0.0, 0.0, 1.0] }
    }

    #[test]
    fn source_snow_size_keeps_native_age_percent_for_draw() {
        // Current JP client snow, first normal frame after 12s prewarm,
        // particle 66. Native's effective-size array holds 0x3b3840b0; the
        // old age round trip produced 0x3b38418a. Keep the authored Size curve.
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
                max: Curve { keys: keys.to_vec(), multiplier: 1.0, pre_wrap: Some(2), post_wrap: Some(2) },
            },
            y: None, z: None,
        };
        let mut system = super::super::test_support::runtime();
        system.size_law = Some(moly_law::particle::size::SizeOverLifetime::from_params(&params).unwrap());
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
        let local_to_world = super::super::birth::source_owner_matrix(&owner);
        for simulation in [SimulationSpace::Local, SimulationSpace::World] {
            let space = |in_world_space| columns(simulation == SimulationSpace::World, in_world_space, &local_to_world, [1.0; 3]);
            params.in_world_space = false;
            let local = velocity_at_age(&VelocityOverLifetime::from_params(&params).unwrap(), &particle, &side(), 17, simulation, &owner, &space(false), 0.0, 0.01, false);
            params.in_world_space = true;
            let world = velocity_at_age(&VelocityOverLifetime::from_params(&params).unwrap(), &particle, &side(), 17, simulation, &owner, &space(true), 0.0, 0.01, false);
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
        let (linear, modifier) = velocity_at_age(&VelocityOverLifetime::from_params(&params).unwrap(), &particle, &side(), 17, SimulationSpace::Local,
            &GlobalTransform::IDENTITY, &UNIT_COLUMNS, 0.0, 0.01, false);
        assert_eq!(linear, [-1.0, 2.0, 3.0]);
        assert_eq!(modifier, 1.0);
    }
}
