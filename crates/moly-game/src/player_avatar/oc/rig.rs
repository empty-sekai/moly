//! The retarget law that poses a humanoid from the player avatar's
//! skeleton, as pure functions over two skeletons.
//!
//! # Frames
//!
//! Both skeletons are posed in the body frame: the frame of the player
//! avatar's model root, in which the avatar stands on `y = 0` facing `+Z`
//! with its left side at `+X`. The humanoid's own model frame reaches it
//! through the fit frame (uniform scale, and a half turn about `Y` for a
//! model that faces `-Z`). Rotations compose as glam does: `a * b` applies
//! `b` first. Every scale is uniform.
//!
//! # The law
//!
//! With `W_A` the avatar joint's rest rotation and `W_B` the humanoid bone's
//! aligned rest rotation (both in the body frame), the avatar joint's current
//! rotation `G_A` gives the delta `M = G_A * W_A^-1`, and the bone is posed to
//! `G_B = M * W_B`; its local rotation is `G_B(parent)^-1 * G_B`. Every
//! driven bone therefore turns by exactly the avatar joint's body-frame
//! delta, twist included, and because the rests are aligned (each limb
//! segment of the humanoid rest points along the avatar's rest segment) the
//! bone directions of both skeletons are equal in every frame. This is the
//! model-space form of the VRM animation retarget formula
//! `q_B = L_B * W_B^-1 * (W_A * L_A^-1 * q_A * W_A^-1) * W_B`.
//!
//! Humanoid bones the avatar lacks (chest, upper chest, neck, shoulders,
//! hands, eyes, jaw, toes) keep their aligned rest local, so they follow the
//! nearest driven ancestor rigidly; fingers keep a relaxed curl
//! ([`super::humanoid::FINGER_CURL`]). The hips take the avatar's hips
//! displacement: the humanoid is fitted to the avatar's rest hips height, so
//! stride, seat and step heights and the root travel carry over unscaled.
//!
//! # Feet
//!
//! The avatar's foot joint is the tip of its leg and turns with the shin. A
//! humanoid foot that turned with the shin would pitch into the ground, and
//! a humanoid leg of other proportions would slide. So each leg is solved by
//! two-bone inverse kinematics: the ankle goes to the avatar's leg tip plus
//! the rest offset between the two, the knee bends toward the thigh's turned
//! forward axis, and the foot keeps its rest orientation turned by the
//! body's heading while the avatar's leg tip is near the ground
//! ([`contact_weight`]), blending into the shin's turn as it lifts.
//!
//! # Sockets
//!
//! The avatar's skeleton stays the animation target, and props, the camera,
//! emoticons and tools follow its joints by name. After the humanoid is
//! posed, each avatar joint's translation is rewritten so the joint sits on
//! its humanoid bone (the hand sockets on the palm, plus the clip's own prop
//! offset); its rotation stays the avatar's.

use super::humanoid::{self, Slot, SLOT_COUNT};
use bevy::math::{Quat, Vec3};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

/// A similarity transform: a point `p` maps to `pos + scale * (rot * p)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Frame {
    pub(crate) pos: Vec3,
    pub(crate) rot: Quat,
    pub(crate) scale: f32,
}

impl Frame {
    pub(crate) const IDENTITY: Frame = Frame {
        pos: Vec3::ZERO,
        rot: Quat::IDENTITY,
        scale: 1.0,
    };

    pub(crate) fn new(pos: Vec3, rot: Quat, scale: f32) -> Self {
        Self { pos, rot, scale }
    }

    /// The frame of a child whose local frame is `local`.
    pub(crate) fn then(&self, local: &Frame) -> Frame {
        Frame {
            pos: self.apply(local.pos),
            rot: (self.rot * local.rot).normalize(),
            scale: self.scale * local.scale,
        }
    }

    pub(crate) fn apply(&self, point: Vec3) -> Vec3 {
        self.pos + self.scale * (self.rot * point)
    }

    /// The point in this frame's local coordinates.
    pub(crate) fn local_point(&self, point: Vec3) -> Vec3 {
        (self.rot.inverse() * (point - self.pos)) / self.scale
    }

    pub(crate) fn inverse(&self) -> Frame {
        let rot = self.rot.inverse();
        let scale = 1.0 / self.scale;
        Frame {
            pos: -(scale * (rot * self.pos)),
            rot,
            scale,
        }
    }
}

/// Joints in parent-first order (every parent index is below its child's),
/// their local frames, and the frame their roots hang from.
#[derive(Clone, Debug)]
pub(crate) struct Skeleton {
    pub(crate) parent: Vec<Option<usize>>,
    pub(crate) local: Vec<Frame>,
    pub(crate) base: Frame,
}

impl Skeleton {
    /// Body-frame joint frames for the given locals.
    pub(crate) fn fk(&self, local: &[Frame]) -> Vec<Frame> {
        let mut frames: Vec<Frame> = Vec::with_capacity(local.len());
        for (joint, local) in local.iter().enumerate() {
            let parent = self.parent[joint].map_or(self.base, |parent| frames[parent]);
            frames.push(parent.then(local));
        }
        frames
    }

    fn parent_frame(&self, frames: &[Frame], joint: usize) -> Frame {
        self.parent[joint].map_or(self.base, |parent| frames[parent])
    }

    fn is_ancestor(&self, ancestor: usize, joint: usize) -> bool {
        let mut cursor = self.parent[joint];
        while let Some(at) = cursor {
            if at == ancestor {
                return true;
            }
            cursor = self.parent[at];
        }
        false
    }

    fn check_order(&self, what: &str) -> Result<(), String> {
        if self.parent.len() != self.local.len() {
            return Err(format!("{what}: parent and local tables differ in length"));
        }
        for (joint, parent) in self.parent.iter().enumerate() {
            if parent.is_some_and(|parent| parent >= joint) {
                return Err(format!("{what}: joint {joint} comes before its parent"));
            }
        }
        Ok(())
    }
}

/// The swing that turns direction `from` onto `to` (identity when either
/// is degenerate).
fn arc(from: Vec3, to: Vec3) -> Quat {
    match (from.try_normalize(), to.try_normalize()) {
        (Some(from), Some(to)) => Quat::from_rotation_arc(from, to),
        _ => Quat::IDENTITY,
    }
}

/// Knee and ankle for a two-bone limb: hip `hip`, segment lengths `upper`
/// and `lower`, ankle target `target`, the knee bending toward `pole`. A
/// target out of reach is clamped onto the reachable shell along its
/// direction.
pub(crate) fn two_bone(hip: Vec3, upper: f32, lower: f32, target: Vec3, pole: Vec3) -> (Vec3, Vec3) {
    const EPS: f32 = 1e-5;
    let offset = target - hip;
    let length = offset.length();
    let dir = offset.try_normalize().unwrap_or(Vec3::NEG_Y);
    let reach = length.clamp((upper - lower).abs() + EPS, upper + lower - EPS).max(EPS);
    let along = (upper * upper - lower * lower + reach * reach) / (2.0 * reach);
    let height = (upper * upper - along * along).max(0.0).sqrt();
    let side = (pole - dir * pole.dot(dir))
        .try_normalize()
        .unwrap_or_else(|| dir.any_orthonormal_vector());
    (hip + dir * along + side * height, hip + dir * reach)
}

/// How flat the humanoid foot is held: 1 while the avatar's leg tip is at
/// most `low` above its rest height, 0 from `high` up, smoothstep between.
pub(crate) fn contact_weight(height: f32, low: f32, high: f32) -> f32 {
    let t = ((height - low) / (high - low)).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// The tip height bounds of [`contact_weight`] as fractions of the avatar's
/// rest hips height. The avatar's own walk keeps its leg tips between 6 % and
/// 32 % of the hips height above the ground (its gait never touches down),
/// its run touches down, and its idle stands on the ground; the band holds
/// the foot flat through the low half of the walk's stride.
pub(crate) const CONTACT_LOW: f32 = 0.10;
pub(crate) const CONTACT_HIGH: f32 = 0.25;

/// One humanoid bone posed from an avatar joint every frame.
#[derive(Clone, Debug)]
pub(crate) struct Driven {
    /// The bone's joint index in the humanoid skeleton.
    pub(crate) joint: usize,
    pub(crate) slot: Slot,
    /// The nearest driven ancestor (index into [`Plan::driven`]).
    anchor: Option<usize>,
    /// The parent's frame relative to the anchor's frame, or to the body
    /// when there is no anchor: the joints between keep their rest.
    link: Frame,
    /// The bone's local translation (its rest).
    rest_pos: Vec3,
    /// Aligned rest rotation and scale in the body frame.
    rest_rot: Quat,
    rest_scale: f32,
}

#[derive(Clone, Debug)]
struct Leg {
    upper: usize,
    lower: usize,
    foot: usize,
    tip: Slot,
    /// The knee in the upper leg's frame, the ankle in the lower leg's.
    knee: Vec3,
    ankle: Vec3,
    /// The ankle's aligned rest position in the body frame.
    ankle_rest: Vec3,
}

#[derive(Clone, Debug)]
enum SocketTarget {
    /// A driven bone's origin.
    Driven(usize),
    /// A point fixed in a driven bone's frame (the palm in the forearm).
    Point(usize, Vec3),
}

#[derive(Clone, Debug)]
struct Socket {
    target: SocketTarget,
    /// The avatar joint's rest translation; a clip's translation away from
    /// it is the prop offset carried over.
    rest_pos: Vec3,
    /// Whether the clip's own translation is added (hand sockets only).
    carries_offset: bool,
}

/// What the plan found, for the log.
#[derive(Clone, Debug)]
pub(crate) struct PlanReport {
    pub(crate) scale: f32,
    pub(crate) turned: bool,
    pub(crate) avatar_hips: f32,
    pub(crate) model_hips: f32,
    pub(crate) aligned: usize,
    pub(crate) curled: usize,
}

/// Everything the per-frame solve needs, computed once from both rests.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    proxy_parent: Vec<Option<usize>>,
    proxy_base: Frame,
    slots: [usize; SLOT_COUNT],
    rest_a: [Frame; SLOT_COUNT],
    pub(crate) driven: Vec<Driven>,
    /// The hips' aligned rest position in the body frame.
    hips_rest: Vec3,
    legs: Vec<Leg>,
    sockets: Vec<Option<Socket>>,
    contact: (f32, f32),
    /// The humanoid's frame in the body frame.
    pub(crate) fit: Frame,
    /// Locals every humanoid bone takes once (aligned rest, finger curl);
    /// the driven ones are rewritten every frame.
    pub(crate) constants: Vec<(usize, Frame)>,
    /// Whether the legs are solved (off only to measure the bare law).
    pub(crate) leg_ik: bool,
    pub(crate) report: PlanReport,
}

/// Segments the humanoid rest is aligned along: (from bone, to bone, avatar
/// from joint, avatar to joint), parents first.
fn alignment_segments() -> Vec<(&'static str, &'static str, Slot, Slot)> {
    let mut list = vec![("spine", "head", Slot::Spine, Slot::Head)];
    for (side, upper, lower, hand, thigh, shin, foot) in [
        ("left", Slot::UpperArmL, Slot::LowerArmL, Slot::HandL, Slot::UpperLegL, Slot::LowerLegL, Slot::FootL),
        ("right", Slot::UpperArmR, Slot::LowerArmR, Slot::HandR, Slot::UpperLegR, Slot::LowerLegR, Slot::FootR),
    ] {
        let bone = |name: &str| -> &'static str {
            humanoid::canonical(&format!("{side}{name}"), false).expect("a VRM 1.0 bone name")
        };
        list.push((bone("UpperArm"), bone("LowerArm"), upper, lower));
        list.push((bone("LowerArm"), bone("Hand"), lower, hand));
        list.push((bone("Hand"), bone("MiddleProximal"), lower, hand));
        list.push((bone("UpperLeg"), bone("LowerLeg"), thigh, shin));
        list.push((bone("LowerLeg"), bone("Foot"), shin, foot));
    }
    list
}

/// Builds the plan from the avatar skeleton (`slots`: its joint per slot)
/// and the humanoid skeleton in its model frame (`bones`: humanoid bone ->
/// joint). Refuses a humanoid whose bones are missing, out of hierarchy or
/// degenerate.
pub(crate) fn plan(
    proxy: &Skeleton,
    slots: [usize; SLOT_COUNT],
    model: &Skeleton,
    bones: &BTreeMap<&'static str, usize>,
) -> Result<Plan, String> {
    proxy.check_order("avatar skeleton")?;
    model.check_order("model skeleton")?;
    for bone in humanoid::REQUIRED_BONES {
        if !bones.contains_key(bone) {
            return Err(format!("the model has no {bone} bone"));
        }
    }
    let proxy_rest = proxy.fk(&proxy.local);
    let rest_a: [Frame; SLOT_COUNT] = std::array::from_fn(|k| proxy_rest[slots[k]]);
    for slot in Slot::ALL {
        if let Some(parent) = slot.parent() {
            if !proxy.is_ancestor(slots[parent.index()], slots[slot.index()]) {
                return Err(format!(
                    "avatar joint {} is not below {}",
                    slot.avatar_name(),
                    parent.avatar_name()
                ));
            }
        }
    }
    let ground_a = rest_a[Slot::FootL.index()].pos.y.min(rest_a[Slot::FootR.index()].pos.y);
    let avatar_hips = rest_a[Slot::Hips.index()].pos.y - ground_a;
    if avatar_hips <= 1e-6 {
        return Err("the avatar's hips are not above its feet".into());
    }

    // Fit: hips height to the avatar's, facing +Z.
    let model_rest = model.fk(&model.local);
    let at = |bone: &str| model_rest[bones[bone]].pos;
    let model_hips = at("hips").y;
    if model_hips <= 1e-6 {
        return Err(format!(
            "the model's hips stand at {model_hips:.4}, not above its ground (y = 0)"
        ));
    }
    let turned = at("leftHand").x < at("rightHand").x;
    let scale = avatar_hips / model_hips;
    let fit = Frame::new(
        Vec3::ZERO,
        if turned {
            Quat::from_rotation_y(std::f32::consts::PI)
        } else {
            Quat::IDENTITY
        },
        scale,
    );
    let skeleton = Skeleton {
        base: fit,
        ..model.clone()
    };
    let file_rest = skeleton.fk(&skeleton.local);

    // Rest alignment: each segment swung onto the avatar's rest segment,
    // parents first; then each foot back to its file orientation (a T-pose
    // foot stands flat).
    let mut local = skeleton.local.clone();
    let mut aligned = 0usize;
    for (from, to, a_from, a_to) in alignment_segments() {
        let (Some(&from), Some(&to)) = (bones.get(from), bones.get(to)) else {
            continue;
        };
        let frames = skeleton.fk(&local);
        let have = frames[to].pos - frames[from].pos;
        let want = rest_a[a_to.index()].pos - rest_a[a_from.index()].pos;
        if have.length() < 1e-6 || want.length() < 1e-6 {
            return Err(format!("the model's {} segment has no length", from));
        }
        let turned_rot = arc(have, want) * frames[from].rot;
        local[from].rot = (skeleton.parent_frame(&frames, from).rot.inverse() * turned_rot).normalize();
        aligned += 1;
    }
    for foot in ["leftFoot", "rightFoot"] {
        let joint = bones[foot];
        let frames = skeleton.fk(&local);
        local[joint].rot =
            (skeleton.parent_frame(&frames, joint).rot.inverse() * file_rest[joint].rot).normalize();
    }
    // Relaxed fingers: each joint turned about the body's Z axis, toward the
    // palm of a T-pose hand (palm down): +X arm curls by -angle, -X by +angle.
    let mut curled = 0usize;
    for (bone, degrees) in humanoid::FINGER_CURL {
        let Some(&joint) = bones.get(bone) else {
            continue;
        };
        let frames = skeleton.fk(&local);
        let sign = if bone.starts_with("left") { -1.0 } else { 1.0 };
        let curl = Quat::from_rotation_z(sign * degrees.to_radians());
        local[joint].rot =
            (skeleton.parent_frame(&frames, joint).rot.inverse() * curl * frames[joint].rot).normalize();
        curled += 1;
    }
    let rest_b = skeleton.fk(&local);

    // Driven bones, parent-first by joint index.
    let mut driven: Vec<Driven> = Vec::new();
    let mut order: Vec<(usize, Slot)> = Slot::ALL
        .iter()
        .filter_map(|slot| slot.driven_bone().map(|bone| (bones[bone], *slot)))
        .collect();
    order.sort_by_key(|(joint, _)| *joint);
    let mut driven_of_slot: [Option<usize>; SLOT_COUNT] = [None; SLOT_COUNT];
    for (joint, slot) in order {
        let anchor = driven
            .iter()
            .enumerate()
            .filter(|(_, d)| skeleton.is_ancestor(d.joint, joint))
            .max_by_key(|(_, d)| d.joint)
            .map(|(index, _)| index);
        // The root drives no bone, so the hips hang from the body; every
        // other driven slot's parent slot drives a bone.
        let expected = if slot == Slot::Hips { None } else { slot.parent() };
        let found = anchor.map(|index| driven[index].slot);
        if found != expected {
            return Err(format!(
                "the model's {} bone is not below its {} bone",
                slot.driven_bone().unwrap_or("?"),
                expected.and_then(Slot::driven_bone).unwrap_or("body root")
            ));
        }
        let parent = skeleton.parent_frame(&rest_b, joint);
        let link = match anchor {
            Some(index) => rest_b[driven[index].joint].inverse().then(&parent),
            None => parent,
        };
        driven_of_slot[slot.index()] = Some(driven.len());
        driven.push(Driven {
            joint,
            slot,
            anchor,
            link,
            rest_pos: local[joint].pos,
            rest_rot: rest_b[joint].rot,
            rest_scale: rest_b[joint].scale,
        });
    }
    let index_of = |slot: Slot| driven_of_slot[slot.index()].expect("every driven slot was placed");

    let mut legs = Vec::new();
    for (upper, lower, foot) in [
        (Slot::UpperLegL, Slot::LowerLegL, Slot::FootL),
        (Slot::UpperLegR, Slot::LowerLegR, Slot::FootR),
    ] {
        let (u, l, f) = (index_of(upper), index_of(lower), index_of(foot));
        let knee = rest_b[driven[u].joint].local_point(rest_b[driven[l].joint].pos);
        let ankle = rest_b[driven[l].joint].local_point(rest_b[driven[f].joint].pos);
        if knee.length() < 1e-6 || ankle.length() < 1e-6 {
            return Err(format!(
                "the model's {} has a segment without length",
                upper.driven_bone().unwrap_or("leg")
            ));
        }
        legs.push(Leg {
            upper: u,
            lower: l,
            foot: f,
            tip: foot,
            knee,
            ankle,
            ankle_rest: rest_b[driven[f].joint].pos,
        });
    }

    let mut sockets: Vec<Option<Socket>> = vec![None; proxy.local.len()];
    for slot in Slot::ALL {
        let Some(bone) = slot.socket_bone() else {
            continue;
        };
        let target = match slot {
            Slot::HandL | Slot::HandR => {
                let lower = index_of(if slot == Slot::HandL {
                    Slot::LowerArmL
                } else {
                    Slot::LowerArmR
                });
                let hand = rest_b[bones[bone]].pos;
                let side = if slot == Slot::HandL { "left" } else { "right" };
                let palm = humanoid::canonical(&format!("{side}MiddleProximal"), false)
                    .and_then(|finger| bones.get(finger))
                    .map_or(hand, |finger| hand.lerp(rest_b[*finger].pos, 0.5));
                SocketTarget::Point(lower, rest_b[driven[lower].joint].local_point(palm))
            }
            other => SocketTarget::Driven(index_of(other)),
        };
        sockets[slots[slot.index()]] = Some(Socket {
            target,
            rest_pos: proxy.local[slots[slot.index()]].pos,
            carries_offset: matches!(slot, Slot::HandL | Slot::HandR),
        });
    }

    let constants = bones
        .values()
        .map(|joint| (*joint, local[*joint]))
        .collect();
    let hips_rest = rest_b[driven[index_of(Slot::Hips)].joint].pos;
    Ok(Plan {
        proxy_parent: proxy.parent.clone(),
        proxy_base: proxy.base,
        slots,
        rest_a,
        driven,
        hips_rest,
        legs,
        sockets,
        contact: (CONTACT_LOW * avatar_hips, CONTACT_HIGH * avatar_hips),
        fit,
        constants,
        leg_ik: true,
        report: PlanReport {
            scale,
            turned,
            avatar_hips,
            model_hips,
            aligned,
            curled,
        },
    })
}

/// One frame's result, reused across frames.
#[derive(Default, Debug)]
pub(crate) struct Solution {
    /// New local frame per driven bone (same order as [`Plan::driven`]).
    pub(crate) driven_local: Vec<Frame>,
    /// Body-frame frame per driven bone.
    pub(crate) driven_frame: Vec<Frame>,
    /// New local translation per avatar joint that is a socket.
    pub(crate) sockets: Vec<(usize, Vec3)>,
    /// Flat-foot weight per leg (left, right).
    pub(crate) contact: [f32; 2],
    proxy_frames: Vec<Frame>,
    refit_frames: Vec<Frame>,
    overrides: Vec<Option<Quat>>,
}

/// Poses the humanoid from the avatar's current locals (`proxy_local`, one
/// per avatar joint, with the clip's own translations) and refits the
/// avatar's sockets onto it.
pub(crate) fn solve(plan: &Plan, proxy_local: &[Frame], out: &mut Solution) {
    let joints = plan.proxy_parent.len();
    assert_eq!(proxy_local.len(), joints, "one local per avatar joint");
    out.proxy_frames.clear();
    for (joint, local) in proxy_local.iter().enumerate() {
        let parent = plan.proxy_parent[joint].map_or(plan.proxy_base, |p| out.proxy_frames[p]);
        out.proxy_frames.push(parent.then(local));
    }
    let a = |slot: Slot| out.proxy_frames[plan.slots[slot.index()]];
    let delta: [Quat; SLOT_COUNT] = std::array::from_fn(|k| {
        (out.proxy_frames[plan.slots[k]].rot * plan.rest_a[k].rot.inverse()).normalize()
    });
    let ratio: [f32; SLOT_COUNT] =
        std::array::from_fn(|k| out.proxy_frames[plan.slots[k]].scale / plan.rest_a[k].scale);
    let root = a(Slot::Root);
    let root_rest = plan.rest_a[Slot::Root.index()];
    let root_rot = delta[Slot::Root.index()];
    let root_scale = ratio[Slot::Root.index()];
    // An avatar joint's current position plus the rest offset to the
    // humanoid point, turned and scaled with the avatar's root.
    let mapped = |slot: Slot, rest_point: Vec3| {
        a(slot).pos + root_scale * (root_rot * (rest_point - plan.rest_a[slot.index()].pos))
    };
    let forward = delta[Slot::Hips.index()] * Vec3::Z;
    let forward = if Vec3::new(forward.x, 0.0, forward.z).length() > 1e-4 {
        forward
    } else {
        root_rot * Vec3::Z
    };
    let heading = Quat::from_rotation_y(forward.x.atan2(forward.z));

    out.driven_local.clear();
    out.driven_frame.clear();
    out.overrides.clear();
    out.overrides.resize(plan.driven.len(), None);
    let hips_target = mapped(Slot::Hips, plan.hips_rest);
    for (index, bone) in plan.driven.iter().enumerate() {
        let parent = match bone.anchor {
            Some(anchor) => out.driven_frame[anchor].then(&bone.link),
            None => bone.link,
        };
        let k = bone.slot.index();
        let rotation = out.overrides[index].unwrap_or(delta[k] * bone.rest_rot);
        let translation = if bone.slot == Slot::Hips {
            parent.local_point(hips_target)
        } else {
            bone.rest_pos
        };
        let scale = bone.rest_scale * ratio[k] / parent.scale;
        let mut local = Frame::new(translation, (parent.rot.inverse() * rotation).normalize(), scale);
        let mut frame = parent.then(&local);
        if plan.leg_ik {
            if let Some(side) = plan.legs.iter().position(|leg| leg.upper == index) {
                let leg = &plan.legs[side];
                let lower = &plan.driven[leg.lower];
                let foot = &plan.driven[leg.foot];
                let hip = frame.pos;
                let knee0 = frame.apply(leg.knee);
                let lower_rot0 = delta[lower.slot.index()] * lower.rest_rot;
                let lower_scale = lower.rest_scale * ratio[lower.slot.index()];
                let ankle0 = knee0 + lower_scale * (lower_rot0 * leg.ankle);
                let target = mapped(leg.tip, leg.ankle_rest);
                let pole = delta[k] * Vec3::Z;
                let (knee, ankle) = two_bone(
                    hip,
                    (knee0 - hip).length(),
                    (ankle0 - knee0).length(),
                    target,
                    pole,
                );
                let swing_upper = arc(knee0 - hip, knee - hip);
                local.rot = (parent.rot.inverse() * swing_upper * rotation).normalize();
                frame = parent.then(&local);
                let knee_now = frame.apply(leg.knee);
                let lower_rot = swing_upper * lower_rot0;
                let ankle_now = knee_now + lower_scale * (lower_rot * leg.ankle);
                let swing_lower = arc(ankle_now - knee_now, ankle - knee_now);
                out.overrides[leg.lower] = Some((swing_lower * lower_rot).normalize());
                let tip = a(leg.tip).pos - root.pos;
                let tip = (root_rot.inverse() * tip) / root_scale;
                let height = tip.y
                    - (plan.rest_a[leg.tip.index()].pos.y - root_rest.pos.y);
                let weight = contact_weight(height, plan.contact.0, plan.contact.1);
                out.contact[side] = weight;
                let follow = swing_lower * swing_upper * delta[foot.slot.index()] * foot.rest_rot;
                let flat = heading * foot.rest_rot;
                out.overrides[leg.foot] = Some(follow.slerp(flat, weight).normalize());
            }
        }
        out.driven_local.push(local);
        out.driven_frame.push(frame);
    }

    // Sockets: the avatar's joints moved onto the humanoid, parents first.
    out.sockets.clear();
    out.refit_frames.clear();
    for (joint, local) in proxy_local.iter().enumerate() {
        let parent = plan.proxy_parent[joint].map_or(plan.proxy_base, |p| out.refit_frames[p]);
        let mut local = *local;
        if let Some(socket) = &plan.sockets[joint] {
            let target = match socket.target {
                SocketTarget::Driven(index) => out.driven_frame[index].pos,
                SocketTarget::Point(index, point) => out.driven_frame[index].apply(point),
            };
            let mut translation = parent.local_point(target);
            if socket.carries_offset {
                translation += local.pos - socket.rest_pos;
            }
            local.pos = translation;
            out.sockets.push((joint, translation));
        }
        out.refit_frames.push(parent.then(&local));
    }
}
