//! NPCAvatarPresenter.TryCancelIfCharacterOverlap 的累计/取消门。
//! 真源先比较旧累计值，再加本帧 dt；取消失败不清计时。

use super::{ObjectiveType, TalkType};

/// IsSomeCharacterTalkData：并非「有任意对话数据」都豁免。
pub fn has_group_talk(current: Option<ObjectiveType>, slot: Option<TalkType>) -> bool {
    slot.is_some()
        && (matches!(current, Some(ObjectiveType::MainSomeCharacterCommunication | ObjectiveType::SubSomeCharacterCommunication))
            || matches!(slot, Some(TalkType::MultipleCharacterFixture | TalkType::CommunicationWhileDoingWait)))
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
    others.iter().find(|other| other.root_distance < 0.46)
        .map_or(1.0, |other| 0.2 + 0.8 * (other.root_distance / 0.46).clamp(0.0, 1.0))
}

/// SetDitherAlpha 同时切换关键字/数值门；只写 alpha 不会启用透明。
pub fn use_dither(alpha: f32) -> bool { alpha <= 0.999 }
