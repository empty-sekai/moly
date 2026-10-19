//! The two bone vocabularies the OC body joins: the humanoid bone names a
//! VRM file declares (the VRM 1.0 set; VRM 0.x thumbs are renamed into it),
//! and the sixteen joints of the player avatar's own skeleton that every
//! player clip animates.
//!
//! A plain glTF file has no declared map, so [`bone_from_node_name`] reads
//! the bone from the node name for the common rig naming families: Unity and
//! Mixamo style (`Hips`, `LeftUpLeg`, `mixamorig:LeftArm`), VRoid style
//! (`J_Bip_L_UpperArm`) and Blender Rigify deform bones (`DEF-thigh.L`).

/// The VRM 1.0 humanoid bone names.
pub const HUMAN_BONES: [&str; 55] = [
    "hips",
    "spine",
    "chest",
    "upperChest",
    "neck",
    "head",
    "leftEye",
    "rightEye",
    "jaw",
    "leftUpperLeg",
    "leftLowerLeg",
    "leftFoot",
    "leftToes",
    "rightUpperLeg",
    "rightLowerLeg",
    "rightFoot",
    "rightToes",
    "leftShoulder",
    "leftUpperArm",
    "leftLowerArm",
    "leftHand",
    "rightShoulder",
    "rightUpperArm",
    "rightLowerArm",
    "rightHand",
    "leftThumbMetacarpal",
    "leftThumbProximal",
    "leftThumbDistal",
    "leftIndexProximal",
    "leftIndexIntermediate",
    "leftIndexDistal",
    "leftMiddleProximal",
    "leftMiddleIntermediate",
    "leftMiddleDistal",
    "leftRingProximal",
    "leftRingIntermediate",
    "leftRingDistal",
    "leftLittleProximal",
    "leftLittleIntermediate",
    "leftLittleDistal",
    "rightThumbMetacarpal",
    "rightThumbProximal",
    "rightThumbDistal",
    "rightIndexProximal",
    "rightIndexIntermediate",
    "rightIndexDistal",
    "rightMiddleProximal",
    "rightMiddleIntermediate",
    "rightMiddleDistal",
    "rightRingProximal",
    "rightRingIntermediate",
    "rightRingDistal",
    "rightLittleProximal",
    "rightLittleIntermediate",
    "rightLittleDistal",
];

/// The bones every humanoid must have (VRM 1.0 humanoid; VRM 0.x also
/// requires chest and neck, which the retarget does not need).
pub const REQUIRED_BONES: [&str; 15] = [
    "hips",
    "spine",
    "head",
    "leftUpperLeg",
    "leftLowerLeg",
    "leftFoot",
    "rightUpperLeg",
    "rightLowerLeg",
    "rightFoot",
    "leftUpperArm",
    "leftLowerArm",
    "leftHand",
    "rightUpperArm",
    "rightLowerArm",
    "rightHand",
];

/// The canonical name of a bone name as a file writes it: a VRM 1.0 name as
/// it is, a VRM 0.x thumb renamed to the VRM 1.0 thumb chain
/// (Proximal, Intermediate, Distal become Metacarpal, Proximal, Distal).
pub fn canonical(name: &str, vrm0: bool) -> Option<&'static str> {
    let renamed = if vrm0 {
        match name {
            "leftThumbProximal" => "leftThumbMetacarpal",
            "leftThumbIntermediate" => "leftThumbProximal",
            "rightThumbProximal" => "rightThumbMetacarpal",
            "rightThumbIntermediate" => "rightThumbProximal",
            other => other,
        }
    } else {
        name
    };
    HUMAN_BONES.iter().copied().find(|bone| *bone == renamed)
}

/// The finger bones and their relaxed curl (degrees about the hand's
/// lateral axis, applied at each joint and accumulated along the finger).
/// The player avatar has no fingers and its clips animate none, so a static
/// relaxed hand is the closest pose; thumbs keep their rest.
pub const FINGER_CURL: [(&str, f32); 24] = [
    ("leftIndexProximal", 10.0),
    ("leftIndexIntermediate", 15.0),
    ("leftIndexDistal", 10.0),
    ("leftMiddleProximal", 12.0),
    ("leftMiddleIntermediate", 17.0),
    ("leftMiddleDistal", 12.0),
    ("leftRingProximal", 14.0),
    ("leftRingIntermediate", 19.0),
    ("leftRingDistal", 14.0),
    ("leftLittleProximal", 16.0),
    ("leftLittleIntermediate", 21.0),
    ("leftLittleDistal", 16.0),
    ("rightIndexProximal", 10.0),
    ("rightIndexIntermediate", 15.0),
    ("rightIndexDistal", 10.0),
    ("rightMiddleProximal", 12.0),
    ("rightMiddleIntermediate", 17.0),
    ("rightMiddleDistal", 12.0),
    ("rightRingProximal", 14.0),
    ("rightRingIntermediate", 19.0),
    ("rightRingDistal", 14.0),
    ("rightLittleProximal", 16.0),
    ("rightLittleIntermediate", 21.0),
    ("rightLittleDistal", 16.0),
];

/// The player avatar's joints that the retarget reads, in parent-first
/// order. `Hand` is the avatar's hand socket at the end of the forearm (it
/// deforms nothing; tools and props mount on it), `Foot` the leg tip on the
/// ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Root,
    Hips,
    Spine,
    Head,
    UpperArmL,
    LowerArmL,
    HandL,
    UpperArmR,
    LowerArmR,
    HandR,
    UpperLegL,
    LowerLegL,
    FootL,
    UpperLegR,
    LowerLegR,
    FootR,
}

pub const SLOT_COUNT: usize = 16;

impl Slot {
    pub const ALL: [Slot; SLOT_COUNT] = [
        Slot::Root,
        Slot::Hips,
        Slot::Spine,
        Slot::Head,
        Slot::UpperArmL,
        Slot::LowerArmL,
        Slot::HandL,
        Slot::UpperArmR,
        Slot::LowerArmR,
        Slot::HandR,
        Slot::UpperLegL,
        Slot::LowerLegL,
        Slot::FootL,
        Slot::UpperLegR,
        Slot::LowerLegR,
        Slot::FootR,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The node name of this joint in the player avatar's model. The right
    /// leg's upper and lower bones are spelled `UpReg_R` and `Reg_R` there.
    pub fn avatar_name(self) -> &'static str {
        match self {
            Slot::Root => "Root",
            Slot::Hips => "Hips",
            Slot::Spine => "Spine",
            Slot::Head => "Head",
            Slot::UpperArmL => "Arm_L",
            Slot::LowerArmL => "ForeArm_L",
            Slot::HandL => "Penlight_L",
            Slot::UpperArmR => "Arm_R",
            Slot::LowerArmR => "ForeArm_R",
            Slot::HandR => "Penlight_R",
            Slot::UpperLegL => "UpLeg_L",
            Slot::LowerLegL => "Leg_L",
            Slot::FootL => "Foot_L",
            Slot::UpperLegR => "UpReg_R",
            Slot::LowerLegR => "Reg_R",
            Slot::FootR => "Foot_R",
        }
    }

    /// The joint's parent among the slots (`None` for the root).
    pub fn parent(self) -> Option<Slot> {
        Some(match self {
            Slot::Root => return None,
            Slot::Hips => Slot::Root,
            Slot::Spine => Slot::Hips,
            Slot::Head | Slot::UpperArmL | Slot::UpperArmR => Slot::Spine,
            Slot::LowerArmL => Slot::UpperArmL,
            Slot::HandL => Slot::LowerArmL,
            Slot::LowerArmR => Slot::UpperArmR,
            Slot::HandR => Slot::LowerArmR,
            Slot::UpperLegL | Slot::UpperLegR => Slot::Hips,
            Slot::LowerLegL => Slot::UpperLegL,
            Slot::FootL => Slot::LowerLegL,
            Slot::LowerLegR => Slot::UpperLegR,
            Slot::FootR => Slot::LowerLegR,
        })
    }

    /// The humanoid bone this joint drives by rotation (`None`: the root,
    /// whose motion reaches the humanoid through the hips, and the hand
    /// sockets, which carry props and not the hand).
    pub fn driven_bone(self) -> Option<&'static str> {
        Some(match self {
            Slot::Root | Slot::HandL | Slot::HandR => return None,
            Slot::Hips => "hips",
            Slot::Spine => "spine",
            Slot::Head => "head",
            Slot::UpperArmL => "leftUpperArm",
            Slot::LowerArmL => "leftLowerArm",
            Slot::UpperArmR => "rightUpperArm",
            Slot::LowerArmR => "rightLowerArm",
            Slot::UpperLegL => "leftUpperLeg",
            Slot::LowerLegL => "leftLowerLeg",
            Slot::FootL => "leftFoot",
            Slot::UpperLegR => "rightUpperLeg",
            Slot::LowerLegR => "rightLowerLeg",
            Slot::FootR => "rightFoot",
        })
    }

    /// The humanoid joint this avatar joint sits on once the avatar's
    /// skeleton is fitted to the OC (`None` for the root).
    pub fn socket_bone(self) -> Option<&'static str> {
        match self {
            Slot::HandL => Some("leftHand"),
            Slot::HandR => Some("rightHand"),
            other => other.driven_bone(),
        }
    }
}

/// Lower-case alphanumerics of a node name.
fn squash(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
    Centre,
}

/// The side of a node name and the name with the side marker removed.
fn split_side(name: &str) -> (Side, String) {
    let lower = name.to_ascii_lowercase();
    for (marker, side) in [("left", Side::Left), ("right", Side::Right)] {
        if let Some(at) = lower.find(marker) {
            let mut rest = lower.clone();
            rest.replace_range(at..at + marker.len(), "");
            return (side, rest);
        }
    }
    // Separated single letters: `_L_`, `.L`, `_l`, ` L`, `L_` at the start.
    let bytes: Vec<char> = lower.chars().collect();
    let is_sep = |c: char| !c.is_ascii_alphanumeric();
    for i in 0..bytes.len() {
        let c = bytes[i];
        if c != 'l' && c != 'r' {
            continue;
        }
        let before = i == 0 || is_sep(bytes[i - 1]);
        let after = i + 1 == bytes.len() || is_sep(bytes[i + 1]);
        if before && after && (i > 0 || bytes.len() > 1) {
            let side = if c == 'l' { Side::Left } else { Side::Right };
            let rest: String = bytes
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, c)| *c)
                .collect();
            return (side, rest);
        }
    }
    (Side::Centre, lower)
}

/// The humanoid bone a node name stands for, or `None`.
pub fn bone_from_node_name(name: &str) -> Option<&'static str> {
    let trimmed = name
        .trim_start_matches("mixamorig:")
        .trim_start_matches("mixamorig_");
    let rigify = trimmed.starts_with("DEF-");
    let trimmed = trimmed.trim_start_matches("DEF-");
    let (side, rest) = split_side(trimmed);
    let mut core = squash(&rest);
    for prefix in ["jbipc", "jbip", "bip01", "bip001", "bip"] {
        if let Some(stripped) = core.strip_prefix(prefix) {
            core = stripped.to_owned();
            break;
        }
    }
    if rigify {
        return rigify_bone(side, &core);
    }
    let centre = match core.as_str() {
        "hips" | "hip" | "pelvis" => Some("hips"),
        "spine" => Some("spine"),
        "chest" | "spine1" => Some("chest"),
        "upperchest" | "spine2" => Some("upperChest"),
        "neck" => Some("neck"),
        "head" => Some("head"),
        _ => None,
    };
    if side == Side::Centre {
        return centre;
    }
    let left = side == Side::Left;
    let pick = |l: &'static str, r: &'static str| Some(if left { l } else { r });
    let core = core.strip_prefix("hand").filter(|c| !c.is_empty()).unwrap_or(&core).to_owned();
    match core.as_str() {
        "shoulder" | "clavicle" => pick("leftShoulder", "rightShoulder"),
        "upperarm" | "arm" | "uparm" => pick("leftUpperArm", "rightUpperArm"),
        "lowerarm" | "forearm" | "elbow" => pick("leftLowerArm", "rightLowerArm"),
        "hand" | "wrist" => pick("leftHand", "rightHand"),
        "upperleg" | "upleg" | "thigh" => pick("leftUpperLeg", "rightUpperLeg"),
        "lowerleg" | "leg" | "shin" | "calf" | "knee" => pick("leftLowerLeg", "rightLowerLeg"),
        "foot" | "ankle" => pick("leftFoot", "rightFoot"),
        "toes" | "toebase" | "toe" => pick("leftToes", "rightToes"),
        "eye" => pick("leftEye", "rightEye"),
        finger => finger_bone(left, finger),
    }
}

fn finger_bone(left: bool, core: &str) -> Option<&'static str> {
    let (stem, number) = core.split_at(core.trim_end_matches(|c: char| c.is_ascii_digit()).len());
    let joint = match number {
        "1" => 0,
        "2" => 1,
        "3" => 2,
        _ => return None,
    };
    let finger = match stem {
        "thumb" => 0,
        "index" => 1,
        "middle" => 2,
        "ring" => 3,
        "little" | "pinky" => 4,
        _ => return None,
    };
    let base = if left { 25 } else { 40 };
    Some(HUMAN_BONES[base + finger * 3 + joint])
}

/// Rigify deform bones: the spine chain is `spine` (hips) up to
/// `spine.006` (head); limbs are `upper_arm`, `forearm`, `hand`, `thigh`,
/// `shin`, `foot`, `toe`, `shoulder`; the first segment of a split limb
/// carries the bone.
fn rigify_bone(side: Side, core: &str) -> Option<&'static str> {
    if side == Side::Centre {
        return match core {
            "spine" => Some("hips"),
            "spine001" => Some("spine"),
            "spine002" => Some("chest"),
            "spine003" => Some("upperChest"),
            "spine004" => Some("neck"),
            "spine006" => Some("head"),
            _ => None,
        };
    }
    let left = side == Side::Left;
    let pick = |l: &'static str, r: &'static str| Some(if left { l } else { r });
    match core {
        "shoulder" => pick("leftShoulder", "rightShoulder"),
        "upperarm" => pick("leftUpperArm", "rightUpperArm"),
        "forearm" => pick("leftLowerArm", "rightLowerArm"),
        "hand" => pick("leftHand", "rightHand"),
        "thigh" => pick("leftUpperLeg", "rightUpperLeg"),
        "shin" => pick("leftLowerLeg", "rightLowerLeg"),
        "foot" => pick("leftFoot", "rightFoot"),
        "toe" => pick("leftToes", "rightToes"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rig_name_families_map_to_the_same_bones() {
        let cases = [
            ("Hips", Some("hips")),
            ("mixamorig:LeftArm", Some("leftUpperArm")),
            ("mixamorig:RightForeArm", Some("rightLowerArm")),
            ("LeftUpLeg", Some("leftUpperLeg")),
            ("LeftLeg", Some("leftLowerLeg")),
            ("Spine1", Some("chest")),
            ("J_Bip_C_UpperChest", Some("upperChest")),
            ("J_Bip_L_UpperArm", Some("leftUpperArm")),
            ("J_Bip_R_LowerLeg", Some("rightLowerLeg")),
            ("J_Bip_L_Index2", Some("leftIndexIntermediate")),
            ("LeftHandPinky3", Some("leftLittleDistal")),
            ("DEF-spine", Some("hips")),
            ("DEF-spine.003", Some("upperChest")),
            ("DEF-thigh.L", Some("leftUpperLeg")),
            ("DEF-forearm.R", Some("rightLowerArm")),
            ("LeftForeArmTwist", None),
            ("LeftArm_end", None),
            ("Armature", None),
        ];
        for (name, bone) in cases {
            assert_eq!(bone_from_node_name(name), bone, "{name}");
        }
    }

    #[test]
    fn vrm0_thumbs_move_into_the_vrm1_chain() {
        assert_eq!(canonical("leftThumbProximal", true), Some("leftThumbMetacarpal"));
        assert_eq!(canonical("leftThumbIntermediate", true), Some("leftThumbProximal"));
        assert_eq!(canonical("leftThumbProximal", false), Some("leftThumbProximal"));
        assert_eq!(canonical("leftThumbIntermediate", false), None);
    }

    #[test]
    fn slot_tables_are_parent_first_and_name_the_avatar_joints() {
        for (index, slot) in Slot::ALL.iter().enumerate() {
            assert_eq!(slot.index(), index);
            if let Some(parent) = slot.parent() {
                assert!(parent.index() < index, "{slot:?}");
            }
        }
        assert_eq!(Slot::UpperLegR.avatar_name(), "UpReg_R");
        assert_eq!(Slot::HandR.socket_bone(), Some("rightHand"));
    }
}
