//! `NPCObjectivePlan`: what the decider answers, and the lower-state
//! override a plan carries.
//!
//! A plan names the upper state it starts (`CreateUpper`), the lower-state
//! factory override its result is handed to (`CreateLowerOverride`), and the
//! data the controller logs with it (the fixture, the action point, the talk
//! data). The controller's `TransitionToPlan` invokes `CreateUpper`, and the
//! presenter's `CreateLowerState` is the override it passes to the lower
//! factory; the override is consumed by the upper phase whether or not the
//! factory used it.
//!
//! A plan whose `CreateUpper` answers no state parks its character Inactive
//! exactly as a transition to a missing upper state does.

use super::LowerState;

/// The upper state a plan asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlanUpper {
    /// The one-frame rest marker.
    Rest,
    /// The one-frame marker that tags its result `Stay`.
    Stay,
}

/// The talk data a plan carries: an opaque handle the lower factory reads.
pub type PlanTalkData = u64;

/// `NPCObjectivePlan`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NpcPlan {
    /// The upper state the plan starts.
    pub upper: PlanUpper,
    /// The fixture the plan targets.
    pub target_fixture: Option<u32>,
    /// The fixture's action point the plan walks to.
    pub action_point: Option<[f32; 3]>,
    /// The talk data the lower factory reads.
    pub talk_data: Option<PlanTalkData>,
    /// `CreateLowerOverride`: the lower the upper's result is handed to.
    pub lower_override: Option<LowerState>,
}

impl NpcPlan {
    /// A plan that starts one of the two marker uppers.
    pub fn marker(upper: PlanUpper) -> Self {
        Self {
            upper,
            target_fixture: None,
            action_point: None,
            talk_data: None,
            lower_override: None,
        }
    }

    /// `CreateLowerOverride`: the override, taken by the upper phase.
    pub fn create_lower_override(&self) -> Option<LowerState> {
        self.lower_override
    }

    /// `CreateUpper`: the marker's upper state.
    pub fn create_upper(&self) -> super::UpperState {
        match self.upper {
            PlanUpper::Rest => super::UpperState::Rest,
            PlanUpper::Stay => super::UpperState::Stay,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_plan_carries_no_override_and_its_upper() {
        let plan = NpcPlan::marker(PlanUpper::Rest);
        assert_eq!(plan.create_lower_override(), None);
        assert_eq!(plan.create_upper(), super::super::UpperState::Rest);
        assert_eq!(NpcPlan::marker(PlanUpper::Stay).create_upper(), super::super::UpperState::Stay);
        let mut overridden = plan;
        overridden.lower_override = Some(LowerState::Rest { duration: 5.0 });
        assert_eq!(overridden.create_lower_override(), Some(LowerState::Rest { duration: 5.0 }));
    }
}
