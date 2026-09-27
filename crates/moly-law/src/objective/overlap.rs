//! NPCAvatarPresenter.TryCancelIfCharacterOverlap 的累计/取消门。
//! 真源先比较旧累计值，再加本帧 dt；取消失败不清计时。

use super::{ObjectiveType, TalkType};

/// IsSomeCharacterTalkData：并非「有任意对话数据」都豁免。
pub fn has_group_talk(current: Option<ObjectiveType>, slot: Option<TalkType>) -> bool {
    slot.is_some()
        && (matches!(current, Some(ObjectiveType::MainSomeCharacterCommunication | ObjectiveType::SubSomeCharacterCommunication))
            || matches!(slot, Some(TalkType::MultipleCharacterFixture | TalkType::CommunicationWhileDoingWait)))
}

/// The action states in which the overlap cancel may run: the bits of
/// 0x100083 = {Idle 0, AutoMove 1, Rest 7, SomeCharacterCommunication 20}.
pub const OVERLAP_CANCEL_STATE_MASK: u32 = 0x0010_0083;

/// One frame of the timer before the cancel attempt. Reset to 0 and
/// false on group talk data or no overlap; while the timer read before this
/// frame's add is below `limit`, add `dt` and false; at or past it, reset
/// and false in a state outside the mask (a state above 20 included), and
/// true (attempt the cancel, timer untouched) in a state inside it. The
/// caller resets the timer only when the cancel reports true: a refused
/// cancel keeps the timer at or past the limit, so the next frame tries
/// again.
pub fn overlap_frame(
    elapsed: &mut f32,
    dt: f32,
    limit: f32,
    overlaps: bool,
    group_talk: bool,
    state_type: u32,
) -> bool {
    if group_talk || !overlaps {
        *elapsed = 0.0;
        return false;
    }
    // One ordered compare, `b.pl` when not below (a NaN timer included).
    if *elapsed < limit {
        *elapsed += dt;
        return false;
    }
    if state_type > 20 || OVERLAP_CANCEL_STATE_MASK & (1 << state_type) == 0 {
        *elapsed = 0.0;
        return false;
    }
    true
}

/// `state_type` 是 NPCActionStateType：Idle=0、AutoMove=1、Rest=7、
/// SomeCharacterCommunication=20；其他状态达到时限后清零而不取消。
pub fn should_cancel(
    elapsed: &mut f32,
    dt: f32,
    limit: f32,
    overlaps: bool,
    group_talk: bool,
    state_type: u8,
    cancellable: bool,
) -> bool {
    if group_talk || !overlaps {
        *elapsed = 0.0;
        return false;
    }
    if *elapsed < limit {
        *elapsed += dt;
        return false;
    }
    if matches!(state_type, 0 | 1 | 7 | 20) {
        if !cancellable {
            return false;
        }
        *elapsed = 0.0;
        return true;
    }
    *elapsed = 0.0;
    false
}

/// 名册顺序的邻居：资格读髋骨距离，透明值读根节点距离。
#[derive(Clone, Copy)]
pub struct DitherNeighbor {
    pub hips_distance: f32,
    pub root_distance: f32,
}

/// NPC 距离透明支；玩家与相机透明由各自分支另算并取最小值。
pub fn npc_dither_alpha(talking: bool, photo: bool, fixture_action: bool, others: &[DitherNeighbor]) -> f32 {
    if talking || photo || fixture_action || !others.iter().any(|other| other.hips_distance < 0.46) {
        return 1.0;
    }
    npc_root_alpha(others.iter().map(|other| other.root_distance))
}

/// The NPC branch's alpha once another NPC is near: the root distance of
/// each other NPC in list order until the first under 0.46 (0 before the
/// first); under 0.46 it is `min(d / 0.46, 1) * 0.8 + 0.2` (0.2 when the
/// quotient is negative), else 1.0.
pub fn npc_root_alpha(root_distances: impl IntoIterator<Item = f32>) -> f32 {
    let mut distance = 0.0f32;
    for d in root_distances {
        distance = d;
        if d < NPC_DITHER_DISTANCE {
            break;
        }
    }
    if distance < NPC_DITHER_DISTANCE {
        let t = distance / NPC_DITHER_DISTANCE;
        if t >= 0.0 {
            arm_fmin(t, 1.0) * DITHER_ALPHA_RANGE + DITHER_ALPHA_MIN
        } else {
            DITHER_ALPHA_MIN
        }
    } else {
        1.0
    }
}

/// The NPC branch's distance (f32 bits `0x3eeb851f`).
pub const NPC_DITHER_DISTANCE: f32 = 0.46;
/// The alpha range above the floor (`0x3f4ccccd`).
pub const DITHER_ALPHA_RANGE: f32 = 0.8;
/// The alpha floor (`0x3e4ccccd`).
pub const DITHER_ALPHA_MIN: f32 = 0.2;
/// The player within this distance of the NPC's hips is near (`0x3f933333`).
pub const PLAYER_DITHER_NEAR_DISTANCE: f32 = 1.15;
/// The player branch's remap distance (`0x3ee66666`).
pub const PLAYER_DITHER_DISTANCE: f32 = 0.45;

/// The minimum as the processor's `fmin` takes it: a NaN either way gives
/// NaN.
fn arm_fmin(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a < b {
        a
    } else {
        b
    }
}

fn length(d: [f32; 3]) -> f32 {
    ((d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]).sqrt()
}

/// Whether the (local) player is near the NPC: within
/// [`PLAYER_DITHER_NEAR_DISTANCE`] of its hips (`player - hips`).
pub fn player_near(player_minus_hips: [f32; 3]) -> bool {
    length(player_minus_hips) < PLAYER_DITHER_NEAR_DISTANCE
}

/// The player branch's alpha while the player is visible:
/// `min(min(d, 0.45) / 0.45, 1) * 0.8 + 0.2` with `d` the distance from the
/// NPC's root to the near player (`player - root`).
pub fn player_dither_alpha(player_minus_root: [f32; 3]) -> f32 {
    let d = length(player_minus_root);
    let t = arm_fmin(arm_fmin(d, PLAYER_DITHER_DISTANCE) / PLAYER_DITHER_DISTANCE, 1.0);
    t * DITHER_ALPHA_RANGE + DITHER_ALPHA_MIN
}

/// The smaller of two alphas as the update takes it (`a < b ? a : b`).
pub fn dither_min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

/// The update's alpha from its branches: the player branch (the near
/// player's `player - root` when the player is near; its remap when the
/// player is shown, the camera branch's value otherwise), the smaller of it
/// and the NPC branch, then, with the player shown and dither enabled (the
/// NPC not talking), the smaller of that and the camera branch. `camera` is
/// the camera branch's alpha, `None` where it cannot be computed: its terms
/// are then left out.
pub fn update_dither_alpha(
    npc_alpha: f32,
    near_player: Option<[f32; 3]>,
    player_shown: bool,
    dither_enable: bool,
    camera: Option<f32>,
) -> f32 {
    let player_alpha = match near_player {
        Some(player_minus_root) if player_shown => player_dither_alpha(player_minus_root),
        Some(_) => camera.unwrap_or(1.0),
        None => 1.0,
    };
    let alpha = dither_min(player_alpha, npc_alpha);
    match camera {
        Some(camera) if player_shown && dither_enable => dither_min(alpha, camera),
        _ => alpha,
    }
}

/// The camera branch's alpha: the NPC's screen depth remapped from
/// `near + start_offset` over `fall_off` to [0, 1] (0 below, capped at 1).
pub fn camera_dither_alpha(depth: f32, near_clip: f32, start_offset: f32, fall_off: f32) -> f32 {
    let start = near_clip + start_offset;
    let end = start + fall_off;
    let t = (depth - start) / (end - start);
    if t >= 0.0 || t.is_nan() {
        arm_fmin(t, 1.0)
    } else {
        0.0
    }
}

/// SetDitherAlpha 同时切换关键字/数值门；只写 alpha 不会启用透明。
pub fn use_dither(alpha: f32) -> bool { alpha <= 0.999 }

#[cfg(test)]
#[path = "overlap_source_cases.rs"]
mod source_cases;

#[cfg(test)]
#[path = "dither_source_cases.rs"]
mod dither_source_cases;
