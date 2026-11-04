//! From the animated skeleton to the parts that fall: one rigid part per bone
//! with its collider, its weight and the joint that holds it to the part
//! above it.
//!
//! The description is worked out from where the bones stand in the world, not
//! from the rig's own numbers. A character may be placed at any uniform scale,
//! and its bones' world positions already carry that scale, so a limb's length,
//! its thickness, the torso's size and the head's radius all come out in the
//! metres the character really occupies. Nothing here knows a metre figure
//! beyond a share of a length it measured.
//!
//! # Shapes
//!
//! - A limb is a capsule from the bone to the joint that continues it (the
//!   upper arm from its bone to the elbow, the forearm on towards the wrist),
//!   with a thickness in proportion to the limb's length, so arms are never
//!   thinner than a share of their own length and legs are thicker by a fixed
//!   factor.
//! - The torso is a box around the chest bone that spans the shoulders and
//!   reaches down towards the pelvis and up to the base of the neck.
//! - The head is a sphere sized from the distance between the head bone and
//!   the top of the model rather than from any bone length: an SD character's
//!   head is large and its bones are short.
//! - The shoulders are small spheres where the arms hold, between the chest
//!   and the upper arms.
//!
//! # Joints
//!
//! Every part hangs from the part its bone hangs from. Spherical joints hold
//! the parts together at a point and limit how far one swings and twists
//! against the other; hinge joints turn about the limb's side axis and swing
//! elbows and knees from the pose the parts fall in. An optional spine part
//! sits between pelvis and chest.
//!
//! # Weight
//!
//! Every shape is weighed as if the body were mostly water (a thousand
//! kilograms per cubic metre of shape), so the parts' weights follow the volume
//! the character occupies at whatever scale it stands at.

use std::collections::BTreeMap;

use avian3d::prelude::Collider;
use bevy::math::Isometry3d;
use bevy::prelude::*;

/// The limits of each joint, in degrees, and what each one constrains.
pub mod joint_table {
    /// The head's swing away from the neck, in any direction.
    pub const NECK_SWING: f32 = 45.0;
    /// The head's turn about the neck.
    pub const NECK_TWIST: f32 = 60.0;
    /// The chest bending over the pelvis through the spine.
    pub const SPINE_SWING: f32 = 20.0;
    /// The chest turning against the pelvis through the spine.
    pub const SPINE_TWIST: f32 = 15.0;
    /// The chest bending over the pelvis where the skeleton has no spine part.
    pub const CHEST_SWING: f32 = 25.0;
    /// The chest turning against the pelvis where there is no spine part.
    pub const CHEST_TWIST: f32 = 20.0;
    /// The pelvis tipping on its parent bone.
    pub const PELVIS_SWING: f32 = 40.0;
    /// The pelvis half turning on its parent bone.
    pub const PELVIS_TWIST: f32 = 25.0;
    /// A shoulder levelling with the chest: the arm lifts and lowers here. The
    /// solver holds a ball joint's twist, and only its twist, to a limit, so
    /// a shoulder's limit is its swing in the chest's plane.
    pub const SHOULDER_SWING: f32 = 75.0;
    /// The upper arm's swing from the shoulder in any direction.
    pub const ARM_SWING: f32 = 85.0;
    /// The upper arm turning about its own length.
    pub const ARM_TWIST: f32 = 60.0;
    /// The upper leg's swing from the pelvis at the hip.
    pub const HIP_SWING: f32 = 75.0;
    /// The upper leg turning about its own length at the hip.
    pub const HIP_TWIST: f32 = 40.0;
    /// The forearm folding towards the upper arm at the elbow, from where it
    /// hangs.
    pub const ELBOW_FOLD: f32 = 140.0;
    /// The forearm bending the other way at the elbow.
    pub const ELBOW_BACK: f32 = 5.0;
    /// The lower leg folding towards the thigh at the knee.
    pub const KNEE_FOLD: f32 = 140.0;
    /// The lower leg bending the other way at the knee.
    pub const KNEE_BACK: f32 = 5.0;
}

/// What a part is, which decides its shape and the joints that hold it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The pelvis, the part the whole body hangs from.
    Pelvis,
    /// The chest, below the neck.
    Chest,
    /// The spine between chest and pelvis.
    Spine,
    /// The head.
    Head,
    /// A shoulder of the chest, from which an arm hangs.
    Shoulder,
    /// An upper arm.
    Arm,
    /// A forearm.
    Elbow,
    /// An upper leg.
    Thigh,
    /// A lower leg.
    Knee,
}

impl Role {
    /// The role as the methods' answers name it.
    pub fn word(self) -> &'static str {
        match self {
            Self::Pelvis => "pelvis",
            Self::Chest => "chest",
            Self::Spine => "spine",
            Self::Head => "head",
            Self::Shoulder => "shoulder",
            Self::Arm => "arm",
            Self::Elbow => "elbow",
            Self::Thigh => "thigh",
            Self::Knee => "knee",
        }
    }
}

/// How a part is drawn as a collider; the shape is also what its weight is
/// read from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A capsule from `from` to `to` (its end caps' centres), in the part's
    /// own frame.
    Capsule { radius: f32, from: Vec3, to: Vec3 },
    Box { half: Vec3 },
    Sphere { radius: f32 },
}

impl Shape {
    /// The collider the part falls with.
    pub fn collider(&self) -> Collider {
        match *self {
            Shape::Capsule { radius, from, to } => Collider::capsule_endpoints(radius, from, to),
            Shape::Box { half } => Collider::cuboid(half.x * 2.0, half.y * 2.0, half.z * 2.0),
            Shape::Sphere { radius } => Collider::sphere(radius),
        }
    }

    /// How far the shape reaches from its middle of mass: the half of its
    /// segment (zero but for a capsule) and the radius around it.
    pub fn reach(&self) -> (Vec3, f32) {
        match *self {
            Shape::Capsule { radius, from, to } => ((to - from) * 0.5, radius),
            Shape::Box { half } => (Vec3::ZERO, half.length()),
            Shape::Sphere { radius } => (Vec3::ZERO, radius),
        }
    }

    /// The weight of the part, from its volume at a body's density.
    pub fn weight(&self) -> f32 {
        const DENSITY: f32 = 1000.0;
        match *self {
            Shape::Capsule { radius, from, to } => {
                let length = from.distance(to);
                DENSITY * (std::f32::consts::PI * radius * radius * length + 4.0 * std::f32::consts::PI * radius.powi(3) / 3.0)
            }
            Shape::Box { half } => DENSITY * 8.0 * half.x * half.y * half.z,
            Shape::Sphere { radius } => DENSITY * 4.0 * std::f32::consts::PI * radius.powi(3) / 3.0,
        }
    }
}

/// How far a joint lets its part turn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Limits {
    /// In any direction: up to `swing` degrees away from the part above, in
    /// any direction round it, and up to `twist` degrees about that part's
    /// own direction. The swing is limited by the solver's swing limit and by
    /// the twist limit alone, so the two limits are both set to the swing
    /// unless the joint's limits name a narrower turn.
    Spherical { swing: f32, twist: f32 },
    /// About one axis only: from `min` to `max` degrees against the pose the
    /// parts fall in.
    Hinge { min: f32, max: f32 },
}

/// One bone as it stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub entity: Entity,
    pub parent: Option<Entity>,
    pub position: Vec3,
    pub rotation: Quat,
    /// The metres per second it was moving at, from where it stood last frame.
    pub velocity: Vec3,
}

/// Where the parts of a character are described from: its bones as they stand
/// this frame, and the crown of its head.
#[derive(Clone, Debug, Default)]
pub struct Skeleton {
    /// The bones the parts are described from, by name.
    pub placed: BTreeMap<String, Placed>,
    /// The spine bones between the pelvis and the chest, from the pelvis up.
    pub spine: Vec<String>,
    /// The head bone.
    pub head: String,
    /// The arms and legs, each from its upper bone.
    pub limbs: Vec<Limb>,
    /// The crown of the head's bone: the topmost point of the model.
    pub head_top: Vec3,
}

/// One limb, from its upper bone to the joint that continues it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limb {
    /// The upper bone (the upper arm or the upper leg).
    pub proximal: String,
    /// The joint bone (the forearm or the lower leg) the upper bone reaches.
    pub distal: String,
    /// Whether this is a leg.
    pub leg: bool,
}

impl Skeleton {
    /// A bone as it stands.
    pub fn bone(&self, name: &str) -> Option<Placed> {
        self.placed.get(name).copied()
    }

    /// The chest bone: the highest of the spine bones the skeleton has, and the
    /// hips bone themselves when the skeleton has no spine bone at all.
    fn chest(&self) -> Option<(&str, Placed)> {
        self.spine.iter().rev().find_map(|name| self.bone(name).map(|placed| (name.as_str(), placed))).or_else(|| self.bone("Hips").map(|hips| ("Hips", hips)))
    }
}

/// One part: the bone it follows and where it falls.
#[derive(Clone, Debug, PartialEq)]
pub struct PartPlan {
    /// The bone the part follows; a shoulder's is the upper arm's bone, which
    /// it shares by being listed separately below.
    pub bone: String,
    /// The entity of that bone.
    pub entity: Entity,
    pub role: Role,
    /// Where the part's frame lies in the world.
    pub position: Vec3,
    /// How the part is turned in the world.
    pub rotation: Quat,
    pub shape: Shape,
    /// The way the part was going, in metres per second, when it was dropped.
    pub velocity: Vec3,
    /// Another part this one never collides with although no joint joins
    /// them: a shoulder shares its arm's bone and keeps clear of the chest.
    pub apart: Option<usize>,
}

/// One joint: the part it is held by, the part it holds, and where it turns.
#[derive(Clone, Debug, PartialEq)]
pub struct JointPlan {
    /// Index of the part the joint is held by.
    pub parent: usize,
    /// Index of the part the joint holds.
    pub child: usize,
    pub limits: Limits,
    /// The point the joint turns about, where the parts fall.
    pub anchor: Vec3,
    /// The direction of the part above, where the parts fall.
    pub axis: Vec3,
    /// The side axis a hinge turns about, where the parts fall.
    pub hinge: Vec3,
}

impl JointPlan {
    /// The joint's frame on each part: the anchor, and a basis that carries
    /// the joint's direction (a hinge's Y, a ball joint's -Z) and a hinge's
    /// side axis (its Z), the same one worked out from the joint on each part, so the two meet where the joint turns about and
    /// the angles start at the pose the parts fall in.
    pub fn frames(&self, parts: &[PartPlan]) -> (Isometry3d, Isometry3d) {
        let frame = |part: &PartPlan| {
            let inverse = part.rotation.inverse();
            let basis = match self.limits {
                // The limb's direction as Y and the hinge as Z: a revolute
                // joint turns about its frame's Z.
                Limits::Hinge { .. } => {
                    let x = self.axis.cross(self.hinge).normalize_or(Vec3::X);
                    Quat::from_mat3(&Mat3::from_cols(x, self.axis, self.hinge))
                }
                // The joint's own direction as the frame's -Z. The physics
                // keeps the frame's Y as its twist axis and lays the swing
                // cone, and the twist measured within it, about an axis
                // across that one: the frame's -Z for a twist axis of Y. The
                // cone then opens round the limb and the twist turns about it.
                Limits::Spherical { .. } => Quat::from_rotation_arc(Vec3::NEG_Z, self.axis),
            };
            Isometry3d::new(inverse * (self.anchor - part.position), inverse * basis)
        };
        (frame(&parts[self.parent]), frame(&parts[self.child]))
    }

    /// The limits, for the status answer.
    pub fn limits_json(&self) -> serde_json::Value {
        match self.limits {
            Limits::Spherical { swing, twist } => serde_json::json!({ "kind": "spherical", "swing": swing, "twist": twist }),
            Limits::Hinge { min, max } => serde_json::json!({ "kind": "hinge", "min": min, "max": max }),
        }
    }
}

/// Why a skeleton cannot be fallen with, as the methods name the refusal.
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal(pub String);

/// How much of a limb's measured length reaches the joint at its far end: the
/// gap the bones' own frames leave is not the limb.
const REACH: f32 = 0.9;
/// A limb capsule's radius as a share of its length (an arm); a leg is thicker
/// by the same factor.
const SLIM: f32 = 0.07;
const LEG_THICKNESS: f32 = 1.7;
/// The forearm's and the lower leg's radius against the upper limb's.
const LOWER_THINNER: f32 = 0.85;
/// The head sphere's radius as a share of half the skull's height, from the
/// head bone at its base to the crown: the skull is about as wide as it is
/// tall.
const HEAD_RADIUS: f32 = 1.0;
/// The base of the neck as a share of the head bone's height over the chest.
const NECK: f32 = 0.8;
/// The chest box's half extents, as shares of the span from the pelvis to the
/// base of the neck: across, up and through. The chest is never thinner across
/// than the pelvis it rests on where there is no spine part between them.
const CHEST_ACROSS: f32 = 0.6;
const CHEST_UP: f32 = 0.5;
const CHEST_THROUGH: f32 = 0.28;
/// The shoulder sphere's radius as a share of the shoulder's distance from
/// the body's middle line.
const SHOULDER: f32 = 0.6;
/// The pelvis box's half extents, as shares of the hips bone's height over
/// the chest.
const PELVIS_ACROSS: f32 = 0.6;
const PELVIS_UP: f32 = 0.5;
const PELVIS_THROUGH: f32 = 0.3;
/// A spine capsule's radius as a share of its length.
const SPINE: f32 = 0.45;
/// The least a part measures, as a share of the body's height, so that a bone
/// standing on its neighbour still makes a part.
const LEAST_REACH: f32 = 0.05;

/// Lays out a skeleton as a ragdoll where it stands: its parts and the joints
/// between them.
pub fn describe(skeleton: &Skeleton) -> Result<(Vec<PartPlan>, Vec<JointPlan>), Refusal> {
    let height = height_of(skeleton);
    if !(height.is_finite() && height > 0.0) {
        let lowest = skeleton.placed.values().map(|placed| placed.position.y).fold(f32::INFINITY, f32::min);
        return Err(Refusal(format!(
            "its bones stand all in one spot, so it has no height (crown at {:.3} m, lowest of {} bones at {lowest:.3} m)",
            skeleton.head_top.y,
            skeleton.placed.len()
        )));
    }
    let least = LEAST_REACH * height;
    let bone = |name: &str| skeleton.bone(name).ok_or_else(|| Refusal(format!("the skeleton has no {name} bone")));
    let hips = bone("Hips")?;
    let (chest_name, chest_at) = skeleton.chest().ok_or_else(|| Refusal("the skeleton has no chest bone".into()))?;
    let head_at = bone(&skeleton.head)?;

    let mut parts: Vec<PartPlan> = Vec::new();
    let mut joints: Vec<JointPlan> = Vec::new();
    // The body's own frame where it stands: its left as X, up as Y and its
    // front as Z, whichever way the character faces.
    let facing = body_frame(skeleton);
    let left = facing * Vec3::X;

    // The pelvis: a level box from the hips halfway to the chest.
    let rise = (chest_at.position.y - hips.position.y).abs().max(least);
    let pelvis = parts.len();
    parts.push(PartPlan {
        bone: "Hips".to_owned(),
        entity: hips.entity,
        role: Role::Pelvis,
        position: hips.position,
        rotation: facing,
        shape: Shape::Box { half: Vec3::new(PELVIS_ACROSS, PELVIS_UP, PELVIS_THROUGH) * rise },
        velocity: hips.velocity,
        apart: None,
    });
    // The chest: a level box from the top of the pelvis to the base of the
    // neck, as wide as the shoulders.
    let neck = ((head_at.position - chest_at.position).length() * NECK).max(least * 0.5);
    let above = (chest_at.position.y - hips.position.y).abs().max(least * 0.5);
    let shoulders: Vec<(&Limb, Placed)> = skeleton
        .limbs
        .iter()
        .filter(|limb| !limb.leg)
        .filter_map(|limb| skeleton.bone(&limb.proximal).map(|placed| (limb, placed)))
        .collect();
    let widest = shoulders.iter().map(|(_, placed)| (placed.position - chest_at.position).dot(left).abs()).fold(least * 0.25, f32::max);
    let span = (above + neck) * CHEST_UP * 0.5;
    // The box is centred on the chest bone, and never lower than resting on
    // the pelvis: where there is no spine bone the chest is read from the hips
    // bone, and the box then sits on top of the pelvis box.
    let pelvis_top = hips.position.y + PELVIS_UP * rise;
    let middle = chest_at.position.y.max(pelvis_top + span);
    // Without a spine bone the chest is described from the hips bone, which it
    // shares with the pelvis. The pelvis remains the part the body hangs from.
    let chest = parts.len();
    parts.push(PartPlan {
        bone: chest_name.to_owned(),
        entity: chest_at.entity,
        role: Role::Chest,
        position: Vec3::new(chest_at.position.x, middle, chest_at.position.z),
        rotation: facing,
        shape: Shape::Box { half: Vec3::new((widest + least * 0.25).max(span * CHEST_ACROSS), span, (above + neck) * CHEST_THROUGH) },
        velocity: chest_at.velocity,
        apart: None,
    });

    // The spine between pelvis and chest, when the skeleton has one.
    let spine_bone = skeleton.spine.first().and_then(|name| skeleton.bone(name).map(|placed| (name.clone(), placed)));
    match &spine_bone {
        Some((name, placed)) => {
            let spine = parts.len();
            let length = (chest_at.position - placed.position).length().max(least * 0.5);
            parts.push(PartPlan {
                bone: name.clone(),
                entity: placed.entity,
                role: Role::Spine,
                position: placed.position,
                rotation: placed.rotation,
                // Towards the chest bone, in the spine bone's own frame.
                shape: Shape::Capsule { radius: SPINE * length * 0.5, from: Vec3::ZERO, to: placed.rotation.inverse() * (chest_at.position - placed.position).normalize_or(Vec3::Y) * length },
                velocity: placed.velocity,
                apart: None,
            });
            joints.push(JointPlan {
                parent: pelvis,
                child: spine,
                limits: Limits::Spherical { swing: joint_table::SPINE_SWING, twist: joint_table::SPINE_TWIST },
                anchor: Vec3::new(hips.position.x, placed.position.y, hips.position.z),
                axis: Vec3::Y,
                hinge: Vec3::X,
            });
            joints.push(JointPlan {
                parent: spine,
                child: chest,
                limits: Limits::Spherical { swing: joint_table::SPINE_SWING, twist: joint_table::SPINE_TWIST },
                anchor: chest_at.position,
                axis: Vec3::Y,
                hinge: Vec3::X,
            });
        }
        None => joints.push(JointPlan {
            parent: pelvis,
            child: chest,
            limits: Limits::Spherical { swing: joint_table::CHEST_SWING, twist: joint_table::CHEST_TWIST },
            anchor: Vec3::new(hips.position.x, pelvis_top, hips.position.z),
            axis: Vec3::Y,
            hinge: Vec3::X,
        }),
    }

    // The head: a sphere the size of the skull, from the head bone at its
    // base up to the crown, held at the neck.
    let crown = (skeleton.head_top - head_at.position).length().max(least);
    let skull = head_at.position + (skeleton.head_top - head_at.position).normalize_or(Vec3::Y) * crown * 0.5;
    let head = parts.len();
    parts.push(PartPlan {
        bone: skeleton.head.clone(),
        entity: head_at.entity,
        role: Role::Head,
        position: skull,
        rotation: head_at.rotation,
        shape: Shape::Sphere { radius: crown * 0.5 * HEAD_RADIUS },
        velocity: head_at.velocity,
        apart: None,
    });
    joints.push(JointPlan {
        parent: chest,
        child: head,
        limits: Limits::Spherical { swing: joint_table::NECK_SWING, twist: joint_table::NECK_TWIST },
        anchor: head_at.position,
        axis: (head_at.position - chest_at.position).normalize_or(Vec3::Y),
        hinge: Vec3::X,
    });

    // The shoulders: a sphere at each arm's upper bone, halfway between the
    // chest's middle and the arm, which levels with the chest when the arm
    // lifts.
    for (limb, placed) in &shoulders {
        let out = (placed.position - chest_at.position).dot(left);
        let side = if out >= 0.0 { 1.0f32 } else { -1.0 };
        let across = out.abs().max(least * 0.5);
        let shoulder = parts.len();
        parts.push(PartPlan {
            bone: limb.proximal.clone(),
            entity: placed.entity,
            role: Role::Shoulder,
            position: placed.position - left * out,
            rotation: placed.rotation,
            shape: Shape::Sphere { radius: across * SHOULDER },
            velocity: placed.velocity,
            apart: Some(chest),
        });
        // Held by the chest, which is laid out before every arm: the joints
        // of the shoulders come before those of the arms they hold.
        joints.push(JointPlan {
            parent: chest,
            child: shoulder,
            limits: Limits::Spherical { swing: joint_table::SHOULDER_SWING, twist: joint_table::SHOULDER_SWING },
            anchor: placed.position,
            axis: Vec3::Y,
            hinge: left * side,
        });
    }

    // The arms and the legs: the upper bone reaches its joint bone, which
    // reaches on towards the hand or the foot.
    for limb in &skeleton.limbs {
        let from = bone(&limb.proximal)?;
        let elbow = bone(&limb.distal)?;
        let length = from.position.distance(elbow.position).max(least);
        let to = from.position + (elbow.position - from.position).normalize_or(Vec3::Y) * length * REACH;
        let thickness = SLIM * length * if limb.leg { LEG_THICKNESS } else { 1.0 };
        let upper = parts.len();
        parts.push(PartPlan {
            bone: limb.proximal.clone(),
            entity: from.entity,
            role: if limb.leg { Role::Thigh } else { Role::Arm },
            position: from.position,
            rotation: from.rotation,
            // Towards the joint bone, in the upper bone's own frame.
            shape: Shape::Capsule { radius: thickness, from: Vec3::ZERO, to: from.rotation.inverse() * (to - from.position) },
            velocity: from.velocity,
            apart: None,
        });
        let parent = if limb.leg {
            pelvis
        } else {
            parts.iter().position(|part| part.role == Role::Shoulder && part.bone == limb.proximal).unwrap_or(chest)
        };
        let axis = (to - from.position).normalize_or(Vec3::Y);
        let hinge = bend_axis(skeleton, limb, axis).unwrap_or_else(|| side_axis(from.rotation, axis));
        joints.push(JointPlan {
            parent,
            child: upper,
            limits: if limb.leg {
                Limits::Spherical { swing: joint_table::HIP_SWING, twist: joint_table::HIP_TWIST }
            } else {
                Limits::Spherical { swing: joint_table::ARM_SWING, twist: joint_table::ARM_TWIST }
            },
            anchor: from.position,
            axis,
            hinge,
        });
        let far = far_end(skeleton, limb).unwrap_or(elbow.position);
        let reach = elbow.position.distance(far).max(least * 0.5);
        let lower_to = elbow.position + (far - elbow.position).normalize_or(axis) * reach * REACH;
        let lower = parts.len();
        parts.push(PartPlan {
            bone: limb.distal.clone(),
            entity: elbow.entity,
            role: if limb.leg { Role::Knee } else { Role::Elbow },
            position: elbow.position,
            rotation: elbow.rotation,
            // Towards the hand or the foot, in the joint bone's own frame.
            shape: Shape::Capsule { radius: thickness * LOWER_THINNER, from: Vec3::ZERO, to: elbow.rotation.inverse() * (lower_to - elbow.position) },
            velocity: elbow.velocity,
            apart: None,
        });
        joints.push(JointPlan {
            parent: upper,
            child: lower,
            limits: if limb.leg {
                Limits::Hinge { min: -joint_table::KNEE_FOLD, max: joint_table::KNEE_BACK }
            } else {
                Limits::Hinge { min: -joint_table::ELBOW_FOLD, max: joint_table::ELBOW_BACK }
            },
            anchor: elbow.position,
            axis,
            hinge,
        });
    }

    Ok(hanging_from_first(parts, joints))
}

/// The parts in the order they hang from one another: the pelvis first and
/// every part after the part that holds it. A joint holds a part listed after
/// it, so the parts of a body can be read from its first to its last.
fn hanging_from_first(parts: Vec<PartPlan>, joints: Vec<JointPlan>) -> (Vec<PartPlan>, Vec<JointPlan>) {
    let mut held_by: Vec<Option<usize>> = vec![None; parts.len()];
    for joint in &joints {
        held_by[joint.child] = Some(joint.parent);
    }
    let mut order: Vec<usize> = Vec::with_capacity(parts.len());
    for part in 0..parts.len() {
        // The chain from the part up to the pelvis, from the pelvis down.
        let mut chain = Vec::new();
        let mut cursor = Some(part);
        while let Some(at) = cursor {
            if order.contains(&at) {
                break;
            }
            chain.push(at);
            cursor = held_by[at];
        }
        order.extend(chain.into_iter().rev());
    }
    let mut now = vec![0usize; parts.len()];
    for (place, was) in order.iter().enumerate() {
        now[*was] = place;
    }
    let mut parts: Vec<Option<PartPlan>> = parts.into_iter().map(Some).collect();
    let parts = order
        .iter()
        .filter_map(|was| parts[*was].take())
        .map(|mut part| {
            part.apart = part.apart.map(|apart| now[apart]);
            part
        })
        .collect();
    let joints = joints
        .into_iter()
        .map(|mut joint| {
            joint.parent = now[joint.parent];
            joint.child = now[joint.child];
            joint
        })
        .collect();
    (parts, joints)
}

/// The axis a limb's joint bends about, read from the body rather than from
/// the bone: a model's bones carry their own axes, which need not lie across
/// the limb. The body's left is from its right upper limb to its left one; a
/// knee folds about the body's right (the lower leg goes back) and an elbow
/// about its left (the forearm comes forward), both square to the limb. A
/// body without both upper limbs, or a limb along its left, has none.
fn bend_axis(skeleton: &Skeleton, limb: &Limb, axis: Vec3) -> Option<Vec3> {
    let (left, right) = if limb.leg { ("LeftUpLeg", "RightUpLeg") } else { ("LeftArm", "RightArm") };
    let left = skeleton.bone(left)?.position - skeleton.bone(right)?.position;
    let bend = if limb.leg { -left } else { left };
    let across = bend - axis * bend.dot(axis);
    (across.length_squared() > 1e-8).then(|| across.normalize())
}

/// The body's frame where it stands: its left (from the right arm to the left
/// one, or the right leg to the left one) laid level as X, up as Y, and its
/// front as Z.
fn body_frame(skeleton: &Skeleton) -> Quat {
    let across = |left: &str, right: &str| Some(skeleton.bone(left)?.position - skeleton.bone(right)?.position);
    let level = |span: Vec3| Vec3::new(span.x, 0.0, span.z).try_normalize();
    let left = across("LeftArm", "RightArm").and_then(level).or_else(|| across("LeftUpLeg", "RightUpLeg").and_then(level)).unwrap_or(Vec3::X);
    Quat::from_mat3(&Mat3::from_cols(left, Vec3::Y, left.cross(Vec3::Y)))
}

/// A limb's side axis where its bone stands: the bone's own Z when it lies
/// across the limb, and a side axis of the limb otherwise.
fn side_axis(rotation: Quat, axis: Vec3) -> Vec3 {
    let across = rotation * Vec3::Z;
    let side = across - axis * across.dot(axis);
    if side.length_squared() > 1e-4 {
        return side.normalize();
    }
    let world = Vec3::Y - axis * axis.y;
    if world.length_squared() > 1e-4 {
        world.normalize().cross(axis).normalize_or(Vec3::X)
    } else {
        Vec3::X
    }
}

/// The bone past a limb's joint: the hand after the forearm, the foot after
/// the lower leg.
fn far_end(skeleton: &Skeleton, limb: &Limb) -> Option<Vec3> {
    let (left, right) = ("LeftHand", "RightHand");
    let (left_leg, right_leg) = ("LeftFoot", "RightFoot");
    let (first, second) = if limb.leg { (left_leg, right_leg) } else { (left, right) };
    let same_side = if limb.distal.starts_with("Left") { first } else { second };
    skeleton.bone(same_side).map(|placed| placed.position)
}

/// The body's height: from the lowest bone to the crown.
pub fn height_of(skeleton: &Skeleton) -> f32 {
    let lowest = skeleton.placed.values().map(|placed| placed.position.y).fold(f32::INFINITY, f32::min);
    if !lowest.is_finite() {
        return 0.0;
    }
    (skeleton.head_top.y - lowest).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(skeleton: &mut Skeleton, name: &str, position: Vec3) -> Entity {
        let entity = Entity::from_raw_u32(skeleton.placed.len() as u32 + 1).expect("an index fits");
        skeleton.placed.insert(name.to_owned(), Placed { entity, parent: None, position, rotation: Quat::IDENTITY, velocity: Vec3::ZERO });
        entity
    }

    /// A humanoid bone chain about a metre and a quarter tall with hanging
    /// arms and standing legs, every length multiplied by `scale`.
    fn chain(scale: f32) -> Skeleton {
        let mut skeleton = Skeleton::default();
        let at = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * scale;
        entity(&mut skeleton, "Hips", at(0.0, 0.5, 0.0));
        entity(&mut skeleton, "Spine", at(0.0, 0.62, 0.0));
        entity(&mut skeleton, "Spine1", at(0.0, 0.75, 0.0));
        entity(&mut skeleton, "Neck", at(0.0, 0.85, 0.0));
        entity(&mut skeleton, "Head", at(0.0, 0.95, 0.0));
        entity(&mut skeleton, "LeftArm", at(0.15, 0.8, 0.0));
        entity(&mut skeleton, "LeftForeArm", at(0.25, 0.6, 0.0));
        entity(&mut skeleton, "LeftHand", at(0.3, 0.45, 0.0));
        entity(&mut skeleton, "RightArm", at(-0.15, 0.8, 0.0));
        entity(&mut skeleton, "RightForeArm", at(-0.25, 0.6, 0.0));
        entity(&mut skeleton, "RightHand", at(-0.3, 0.45, 0.0));
        entity(&mut skeleton, "LeftUpLeg", at(0.1, 0.45, 0.0));
        entity(&mut skeleton, "LeftLeg", at(0.1, 0.25, 0.0));
        entity(&mut skeleton, "LeftFoot", at(0.1, 0.02, 0.05));
        entity(&mut skeleton, "RightUpLeg", at(-0.1, 0.45, 0.0));
        entity(&mut skeleton, "RightLeg", at(-0.1, 0.25, 0.0));
        entity(&mut skeleton, "RightFoot", at(-0.1, 0.02, 0.05));
        skeleton.spine = vec!["Spine".to_owned(), "Spine1".to_owned()];
        skeleton.head = "Head".to_owned();
        let limb = |proximal: &str, distal: &str, leg: bool| Limb { proximal: proximal.to_owned(), distal: distal.to_owned(), leg };
        skeleton.limbs = vec![
            limb("LeftArm", "LeftForeArm", false),
            limb("RightArm", "RightForeArm", false),
            limb("LeftUpLeg", "LeftLeg", true),
            limb("RightUpLeg", "RightLeg", true),
        ];
        skeleton.head_top = at(0.0, 1.25, 0.0);
        skeleton
    }

    #[test]
    fn a_knee_folds_back_and_an_elbow_forward_however_the_bones_are_turned() {
        let mut skeleton = chain(1.0);
        for (index, placed) in skeleton.placed.values_mut().enumerate() {
            placed.rotation = Quat::from_euler(EulerRot::YXZ, 0.7 * index as f32, 1.3, -0.4 * index as f32);
        }
        let (parts, joints) = describe(&skeleton).expect("a chain describes");
        // The chain faces +z (its feet point there) with its left at +x.
        for joint in &joints {
            let Limits::Hinge { .. } = joint.limits else { continue };
            // A fold is a turn the negative way about the hinge, from the
            // limb hanging or standing straight down.
            let folded = Quat::from_axis_angle(joint.hinge, -0.5) * Vec3::NEG_Y;
            match parts[joint.child].role {
                Role::Knee => assert!(folded.z < -0.4, "a knee folds to {folded}"),
                Role::Elbow => assert!(folded.z > 0.4, "an elbow folds to {folded}"),
                role => panic!("a hinge holds a {role:?}"),
            }
        }
    }

    #[test]
    fn a_limb_reaches_towards_its_next_bone_however_its_bone_is_turned() {
        let mut skeleton = chain(1.0);
        // Bones turned every which way, as a model's own bones are.
        for (index, placed) in skeleton.placed.values_mut().enumerate() {
            placed.rotation = Quat::from_euler(EulerRot::YXZ, 0.7 * index as f32, 1.3, -0.4 * index as f32);
        }
        let (parts, _) = describe(&skeleton).expect("a chain describes");
        let towards = |from: &str, to: &str| (skeleton.placed[to].position - skeleton.placed[from].position).normalize();
        for (bone, next) in [("Spine", "Spine1"), ("LeftArm", "LeftForeArm"), ("RightUpLeg", "RightLeg"), ("LeftForeArm", "LeftHand"), ("RightLeg", "RightFoot")] {
            let part = parts.iter().find(|part| part.bone == bone && matches!(part.shape, Shape::Capsule { .. })).unwrap_or_else(|| panic!("{bone} has a capsule"));
            let Shape::Capsule { from, to, .. } = part.shape else { unreachable!() };
            let reach = (part.rotation * (to - from)).normalize();
            assert!(reach.dot(towards(bone, next)) > 0.999, "{bone} reaches {reach} in the world, not towards {next}");
        }
    }

    fn extent(shape: &Shape) -> f32 {
        match *shape {
            Shape::Capsule { radius, from, to } => radius + from.distance(to),
            Shape::Box { half } => half.length(),
            Shape::Sphere { radius } => radius,
        }
    }

    #[test]
    fn every_bone_gets_one_part_and_each_part_hangs_from_one_above_it() {
        let (parts, joints) = describe(&chain(1.0)).unwrap();
        let roles: Vec<Role> = parts.iter().map(|part| part.role).collect();
        for role in [Role::Pelvis, Role::Chest, Role::Head, Role::Shoulder, Role::Arm, Role::Elbow, Role::Thigh, Role::Knee] {
            assert!(roles.contains(&role), "{role:?} has no part");
        }
        assert_eq!(roles.iter().filter(|role| **role == Role::Elbow).count(), 2);
        assert_eq!(roles.iter().filter(|role| **role == Role::Knee).count(), 2);
        let mut held = vec![0usize; parts.len()];
        for joint in &joints {
            held[joint.child] += 1;
            // The body hangs from its pelvis: every joint holds a part on from
            // a part nearer the pelvis, and the parts are listed that way.
            assert!(joint.parent < joint.child, "{:?} is held by a part listed after it", parts[joint.child].role);
        }
        assert_eq!(parts[0].role, Role::Pelvis, "the body hangs from the pelvis");
        assert_eq!(held[0], 0, "the pelvis is held by nothing");
        for (part, held) in parts.iter().zip(&held).skip(1) {
            assert_eq!(*held, 1, "{:?} is not held exactly once", part.role);
        }
        // The arms hang from shoulders, the legs from the pelvis.
        let arm = roles.iter().position(|role| *role == Role::Arm).unwrap();
        let joint = joints.iter().find(|joint| joint.child == arm).unwrap();
        assert_eq!(parts[joint.parent].role, Role::Shoulder);
        let thigh = roles.iter().position(|role| *role == Role::Thigh).unwrap();
        let joint = joints.iter().find(|joint| joint.child == thigh).unwrap();
        assert_eq!(parts[joint.parent].role, Role::Pelvis);
    }

    #[test]
    fn a_giant_character_gets_a_giant_ragdoll() {
        let (small, _) = describe(&chain(1.0)).unwrap();
        let (giant, _) = describe(&chain(20.0)).unwrap();
        assert_eq!(small.len(), giant.len());
        for (small, giant) in small.iter().zip(&giant) {
            let ratio = extent(&giant.shape) / extent(&small.shape);
            assert!((ratio - 20.0).abs() < 0.2, "{:?} measures at {ratio} times", small.role);
            let weight = giant.shape.weight() / small.shape.weight();
            assert!((weight / 8000.0 - 1.0).abs() < 0.1, "{:?} weighs at {weight} times", small.role);
        }
    }

    #[test]
    fn a_body_facing_any_way_is_laid_out_the_same_turned_with_it() {
        // The same chain turned about the vertical, as a character faces
        // wherever it last walked.
        let turn = Quat::from_rotation_y(2.3);
        let at = Vec3::new(3.0, 0.5, -2.0);
        let mut turned = chain(1.0);
        for placed in turned.placed.values_mut() {
            placed.position = at + turn * placed.position;
            placed.rotation = turn * placed.rotation;
        }
        turned.head_top = at + turn * turned.head_top;
        let (parts, joints) = describe(&chain(1.0)).unwrap();
        let (turned_parts, turned_joints) = describe(&turned).unwrap();
        assert_eq!(parts.len(), turned_parts.len());
        for (part, turned_part) in parts.iter().zip(&turned_parts) {
            let ((half, radius), (turned_half, turned_radius)) = (part.shape.reach(), turned_part.shape.reach());
            let same = std::mem::discriminant(&part.shape) == std::mem::discriminant(&turned_part.shape)
                && half.distance(turned_half) < 1e-4
                && (radius - turned_radius).abs() < 1e-4
                && (part.shape.weight() - turned_part.shape.weight()).abs() < part.shape.weight() * 1e-3;
            assert!(same, "{:?} {} changed its shape with the facing: {:?} against {:?}", part.role, part.bone, part.shape, turned_part.shape);
            assert!((at + turn * part.position).distance(turned_part.position) < 1e-4, "{:?} {} moved against the body", part.role, part.bone);
            assert!((turn * part.rotation).angle_between(turned_part.rotation) < 1e-3, "{:?} {} turned against the body", part.role, part.bone);
        }
        for (joint, turned_joint) in joints.iter().zip(&turned_joints) {
            assert!((at + turn * joint.anchor).distance(turned_joint.anchor) < 1e-4);
        }
    }

    #[test]
    fn the_head_is_sized_by_the_crown_not_by_a_bone() {
        let (parts, _) = describe(&chain(1.0)).unwrap();
        let head = parts.iter().find(|part| part.role == Role::Head).unwrap();
        let Shape::Sphere { radius } = head.shape else {
            panic!("the head is a sphere");
        };
        // The crown is 0.3 m above the head bone; the neck bone is 0.1 m. The
        // skull spans the 0.3 m between the head bone and the crown, so the
        // sphere sits halfway up it, reaching down to the head bone and up to
        // the crown and no further.
        assert!((radius - 0.15 * HEAD_RADIUS).abs() < 1e-5, "{radius}");
        assert!(head.position.distance(Vec3::new(0.0, 1.1, 0.0)) < 1e-5, "{}", head.position);
    }

    #[test]
    fn elbows_and_knees_hinge_and_the_rest_turn() {
        let (parts, joints) = describe(&chain(1.0)).unwrap();
        for joint in &joints {
            match (parts[joint.child].role, joint.limits) {
                (Role::Elbow | Role::Knee, Limits::Hinge { min, max }) => {
                    assert!(min < 0.0 && max > 0.0, "a hinge swings from where it hangs");
                    assert!(joint.hinge.dot(joint.axis).abs() < 1e-3, "a hinge's side axis lies across the limb");
                }
                (Role::Elbow | Role::Knee, Limits::Spherical { .. }) => panic!("an elbow or a knee hinges"),
                (_, Limits::Spherical { .. }) => {}
                (role, Limits::Hinge { .. }) => panic!("{role:?} is not a hinge"),
            }
        }
    }

    #[test]
    fn a_joint_frame_sits_on_both_parts_where_the_joint_turns() {
        let mut skeleton = chain(1.0);
        // The body falls in a pose: every bone a quarter turn about its own
        // Z, as a character twisted round its own upright.
        let turned = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
        for placed in skeleton.placed.values_mut() {
            placed.rotation = turned;
        }
        let (parts, joints) = describe(&skeleton).unwrap();
        for joint in &joints {
            let (above, below) = joint.frames(&parts);
            let on_above = Isometry3d::new(parts[joint.parent].position, parts[joint.parent].rotation) * above;
            let on_below = Isometry3d::new(parts[joint.child].position, parts[joint.child].rotation) * below;
            // The frames meet on the parts, and meet where the joint turns.
            let role = parts[joint.child].role;
            assert!(on_above.translation.distance(on_below.translation) < 1e-5, "{role:?} is held at two different points");
            assert!(Vec3::from(on_above.translation).distance(joint.anchor) < 1e-5, "{role:?} turns away from its joint");
            // The solver reads the joint's rest pose from where its two frames
            // lie at spawn (avian turns each frame by its body's rotation and
            // compares the results), so they must coincide there. Which way
            // round the frame sits on the parts is free.
            // Compared by the axes the two frames carry: the angle between two
            // nearly equal rotations is read through an arc cosine, whose
            // single precision noise is of the order of a thousandth.
            let apart = [Vec3::X, Vec3::Y, Vec3::Z].map(|axis| (on_above.rotation * axis).distance(on_below.rotation * axis)).into_iter().fold(0.0f32, f32::max);
            assert!(apart < 1e-4, "{role:?} is turned on its two parts by {apart}");
            // The frames' own axes carry the joint: a hinge lies along its
            // limb on Y and turns about its Z; a ball joint's cone opens
            // round its frame's -Z, so the limb lies along that.
            match joint.limits {
                Limits::Hinge { .. } => {
                    assert!((on_above.rotation * Vec3::Y).dot(joint.axis) > 1.0 - 1e-4, "{role:?} does not lie along its limb");
                    assert!((on_above.rotation * Vec3::Z).dot(joint.hinge).abs() > 1.0 - 1e-4, "{role:?} does not hinge about its side axis");
                }
                Limits::Spherical { .. } => {
                    assert!((on_above.rotation * Vec3::NEG_Z).dot(joint.axis) > 1.0 - 1e-4, "{role:?} does not swing round its joint's direction");
                }
            }
        }
    }

    #[test]
    fn a_parts_speed_is_its_bones_speed() {
        let mut skeleton = chain(1.0);
        skeleton.placed.get_mut("LeftArm").unwrap().velocity = Vec3::new(0.4, 0.0, 0.0);
        let (parts, _) = describe(&skeleton).unwrap();
        let arm = parts.iter().find(|part| part.bone == "LeftArm").unwrap();
        assert_eq!(arm.velocity, Vec3::new(0.4, 0.0, 0.0));
        let knee = parts.iter().find(|part| part.bone == "LeftLeg").unwrap();
        assert_eq!(knee.velocity, Vec3::ZERO);
        assert_eq!(arm.entity, skeleton.bone("LeftArm").unwrap().entity);
    }

    #[test]
    fn a_missing_bone_is_refused_by_its_name() {
        let mut skeleton = chain(1.0);
        skeleton.placed.remove("LeftArm");
        assert_eq!(describe(&skeleton).unwrap_err().0, "the skeleton has no LeftArm bone");
        let mut skeleton = chain(1.0);
        skeleton.placed.remove("Head");
        assert_eq!(describe(&skeleton).unwrap_err().0, "the skeleton has no Head bone");
        let mut skeleton = chain(1.0);
        skeleton.placed.clear();
        assert!(describe(&skeleton).unwrap_err().0.contains("no height"));
    }

    #[test]
    fn a_skeleton_without_a_spine_part_holds_the_chest_to_the_pelvis() {
        let mut skeleton = chain(1.0);
        skeleton.spine.clear();
        let (parts, joints) = describe(&skeleton).unwrap();
        assert!(!parts.iter().any(|part| part.role == Role::Spine));
        let waist = joints.iter().find(|joint| parts[joint.child].role == Role::Chest).unwrap();
        assert_eq!(parts[waist.parent].role, Role::Pelvis);
        assert_eq!(waist.limits, Limits::Spherical { swing: joint_table::CHEST_SWING, twist: joint_table::CHEST_TWIST });
        // The hips bone holds the pelvis, and the chest sits on top of it.
        let pelvis = &parts[waist.parent];
        assert_eq!(pelvis.bone, "Hips");
        assert_eq!(parts[waist.child].bone, "Hips");
        let half_height = |part: &PartPlan| match part.shape {
            Shape::Box { half } => half.y,
            _ => panic!("a torso part is a box"),
        };
        let pelvis_top = pelvis.position.y + half_height(pelvis);
        let chest = &parts[waist.child];
        assert!((chest.position.y - half_height(chest) - pelvis_top).abs() < 1e-5, "the chest rests on the pelvis");
        assert!((waist.anchor.y - pelvis_top).abs() < 1e-5, "the joint turns where the two boxes meet");
    }

    #[test]
    fn the_height_reaches_from_the_lowest_bone_to_the_crown() {
        let skeleton = chain(3.0);
        // The feet stand at 0.02 m times three, the crown at 1.25 m times
        // three; the deeper z of the feet changes no height.
        assert!((height_of(&skeleton) - (1.25 - 0.02) * 3.0).abs() < 1e-4);
    }
}
