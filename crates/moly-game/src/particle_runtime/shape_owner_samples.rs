//! Current native observations plus the real scalar birth caller's owner split.
//!
//! These tests qualify only the final owner operation. The legacy scalar
//! `spawn_one` still does not reproduce the native four-wide Shape RNG, native
//! quaternion construction or both EmitterStoreData normalization steps.
use super::*;
use moly_law::particle::{
    schema::{ShapeControls, ShapeParams},
    MinMaxCurve,
};

// shape-birth-current.json, transformRows[0] and transformRows[3], shapeExit.
// Current JP libunity SHA256:
// 937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9
// EmitterStoreData (0xf08e90, 2856 bytes) SHA256:
// af18a68a808ae9c82c63e1788f9c48e03d73dcddf224e3a955c46e5c5aaba134
// Local data is captured after both source normalization operations. World
// data is independently captured after the outer owner multiply. All eight
// storage lanes are retained, including the three padded lanes of the 5 birth
// request. The probe supplied STATE+0x150=[1,1,1] in both cases; this owner
// 3x3 must not be substituted for that distinct Shape scale input.
const LOCAL_DIRECTION_BITS: [[u32; 3]; 8] = [
    [0xbf517538, 0x3d0c48be, 0x3f12ec6b],
    [0xbe0b1ed1, 0x3e4c6120, 0x3f786d27],
    [0x3f52eb68, 0x3d824358, 0x3f102a77],
    [0xbf759fed, 0x3b86bb8c, 0xbe9044e1],
    [0xbf20277e, 0x3e7ff401, 0x3f3d3022],
    [0x3f5396f5, 0x3c9e1ce1, 0x3f1004cd],
    [0xbeec77f6, 0x3d04b6dd, 0xbf62e8cc],
    [0x3f3e4415, 0x3e5c70a8, 0x3f222a75],
];
const WORLD_DIRECTION_BITS: [[u32; 3]; 8] = [
    [0x4092ec6b, 0x3e0c48be, 0x3fd17538],
    [0x40f86d27, 0x3f4c6120, 0x3e8b1ed1],
    [0x40902a77, 0x3e824358, 0xbfd2eb68],
    [0xc01044e1, 0x3c86bb8c, 0x3ff59fed],
    [0x40bd3022, 0x3f7ff401, 0x3fa0277e],
    [0x409004cd, 0x3d9e1ce1, 0xbfd396f5],
    [0xc0e2e8cc, 0x3e04b6dd, 0x3f6c77f6],
    [0x40a22a75, 0x3f5c70a8, 0xbfbe4415],
];

fn source_owner() -> Mat4 {
    Mat4::from_cols_array(&[
        0.0, 0.0, -2.0, 0.0, 0.0, 4.0, 0.0, 0.0, 8.0, 0.0, 0.0, 0.0, 5.0, 6.0, 7.0, 1.0,
    ])
}

fn reflected_owner() -> GlobalTransform {
    let reflection = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
    GlobalTransform::from(reflection * source_owner() * reflection)
}

#[test]
fn current_native_shape_owner_outputs_match_all_eight_storage_lanes_bitwise() {
    let direct_owner = GlobalTransform::from(source_owner());
    let runtime_owner = reflected_owner();
    for (lane, (local, expected)) in LOCAL_DIRECTION_BITS
        .into_iter()
        .zip(WORLD_DIRECTION_BITS)
        .enumerate()
    {
        let input = Vec3::from_array(local.map(f32::from_bits));
        let actual = apply_world_owner_direction(&direct_owner, input);
        assert_eq!(
            actual.to_array().map(f32::to_bits),
            expected,
            "source lane {lane}"
        );
        let runtime =
            apply_world_owner_direction(&runtime_owner, crate::particle_geometry::reflect(input));
        let expected_runtime =
            crate::particle_geometry::reflect(Vec3::from_array(expected.map(f32::from_bits)));
        assert_eq!(
            runtime.to_array().map(f32::to_bits),
            expected_runtime.to_array().map(f32::to_bits),
            "reflected lane {lane}"
        );
        assert!(actual.length() > 1.0, "native owner retains its scale");
    }
}

fn context() -> Context {
    Context {
        sky: GlobalTransform::IDENTITY,
        camera: GlobalTransform::IDENTITY,
        site: GlobalTransform::IDENTITY,
    }
}

fn empty_runtime(shape: bool, simulation_space: SimulationSpace) -> Runtime {
    let mut system = test_support::runtime();
    system.pool.clear();
    system.side.clear();
    system.emitter.simulation_space = simulation_space;
    system.emitter.start.speed = MinMaxCurve::Constant(1.0);
    system.emitter.shape_enabled = Some(shape);
    system.emitter.shape = shape.then(|| ShapeParams {
        shape_type: "Hemisphere".into(),
        radius: 50.0,
        radius_thickness: 1.0,
        arc: 360.0,
        rotation: [-90.0, 0.0, 0.0],
        position: [0.0; 3],
        controls: ShapeControls {
            scale: Some([1.0, 1.0, 0.1599999964237213]),
            random_position: Some(0.0),
            ..Default::default()
        },
    });
    system.kind = EffectKind::Site;
    system
}

#[test]
fn real_spawn_one_routes_shape_through_non_normalizing_world_owner() {
    let mut local = empty_runtime(true, SimulationSpace::Local);
    let mut world = empty_runtime(true, SimulationSpace::World);
    world.node_affine = reflected_owner();
    for _ in 0..8 {
        spawn_one(&mut local, &context());
        spawn_one(&mut world, &context());
    }
    for (local, world) in local.pool.iter().zip(&world.pool) {
        let [x, y, z] = local.velocity;
        let expected = [-8.0 * z, 4.0 * y, 2.0 * x];
        assert_eq!(world.velocity.map(f32::to_bits), expected.map(f32::to_bits));
        assert!(Vec3::from_array(world.velocity).length() > 1.0);
    }
}

#[test]
fn real_spawn_one_keeps_no_shape_initial_direction_normalized() {
    let mut system = empty_runtime(false, SimulationSpace::World);
    system.node_affine = reflected_owner();
    spawn_one(&mut system, &context());
    // Initial.Start 0xd53e54 reads matrix Z; 0xd53e74..0xd53f60 performs
    // FRSQRTE plus two FRSQRTS refinements and a small-vector mask. Shape.Start
    // overrides that result only when Shape is enabled.
    assert_eq!(system.pool[0].velocity, [-1.0, 0.0, 0.0]);
}
