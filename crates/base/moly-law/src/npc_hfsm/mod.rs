//! The NPC machine: the controller's phases, the upper and lower state
//! families, the result the upper layer hands the lower one, and the
//! per-state flags the presenter reads.
//!
//! One character's machine has four phases ([`NpcPhase`]) and two state
//! objects: the upper state ([`UpperState`]) is a move, and the lower state
//! ([`LowerState`]) is the action at its end. A phase names which object a
//! frame updates: Upper updates the upper state; Lower and Rest update the
//! lower state (Rest is the lower state running with no upper state above
//! it); Inactive updates nothing and counts towards the recovery.
//!
//! The upper states hand over an [`UpperResult`] when they complete;
//! [`lower_state_for_upper`] is the presenter's result-to-lower choice, and
//! the controller falls back to [`LowerState::Idle`] when that choice
//! produces nothing.
//!
//! This module holds the shape only; the state bodies arrive with the
//! controller that runs them.

pub mod decider;
pub mod lower_idle;
pub mod plan;
pub mod timing;
pub mod upper;

use crate::objective::TalkType;

pub use lower_idle::IdleLower;
pub use plan::{NpcPlan, PlanTalkData, PlanUpper};
pub use timing::{
    ACTION_STATE_TALK, IDLE_DURATION_MIN, INACTIVE_RECOVERY_SECONDS, MAX_TRANSITIONS_PER_FRAME,
    REST_AFTER_UPPER_SECONDS,
};
pub use upper::{UpperStateWeight, decide_upper, set_upper_weight};

/// `HFSMPhase`: the phase a frame updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum NpcPhase {
    /// The upper state runs.
    Upper = 0,
    /// The lower state runs.
    Lower = 1,
    /// The lower state runs as the rest between objectives.
    Rest = 2,
    /// Nothing runs; the recovery timer counts towards the next decision.
    #[default]
    Inactive = 3,
}

impl NpcPhase {
    /// Every phase, in value order.
    pub const ALL: [NpcPhase; 4] = [
        NpcPhase::Upper,
        NpcPhase::Lower,
        NpcPhase::Rest,
        NpcPhase::Inactive,
    ];

    /// The phase of a raw value; a value outside the four is none.
    pub fn from_value(value: i32) -> Option<NpcPhase> {
        match value {
            0 => Some(NpcPhase::Upper),
            1 => Some(NpcPhase::Lower),
            2 => Some(NpcPhase::Rest),
            3 => Some(NpcPhase::Inactive),
            _ => None,
        }
    }

    /// The raw value.
    pub fn value(self) -> i32 {
        self as i32
    }

    /// `IsLowerStateRunningPhase`: a lower state runs in this phase.
    pub fn is_lower_running(self) -> bool {
        matches!(self, NpcPhase::Lower | NpcPhase::Rest)
    }
}

/// `NPCUpperStateType`: the eight upper states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum UpperType {
    RandomWalk = 0,
    MoveToFixture = 1,
    MoveToPlayer = 2,
    MoveToNPC = 3,
    Rest = 4,
    ChangeSite = 5,
    MoveToPosition = 6,
    Stay = 7,
}

impl UpperType {
    /// Every upper state, in type order.
    pub const ALL: [UpperType; 8] = [
        UpperType::RandomWalk,
        UpperType::MoveToFixture,
        UpperType::MoveToPlayer,
        UpperType::MoveToNPC,
        UpperType::Rest,
        UpperType::ChangeSite,
        UpperType::MoveToPosition,
        UpperType::Stay,
    ];

    /// The type of a raw value; a value outside the eight is none.
    pub fn from_value(value: i32) -> Option<UpperType> {
        UpperType::ALL.get(usize::try_from(value).ok()?).copied()
    }

    /// The raw value.
    pub fn value(self) -> i32 {
        self as i32
    }
}

/// The upper state a machine holds. Every upper state has the same
/// three-part shape (an enter that starts one move, an update that completes
/// once the move stops, a result for the lower layer); the payload of a
/// state is one variant field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UpperState {
    /// A walk to a point sampled near the character.
    RandomWalk,
    /// A walk to a fixture's action point (or the fixture's own position),
    /// and the room to recruit members.
    MoveToFixture { fixture: Option<u32>, can_be_recruited: bool },
    /// A walk to the player's position when the move started.
    MoveToPlayer,
    /// A walk to another character's position when the move started.
    MoveToNpc { npc: u32 },
    /// The one-frame rest marker: it completes on its first update.
    Rest,
    /// A walk to the door of the site being moved to.
    ChangeSite,
    /// A walk to a given position.
    MoveToPosition,
    /// The one-frame marker that tags its result `Stay`.
    Stay,
}

impl UpperState {
    /// The type of this state.
    pub fn upper_type(&self) -> UpperType {
        match self {
            UpperState::RandomWalk => UpperType::RandomWalk,
            UpperState::MoveToFixture { .. } => UpperType::MoveToFixture,
            UpperState::MoveToPlayer => UpperType::MoveToPlayer,
            UpperState::MoveToNpc { .. } => UpperType::MoveToNPC,
            UpperState::Rest => UpperType::Rest,
            UpperState::ChangeSite => UpperType::ChangeSite,
            UpperState::MoveToPosition => UpperType::MoveToPosition,
            UpperState::Stay => UpperType::Stay,
        }
    }

    /// `get_CanInterrupt`: every upper state can be interrupted.
    pub fn can_interrupt(&self) -> bool {
        true
    }

    /// `get_CanBeRecruited`: the fixture move alone can be built for a
    /// member that is already joining a fixture action.
    pub fn can_be_recruited(&self) -> bool {
        match self {
            UpperState::MoveToFixture { can_be_recruited, .. } => *can_be_recruited,
            _ => true,
        }
    }

    /// Whether the state completes on a later update or starts one: the two
    /// markers (`Rest`, `Stay`) have no mover, every walk does.
    pub fn has_mover(&self) -> bool {
        !matches!(self, UpperState::Rest | UpperState::Stay)
    }
}

/// `NPCUpperStateBase`: the fields every upper state carries beside its own.
/// `Enter()` zeroes the elapsed time, clears `IsComplete` and calls the
/// state's `OnEnter`; `Update()` adds `Time.deltaTime` and then calls
/// `OnUpdate`. The base itself has no mover: each concrete state declares
/// its own running flag, which starts set and is cleared once the move
/// stops, and the inherited `OnUpdate` completes the state as soon as the
/// flag is clear.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UpperBase {
    /// The state's own running flag; `OnUpdate` completes once it is clear.
    pub is_moving: bool,
}

impl UpperBase {
    /// `Enter()`: a fresh entry, not complete, running when the state has a
    /// mover.
    pub fn enter(&mut self, has_mover: bool) -> UpperEntry {
        UpperEntry {
            is_complete: false,
            elapsed: 0.0,
            is_moving: has_mover,
        }
    }
}

/// The state while it runs: the base's clock and flags.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UpperEntry {
    /// `IsComplete`.
    pub is_complete: bool,
    /// `_elapsedTime`: `Time.deltaTime` summed since the entry.
    pub elapsed: f32,
    /// The state's running flag.
    pub is_moving: bool,
}

impl UpperEntry {
    /// `Update()`: add this frame's time and run the state's `OnUpdate`. The
    /// inherited body alone (`is_moving` clear) completes the state; a state
    /// that is still moving stays incomplete.
    pub fn update(&mut self, dt: f32) {
        self.elapsed += dt;
        if !self.is_moving {
            self.is_complete = true;
        }
    }

    /// `Exit()`: the state leaves; the entry keeps nothing.
    pub fn exit(&mut self) {
        self.is_moving = false;
    }

    /// `get_CanInterrupt`: every upper state answers true.
    pub fn can_interrupt(&self) -> bool {
        true
    }
}

/// `AvatarMoveStatus`: how an upper state's move ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum AvatarMoveStatus {
    NoneRoute = -2,
    InValid = -1,
    InProgress = 0,
    Success = 1,
    Stacked = 2,
    InTalk = 3,
    InOverlapPosition = 4,
    IsUsingFixture = 5,
}

impl AvatarMoveStatus {
    /// The status of a raw value; a value outside the set is none.
    pub fn from_value(value: i32) -> Option<AvatarMoveStatus> {
        Some(match value {
            -2 => AvatarMoveStatus::NoneRoute,
            -1 => AvatarMoveStatus::InValid,
            0 => AvatarMoveStatus::InProgress,
            1 => AvatarMoveStatus::Success,
            2 => AvatarMoveStatus::Stacked,
            3 => AvatarMoveStatus::InTalk,
            4 => AvatarMoveStatus::InOverlapPosition,
            5 => AvatarMoveStatus::IsUsingFixture,
            _ => return None,
        })
    }

    /// The raw value.
    pub fn value(self) -> i32 {
        self as i32
    }
}

/// `NPCUpperStateResult`: what an upper state hands the controller when it
/// completes. The controller accepts `Success` alone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpperResult {
    /// The upper state that produced the result.
    pub state: UpperType,
    /// Where the state's move ended; a `NaN` component marks a failure.
    pub arrival: [f32; 3],
    /// The fixture the state used.
    pub target_fixture: Option<u32>,
    /// The character the state walked to.
    pub target_npc: Option<u32>,
    /// How the move ended.
    pub move_status: AvatarMoveStatus,
}

impl UpperResult {
    /// `IsMoveFailedResult`: every status but `Success`.
    pub fn is_move_failed(&self) -> bool {
        self.move_status != AvatarMoveStatus::Success
    }
}

/// `NPCLowerStateType`: the 25 lower states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LowerType {
    Idle = 0,
    Rest,
    TutorialWait,
    CutScene,
    EntrySite,
    EntryStandby,
    PhotoShot,
    GeneralTalk,
    Greeting,
    HighPriorityFixtureReaction,
    AfterEditReaction,
    CommonFixtureTweet,
    SingleFixtureAction,
    MultiFixtureAction,
    SubFixtureAction,
    NoneTalkFixtureAction,
    CommonSomeCharacterCommunication,
    CommunicationWhileDoingWait,
    MainCommunication,
    SubCommunication,
    SubCommunicationWhileDoingWait,
    SubCommonSomeCharacterWait,
    ChangeSite,
    BirthdayParty,
    ImmediatelyTimeline,
}

impl LowerType {
    /// Every lower state, in type order.
    pub const ALL: [LowerType; 25] = [
        LowerType::Idle,
        LowerType::Rest,
        LowerType::TutorialWait,
        LowerType::CutScene,
        LowerType::EntrySite,
        LowerType::EntryStandby,
        LowerType::PhotoShot,
        LowerType::GeneralTalk,
        LowerType::Greeting,
        LowerType::HighPriorityFixtureReaction,
        LowerType::AfterEditReaction,
        LowerType::CommonFixtureTweet,
        LowerType::SingleFixtureAction,
        LowerType::MultiFixtureAction,
        LowerType::SubFixtureAction,
        LowerType::NoneTalkFixtureAction,
        LowerType::CommonSomeCharacterCommunication,
        LowerType::CommunicationWhileDoingWait,
        LowerType::MainCommunication,
        LowerType::SubCommunication,
        LowerType::SubCommunicationWhileDoingWait,
        LowerType::SubCommonSomeCharacterWait,
        LowerType::ChangeSite,
        LowerType::BirthdayParty,
        LowerType::ImmediatelyTimeline,
    ];

    /// The type of a raw value; a value outside the 25 is none.
    pub fn from_value(value: i32) -> Option<LowerType> {
        LowerType::ALL.get(usize::try_from(value).ok()?).copied()
    }

    /// The raw value.
    pub fn value(self) -> i32 {
        self as i32
    }
}

/// The lower state a machine holds. The two timed states carry their pause;
/// the bodies of the others keep their own timers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LowerState {
    /// The universal fallback: a pause of 5–8 s drawn when it is built.
    Idle { duration: f32 },
    /// The rest between objectives, `max(PauseTime, 0)` s; the rest the
    /// presenter builds from a `Rest` upper state is a fixed 5 s.
    Rest { duration: f32 },
    TutorialWait,
    CutScene,
    EntrySite,
    EntryStandby,
    PhotoShot,
    GeneralTalk,
    Greeting,
    HighPriorityFixtureReaction,
    AfterEditReaction,
    CommonFixtureTweet,
    SingleFixtureAction,
    MultiFixtureAction,
    SubFixtureAction,
    NoneTalkFixtureAction,
    CommonSomeCharacterCommunication,
    CommunicationWhileDoingWait,
    MainCommunication,
    SubCommunication,
    SubCommunicationWhileDoingWait,
    SubCommonSomeCharacterWait,
    ChangeSite,
    BirthdayParty,
    ImmediatelyTimeline,
}

impl LowerState {
    /// The type of this state.
    pub fn lower_type(&self) -> LowerType {
        match self {
            LowerState::Idle { .. } => LowerType::Idle,
            LowerState::Rest { .. } => LowerType::Rest,
            LowerState::TutorialWait => LowerType::TutorialWait,
            LowerState::CutScene => LowerType::CutScene,
            LowerState::EntrySite => LowerType::EntrySite,
            LowerState::EntryStandby => LowerType::EntryStandby,
            LowerState::PhotoShot => LowerType::PhotoShot,
            LowerState::GeneralTalk => LowerType::GeneralTalk,
            LowerState::Greeting => LowerType::Greeting,
            LowerState::HighPriorityFixtureReaction => LowerType::HighPriorityFixtureReaction,
            LowerState::AfterEditReaction => LowerType::AfterEditReaction,
            LowerState::CommonFixtureTweet => LowerType::CommonFixtureTweet,
            LowerState::SingleFixtureAction => LowerType::SingleFixtureAction,
            LowerState::MultiFixtureAction => LowerType::MultiFixtureAction,
            LowerState::SubFixtureAction => LowerType::SubFixtureAction,
            LowerState::NoneTalkFixtureAction => LowerType::NoneTalkFixtureAction,
            LowerState::CommonSomeCharacterCommunication => LowerType::CommonSomeCharacterCommunication,
            LowerState::CommunicationWhileDoingWait => LowerType::CommunicationWhileDoingWait,
            LowerState::MainCommunication => LowerType::MainCommunication,
            LowerState::SubCommunication => LowerType::SubCommunication,
            LowerState::SubCommunicationWhileDoingWait => LowerType::SubCommunicationWhileDoingWait,
            LowerState::SubCommonSomeCharacterWait => LowerType::SubCommonSomeCharacterWait,
            LowerState::ChangeSite => LowerType::ChangeSite,
            LowerState::BirthdayParty => LowerType::BirthdayParty,
            LowerState::ImmediatelyTimeline => LowerType::ImmediatelyTimeline,
        }
    }
}

/// The lower state's runtime flags: the base class defaults, except for the
/// state bodies that write their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LowerFlags {
    /// `get_CanInterrupt`.
    pub can_interrupt: bool,
    /// `get_CanBeRecruited`.
    pub can_be_recruited: bool,
    /// `get_BlocksPhotoShotCapture`.
    pub blocks_photo_shot_capture: bool,
    /// `get_IsMultiCharacterAction`.
    pub is_multi_character_action: bool,
}

impl LowerState {
    /// The state's flags from the state bodies' own writers.
    pub fn flags(&self) -> LowerFlags {
        use LowerState as L;
        let mut flags = LowerFlags {
            can_interrupt: true,
            can_be_recruited: true,
            blocks_photo_shot_capture: false,
            is_multi_character_action: false,
        };
        // The cut-scene and entry-site states never answer a cancel; the
        // sub fixture action and the immediate timeline do not either. The
        // fixture and change-site actions stop answering one while their
        // own phase runs (a playing timeline, a committed site), which the
        // state body decides.
        if matches!(
            self,
            L::CutScene | L::EntrySite | L::SubFixtureAction | L::ImmediatelyTimeline
        ) {
            flags.can_interrupt = false;
        }
        if matches!(
            self,
            L::HighPriorityFixtureReaction
                | L::CommonFixtureTweet
                | L::SingleFixtureAction
                | L::MultiFixtureAction
                | L::SubFixtureAction
                | L::ImmediatelyTimeline
        ) {
            flags.can_be_recruited = false;
        }
        if matches!(
            self,
            L::HighPriorityFixtureReaction
                | L::CommonFixtureTweet
                | L::SingleFixtureAction
                | L::MultiFixtureAction
        ) {
            flags.blocks_photo_shot_capture = true;
        }
        if matches!(
            self,
            L::SubFixtureAction
                | L::MultiFixtureAction
                | L::CommonSomeCharacterCommunication
                | L::CommunicationWhileDoingWait
                | L::MainCommunication
                | L::SubCommunication
        ) {
            flags.is_multi_character_action = true;
        }
        flags
    }
}

/// The fallback idle's pause: `Random.Range(5.0f, 8.0f)`, one engine float
/// draw taken when the state is constructed (not when it is entered).
pub fn idle_duration(draw: &mut dyn FnMut(f32, f32) -> f32) -> f32 {
    draw(5.0, 8.0)
}

/// The presenter's `CreateLowerState`: the lower state it builds from an
/// upper state's result.
pub fn lower_state_for_upper(result: &UpperResult) -> LowerState {
    match result.state {
        UpperType::RandomWalk | UpperType::MoveToNPC | UpperType::MoveToPosition | UpperType::Stay => {
            LowerState::GeneralTalk
        }
        UpperType::MoveToFixture => match result.target_fixture {
            // The fixture state is the talk-data factory's (`fixture_lower`);
            // the spanning single action stands in until the factory's
            // lotteries are ported.
            Some(_) => LowerState::SingleFixtureAction,
            None => LowerState::Idle { duration: IDLE_DURATION_MIN },
        },
        UpperType::MoveToPlayer => LowerState::Greeting,
        UpperType::Rest => LowerState::Rest { duration: REST_AFTER_UPPER_SECONDS },
        UpperType::ChangeSite => LowerState::ChangeSite,
    }
}

/// `CreateFixtureLowerStateFromTalkData`'s choice among the three fixture
/// lowers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureLower {
    Single,
    Multi,
    CommunicationWhileDoingWait,
}

/// The fixture lower a talk's pre-action selects: a waiting-with-
/// communication pre-action without a timeline group is the while-doing-
/// wait state, a pre-action with a timeline group and two or more members
/// is the multi-member action, and everything else is the single action.
pub fn fixture_lower(
    pre_action_timeline_group: Option<i32>,
    is_wait_with_communication: bool,
    member_count: usize,
) -> FixtureLower {
    if pre_action_timeline_group.is_none() && is_wait_with_communication {
        return FixtureLower::CommunicationWhileDoingWait;
    }
    if pre_action_timeline_group.is_some() && member_count >= 2 {
        return FixtureLower::Multi;
    }
    FixtureLower::Single
}

/// `ShouldReturnToTutorialWaiting`: the tutorial runs and the character's
/// talk data is the tutorial-waiting kind.
pub fn should_return_to_tutorial_waiting(is_tutorial: bool, talk: Option<TalkType>) -> bool {
    is_tutorial && talk == Some(TalkType::TutorialWaiting)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_are_the_four_source_values() {
        // The constructor writes 3 (Inactive); the update switches on 0-3.
        for (value, phase) in [
            (0, NpcPhase::Upper),
            (1, NpcPhase::Lower),
            (2, NpcPhase::Rest),
            (3, NpcPhase::Inactive),
        ] {
            assert_eq!(NpcPhase::from_value(value), Some(phase));
            assert_eq!(phase.value(), value);
        }
        assert_eq!(NpcPhase::from_value(4), None);
        assert_eq!(NpcPhase::from_value(-1), None);
        assert_eq!(NpcPhase::default(), NpcPhase::Inactive);
    }

    #[test]
    fn lower_running_phase_is_the_unsigned_pair() {
        // IsLowerStateRunningPhase: (phase - 1) < 2, unsigned.
        for phase in NpcPhase::ALL {
            assert_eq!(phase.is_lower_running(), ((phase.value() - 1) as u32) < 2);
        }
    }

    #[test]
    fn upper_types_are_the_eight_values() {
        let values: Vec<i32> = UpperType::ALL.iter().map(|kind| kind.value()).collect();
        assert_eq!(values, (0..8).collect::<Vec<i32>>());
        assert_eq!(UpperType::from_value(8), None);
        assert_eq!(UpperType::from_value(-1), None);
        assert_eq!(UpperState::Stay.upper_type(), UpperType::Stay);
        assert!(UpperState::MoveToPosition.can_interrupt());
        assert!(!UpperState::MoveToFixture { fixture: Some(1), can_be_recruited: false }.can_be_recruited());
    }

    #[test]
    fn lower_types_are_the_twenty_five_values() {
        for (index, kind) in LowerType::ALL.iter().enumerate() {
            assert_eq!(kind.value(), index as i32);
            assert_eq!(LowerType::from_value(index as i32), Some(*kind));
        }
        assert_eq!(LowerType::from_value(25), None);
        assert_eq!(LowerType::from_value(-1), None);
    }

    #[test]
    fn move_status_set_is_the_source_values() {
        for value in -2..=5 {
            assert_eq!(
                AvatarMoveStatus::from_value(value).map(AvatarMoveStatus::value),
                Some(value)
            );
        }
        assert_eq!(AvatarMoveStatus::from_value(6), None);
        let success = UpperResult {
            state: UpperType::RandomWalk,
            arrival: [0.0; 3],
            target_fixture: None,
            target_npc: None,
            move_status: AvatarMoveStatus::Success,
        };
        assert!(!success.is_move_failed());
        assert!(UpperResult { move_status: AvatarMoveStatus::NoneRoute, ..success }.is_move_failed());
    }

    #[test]
    fn create_lower_state_switch_reads_the_state_type() {
        let base = UpperResult {
            state: UpperType::RandomWalk,
            arrival: [1.0, 0.0, 2.0],
            target_fixture: None,
            target_npc: None,
            move_status: AvatarMoveStatus::Success,
        };
        for state in [
            UpperType::RandomWalk,
            UpperType::MoveToNPC,
            UpperType::MoveToPosition,
            UpperType::Stay,
        ] {
            assert_eq!(
                lower_state_for_upper(&UpperResult { state, ..base }),
                LowerState::GeneralTalk
            );
        }
        assert_eq!(
            lower_state_for_upper(&UpperResult { state: UpperType::MoveToPlayer, ..base }),
            LowerState::Greeting
        );
        assert_eq!(
            lower_state_for_upper(&UpperResult { state: UpperType::Rest, ..base }),
            LowerState::Rest { duration: 5.0 }
        );
        assert_eq!(
            lower_state_for_upper(&UpperResult { state: UpperType::ChangeSite, ..base }),
            LowerState::ChangeSite
        );
        assert!(matches!(
            lower_state_for_upper(&UpperResult { state: UpperType::MoveToFixture, ..base }),
            LowerState::Idle { .. }
        ));
        assert_eq!(
            lower_state_for_upper(&UpperResult {
                state: UpperType::MoveToFixture,
                target_fixture: Some(9),
                ..base
            }),
            LowerState::SingleFixtureAction
        );
    }

    #[test]
    fn fixture_lower_choice_follows_the_talk_rows() {
        assert_eq!(fixture_lower(None, true, 1), FixtureLower::CommunicationWhileDoingWait);
        assert_eq!(fixture_lower(Some(3), true, 2), FixtureLower::Multi);
        assert_eq!(fixture_lower(Some(3), false, 2), FixtureLower::Multi);
        assert_eq!(fixture_lower(Some(3), false, 1), FixtureLower::Single);
        assert_eq!(fixture_lower(None, false, 4), FixtureLower::Single);
    }

    #[test]
    fn state_flags_follow_the_state_bodies() {
        assert!(!LowerState::CutScene.flags().can_interrupt);
        assert!(!LowerState::EntrySite.flags().can_interrupt);
        assert!(LowerState::EntryStandby.flags().can_interrupt);
        assert!(LowerState::GeneralTalk.flags().can_interrupt);
        assert!(LowerState::Idle { duration: 5.0 }.flags().can_be_recruited);
        assert!(!LowerState::MultiFixtureAction.flags().can_be_recruited);
        assert!(LowerState::HighPriorityFixtureReaction.flags().blocks_photo_shot_capture);
        assert!(LowerState::MultiFixtureAction.flags().is_multi_character_action);
        assert!(!LowerState::SingleFixtureAction.flags().is_multi_character_action);
    }

    #[test]
    fn idle_duration_is_one_draw_of_the_source_range() {
        // Random.Range(5.0f, 8.0f): one float draw over the range.
        let mut calls = 0;
        let mut range = (0.0f32, 0.0f32);
        let duration = idle_duration(&mut |low, high| {
            calls += 1;
            range = (low, high);
            6.31
        });
        assert_eq!(calls, 1);
        assert_eq!(range, (5.0, 8.0));
        assert_eq!(duration, 6.31);
    }

    #[test]
    fn tutorial_wait_test_reads_flag_and_talk_kind() {
        assert!(should_return_to_tutorial_waiting(true, Some(TalkType::TutorialWaiting)));
        assert!(!should_return_to_tutorial_waiting(false, Some(TalkType::TutorialWaiting)));
        assert!(!should_return_to_tutorial_waiting(true, Some(TalkType::Common)));
        assert!(!should_return_to_tutorial_waiting(true, None));
    }
}