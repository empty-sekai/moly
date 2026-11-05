//! Dropping a character: from its animation to a body of rigid parts.
//!
//! A drop is asked for by the control surface's `moly.ragdoll.drop` method or
//! by a step of a script (`npc.ragdoll`); no other state drops a character, and
//! a script's take of a character does not. The request is carried out at
//! once, in the frame that asks:
//!
//! - a request of this frame runs after the frame's animation systems and
//!   before its physics step, so the bones are read where the animation of the
//!   frame leaves them;
//! - a request made between frames, from a script's step, runs before the next
//!   frame's animation, after the transforms of the last frame were
//!   propagated, so the body starts from the pose the frame showed.
//!
//! The fall takes a character from its AI only when no script holds it, and
//! gives it back to its AI once it has got up only if it took it: a character
//! a script holds stays the script's.
//!
//! Every part becomes a dynamic body with one collider, and every joint a
//! spherical or hinge joint between two parts of the same character, which
//! stops those two parts colliding. The body falls into a scene of its own: the
//! floor under it, the furniture around it and the other bodies falling near it
//! ([`crate::collision`]).
//!
//! A part's body has the part's frame as its own: the shape is laid out in it
//! and the body's mass is centred where the shape's is. The part's frame is
//! not its bone's (a box part stands level where its bone is turned), so the
//! bone is kept as seen from the part ([`Part::bone_offset`]), and every bone
//! written back goes through [`Part::bone_at`].
//!
//! Avian allows sixteen components in one spawn, so a part's body is spawned
//! in two: its shape, its place and its speeds, and then its mass and the
//! forces it is tuned by. The fixed joint that keeps a shoulder clear of the
//! chest is a joint of no freedom, spawned with the body's other joints.

use avian3d::prelude::*;
use bevy::prelude::*;
use serde_json::{json, Value};

use crate::collision;
use crate::description::{self, JointPlan, Limb, Limits, PartPlan, Role, Skeleton};
use crate::rig::{self, Bones};
use crate::writeback::Previous;
use crate::RagdollSettings;
use moly_features::act::Taken;
use moly_features::character::{BodyOwner, CharacterModel, Falling, MotionDriver};
use moly_features::npc::CharacterUnitId;
use moly_features::npc_take::DirectedOwned;

/// What was asked of a character's body.
#[derive(Clone, Debug)]
pub enum Ask {
    /// Fall, with an optional push in metres per second per metre of the
    /// character's height on the part of the bone named (the pelvis when no
    /// bone is named).
    Drop { impulse: Option<Vec3>, at: Option<String> },
    /// Get up now, blending over the settings' recovery time.
    Recover,
}

/// One part of a falling body.
pub struct Part {
    /// The part's bone.
    pub bone: Entity,
    pub role: Role,
    /// The part's collider, as a body whose frame is the part's frame.
    pub body: Entity,
    /// Where the bone lies seen from the part's frame: fixed for the body's
    /// life. A box part is level where its bone is turned, so the two
    /// frames differ by this much.
    pub bone_offset: Isometry3d,
    /// The part's frame where it fell.
    pub position: Vec3,
    pub rotation: Quat,
    /// How far the part's shape reaches from its middle of mass, in the
    /// part's frame (see [`description::Shape::reach`]).
    pub reach: (Vec3, f32),
}

impl Part {
    /// The part's frame as its body lies now.
    pub fn frame(&self, world: &World) -> Option<(Vec3, Quat)> {
        let position = world.get::<Position>(self.body)?;
        let rotation = world.get::<Rotation>(self.body)?;
        Some((position.0, rotation.0))
    }

    /// Where the bone lies in the world with the part's frame at `position`
    /// and `rotation`.
    pub fn bone_at(&self, position: Vec3, rotation: Quat) -> Isometry3d {
        Isometry3d::new(position, rotation) * self.bone_offset
    }
}

/// How far any point of a part reaching `reach` moved between two of its
/// frames: its middle of mass, the ends of its segment and the rim round it.
/// A limb rolling on its own length moves only its surface, by the turn
/// times its radius.
pub fn moved(reach: (Vec3, f32), from: (Vec3, Quat), to: (Vec3, Quat)) -> f32 {
    let (half, radius) = reach;
    from.0.distance(to.0) + (to.1 * half - from.1 * half).length() + from.1.angle_between(to.1) * radius
}

/// How fast the parts of a body move from one frame to the next, as the
/// frames are drawn.
///
/// The physics takes many small steps in a frame, and the speeds it keeps on
/// the bodies carry what those steps push back and forth within the frame: a
/// limb held between the floor and its joint is nudged both ways at every
/// step, and its speed reads well above the nothing it moves on screen. The
/// way the parts actually travel from one drawn frame to the next is the
/// motion anyone sees, and the body is at rest when that stops.
#[derive(Default)]
pub struct Seen {
    /// The parts' frames when the last frame's physics ended.
    frames: Vec<(Vec3, Quat)>,
    /// How fast any point of each part moved over the last frame, in metres
    /// per second; empty before the body has been seen over a frame.
    speeds: Vec<f32>,
}

impl Seen {
    /// Takes the parts' `frames` at the end of a frame `delta` seconds long.
    pub fn record(&mut self, reaches: &[(Vec3, f32)], frames: Vec<(Vec3, Quat)>, delta: f32) {
        if delta > 0.0 && self.frames.len() == frames.len() && reaches.len() == frames.len() {
            self.speeds = reaches.iter().zip(self.frames.iter().zip(&frames)).map(|(reach, (from, to))| moved(*reach, *from, *to) / delta).collect();
        }
        self.frames = frames;
    }

    /// How fast the part at `index` (the pelvis first) moved over the last
    /// frame; nothing before the body has been seen over one.
    pub fn speed(&self, index: usize) -> Option<f32> {
        self.speeds.get(index).copied()
    }

    /// The fastest any part moved over the last frame; endless before the
    /// body has been seen over one, so that it is never at rest unseen.
    pub fn fastest(&self) -> f32 {
        if self.speeds.is_empty() {
            return f32::INFINITY;
        }
        self.speeds.iter().copied().fold(0.0, f32::max)
    }
}

/// A character that has fallen and has not been given up.
#[derive(Component)]
pub struct Drop {
    /// The pelvis's part, the first the description lays out.
    pub root: Part,
    /// The body's other parts, from the pelvis down.
    pub parts: Vec<Part>,
    pub joints: Vec<Entity>,
    /// The static bodies of the scene it falls in.
    pub scene: Vec<Entity>,
    /// Every named node under the model with the pose it held when the body
    /// fell: a node without a part keeps that pose against its parent.
    pub held: Vec<(Entity, Transform)>,
    /// The bones the body was described from.
    pub bones: Bones,
    /// The body's height: speeds and distances are read per metre of it.
    pub height: f32,
    /// The ground the body falls onto.
    pub floor: f32,
    /// Where the pelvis was facing when the character fell, on the ground.
    pub facing: Vec3,
    /// How high the pelvis's bone stood over the floor when it fell.
    pub pelvis_height: f32,
    /// The bone the character was dropped from, as a status answer says it.
    pub at: String,
    /// The game seconds the body has fallen.
    pub how_long: f32,
    /// How fast its parts have moved over the last frame.
    pub seen: Seen,
    /// The recovery in progress, once the body has come to rest.
    pub getting_up: Option<crate::recover::Getting>,
    /// The animator whose running animations are drawn once in this frame,
    /// so that the falling body starts from the pose the characters see.
    pub seek: Option<Entity>,
    /// The fall took the character from its AI itself (no script held it),
    /// so the character goes back to its AI once it has got up.
    pub took_from_ai: bool,
}

impl Drop {
    /// The pelvis's part.
    pub fn root_part(&self) -> &Part {
        &self.root
    }

    /// Every part, the pelvis first.
    pub fn all_parts(&self) -> impl Iterator<Item = &Part> {
        std::iter::once(&self.root).chain(self.parts.iter())
    }

    /// The first part of a role.
    pub fn of_role(&self, role: Role) -> Option<&Part> {
        self.all_parts().find(|part| part.role == role)
    }

    /// The fastest any point of any part moved over the last frame, in
    /// metres per second (see `Seen`).
    pub fn fastest(&self) -> f32 {
        self.seen.fastest()
    }

    /// Every entity that goes with the body: its parts, its joints and its
    /// scene.
    pub fn entities(&self) -> Vec<Entity> {
        let mut entities: Vec<Entity> = self.all_parts().map(|part| part.body).collect();
        entities.extend(self.joints.iter().copied());
        entities.extend(self.scene.iter().copied());
        entities
    }
}

/// What a drop or a recovery answers with.
pub struct Answer {
    pub note: String,
    /// The body the character fell from, for a drop.
    pub body: Value,
}

/// Update, after the frame's animation and before its physics: every falling
/// body ages by the frame.
pub(crate) fn step(world: &mut World) {
    let delta = world.resource::<Time>().delta_secs();
    let falling: Vec<Entity> = world.query_filtered::<Entity, With<Drop>>().iter(world).collect();
    for member in falling {
        if let Some(mut drop) = world.get_mut::<Drop>(member) {
            drop.how_long += delta;
        }
    }
}

/// Update, after the frame's physics: how far every falling body's parts
/// moved in it.
pub(crate) fn watch(world: &mut World) {
    let delta = world.resource::<Time>().delta_secs();
    let falling: Vec<Entity> = world.query_filtered::<Entity, With<Drop>>().iter(world).collect();
    for member in falling {
        let Some(drop) = world.get::<Drop>(member) else {
            continue;
        };
        let reaches: Vec<(Vec3, f32)> = drop.all_parts().map(|part| part.reach).collect();
        let frames: Vec<(Vec3, Quat)> = drop.all_parts().map(|part| part.frame(world).unwrap_or((part.position, part.rotation))).collect();
        if let Some(mut drop) = world.get_mut::<Drop>(member) {
            drop.seen.record(&reaches, frames, delta);
        }
    }
}

/// The body a character was dropped from, for the answers.
pub fn where_from(world: &World, member: Entity) -> Value {
    let Some(drop) = world.get::<Drop>(member) else {
        return Value::Null;
    };
    let pelvis = drop.root_part().frame(world).map_or(drop.root_part().position, |(position, _)| position);
    json!({
        "at": drop.at,
        "height": drop.height,
        "floor": drop.floor,
        "pelvis_height": pelvis.y - drop.floor,
        "parts": drop.all_parts().count(),
        "joints": drop.joints.len(),
    })
}

/// Ends a fall whose body another state has taken away: the parts, their
/// joints and their scene are removed, and the bones are left as the
/// animation will set them from the next frame on. The character is not
/// given back to its AI, which the state that took it decides.
pub fn take_away(world: &mut World, member: Entity) {
    let Some(drop) = world.get_mut::<Drop>(member) else {
        return;
    };
    for entity in drop.entities() {
        world.despawn(entity);
    }
    world.entity_mut(member).remove::<(Drop, Falling)>();
}

/// Carries out one request at once, in the caller's frame.
pub fn carry_out(world: &mut World, member: Entity, ask: Ask) -> Result<Answer, String> {
    let unit = world.get::<CharacterUnitId>(member).map_or(0, |unit| unit.0);
    if let Some(drop) = world.get::<Drop>(member) {
        return match ask {
            Ask::Drop { .. } => Err(format!("character {unit} is falling already")),
            Ask::Recover if drop.getting_up.is_some() => Err(format!("character {unit} is already getting up")),
            Ask::Recover => {
                crate::recover::begin(world, member)?;
                Ok(Answer { note: format!("character {unit} is getting up"), body: where_from(world, member) })
            }
        };
    }
    match ask {
        Ask::Recover => Err(format!("character {unit} is not falling")),
        Ask::Drop { impulse, at } => fall(world, member, unit, impulse, at),
    }
}

/// The control surface's drop: the character falls at once, and the answer is
/// its note and where the body fell from.
pub fn drop_now(world: &mut World, unit: u32, impulse: Option<Vec3>, at: Option<String>) -> Result<Answer, String> {
    let member = Taken::body(world, unit)?;
    carry_out(world, member, Ask::Drop { impulse, at })
}

/// The control surface's recovery: the character gets up at once.
pub fn recover_now(world: &mut World, unit: u32) -> Result<Answer, String> {
    let member = Taken::body(world, unit)?;
    carry_out(world, member, Ask::Recover)
}

/// The index of the part the bone named `at` follows, or of the pelvis's part.
fn push_bone(bones: &Bones, plans: &[PartPlan], unit: u32, at: Option<&str>) -> Result<usize, String> {
    let bone = match at {
        None => bones.entity("Hips").ok_or_else(|| format!("character {unit} cannot fall: its skeleton has no Hips bone"))?,
        Some(name) => bones.get(name).ok_or_else(|| format!("character {unit} has no bone {name:?}"))?.entity,
    };
    plans
        .iter()
        .position(|plan| plan.entity == bone)
        .ok_or_else(|| format!("character {unit} has no falling part on the bone {}", at.unwrap_or("Hips")))
}

/// The skeleton the parts are described from: the bones' places and the speed
/// each one moved at since the last propagated frame, with the crown of the
/// head taken from above the head bone.
fn skeleton(world: &World, bones: &Bones) -> Result<Skeleton, String> {
    let previous = world.get_resource::<Previous>();
    let delta = world.get_resource::<Time>().map_or(0.0, |time| time.delta_secs());
    let at = |name: &str| -> Option<description::Placed> {
        let bone = bones.get(name)?;
        let global = world.get::<GlobalTransform>(bone.entity)?;
        let (_, rotation, position) = global.to_scale_rotation_translation();
        let velocity = match (previous, delta > 0.0) {
            (Some(previous), true) => previous.get(bone.entity).map_or(Vec3::ZERO, |then| (position - then.translation()) / delta),
            _ => Vec3::ZERO,
        };
        Some(description::Placed { entity: bone.entity, parent: None, position, rotation, velocity })
    };
    let mut skeleton = Skeleton::default();
    for name in bones.names() {
        if let Some(placed) = at(name) {
            skeleton.placed.insert(name.to_owned(), placed);
        }
    }
    skeleton.spine = ["Spine", "Spine1"].iter().filter(|name| skeleton.bone(name).is_some()).map(|name| (*name).to_owned()).collect();
    skeleton.head = "Head".to_owned();
    let limb = |proximal: &str, distal: &str, leg: bool| Limb { proximal: proximal.to_owned(), distal: distal.to_owned(), leg };
    skeleton.limbs = vec![
        limb("LeftArm", "LeftForeArm", false),
        limb("RightArm", "RightForeArm", false),
        limb("LeftUpLeg", "LeftLeg", true),
        limb("RightUpLeg", "RightLeg", true),
    ];
    // The crown: the highest of everything hung under the head, so that the
    // head's sphere reaches the top of the hair rather than the skull.
    let head = bones.entity("Head").ok_or_else(|| "its skeleton has no Head bone".to_owned())?;
    let mut crown = world.get::<GlobalTransform>(head).map_or(Vec3::ZERO, |global| global.translation());
    let mut stack = vec![head];
    while let Some(entity) = stack.pop() {
        if let Some(global) = world.get::<GlobalTransform>(entity) {
            crown.y = crown.y.max(global.translation().y);
        }
        if let Some(kids) = world.get::<Children>(entity) {
            stack.extend(kids.iter());
        }
    }
    skeleton.head_top = crown;
    Ok(skeleton)
}

/// Every named node under the character's model, with the pose it holds.
fn held_nodes(world: &World, member: Entity) -> Vec<(Entity, Transform)> {
    let model = world.get::<Children>(member).and_then(|kids| kids.iter().find(|kid| world.get::<CharacterModel>(*kid).is_some()));
    let Some(model) = model else {
        return Vec::new();
    };
    let mut held = Vec::new();
    let mut stack = vec![model];
    while let Some(entity) = stack.pop() {
        if world.get::<Name>(entity).is_some() {
            if let Some(pose) = world.get::<Transform>(entity) {
                held.push((entity, *pose));
            }
            if let Some(kids) = world.get::<Children>(entity) {
                stack.extend(kids.iter());
            }
        }
    }
    held
}

/// What one part's body is made of, besides the shape it falls with.
struct Body {
    /// The shape's middle of mass in the part's frame.
    center: Vec3,
    weight: f32,
    /// The body's resistance to turning, the same about all its axes.
    inertia: f32,
}

/// The least share of the whole body's weight a part weighs. A part much
/// lighter than the parts it is joined to (the short spine between the pelvis
/// and the chest, a thin forearm) cannot hold its joints against them and
/// keeps turning in place; at this share the heaviest part (an SD
/// character's head) outweighs the lightest about ten times over.
const LEAST_SHARE: f32 = 0.04;

/// The part's body: the shape's mass properties at a density of one, scaled
/// to the part's weight, which is its shape's own and at least `least`. The
/// resistance to turning grows with the weight as the mass does, so the scale
/// carries over to it unchanged.
fn body_of(plan: &PartPlan, collider: &Collider, settings: &RagdollSettings, least: f32) -> Body {
    let weight = plan.shape.weight().max(least) * settings.mass_scale;
    let unit = collider.mass_properties(1.0);
    let per_unit = if unit.mass > 0.0 { weight / unit.mass } else { 0.0 };
    let inertia = (unit.principal_angular_inertia * per_unit).max(Vec3::ZERO).max_element().max(f32::MIN_POSITIVE);
    Body { center: unit.center_of_mass, weight, inertia }
}

/// The bodies: one dynamic body per part, its frame the part's frame (the
/// shape is laid out in it) and its mass at the shape's middle.
fn spawn_parts(world: &mut World, member: Entity, plans: &[PartPlan], settings: &RagdollSettings) -> (Vec<Entity>, Vec<f32>) {
    let mut bodies = Vec::with_capacity(plans.len());
    let mut inertias = Vec::with_capacity(plans.len());
    let least = plans.iter().map(|plan| plan.shape.weight()).sum::<f32>() * LEAST_SHARE;
    for plan in plans {
        // The shape goes into the spawn and the rest of what the body is made
        // of stays with the part.
        let collider = plan.shape.collider();
        let body = body_of(plan, &collider, settings, least);
        let start = Transform::from_translation(plan.position).with_rotation(plan.rotation);
        let entity = world
            .spawn((
                RigidBody::Dynamic,
                collider,
                start,
                GlobalTransform::from(start),
                Position::from(plan.position),
                Rotation::from(plan.rotation),
                LinearVelocity(plan.velocity),
                LinearDamping(settings.linear_damping),
                AngularDamping(settings.angular_damping),
                GravityScale(settings.gravity_scale),
                collision::part_layers(),
                // A body of parts wakes and sleeps as a whole: one limb at
                // rest does not freeze the chest above it.
                SleepingDisabled,
            ))
            .id();
        // The weight and how the body turns are the shape's own, at the
        // density it was weighed at, so the collider contributes nothing more.
        world.entity_mut(entity).insert((
            Mass(body.weight),
            AngularInertia::new(Vec3::splat(body.inertia)),
            CenterOfMass(body.center),
            NoAutoMass,
            NoAutoAngularInertia,
            NoAutoCenterOfMass,
            collision::PartOf(member),
            ActiveCollisionHooks::FILTER_PAIRS,
        ));
        bodies.push(entity);
        inertias.push(body.inertia);
    }
    (bodies, inertias)
}

/// Builds the body of the character `member` and drops it.
fn fall(world: &mut World, member: Entity, unit: u32, impulse: Option<Vec3>, at: Option<String>) -> Result<Answer, String> {
    // Taking the character from its AI: what a script's take refuses is
    // refused here by the same name. A character a script holds stays the
    // script's; one taken here goes back to its AI once it has got up.
    let took_from_ai = world.get::<DirectedOwned>(member).is_none();
    if took_from_ai {
        moly_features::npc_take::take(world, unit).map_err(|refusal| format!("character {unit} cannot fall: {refusal}"))?;
    }
    let settings = *world.resource::<RagdollSettings>();
    let bones = rig::read_in_world(world, member).map_err(|missing| format!("character {unit} cannot fall: {}", missing.0))?;
    let shape = skeleton(world, &bones).map_err(|reason| format!("character {unit} cannot fall: {reason}"))?;
    let (plans, joint_plans) = description::describe(&shape).map_err(|refusal| format!("character {unit} cannot fall: {}", refusal.0))?;
    let push = push_bone(&bones, &plans, unit, at.as_deref())?;
    let height = description::height_of(&shape);
    let held = held_nodes(world, member);

    // The ground under the feet and the scene around the body.
    let lowest = shape.placed.values().map(|placed| placed.position.y).fold(f32::INFINITY, f32::min);
    let pelvis_at = shape.bone("Hips").map_or(shape.head_top, |placed| placed.position);
    let feet = Vec3::new(pelvis_at.x, lowest, pelvis_at.z);
    let floor = moly_features::ragdoll_ground::floor_at(world, [feet.x, feet.z]).unwrap_or(feet.y);
    let surroundings = collision::build(world, pelvis_at, feet, height, floor, member);
    let scene = collision::spawn(world, surroundings);

    let (bodies, joints) = spawn_body(world, member, &plans, &joint_plans, &settings, height);
    let parts: Vec<Part> = plans
        .iter()
        .zip(&bodies)
        .map(|(plan, body)| {
            // The bone where it stood, seen from the part laid out on it.
            let frame = Isometry3d::new(plan.position, plan.rotation);
            let bone = shape.bone(&plan.bone).map_or(frame, |placed| Isometry3d::new(placed.position, placed.rotation));
            Part { bone: plan.entity, role: plan.role, body: *body, bone_offset: frame.inverse() * bone, position: plan.position, rotation: plan.rotation, reach: plan.shape.reach() }
        })
        .collect();

    let mut parts = parts.into_iter();
    let root = parts.next().ok_or_else(|| format!("character {unit} cannot fall: its body has no parts"))?;
    let parts: Vec<Part> = parts.collect();

    // The push, read per metre of the character's height.
    if let Some(impulse) = impulse.filter(|impulse| impulse.is_finite()) {
        let mut bodies_in_motion = world.query::<(&mut LinearVelocity, &RigidBody)>();
        if let Ok((mut velocity, kind)) = bodies_in_motion.get_mut(world, bodies[push]) {
            if *kind == RigidBody::Dynamic {
                velocity.0 += impulse * height;
            }
        }
    }

    // The facing the character fell with, on the ground.
    let facing = shape.bone("Hips").map(|placed| placed.rotation).unwrap_or(Quat::IDENTITY) * Vec3::Z;
    let mut drop = Drop {
        root,
        parts,
        joints,
        scene,
        held,
        bones,
        height,
        floor,
        facing: Vec3::new(facing.x, 0.0, facing.z).normalize_or(Vec3::Z),
        pelvis_height: pelvis_at.y - floor,
        at: at.unwrap_or_else(|| "Hips".to_owned()),
        how_long: 0.0,
        seen: Seen::default(),
        getting_up: None,
        seek: None,
        took_from_ai,
    };
    // The animation hands the body over to the ragdoll; another state that
    // took it meanwhile keeps it.
    if let Some(mut driver) = world.get_mut::<MotionDriver>(member) {
        let token = driver.take(BodyOwner::Ragdoll);
        if token.owner() == BodyOwner::Ragdoll {
            drop.seek = Some(driver.player);
        }
    }
    let count = drop.all_parts().count();
    // The body's pose is kept until the character gets up: a look-at leaves
    // the fallen head alone.
    world.entity_mut(member).insert((drop, Falling));
    info!("[ragdoll] character {unit} dropped: {count} parts over {height:.2} m tall, floor at {floor:.3}");
    Ok(Answer { note: format!("character {unit} is falling ({count} parts)"), body: where_from(world, member) })
}

/// The body of a character `member`: one dynamic body per part and the
/// joints between them, in the order of the plans.
fn spawn_body(world: &mut World, member: Entity, plans: &[PartPlan], joint_plans: &[JointPlan], settings: &RagdollSettings, height: f32) -> (Vec<Entity>, Vec<Entity>) {
    let (bodies, inertias) = spawn_parts(world, member, plans, settings);
    let friction = JointDamping { linear: 0.0, angular: joint_friction(height) };
    let mut joints: Vec<Entity> = joint_plans
        .iter()
        .map(|joint| {
            let limit = settings.joint_compliance + limit_compliance(inertias[joint.parent], inertias[joint.child]);
            let entity = spawn_joint(world, joint, plans, &bodies, settings.joint_compliance, limit);
            world.entity_mut(entity).insert(friction);
            entity
        })
        .collect();
    // A shoulder shares its arm's bone, so no joint joins it to the chest it
    // rests against: a joint of no freedom keeps the two as they stand.
    for (index, plan) in plans.iter().enumerate() {
        let Some(apart) = plan.apart else {
            continue;
        };
        let holder = Isometry3d::new(plans[apart].position, plans[apart].rotation);
        let held = Isometry3d::new(plan.position, plan.rotation);
        let steady = FixedJoint::new(bodies[apart], bodies[index])
            .with_local_frame1(holder.inverse() * held)
            .with_local_frame2(Isometry3d::IDENTITY);
        joints.push(world.spawn((steady, JointCollisionDisabled)).id());
    }
    (bodies, joints)
}

/// How far a joint's limits give, against how hard its two parts are to turn:
/// at a give of `g`, a step of the physics takes back one part in `1 + g` of
/// what a part has turned past its limit, a quarter at this give. A limit that gives nothing fights the floor that pushes a
/// limb against it, and the limb shakes in place; one that gives a set amount
/// would give a giant's joints next to nothing to hold and a small body's far
/// too much, so the give follows the parts' own resistance to turning.
const LIMIT_GIVE: f32 = 3.0;

/// How strongly a joint holds back its two parts turning against each other,
/// as a share of the body's own swinging pace: a limb swinging from its joint
/// loses about this much of its swing in each turn of the swing.
///
/// A body's own joints and muscles hold its limbs back as they swing; without
/// that, an arm hanging off a body lying on its side swings on like a
/// pendulum for seconds. A limb swings at a pace that slows with the square
/// root of the body's height, so a set damping would stop a giant's limbs
/// short and leave a small body's flailing: the damping follows that pace.
const JOINT_FRICTION: f32 = 1.0;

/// The rate, per second, at which a body `height` metres tall damps its parts'
/// turning against each other at every joint (see [`JOINT_FRICTION`]).
fn joint_friction(height: f32) -> f32 {
    JOINT_FRICTION * (9.81 / height.max(0.01)).sqrt()
}

/// The compliance of a joint's limits between parts as hard to turn as
/// `first` and `second`, at the physics step of a frame at sixty a second
/// (see [`LIMIT_GIVE`]).
fn limit_compliance(first: f32, second: f32) -> f32 {
    let step = 1.0 / (60.0 * crate::SUBSTEPS as f32);
    LIMIT_GIVE * (1.0 / first + 1.0 / second) * step * step
}

/// One joint's frames on the two parts' bodies, and the joint itself: its
/// point held with `compliance`, its limits with `limit`.
fn spawn_joint(world: &mut World, plan: &JointPlan, plans: &[PartPlan], bodies: &[Entity], compliance: f32, limit: f32) -> Entity {
    let (frame1, frame2) = plan.frames(plans);
    let (parent, child) = (bodies[plan.parent], bodies[plan.child]);
    match plan.limits {
        Limits::Spherical { swing, twist } => {
            let joint = SphericalJoint::new(parent, child)
                .with_local_frame1(frame1)
                .with_local_frame2(frame2)
                .with_swing_limits(-swing.to_radians(), swing.to_radians())
                .with_twist_limits(-twist.to_radians(), twist.to_radians())
                .with_point_compliance(compliance)
                .with_swing_compliance(limit)
                .with_twist_compliance(limit);
            world.spawn((joint, JointCollisionDisabled)).id()
        }
        Limits::Hinge { min, max } => {
            let joint = RevoluteJoint::new(parent, child)
                .with_local_frame1(frame1)
                .with_local_frame2(frame2)
                .with_angle_limits(min.to_radians(), max.to_radians())
                .with_point_compliance(compliance)
                .with_align_compliance(compliance)
                .with_limit_compliance(limit);
            world.spawn((joint, JointCollisionDisabled)).id()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_s_mass_is_centred_at_the_middle_of_its_shape() {
        let plan = PartPlan {
            bone: "LeftArm".to_owned(),
            entity: Entity::PLACEHOLDER,
            role: Role::Arm,
            position: Vec3::new(0.2, 0.8, 0.0),
            rotation: Quat::IDENTITY,
            shape: description::Shape::Capsule { radius: 0.02, from: Vec3::ZERO, to: Vec3::new(0.0, -0.2, 0.0) },
            velocity: Vec3::ZERO,
            apart: None,
        };
        let body = body_of(&plan, &plan.shape.collider(), &RagdollSettings::default(), 0.0);
        // A capsule on its own axis has its middle of mass at its middle, in
        // the part's frame.
        assert!(body.center.distance(Vec3::new(0.0, -0.1, 0.0)) < 1e-5);
        assert!(body.weight > 0.0 && body.inertia > 0.0);
    }

    /// How a body fell, per metre of its height: the lowest any part's frame
    /// went under the floor, the farthest any joint came apart over the whole
    /// fall and over its second half, and the fastest any part moved at the
    /// end.
    #[derive(Debug)]
    struct Fell {
        under: f32,
        apart: f32,
        apart_late: f32,
        speed_end: f32,
        /// The farthest any hinge turned past its limits, in degrees.
        beyond: f32,
        /// When the body had first lain at rest for the settle time, in
        /// seconds of the fall, as a fall reads it to get up.
        settled: Option<f32>,
    }

    /// How a body of a character `tall` metres tall falls on a floor for
    /// `seconds` in a world of its own, stepped at sixty frames a second.
    fn fall_alone(tall: f32, settings: RagdollSettings, seconds: f32) -> Fell {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f32(1.0 / 60.0)));
        crate::add_physics(&mut app);
        app.finish();
        app.cleanup();
        app.update();
        let mut skeleton = crate::tests::chain(tall / 1.25);
        // Bones turned every which way, as a model's own bones are.
        for (index, placed) in skeleton.placed.values_mut().enumerate() {
            placed.rotation = Quat::from_euler(EulerRot::YXZ, 0.7 * index as f32, 1.3, -0.4 * index as f32);
        }
        let (plans, joint_plans) = description::describe(&skeleton).expect("a chain describes");
        let height = description::height_of(&skeleton);
        let world = app.world_mut();
        collision::spawn(world, collision::Scene { bodies: vec![collision::floor(0.0, Vec3::ZERO, height)] });
        let member = world.spawn_empty().id();
        let (bodies, _) = spawn_body(world, member, &plans, &joint_plans, &settings, height);
        let frames = (seconds * 60.0) as u32;
        let mut fell = Fell { under: 0.0, apart: 0.0, apart_late: 0.0, speed_end: 0.0, beyond: 0.0, settled: None };
        let mut resting = 0.0f32;
        // The fall's own measure of rest, on the body it has spawned.
        let reaches: Vec<(Vec3, f32)> = plans.iter().map(|plan| plan.shape.reach()).collect();
        let mut seen = Seen::default();
        for frame in 0..frames {
            app.update();
            let world = app.world();
            let pose = |index: usize| {
                let position = world.get::<Position>(bodies[index]).map_or(Vec3::NAN, |position| position.0);
                let rotation = world.get::<Rotation>(bodies[index]).map_or(Quat::IDENTITY, |rotation| rotation.0);
                Isometry3d::new(position, rotation)
            };
            seen.record(&reaches, (0..plans.len()).map(|index| (Vec3::from(pose(index).translation), pose(index).rotation)).collect(), 1.0 / 60.0);
            resting = if seen.fastest() < settings.rest_speed(height) { resting + 1.0 / 60.0 } else { 0.0 };
            if resting >= settings.settle_seconds && fell.settled.is_none() {
                fell.settled = Some(frame as f32 / 60.0);
            }
            for index in 0..plans.len() {
                fell.under = fell.under.max(-Vec3::from(pose(index).translation).y);
            }
            for joint in &joint_plans {
                let (frame1, frame2) = joint.frames(&plans);
                let gap = Vec3::from((pose(joint.parent) * frame1).translation).distance(Vec3::from((pose(joint.child) * frame2).translation));
                let gap = if gap.is_finite() { gap } else { f32::INFINITY };
                fell.apart = fell.apart.max(gap);
                if frame >= frames / 2 {
                    fell.apart_late = fell.apart_late.max(gap);
                }
                if let Limits::Hinge { min, max } = joint.limits {
                    // The turn from the frame on the part above to the one on
                    // the part below, about the hinge (the frames' Z).
                    let turn = (pose(joint.parent) * frame1).rotation.inverse() * (pose(joint.child) * frame2).rotation;
                    let angle = (2.0 * turn.z.atan2(turn.w)).to_degrees();
                    let angle = if angle > 180.0 { angle - 360.0 } else if angle < -180.0 { angle + 360.0 } else { angle };
                    fell.beyond = fell.beyond.max(angle - max).max(min - angle);
                }
            }
        }
        fell.speed_end = seen.fastest();
        Fell { under: fell.under / height, apart: fell.apart / height, apart_late: fell.apart_late / height, speed_end: fell.speed_end / height, beyond: fell.beyond, settled: fell.settled }
    }

    #[test]
    fn a_ball_turns_as_a_solid_ball_of_its_weight() {
        let plan = PartPlan {
            bone: "Head".to_owned(),
            entity: Entity::PLACEHOLDER,
            role: Role::Head,
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            shape: description::Shape::Sphere { radius: 0.05 },
            velocity: Vec3::ZERO,
            apart: None,
        };
        let body = body_of(&plan, &plan.shape.collider(), &RagdollSettings::default(), 0.0);
        // A solid ball of mass m and radius r turns with 2/5 m r squared.
        let expected = 0.4 * body.weight * 0.05 * 0.05;
        assert!((body.inertia - expected).abs() < expected * 1e-3, "{} against {expected}", body.inertia);
    }


    /// Where a ball joint held to a fixed point, with a narrow cone and a wide
    /// twist, brings a body turned `start` from the joint's rest pose after a
    /// second without gravity.
    fn held_by_a_ball_joint(start: Quat) -> Quat {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f32(1.0 / 60.0)));
        crate::add_physics(&mut app);
        app.insert_resource(Gravity(Vec3::ZERO));
        app.finish();
        app.cleanup();
        app.update();
        let world = app.world_mut();
        let fixed = world.spawn((RigidBody::Static, Transform::IDENTITY)).id();
        let start_at = Transform::from_rotation(start);
        let body = world
            .spawn((
                RigidBody::Dynamic,
                start_at,
                GlobalTransform::from(start_at),
                Rotation::from(start),
                Mass(1.0),
                AngularInertia::new(Vec3::splat(0.01)),
                NoAutoMass,
                NoAutoAngularInertia,
                AngularDamping(5.0),
                SleepingDisabled,
            ))
            .id();
        let joint = SphericalJoint::new(fixed, body).with_swing_limits(-0.1, 0.1).with_twist_limits(-1.5, 1.5);
        world.spawn((joint, JointCollisionDisabled));
        for _ in 0..60 {
            app.update();
        }
        app.world().get::<Rotation>(body).expect("the body has a rotation").0
    }

    #[test]
    fn a_ball_joint_s_cone_opens_round_its_frame_s_negative_z() {
        // The joint frames lay a limb along their -Z (see `JointPlan::frames`)
        // because that is the axis the physics measures a ball joint's swing
        // from. Turned about -Z, a body only twists, which the wide twist
        // limit lets it keep; turned about Y or X, it swings, and the narrow
        // cone brings it back.
        let turned = |axis: Vec3| {
            let end = held_by_a_ball_joint(Quat::from_axis_angle(axis, 0.6));
            end.angle_between(Quat::IDENTITY)
        };
        let about_z = turned(Vec3::NEG_Z);
        assert!((about_z - 0.6).abs() < 0.05, "a twist about -Z ends {about_z} rad from rest");
        for axis in [Vec3::Y, Vec3::X] {
            let swing = turned(axis);
            assert!(swing < 0.15, "a swing about {axis} ends {swing} rad from rest");
        }
    }

    #[test]
    fn a_body_falls_and_holds_together_at_any_size() {
        // A fall takes longer the taller the body, as the square root of its
        // height: eight seconds for a body under a metre tall.
        let falls: Vec<(f32, Fell)> = [0.72, 1.6, 20.0].into_iter().map(|tall| (tall, fall_alone(tall, RagdollSettings::default(), 8.0 * (tall / 0.72f32).sqrt()))).collect();
        for (tall, fell) in &falls {
            println!("{tall} m tall: {fell:?}");
        }
        // Every size lands on the floor rather than through it, its joints
        // hold once it has landed, its hinges stay within a few degrees of
        // their limits, and it ends slower than the speed the fall takes for
        // rest.
        let rest = RagdollSettings::default().settle_speed;
        for (tall, fell) in &falls {
            assert!(fell.under < 0.05 && fell.apart_late < 0.01 && fell.beyond < 10.0 && fell.speed_end < rest, "a body {tall} m tall fell as {fell:?}");
        }
    }

    #[test]
    fn a_bone_is_carried_by_its_part_as_it_turns() {
        // A part laid out level on a bone turned a quarter about z.
        let bone = Isometry3d::new(Vec3::new(0.2, 0.8, 0.0), Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        let frame = Isometry3d::new(Vec3::new(0.2, 0.8, 0.0), Quat::IDENTITY);
        let part = Part { bone: Entity::PLACEHOLDER, role: Role::Arm, body: Entity::PLACEHOLDER, bone_offset: frame.inverse() * bone, position: Vec3::from(frame.translation), rotation: frame.rotation, reach: (Vec3::ZERO, 0.0) };
        // Where it fell, the bone is back where it stood.
        let back = part.bone_at(Vec3::from(frame.translation), frame.rotation);
        assert!(Vec3::from(back.translation).distance(Vec3::from(bone.translation)) < 1e-5);
        assert!(back.rotation.angle_between(bone.rotation) < 1e-3);
        // With the part turned a half about y, the bone turns with it.
        let turn = Quat::from_rotation_y(std::f32::consts::PI);
        let turned = part.bone_at(Vec3::new(1.0, 0.0, 0.0), turn);
        assert!(turned.rotation.angle_between(turn * bone.rotation) < 1e-3);
    }
}
