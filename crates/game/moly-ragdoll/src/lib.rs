//! Ragdolls for the characters on screen: a character can be dropped from the
//! animation that holds it and fall as a body of rigid parts, then get up
//! again into its standing animation.
//!
//! # What a drop does
//!
//! A drop is asked for by the `moly.ragdoll.drop` method of the control
//! surface or by a script's `npc.ragdoll` step. It takes the character from
//! its AI when no script holds it, takes its body away from the animation,
//! and builds a part for every bone it can describe ([`description`]) where
//! the bones stand. Each part starts with the speed its bone was moving at, so
//! a character walking when it is dropped keeps walking into the fall. An
//! optional push, given in metres per second per metre of the character's
//! height, is applied at the part of the bone the caller names. Until the
//! character gets up, every part's world rotation is written back on its bone
//! each frame after the animation and before the transforms are propagated, so
//! the hair chains and the skinned shadows follow the fallen pose in the same
//! frame, and a look-at leaves the fallen head alone.
//!
//! # Getting up
//!
//! When every part has stayed slower than the settings' settle speed for the
//! settle time, or when the character has fallen for its longest time, it gets
//! up: the pose blends over the recovery time from where the parts lie back
//! into the standing animation, the character is placed where its pelvis came
//! to rest on the ground and turned towards where it was facing when it fell,
//! the parts are removed, the animation is handed back, and the character goes
//! back to its AI if the fall took it from there.
//!
//! # Scale
//!
//! A character may stand in the world at any uniform scale, from a toy to a
//! giant thirty times as tall, so nothing about it is sized in fixed metres:
//! every collider and weight comes from the bones' world positions, and every
//! speed and distance the settings give is read per metre of the character's
//! own height ([`settings`]).
//!
//! # Frame order
//!
//! The physics runs in `Update` on the game's variable frame time, which
//! follows the game clock that `moly.time` pauses and scales: a paused game
//! steps no physics and ages no ragdoll. A drop goes after the frame's
//! animation systems and before its physics step; the choice to get up, and
//! the cleanup after a drop another state took away, go after the step. The
//! bone rotations and the blend of a recovery are written in `PostUpdate`
//! after the animation systems and the character look-at, before the
//! transform propagation the cloth pass reads.

mod collision;
pub mod description;
mod drop;
mod hooks;
mod recover;
mod remote;
mod rig;
pub mod settings;
mod settle;
mod writeback;

use avian3d::prelude::*;
use bevy::prelude::*;

pub use description::joint_table;
pub use drop::{drop_now, recover_now, take_away, Drop};
pub use moly_features::character::BodyOwner;
pub use settings::{RagdollSettings, KNOBS};

/// Registers the ragdoll: the physics engine on the game's frame time, the
/// systems at the points of the frame the parts need, the cleanup after a drop
/// another state takes away, and the `moly.ragdoll.*` methods of the control
/// surface.
pub fn install(app: &mut App) {
    // The step reads the game clock's own delta, at the clock's speed: a
    // paused or slowed game pauses or slows the falls with everything else.
    app.insert_resource(Time::<Physics>::default().with_relative_speed(1.0))
        .init_resource::<RagdollSettings>()
        .init_resource::<settle::Settling>()
        .init_resource::<writeback::Previous>();
    add_physics(app);
    app.add_systems(First, writeback::remember.after(bevy::time::TimeSystems))
        .add_systems(
            Update,
            (
                drop::step
                    .after(bevy::app::AnimationSystems)
                    .before(PhysicsSystems::StepSimulation),
                drop::watch.after(PhysicsSystems::StepSimulation),
                recover::begin_due,
                hooks::clean_up.after(recover::begin_due),
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            (writeback::bones, recover::blend)
                .chain()
                .after(bevy::app::AnimationSystems)
                .after(moly_features::npc_ik::HeadSolve)
                .before(bevy::transform::TransformSystems::Propagate)
                .before(moly_features::cloth_runtime::advance),
        );
    remote::install(app);
}

/// How many steps the physics takes per frame. A small character's thin
/// limbs need the finer steps to lie still on the floor and to stay joined
/// when they land; only falling bodies run in this physics, so the cost is
/// that of a few dozen bodies.
pub(crate) const SUBSTEPS: u32 = 16;

/// The physics as the falls run it: on the game's frame time, with the pair
/// filter that keeps a body's own parts apart, at [`SUBSTEPS`] steps a frame.
pub(crate) fn add_physics(app: &mut App) {
    app.add_plugins(PhysicsPlugins::new(Update).with_collision_hooks::<collision::OwnBody>()).insert_resource(SubstepCount(SUBSTEPS));
}

/// The fall requests a host installs so that a script step can make a
/// character fall where it stands.
pub fn fall_requests() -> moly_features::act::FallRequests {
    hooks::fall_requests()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::description::{describe, joint_table, Limits, Placed, Role, Skeleton};

    /// The bones of the body's middle: a limb, the head or the chest may hang
    /// from any of them, and the pelvis holds both legs.
    const MIDDLE: &[&str] = &["Hips", "Spine", "Spine1", "Neck", "Head", "LeftArm", "RightArm", "LeftUpLeg", "RightUpLeg"];

    /// A bone chain of a small character, every length multiplied by `scale`.
    pub(crate) fn chain(scale: f32) -> Skeleton {
        let mut skeleton = Skeleton::default();
        let mut next = 1u32;
        let mut put = |name: &str, x: f32, y: f32, z: f32| {
            let entity = Entity::from_raw_u32(next).expect("an index fits");
            next += 1;
            skeleton.placed.insert(name.to_owned(), Placed { entity, parent: None, position: Vec3::new(x, y, z) * scale, rotation: Quat::IDENTITY, velocity: Vec3::ZERO });
        };
        put("Hips", 0.0, 0.5, 0.0);
        put("Spine", 0.0, 0.62, 0.0);
        put("Spine1", 0.0, 0.75, 0.0);
        put("Neck", 0.0, 0.85, 0.0);
        put("Head", 0.0, 0.95, 0.0);
        put("LeftArm", 0.15, 0.8, 0.0);
        put("LeftForeArm", 0.25, 0.6, 0.0);
        put("LeftHand", 0.3, 0.45, 0.0);
        put("RightArm", -0.15, 0.8, 0.0);
        put("RightForeArm", -0.25, 0.6, 0.0);
        put("RightHand", -0.3, 0.45, 0.0);
        put("LeftUpLeg", 0.1, 0.45, 0.0);
        put("LeftLeg", 0.1, 0.25, 0.0);
        put("LeftFoot", 0.1, 0.02, 0.05);
        put("RightUpLeg", -0.1, 0.45, 0.0);
        put("RightLeg", -0.1, 0.25, 0.0);
        put("RightFoot", -0.1, 0.02, 0.05);
        skeleton.spine = vec!["Spine".to_owned(), "Spine1".to_owned()];
        skeleton.head = "Head".to_owned();
        let limb = |proximal: &str, distal: &str, leg: bool| crate::description::Limb { proximal: proximal.to_owned(), distal: distal.to_owned(), leg };
        skeleton.limbs = vec![
            limb("LeftArm", "LeftForeArm", false),
            limb("RightArm", "RightForeArm", false),
            limb("LeftUpLeg", "LeftLeg", true),
            limb("RightUpLeg", "RightLeg", true),
        ];
        skeleton.head_top = Vec3::new(0.0, 1.25, 0.0) * scale;
        skeleton
    }

    #[test]
    fn the_plugin_runs_physics_at_the_game_clock_speed() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        install(&mut app);
        assert_eq!(app.world().resource::<Time<Physics>>().relative_speed(), 1.0);
        assert_eq!(*app.world().resource::<RagdollSettings>(), RagdollSettings::default());
    }

    #[test]
    fn a_body_is_one_part_per_role_and_its_joints_reach_up() {
        let (parts, joints) = describe(&chain(1.0)).expect("a chain describes");
        let bones: Vec<&str> = parts.iter().map(|part| part.bone.as_str()).collect();
        for index in 1..parts.len() {
            let held: Vec<_> = joints.iter().filter(|joint| joint.child == index).collect();
            assert_eq!(held.len(), 1, "{:?} is held once", parts[index].role);
            let above = held[0].parent;
            // The body is held from the pelvis: a part hangs from a part
            // listed before it.
            assert!(above < index, "{:?} is held by a part listed before it", parts[index].role);
            // A part is held by the part of a bone its own hangs from.
            let bone = bones[index];
            let parent_bone = bones[above];
            let same_side = (bone.starts_with("Left") && parent_bone.starts_with("Left")) || (bone.starts_with("Right") && parent_bone.starts_with("Right"));
            assert!(same_side || MIDDLE.contains(&parent_bone), "{:?} on {bone} hangs from {parent_bone}", parts[index].role);
        }
        assert_eq!(parts[0].role, Role::Pelvis, "the body hangs from the pelvis");
        for joint in &joints {
            match (parts[joint.child].role, joint.limits) {
                (Role::Elbow | Role::Knee, Limits::Hinge { min, max }) => assert!(min < 0.0 && max > 0.0),
                (Role::Elbow | Role::Knee, Limits::Spherical { .. }) => panic!("an elbow or a knee hinges"),
                (role, Limits::Hinge { .. }) => panic!("{role:?} is not a hinge"),
                (_, Limits::Spherical { .. }) => {}
            }
        }
    }

    #[test]
    fn a_giant_falls_with_a_giant_body() {
        let (small, _) = describe(&chain(1.0)).unwrap();
        let (giant, _) = describe(&chain(30.0)).unwrap();
        let weight = |parts: &[crate::description::PartPlan]| parts.iter().map(|part| part.shape.weight()).sum::<f32>();
        // Thirty times the height is twenty-seven thousand times the weight.
        assert!((weight(&giant) / weight(&small) / 27_000.0 - 1.0).abs() < 0.05);
    }

    #[test]
    fn the_joint_table_keeps_elbows_and_knees_one_way() {
        const {
            assert!(joint_table::ELBOW_FOLD > 0.0 && joint_table::ELBOW_BACK < joint_table::ELBOW_FOLD);
            assert!(joint_table::KNEE_FOLD > 0.0 && joint_table::KNEE_BACK < joint_table::KNEE_FOLD);
        }
    }

    #[test]
    fn every_knob_starts_inside_its_range() {
        let settings = RagdollSettings::default();
        let answer = settings.to_json();
        for knob in KNOBS {
            assert!(knob.admits(knob.default), "{} starts out of range", knob.key);
            let started = answer[knob.key].as_f64().unwrap_or_else(|| panic!("{} has no value", knob.key));
            assert!((started as f32 - knob.default).abs() < f32::EPSILON, "{} answers {started} where it starts at {}", knob.key, knob.default);
        }
    }
}
