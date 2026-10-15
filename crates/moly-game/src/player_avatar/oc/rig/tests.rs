//! The retarget law on synthetic skeletons: a small source rig shaped like
//! the player avatar (one spine joint, hand and foot tips, upper-arm rest
//! turned 90 degrees about X) walking in place with planted feet, and
//! humanoid targets with adult proportions, extra bones, random rest
//! rotations and an A-pose. No model or clip data is read.

use super::*;
use std::f32::consts::PI;

fn aa(axis: Vec3, degrees: f32) -> Quat {
    Quat::from_axis_angle(axis.normalize(), degrees.to_radians())
}

/// The angle between two directions, from the cross and dot products: an
/// arccosine of the dot product alone resolves no angle below about 0.02
/// degrees in single precision.
fn angle_deg(a: Vec3, b: Vec3) -> f32 {
    let (a, b) = (a.normalize(), b.normalize());
    a.cross(b).length().atan2(a.dot(b)).to_degrees()
}

/// A skeleton from rest world positions and rotations, parent-first.
fn from_world(bones: &[(Option<usize>, Vec3, Quat)], base: Frame) -> Skeleton {
    let mut parent = Vec::new();
    let mut local = Vec::new();
    for (p, pos, rot) in bones {
        parent.push(*p);
        local.push(match p {
            None => Frame::new(*pos, *rot, 1.0),
            Some(p) => {
                let (ppos, prot) = (bones[*p].1, bones[*p].2);
                Frame::new(prot.inverse() * (*pos - ppos), prot.inverse() * *rot, 1.0)
            }
        });
    }
    Skeleton { parent, local, base }
}

/// Deterministic unit quaternions.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }
    fn quat(&mut self) -> Quat {
        Quat::from_xyzw(self.next(), self.next(), self.next(), self.next()).normalize()
    }
}

const ARM_REST_DEG: f32 = 90.0;

/// Source joints: 0 Root, 1 Hips, 2 Spine, 3 Head, 4 HeadTop, 5..7 left
/// arm, forearm, hand, 8..10 right, 11..13 left thigh, shin, foot tip,
/// 14..16 right.
fn source_rig() -> (Skeleton, [usize; SLOT_COUNT]) {
    let arm = aa(Vec3::X, ARM_REST_DEG);
    let mut bones = vec![
        (None, Vec3::ZERO, Quat::IDENTITY),
        (Some(0), Vec3::new(0.0, 0.25, 0.0), Quat::IDENTITY),
        (Some(1), Vec3::new(0.0, 0.40, 0.0), Quat::IDENTITY),
        (Some(2), Vec3::new(0.0, 0.48, 0.0), Quat::IDENTITY),
        (Some(3), Vec3::new(0.0, 0.80, 0.0), Quat::IDENTITY),
    ];
    for sx in [1.0, -1.0] {
        let first = bones.len();
        bones.push((Some(2), Vec3::new(0.12 * sx, 0.44, 0.0), arm));
        bones.push((Some(first), Vec3::new(0.25 * sx, 0.44, 0.0), arm));
        bones.push((Some(first + 1), Vec3::new(0.40 * sx, 0.44, 0.0), arm));
    }
    for sx in [1.0, -1.0] {
        let first = bones.len();
        bones.push((Some(1), Vec3::new(0.07 * sx, 0.19, 0.0), Quat::IDENTITY));
        bones.push((Some(first), Vec3::new(0.07 * sx, 0.10, 0.0), Quat::IDENTITY));
        bones.push((Some(first + 1), Vec3::new(0.07 * sx, 0.0, 0.0), Quat::IDENTITY));
    }
    let slots = [0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    (from_world(&bones, Frame::IDENTITY), slots)
}

#[derive(Clone, Copy, PartialEq)]
enum Variant {
    /// Identity rests, T-pose.
    Identity,
    /// Random rest rotations on every bone, T-pose.
    Random,
    /// Random rests, arms 40 degrees down, legs 6 degrees apart.
    APose,
    /// Identity rests, facing -Z.
    Backward,
}

/// A humanoid in a VRM-like layout (facing +Z unless `Backward`), with a
/// non-humanoid root above the hips and tip joints.
fn target_rig(variant: Variant, rng: &mut Rng) -> (Skeleton, BTreeMap<&'static str, usize>) {
    let mut names: Vec<(&'static str, Option<&'static str>, Vec3)> = vec![
        ("Root", None, Vec3::ZERO),
        ("hips", Some("Root"), Vec3::new(0.0, 0.95, 0.0)),
        ("spine", Some("hips"), Vec3::new(0.0, 1.05, 0.0)),
        ("chest", Some("spine"), Vec3::new(0.0, 1.17, 0.0)),
        ("upperChest", Some("chest"), Vec3::new(0.0, 1.29, 0.0)),
        ("neck", Some("upperChest"), Vec3::new(0.0, 1.42, 0.0)),
        ("head", Some("neck"), Vec3::new(0.0, 1.52, 0.0)),
        ("headTop", Some("head"), Vec3::new(0.0, 1.72, 0.0)),
    ];
    for (side, sx) in [("left", 1.0f32), ("right", -1.0)] {
        let n = |s: &str| -> &'static str {
            humanoid::canonical(&format!("{side}{s}"), false)
                .unwrap_or_else(|| Box::leak(format!("{side}{s}").into_boxed_str()))
        };
        let mut arm = vec![
            (n("Shoulder"), Some("upperChest"), Vec3::new(0.03 * sx, 1.38, 0.0)),
            (n("UpperArm"), Some(n("Shoulder")), Vec3::new(0.15 * sx, 1.38, 0.0)),
            (n("LowerArm"), Some(n("UpperArm")), Vec3::new(0.43 * sx, 1.38, 0.0)),
            (n("Hand"), Some(n("LowerArm")), Vec3::new(0.68 * sx, 1.38, 0.0)),
            (n("MiddleProximal"), Some(n("Hand")), Vec3::new(0.78 * sx, 1.38, 0.0)),
            (n("MiddleIntermediate"), Some(n("MiddleProximal")), Vec3::new(0.82 * sx, 1.38, 0.0)),
        ];
        let mut leg = vec![
            (n("UpperLeg"), Some("hips"), Vec3::new(0.09 * sx, 0.90, 0.0)),
            (n("LowerLeg"), Some(n("UpperLeg")), Vec3::new(0.09 * sx, 0.47, 0.0)),
            (n("Foot"), Some(n("LowerLeg")), Vec3::new(0.09 * sx, 0.08, -0.03)),
            (n("Toes"), Some(n("Foot")), Vec3::new(0.09 * sx, 0.02, 0.10)),
            (n("ToeTip"), Some(n("Toes")), Vec3::new(0.09 * sx, 0.01, 0.18)),
        ];
        if variant == Variant::APose {
            let pivot = arm[1].2;
            let down = aa(Vec3::Z, -40.0 * sx);
            for entry in arm.iter_mut().skip(2) {
                entry.2 = pivot + down * (entry.2 - pivot);
            }
            let pivot = leg[0].2;
            let apart = aa(Vec3::Z, 6.0 * sx);
            for entry in leg.iter_mut().skip(1) {
                entry.2 = pivot + apart * (entry.2 - pivot);
            }
        }
        names.extend(arm);
        names.extend(leg);
    }
    let index = |name: &str, list: &[(&str, Option<&str>, Vec3)]| {
        list.iter().position(|(n, _, _)| *n == name).unwrap()
    };
    let turn = if variant == Variant::Backward { aa(Vec3::Y, 180.0) } else { Quat::IDENTITY };
    let mut bones = Vec::new();
    for (name, parent, pos) in &names {
        let rot = if matches!(variant, Variant::Identity | Variant::Backward) || *name == "Root" {
            Quat::IDENTITY
        } else {
            rng.quat()
        };
        bones.push((parent.map(|p| index(p, &names)), turn * *pos, turn * rot));
    }
    let mut map = BTreeMap::new();
    for (i, (name, _, _)) in names.iter().enumerate() {
        if let Some(bone) = humanoid::canonical(name, false) {
            map.insert(bone, i);
        }
    }
    (from_world(&bones, Frame::IDENTITY), map)
}

const CYCLE: f32 = 0.8;
const STANCE: f32 = 0.6;
const HALF_STRIDE: f32 = 0.06;
const SPEED: f32 = 2.0 * HALF_STRIDE / (STANCE * CYCLE);

fn foot_target(phase: f32, side_x: f32) -> (Vec3, bool) {
    if phase < STANCE {
        let u = phase / STANCE;
        (Vec3::new(side_x, 0.0, HALF_STRIDE - 2.0 * HALF_STRIDE * u), true)
    } else {
        let u = (phase - STANCE) / (1.0 - STANCE);
        let z = -HALF_STRIDE + 2.0 * HALF_STRIDE * (0.5 - 0.5 * (PI * u).cos());
        (Vec3::new(side_x, 0.035 * (PI * u).sin(), z), false)
    }
}

/// The source walk at time `t`: rotations on every joint, translation on
/// the hips only, the foot tips planted on the ground during stance (the
/// legs are posed by IK on the source side).
fn source_pose(source: &Skeleton, t: f32) -> (Vec<Frame>, [bool; 2]) {
    let w = 2.0 * PI / CYCLE;
    let mut local = source.local.clone();
    local[1].pos = Vec3::new(0.004 * (w * t).sin(), 0.228 + 0.004 * (2.0 * w * t).cos(), 0.0);
    local[1].rot = aa(Vec3::Y, 6.0 * (w * t).sin()) * aa(Vec3::Z, 3.0 * (w * t).sin());
    local[2].rot = aa(Vec3::Y, -5.0 * (w * t).sin()) * aa(Vec3::X, 4.0);
    local[3].rot = aa(Vec3::X, 3.0 * (2.0 * w * t).sin());
    for (arm, sx, ph) in [(5usize, 1.0f32, 0.0f32), (8, -1.0, 0.5)] {
        let swing = 25.0 * (w * t + PI * (1.0 - 2.0 * ph) + PI).sin();
        let delta = aa(Vec3::X, -swing * sx) * aa(Vec3::Z, -75.0 * sx);
        local[arm].rot = delta * source.local[arm].rot;
        local[arm + 1].rot = aa(Vec3::Z, (20.0 + 10.0 * (w * t).sin()) * sx);
    }
    let mut stance = [false; 2];
    for (side, (up, sx, ph)) in [(11usize, 1.0f32, 0.0f32), (14, -1.0, 0.5)].into_iter().enumerate() {
        let frames = source.fk(&local);
        let phase = (t / CYCLE + ph).rem_euclid(1.0);
        let (target, planted) = foot_target(phase, 0.07 * sx);
        stance[side] = planted;
        let (hip, knee0, tip0) = (frames[up].pos, frames[up + 1].pos, frames[up + 2].pos);
        let pole = frames[1].rot * Vec3::Z;
        let (knee, ankle) =
            two_bone(hip, (knee0 - hip).length(), (tip0 - knee0).length(), target, pole);
        let upper = arc(knee0 - hip, knee - hip) * frames[up].rot;
        local[up].rot = frames[1].rot.inverse() * upper;
        let shin = upper * local[up + 1].rot;
        let shin_now = shin * local[up + 2].pos;
        let lower = arc(shin_now, ankle - knee) * shin;
        local[up + 1].rot = upper.inverse() * lower;
    }
    (local, stance)
}

/// Source segments against target segments: (source from, to, target from, to).
fn segments(map: &BTreeMap<&'static str, usize>) -> Vec<(usize, usize, usize, usize)> {
    let b = |name: &str| map[name];
    let mut list = vec![(2, 3, b("spine"), b("head")), (3, 4, b("head"), b("head") + 1)];
    for (s, side) in [(5usize, "left"), (8, "right")] {
        let n = |x: &str| b(humanoid::canonical(&format!("{side}{x}"), false).unwrap());
        list.push((s, s + 1, n("UpperArm"), n("LowerArm")));
        list.push((s + 1, s + 2, n("LowerArm"), n("Hand")));
        list.push((s + 1, s + 2, n("Hand"), n("MiddleProximal")));
    }
    for (s, side) in [(11usize, "left"), (14, "right")] {
        let n = |x: &str| b(humanoid::canonical(&format!("{side}{x}"), false).unwrap());
        list.push((s, s + 1, n("UpperLeg"), n("LowerLeg")));
        list.push((s + 1, s + 2, n("LowerLeg"), n("Foot")));
    }
    list
}

struct Measure {
    direction: f32,
    slide: f32,
    vertical: f32,
    tilt: f32,
    socket: f32,
    stance_frames: usize,
}

/// Two walk cycles at 60 Hz. Slide and vertical drift are fractions of
/// the target's fitted leg length; the tilt is the foot's up axis against
/// its rest; the socket gap is the largest distance between a refitted
/// source joint and its target point.
fn run(variant: Variant, leg_ik: bool, naive: bool) -> Measure {
    let mut rng = Rng(0x2026_1014);
    let (source, slots) = source_rig();
    let (target, map) = target_rig(variant, &mut rng);
    let mut plan = plan(&source, slots, &target, &map).expect("the target is accepted");
    plan.leg_ik = leg_ik;
    let skeleton = Skeleton { base: plan.fit, ..target.clone() };
    let mut local = target.local.clone();
    for (joint, frame) in &plan.constants {
        local[*joint] = *frame;
    }
    let file_rest = skeleton.fk(&skeleton.local);
    let leg_length = {
        let rest = skeleton.fk(&local);
        (rest[map["leftLowerLeg"]].pos - rest[map["leftUpperLeg"]].pos).length()
            + (rest[map["leftFoot"]].pos - rest[map["leftLowerLeg"]].pos).length()
    };
    let mut out = Solution::default();
    let mut m = Measure { direction: 0.0, slide: 0.0, vertical: 0.0, tilt: 0.0, socket: 0.0, stance_frames: 0 };
    let mut contact: [Option<Vec3>; 2] = [None; 2];
    for frame in 0..=96 {
        let t = frame as f32 / 60.0;
        let (pose, stance) = source_pose(&source, t);
        let a = source.fk(&pose);
        solve(&plan, &pose, &mut out);
        let mut posed = local.clone();
        if naive {
            for (k, slot) in Slot::ALL.iter().enumerate() {
                if let Some(bone) = slot.driven_bone() {
                    posed[map[bone]].rot = pose[slots[k]].rot;
                }
            }
        } else {
            for (bone, frame) in plan.driven.iter().zip(&out.driven_local) {
                posed[bone.joint] = *frame;
            }
        }
        let b = skeleton.fk(&posed);
        for (af, at, bf, bt) in segments(&map) {
            if leg_ik && af >= 11 {
                continue;
            }
            m.direction = m.direction.max(angle_deg(a[at].pos - a[af].pos, b[bt].pos - b[bf].pos));
        }
        if naive {
            continue;
        }
        let mut refit = pose.clone();
        for (joint, translation) in &out.sockets {
            refit[*joint].pos = *translation;
        }
        let r = source.fk(&refit);
        for (slot, bone) in [(Slot::Head, "head"), (Slot::Hips, "hips"), (Slot::LowerArmL, "leftLowerArm")] {
            m.socket = m.socket.max((r[slots[slot.index()]].pos - b[map[bone]].pos).length());
        }
        if leg_ik {
            for (lower_slot, bone) in [(Slot::LowerLegL, "leftLowerLeg"), (Slot::FootR, "rightFoot")] {
                m.socket = m.socket.max((r[slots[lower_slot.index()]].pos - b[map[bone]].pos).length());
            }
        }
        let root = Vec3::new(0.0, 0.0, SPEED * t);
        for (side, foot) in ["leftFoot", "rightFoot"].into_iter().enumerate() {
            if !stance[side] {
                contact[side] = None;
                continue;
            }
            m.stance_frames += 1;
            let world = root + b[map[foot]].pos;
            let start = *contact[side].get_or_insert(world);
            let d = world - start;
            m.slide = m.slide.max(Vec3::new(d.x, 0.0, d.z).length() / leg_length);
            m.vertical = m.vertical.max(d.y.abs() / leg_length);
            let up = (b[map[foot]].rot * file_rest[map[foot]].rot.inverse()) * Vec3::Y;
            m.tilt = m.tilt.max(angle_deg(up, Vec3::Y));
        }
    }
    m
}

#[test]
fn bone_directions_follow_the_source_on_every_target() {
    for variant in [Variant::Identity, Variant::Random, Variant::APose, Variant::Backward] {
        let m = run(variant, false, false);
        assert!(m.direction < 1e-2, "direction error {} deg", m.direction);
        assert!(m.socket < 1e-4, "socket gap {}", m.socket);
    }
}

#[test]
fn a_naive_rotation_copy_misses_the_source() {
    // Positive control: the direction metric has signal on these rigs.
    let m = run(Variant::Random, false, true);
    assert!(m.direction > 1.0, "naive direction error {} deg", m.direction);
}

#[test]
fn rotation_alone_slides_and_tilts_the_feet() {
    // Positive control for the foot metrics: without the leg solve the
    // target's feet move during stance and pitch with the shin.
    let m = run(Variant::Random, false, false);
    assert!(m.stance_frames > 0);
    assert!(m.slide > 0.01 || m.vertical > 0.01, "slide {} vertical {}", m.slide, m.vertical);
    assert!(m.tilt > 5.0, "tilt {} deg", m.tilt);
}

#[test]
fn leg_ik_plants_flat_feet_during_stance() {
    for variant in [Variant::Identity, Variant::Random, Variant::APose, Variant::Backward] {
        let m = run(variant, true, false);
        assert!(m.stance_frames > 0);
        assert!(m.slide < 1e-3, "slide {} of the leg", m.slide);
        assert!(m.vertical < 1e-3, "vertical drift {} of the leg", m.vertical);
        assert!(m.tilt < 1e-2, "foot tilt {} deg", m.tilt);
        assert!(m.direction < 1e-2, "upper body direction error {} deg", m.direction);
        assert!(m.socket < 1e-4, "socket gap {}", m.socket);
    }
}

#[test]
fn the_fit_matches_hips_heights_and_turns_backward_models() {
    let mut rng = Rng(7);
    let (source, slots) = source_rig();
    let (target, map) = target_rig(Variant::Backward, &mut rng);
    let plan = plan(&source, slots, &target, &map).unwrap();
    assert!(plan.report.turned);
    assert!((plan.report.scale - 0.25 / 0.95).abs() < 1e-6);
    let (target, map) = target_rig(Variant::Identity, &mut rng);
    let plan = super::plan(&source, slots, &target, &map).unwrap();
    assert!(!plan.report.turned);
    assert_eq!(plan.report.curled, 4);
}

#[test]
fn bones_out_of_hierarchy_are_refused() {
    let mut rng = Rng(9);
    let (source, slots) = source_rig();
    let (mut target, map) = target_rig(Variant::Identity, &mut rng);
    // Hang the left lower arm from the chest instead of the upper arm.
    target.parent[map["leftLowerArm"]] = Some(map["chest"]);
    let error = plan(&source, slots, &target, &map).unwrap_err();
    assert!(error.contains("leftLowerArm"), "{error}");
}

#[test]
fn two_bone_reaches_and_clamps() {
    let hip = Vec3::new(0.0, 1.0, 0.0);
    let (knee, ankle) = two_bone(hip, 0.5, 0.5, Vec3::new(0.0, 0.2, 0.1), Vec3::Z);
    assert!(((knee - hip).length() - 0.5).abs() < 1e-5);
    assert!(((ankle - knee).length() - 0.5).abs() < 1e-5);
    assert!((ankle - Vec3::new(0.0, 0.2, 0.1)).length() < 1e-5);
    assert!(knee.z > 0.0, "the knee bends toward the pole");
    let (_, far) = two_bone(hip, 0.5, 0.5, Vec3::new(0.0, -5.0, 0.0), Vec3::Z);
    assert!(((far - hip).length() - 1.0).abs() < 1e-4);
    assert_eq!(contact_weight(0.0, 0.1, 0.2), 1.0);
    assert_eq!(contact_weight(0.3, 0.1, 0.2), 0.0);
    assert!((contact_weight(0.15, 0.1, 0.2) - 0.5).abs() < 1e-6);
}
