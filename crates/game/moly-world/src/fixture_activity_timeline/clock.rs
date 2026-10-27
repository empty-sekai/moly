//! Frame-active LoopFlag callbacks, not continuous interval catch-up.

use super::{TimelineClipKey, TimelineDefinition, TimelineOwnerKind, TimelinePayload};
use std::collections::HashSet;

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
    /// A step item view turned its loop flag off (`ChangeLoopFlag(false)`).
    /// No other owner sets it.
    loop_flag_off: bool,
    /// A scene director view paused the director: its time does not advance
    /// and the held time is sampled every frame. No other owner sets it.
    paused: bool,
    speed: Option<f64>,
    section_active: HashSet<TimelineClipKey>,
    pub section_stopped: bool,
}

#[cfg(test)]
mod tests {
    use super::super::source::BlendCurve;
    use super::*;
    use crate::fixture_activity_timeline::{SourceAssetId, TimelineClip, TimelineTrack};

    fn definition(payloads: Vec<(f64, f64, TimelinePayload)>) -> TimelineDefinition {
        let identity = SourceAssetId {
            file: "synthetic".into(),
            path_id: "1".into(),
        };
        TimelineDefinition {
            package: "synthetic".into(),
            prefab: "step".into(),
            fixture_view: None,
            director: identity.clone(),
            director_transforms: None,
            timeline: identity.clone(),
            duration: 8.0,
            tracks: vec![TimelineTrack {
                identity: identity.clone(),
                class: "SectionControlTrack".into(),
                name: "Sections".into(),
                markers: Vec::new(),
                settings: serde_json::Value::Null,
                infinite: None,
                clips: payloads
                    .into_iter()
                    .enumerate()
                    .map(|(clip_index, (start, duration, payload))| TimelineClip {
                        key: TimelineClipKey {
                            track: identity.clone(),
                            clip_index,
                        },
                        playable: None,
                        effect_binding: None,
                        exposed_source: None,
                        start,
                        duration,
                        clip_in: 0.0,
                        time_scale: 1.0,
                        ease_in: 0.0,
                        ease_out: 0.0,
                        blend_in: 0.0,
                        blend_out: 0.0,
                        mix_in: BlendCurve::empty(),
                        mix_out: BlendCurve::empty(),
                        pre_extrapolation: 0,
                        post_extrapolation: 0,
                        pre_time: 0.0,
                        post_time: 0.0,
                        payload,
                        source_envelope: serde_json::Value::Null,
                        source_payload: serde_json::Value::Null,
                    })
                    .collect(),
            }],
        }
    }

    #[test]
    fn section_pause_holds_then_named_jump_and_play_reach_stop() {
        let definition = definition(vec![
            (1.0, 0.2, TimelinePayload::SectionPause),
            (
                3.0,
                0.2,
                TimelinePayload::SectionJumpTarget {
                    name: "Finish".into(),
                },
            ),
            (4.0, 0.2, TimelinePayload::SectionStop),
        ]);
        let mut clock = Clock::default();
        clock.advance(&definition, 0.1, TimelineOwnerKind::StepItem);
        clock.set_speed(2.0);
        clock.advance(&definition, 0.55, TimelineOwnerKind::StepItem);
        assert!((clock.time - 1.1).abs() < 1e-9);
        assert!(clock.is_paused());
        clock.advance(&definition, 5.0, TimelineOwnerKind::StepItem);
        assert!((clock.time - 1.1).abs() < 1e-9);
        clock.jump_to(3.0);
        assert!(clock.is_paused());
        clock.set_paused(false);
        clock.advance(&definition, 0.55, TimelineOwnerKind::StepItem);
        assert!(clock.section_stopped);
        assert!((clock.sampled_time - 4.1).abs() < 1e-9);
    }

    #[test]
    fn skipping_a_section_interval_does_not_invent_its_entry() {
        let definition = definition(vec![(1.0, 0.1, TimelinePayload::SectionStop)]);
        let mut clock = Clock::default();
        clock.advance(&definition, 0.0, TimelineOwnerKind::StepItem);
        clock.advance(&definition, 2.0, TimelineOwnerKind::StepItem);
        assert!(!clock.section_stopped);
    }
}

impl Clock {
    pub fn request_end(&mut self) {
        self.end_requested = true;
    }

    pub fn set_loop_flag(&mut self, state: bool) {
        self.loop_flag_off = !state;
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn set_speed(&mut self, speed: f64) {
        self.speed = Some(speed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// JumpTo first disables LoopFlag, then writes the named clip's start.
    /// Play is a separate operation; a paused director remains paused.
    pub fn jump_to(&mut self, time: f64) {
        self.loop_flag_off = true;
        self.time = time;
    }

    fn sections(&mut self, definition: &TimelineDefinition, owner: TimelineOwnerKind) {
        if owner != TimelineOwnerKind::StepItem {
            return;
        }
        let mut active = HashSet::new();
        for clip in definition
            .tracks
            .iter()
            .filter(|track| track.class == "SectionControlTrack")
            .flat_map(|track| &track.clips)
        {
            if !clip.contains(self.sampled_time) {
                continue;
            }
            active.insert(clip.key.clone());
            if self.section_active.contains(&clip.key) {
                continue;
            }
            match clip.payload {
                TimelinePayload::SectionPause => self.paused = true,
                TimelinePayload::SectionStop => self.section_stopped = true,
                _ => {}
            }
        }
        self.section_active = active;
    }

    /// The next frame samples `time` (a view's `MoveEndTime`).
    pub fn move_to(&mut self, time: f64) {
        self.time = time;
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
        let step = if first || self.paused {
            0.0
        } else {
            delta * self.speed.unwrap_or(1.0)
        };
        self.sampled_time = (self.time + step).min(definition.duration);
        self.time = self.sampled_time;
        let Some(clip) = loop_clip else {
            self.sections(definition, owner);
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
        let loop_flag = looping && !self.end_requested && !self.loop_flag_off;

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
        self.sections(definition, owner);
    }
}
