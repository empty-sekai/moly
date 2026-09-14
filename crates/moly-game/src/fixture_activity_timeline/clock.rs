//! Frame-active LoopFlag callbacks, not continuous interval catch-up.

use super::{TimelineDefinition, TimelineOwnerKind, TimelinePayload};

#[derive(Default)]
pub(super) struct Clock {
    /// Director time after this frame's callbacks; next frame advances this.
    pub time: f64,
    /// The same clock's time cached by TimelinePlayable before callbacks.
    /// All body, face and SE tracks evaluate this value for the current frame.
    /// These are two observations of one clock, not independently advanced clocks.
    pub sampled_time: f64,
    pub loop_started: bool,
    pub loop_active: bool,
    pub enable_talk: bool,
    pub started: bool,
    end_requested: bool,
    player_end_applied: bool,
    previous_loop_active: bool,
    entered_end: Option<f64>,
}

impl Clock {
    pub fn request_end(&mut self) {
        self.end_requested = true;
    }

    pub fn advance(
        &mut self,
        definition: &TimelineDefinition,
        delta: f64,
        owner: TimelineOwnerKind,
    ) {
        let loop_clip = definition
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .find(|c| matches!(c.payload, TimelinePayload::LoopFlag { .. }));
        let first = !self.started;
        self.started = true;
        // Player ChangeLoopFlag(false) invokes SkipLoop and writes the cached
        // EndTime. NPC ChangeLoopFlag(false) only writes LoopFlag=false, so the
        // remainder of its current loop still plays before E begins.
        if owner == TimelineOwnerKind::Player
            && self.end_requested
            && self.loop_started
            && !self.player_end_applied
        {
            if let Some(end) = self.entered_end {
                self.time = end;
                self.player_end_applied = true;
            }
        }
        self.sampled_time = (self.time + if first { 0.0 } else { delta }).min(definition.duration);
        self.time = self.sampled_time;
        let Some(clip) = loop_clip else {
            return;
        };
        let TimelinePayload::LoopFlag {
            looping,
            enable_talk,
        } = clip.payload
        else {
            unreachable!()
        };
        let active = clip.contains(self.sampled_time);
        let loop_flag = looping && !self.end_requested;

        // TimelinePlayable only disables elements active on the previous frame
        // and evaluates the current point's intersection. A frame that jumps
        // completely over the interval must not invent an OnBehaviourPlay.
        if self.previous_loop_active && !active && self.loop_started {
            if loop_flag {
                // Exactly one OnBehaviourPause callback, exactly one subtraction.
                // A large overrun is not modulo-replayed through imaginary frames.
                self.time -= clip.duration;
            } else if enable_talk {
                self.enable_talk = false;
            }
        }
        if active && !self.previous_loop_active {
            if enable_talk {
                self.enable_talk = true;
            }
            // OnBehaviourPlay reads the current Director value and stores its
            // own EndTime. The entry frame's overrun remains in this value.
            self.entered_end = Some(self.time + clip.duration);
            self.loop_started = true;
            if owner == TimelineOwnerKind::Player && self.end_requested && !self.player_end_applied
            {
                self.time = self.entered_end.expect("set on this entry");
                self.player_end_applied = true;
            }
        }
        self.loop_active = active;
        self.previous_loop_active = active;
    }
}
