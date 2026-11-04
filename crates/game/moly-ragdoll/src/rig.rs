//! Reading a character's bones as they stand: which humanoid bones its model
//! has, where each one is in the world, and the crown of its head.
//!
//! A character is a roster member entity with the character model as its
//! child, and the model's scene holds the bones as named nodes. Bones are read
//! by name inside that member's own tree only, so another member's bones (or a
//! furniture node of the same name) never mix in. The first node of a name in
//! the member's tree is taken. A bone the character lacks is a refusal at the
//! drop that names the bones; a missing neck or crown only means the head is
//! sized from the head bone itself.

use std::collections::HashMap;

use bevy::prelude::*;

use moly_features::character::CharacterModel;

/// A bone the description uses, and the names a character's export may give
/// it, the likeliest first.
#[derive(Clone, Copy, Debug)]
pub struct Probe {
    pub name: &'static str,
    pub names: &'static [&'static str],
}

/// The chain from the pelvis to the head's crown, and the bones each limb
/// reaches: the upper bone, the joint bone and the end bone.
pub const PROBES: &[Probe] = &[
    Probe { name: "Hips", names: &["Hips"] },
    Probe { name: "Spine", names: &["Spine"] },
    Probe { name: "Spine1", names: &["Spine1", "Chest"] },
    Probe { name: "Neck", names: &["Neck"] },
    Probe { name: "Head", names: &["Head"] },
    Probe { name: "HeadRoot", names: &["HeadRoot"] },
    Probe { name: "LeftArm", names: &["LeftArm", "Arm.L", "UpperArm.L"] },
    Probe { name: "LeftForeArm", names: &["LeftForeArm", "ForeArm.L", "LowerArm.L"] },
    Probe { name: "LeftHand", names: &["LeftHand", "Hand.L"] },
    Probe { name: "RightArm", names: &["RightArm", "Arm.R", "UpperArm.R"] },
    Probe { name: "RightForeArm", names: &["RightForeArm", "ForeArm.R", "LowerArm.R"] },
    Probe { name: "RightHand", names: &["RightHand", "Hand.R"] },
    Probe { name: "LeftUpLeg", names: &["LeftUpLeg", "UpLeg.L", "Thigh.L"] },
    Probe { name: "LeftLeg", names: &["LeftLeg", "Leg.L", "Calf.L"] },
    Probe { name: "LeftFoot", names: &["LeftFoot", "Foot.L"] },
    Probe { name: "RightUpLeg", names: &["RightUpLeg", "UpLeg.R", "Thigh.R"] },
    Probe { name: "RightLeg", names: &["RightLeg", "Leg.R", "Calf.R"] },
    Probe { name: "RightFoot", names: &["RightFoot", "Foot.R"] },
];

/// The probes a character may lack: the neck and the crown only size and place
/// the head, and the hands and the feet only lengthen a limb.
const OPTIONAL: &[&str] = &["Neck", "HeadRoot", "LeftHand", "RightHand", "LeftFoot", "RightFoot"];

/// One bone as the character's tree holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bone {
    pub entity: Entity,
    /// The largest of the bone's world scale axes: the scale its lengths carry
    /// on a character standing at one uniform scale.
    pub scale: f32,
}

/// The bones of one character that its model has, by probe name.
#[derive(Clone, Debug, Default)]
pub struct Bones {
    found: HashMap<&'static str, Bone>,
}

impl Bones {
    /// A bone by probe name.
    pub fn get(&self, name: &str) -> Option<Bone> {
        self.found.get(name).copied()
    }

    /// The entity of a bone by probe name.
    pub fn entity(&self, name: &str) -> Option<Entity> {
        self.get(name).map(|bone| bone.entity)
    }

    /// The largest scale any of the bones stands at: a character stands at one
    /// uniform scale, so this is the scale of all of it.
    pub fn scale(&self) -> f32 {
        self.found.values().map(|bone| bone.scale).fold(0.0f32, f32::max)
    }

    /// The bones' names, for the answers.
    pub fn names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.found.keys().copied().collect();
        names.sort_unstable();
        names
    }
}

/// Why a character cannot fall: no model on screen, or the bones it lacks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missing(pub String);

/// The named nodes of a tree, from the model down, in no order.
type Named = HashMap<String, Entity>;

/// Reads the bones of the character `member` from the world between the
/// frames: a request carried out at once has no system's queries to read with.
pub fn read_in_world(world: &mut World, member: Entity) -> Result<Bones, Missing> {
    let member_children = world.get::<Children>(member).map(|kids| kids.to_vec()).unwrap_or_default();
    let model = member_children.into_iter().find(|kid| world.get::<CharacterModel>(*kid).is_some()).ok_or_else(|| Missing("it has no model on screen".to_owned()))?;
    let mut by_name: Named = HashMap::new();
    let mut stack = vec![model];
    while let Some(entity) = stack.pop() {
        if let Some(name) = world.get::<Name>(entity) {
            by_name.entry(name.as_str().to_owned()).or_insert(entity);
        }
        if let Some(kids) = world.get::<Children>(entity) {
            stack.extend(kids.iter());
        }
    }
    let mut found = HashMap::new();
    let mut absent = Vec::new();
    for probe in PROBES {
        match probe.names.iter().find_map(|name| by_name.get(*name).copied()) {
            Some(entity) => {
                let scale = world.get::<GlobalTransform>(entity).map_or(1.0, |global| global.scale().abs().max_element());
                found.insert(probe.name, Bone { entity, scale });
            }
            None => absent.push(probe.name),
        }
    }
    missing(found, absent)
}

/// A reading with the bones a tree lacks.
fn missing(found: HashMap<&'static str, Bone>, absent: Vec<&'static str>) -> Result<Bones, Missing> {
    let required: Vec<&str> = absent.into_iter().filter(|name| !OPTIONAL.contains(name)).collect();
    if !required.is_empty() {
        return Err(Missing(format!("its skeleton has no {} bone", required.join(" bone, no "))));
    }
    Ok(Bones { found })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe of a bone by the name the description uses.
    fn probe(name: &str) -> Option<&'static Probe> {
        PROBES.iter().find(|probe| probe.name == name)
    }

    /// The chain every character is expected to have.
    const CHAIN: &[&str] = &["Hips", "Spine", "Spine1", "Neck", "Head", "LeftArm", "LeftForeArm", "RightArm", "RightForeArm", "LeftUpLeg", "LeftLeg", "RightUpLeg", "RightLeg"];

    /// A member with its model, whose bones are then spawned by `put`, and the
    /// bones as the reading finds them.
    fn read_model(world: &mut World, put: impl FnOnce(&mut World, Entity)) -> Result<Bones, Missing> {
        let member = world.spawn(Name::new("member")).id();
        let model = world.spawn((Name::new("model"), CharacterModel, ChildOf(member), Transform::IDENTITY, GlobalTransform::IDENTITY)).id();
        put(world, model);
        // The parent-child hook leaves the members' child lists to the
        // deferred commands, which a frame would apply.
        world.flush();
        read_in_world(world, member)
    }

    /// The chain with every bone standing at `scale`.
    fn read_chain(world: &mut World, bones: &[&'static str], scale: f32) -> Result<Bones, Missing> {
        read_model(world, |world, model| {
            for name in bones {
                world.spawn((Name::new(*name), ChildOf(model), Transform::from_scale(Vec3::splat(scale)), GlobalTransform::from_scale(Vec3::splat(scale))));
            }
        })
    }

    #[test]
    fn every_part_bone_has_a_probe_and_only_the_crown_is_optional() {
        for name in ["Hips", "Spine1", "Head", "LeftArm", "LeftForeArm", "LeftUpLeg", "LeftLeg", "RightArm", "RightForeArm", "RightUpLeg", "RightLeg"] {
            let probe = probe(name).unwrap_or_else(|| panic!("{name} has no probe"));
            assert!(!OPTIONAL.contains(&probe.name), "{name} is required");
        }
        assert!(probe("HeadRoot").is_some_and(|probe| OPTIONAL.contains(&probe.name)));
    }

    #[test]
    fn a_chain_is_read_by_the_names_it_has() {
        let mut world = World::new();
        let bones = read_chain(&mut world, CHAIN, 1.0).expect("a full chain reads");
        let mut expected = CHAIN.to_vec();
        expected.sort_unstable();
        assert_eq!(bones.names(), expected);
        assert!(bones.entity("LeftForeArm").is_some());
    }

    #[test]
    fn a_bone_the_character_lacks_is_named() {
        let mut world = World::new();
        let absent: Vec<&str> = CHAIN.iter().copied().filter(|name| *name != "LeftArm" && *name != "RightLeg").collect();
        let missing = read_chain(&mut world, &absent, 1.0).unwrap_err();
        assert_eq!(missing.0, "its skeleton has no LeftArm bone, no RightLeg bone");
    }

    #[test]
    fn the_neck_and_the_crown_may_be_absent() {
        let mut world = World::new();
        let present: Vec<&str> = CHAIN.iter().copied().filter(|name| *name != "Neck").collect();
        assert!(read_chain(&mut world, &present, 1.0).is_ok());
    }

    #[test]
    fn a_body_that_stands_bigger_reads_at_its_scale() {
        let mut world = World::new();
        // The model's scene is drawn three times the size and the bones are
        // twice the size of their own model: the body stands six times as big.
        let bones = read_model(&mut world, |world, model| {
            world.entity_mut(model).insert((Transform::from_scale(Vec3::splat(3.0)), GlobalTransform::from_scale(Vec3::splat(3.0))));
            for name in CHAIN {
                world.spawn((Name::new(*name), ChildOf(model), Transform::from_scale(Vec3::splat(2.0)), GlobalTransform::from_scale(Vec3::splat(6.0))));
            }
        })
        .expect("a full chain reads");
        assert!((bones.scale() - 6.0).abs() < 1e-6);
    }

    #[test]
    fn a_model_drawn_bigger_reads_at_the_model_s_scale() {
        let mut world = World::new();
        let bones = read_model(&mut world, |world, model| {
            world.entity_mut(model).insert((Transform::from_scale(Vec3::splat(3.0)), GlobalTransform::from_scale(Vec3::splat(3.0))));
            for name in CHAIN {
                world.spawn((Name::new(*name), ChildOf(model), Transform::IDENTITY, GlobalTransform::from_scale(Vec3::splat(3.0))));
            }
        })
        .expect("a full chain reads");
        assert!((bones.scale() - 3.0).abs() < 1e-6);
    }

    #[test]
    fn a_member_without_a_model_is_refused() {
        let mut world = World::new();
        let member = world.spawn(Name::new("member")).id();
        let missing = read_in_world(&mut world, member).unwrap_err();
        assert_eq!(missing.0, "it has no model on screen");
    }
}
