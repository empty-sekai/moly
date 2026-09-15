//! The talk engine's delayed NPC face commands and their shared clock.
//!
//! Enqueue captures a logical character id, not an entity/material handle. The
//! current conversation resolves that id when a command actually runs. The
//! queue is engine-scoped: stopping a talk stops its clock without clearing or
//! flushing future commands. Starting the next loop resets time, not the queue.
//!
//! MySekaiTalkEngine's ChangeNPCEye/ChangeNPCMouth
//! append elapsed+time; LoopUpdate/ExecuteElapsedCommands drain due commands;
//! WaitClick's resumed edge calls ExecuteAllDelayCommands. OnComplete is due-only,
//! and Dispose/Skip cancel the loop without clearing the static delay list.
//! This module contains the NPC-face projection, not the other delayed producers.

use std::cmp::Ordering;

use bevy::prelude::{info, warn, Assets, Commands, Handle, Resource, World};
use moly_law::talk::FaceSlot;
use serde_json::Value;

use crate::alone_action_runtime::{apply_eye, apply_mouth_pattern, FacialTables};
use crate::character_material::CharacterMaterial;

#[derive(Debug, Clone)]
pub(crate) struct FaceCommand {
    pub unit: i64,
    pub slot: FaceSlot,
    pub pattern: String,
    /// Diagnostics only; resolution uses the current engine character list.
    pub source_talk: i32,
    pub source_step: usize,
}

impl FaceCommand {
    /// Called only after resolving the logical id against the current cast.
    /// FindBy returns a zero-valued pattern when a key is absent; this is not
    /// the material helper's fallback cell. Both script backends use this path.
    pub(crate) fn apply(
        &self,
        tables: &FacialTables,
        materials: &mut Assets<CharacterMaterial>,
        eye: &Handle<CharacterMaterial>,
        mouth: &Handle<CharacterMaterial>,
    ) {
        let pattern = if self.pattern.is_ascii() {
            self.pattern.as_str()
        } else {
            "<non-ascii>"
        };
        let index = match self.slot {
            FaceSlot::Eye => tables.eye_open(&self.pattern),
            FaceSlot::Mouth => tables.lip_close(&self.pattern),
        };
        if index.is_none() && !(self.slot == FaceSlot::Eye && self.pattern.is_empty()) {
            warn!(
                "[talk-face] source={} step={} unit={} slot={:?} pattern={} is absent; applying the zero-valued pattern",
                self.source_talk, self.source_step, self.unit, self.slot, pattern,
            );
        }
        let index = index.unwrap_or(0);
        let cell = match self.slot {
            FaceSlot::Eye => apply_eye(materials, eye, Some(&index)),
            FaceSlot::Mouth => apply_mouth_pattern(
                materials,
                mouth,
                tables.lip_pattern(&self.pattern).unwrap_or_default(),
            ),
        };
        info!(
            "[talk-face] source={} step={} unit={} slot={:?} pattern={} index={} cell={},{}",
            self.source_talk,
            self.source_step,
            self.unit,
            self.slot,
            pattern,
            index,
            cell.0,
            cell.1,
        );
    }
}

struct ScheduledFace {
    deadline: f32,
    command: FaceCommand,
}

#[derive(Resource, Default)]
pub(crate) struct DelayedFaces {
    elapsed: f32,
    running: bool,
    awaiting_first_yield: bool,
    pending: Vec<ScheduledFace>,
}

impl DelayedFaces {
    /// LoopUpdate's initial edge. The source queue is static and is not
    /// reconstructed with each talk-engine instance.
    pub(crate) fn start_loop(&mut self) {
        self.elapsed = 0.0;
        self.running = true;
        self.awaiting_first_yield = true;
    }

    /// Dispose/Skip cancel the update loop, not the stored future callbacks.
    /// Do not turn this into wait_clicked() or pending.clear().
    pub(crate) fn stop_loop(&mut self) {
        self.running = false;
        self.awaiting_first_yield = false;
    }

    /// Explicit user cancellation owns the currently playing talk and must not
    /// leak its future callbacks into a replacement. Natural Dispose retains
    /// the engine queue; this targeted path removes only the cancelled owner.
    pub(crate) fn cancel_talk(&mut self, talk_id: i32) {
        self.pending
            .retain(|entry| entry.command.source_talk != talk_id);
        self.stop_loop();
    }

    pub(crate) fn enqueue(&mut self, delay_seconds: f32, command: FaceCommand) {
        self.pending.push(ScheduledFace {
            deadline: self.elapsed + delay_seconds,
            command,
        });
    }

    /// Call once per active talk frame, before advancing its Lua step stream.
    /// The first call is the initialization edge immediately before the first
    /// Lua dispatch: LoopUpdate resets its clock and yields without incrementing.
    /// Subsequent calls resume that Update yield and add Time.deltaTime.
    /// Do not skip the initialization call and first call this a frame later.
    /// Delay-zero commands are queued too; enqueue itself never writes a face.
    pub(crate) fn advance(&mut self, delta_seconds: f32) -> Vec<FaceCommand> {
        if !self.running {
            return Vec::new();
        }
        if self.awaiting_first_yield {
            self.awaiting_first_yield = false;
            return Vec::new();
        }
        self.elapsed += delta_seconds;
        self.elapsed_commands()
    }

    /// OnComplete processes only commands already due, without adding time.
    /// The owner subsequently disposes/stops the loop; it must not force future
    /// commands to run merely because the script has no more instructions.
    pub(crate) fn on_complete(&mut self) -> Vec<FaceCommand> {
        self.elapsed_commands()
    }

    /// WaitClick resumed after the UI's WaitClicked task, before the next Lua
    /// statement: execute every pending command in deadline order, then clear.
    /// This is NOT the raw mouse click or the click that only reveals letters.
    pub(crate) fn wait_clicked(&mut self) -> Vec<FaceCommand> {
        let mut all = std::mem::take(&mut self.pending);
        all.sort_by(|a, b| compare_deadlines(a.deadline, b.deadline));
        all.into_iter().map(|entry| entry.command).collect()
    }

    fn elapsed_commands(&mut self) -> Vec<FaceCommand> {
        // Fix the due set before the caller dispatches any effects. Commands
        // later enqueued by the caller cannot enter this returned update batch.
        let mut due = Vec::new();
        let mut future = Vec::new();
        for entry in std::mem::take(&mut self.pending) {
            if self.elapsed >= entry.deadline {
                due.push(entry);
            } else {
                future.push(entry);
            }
        }
        self.pending = future;
        due.sort_by(|a, b| compare_deadlines(a.deadline, b.deadline));
        due.into_iter().map(|entry| entry.command).collect()
    }
}

/// Same ordering as Single.CompareTo: NaN precedes a non-NaN, signed zeroes
/// compare equal. Stable sorting retains enqueue order for equal deadlines.
fn compare_deadlines(a: f32, b: f32) -> Ordering {
    if a < b {
        Ordering::Less
    } else if a > b {
        Ordering::Greater
    } else if a == b {
        Ordering::Equal
    } else {
        match (a.is_nan(), b.is_nan()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less,
            _ => Ordering::Greater,
        }
    }
}

/// The extractor always supplies delaySeconds for these commands. Explicit
/// null is an omitted/explicit-nil Lua argument (the bridge reads nil as zero).
/// A missing field is an old lossy export, not evidence of an authored zero.
pub(crate) fn delay_seconds(step: &Value) -> Result<f32, String> {
    let value: Result<f64, String> = match step.get("delaySeconds") {
        Some(Value::Null) => Ok(0.0),
        Some(Value::String(value)) => value
            .trim()
            .parse::<f64>()
            .map_err(|_| "NPC face delaySeconds remains an unresolved source expression".into()),
        Some(value) => value
            .as_f64()
            .ok_or_else(|| "NPC face delaySeconds is not a resolved number/null".into()),
        None => Err("NPC face delaySeconds metadata is absent; re-extract the scenario".into()),
    };
    let value = value?;
    if !value.is_finite() || value < 0.0 || value > f32::MAX as f64 {
        return Err("NPC face delaySeconds is not a finite nonnegative f32".into());
    }
    Ok(value as f32)
}

/// Start at a real conversation-construction edge, before its update systems
/// run. The resource is initialized once alongside the talk-engine resources.
pub(crate) fn start_loop(commands: &mut Commands) {
    commands.queue(|world: &mut World| world.resource_mut::<DelayedFaces>().start_loop());
}

/// Stop at the owner's disposal edge without executing or discarding callbacks.
pub(crate) fn stop_loop(commands: &mut Commands) {
    commands.queue(|world: &mut World| world.resource_mut::<DelayedFaces>().stop_loop());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(source_talk: i32, source_step: usize) -> FaceCommand {
        FaceCommand {
            unit: 1,
            slot: FaceSlot::Eye,
            pattern: "normal".into(),
            source_talk,
            source_step,
        }
    }

    #[test]
    fn cancelling_one_talk_retains_unrelated_delayed_faces() {
        let mut delayed = DelayedFaces::default();
        delayed.start_loop();
        delayed.enqueue(10.0, command(10, 0));
        delayed.enqueue(10.0, command(11, 1));
        delayed.cancel_talk(10);
        delayed.start_loop();
        let remaining = delayed.wait_clicked();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].source_talk, 11);
        assert_eq!(remaining[0].source_step, 1);
    }

    #[test]
    fn cancelled_delayed_faces_do_not_run_in_replacement_owner() {
        let mut delayed = DelayedFaces::default();
        delayed.start_loop();
        delayed.enqueue(0.25, command(10, 0));
        delayed.cancel_talk(10);
        delayed.start_loop();
        delayed.enqueue(0.0, command(12, 2));
        assert!(delayed.advance(1.0).is_empty());
        let due = delayed.advance(0.0);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].source_talk, 12);
    }

    #[test]
    fn delay_parser_rejects_nonfinite_negative_and_f32_overflow() {
        for value in [
            serde_json::json!("Infinity"),
            serde_json::json!(-1.0),
            serde_json::json!(1e100),
        ] {
            let step = serde_json::json!({"delaySeconds": value});
            assert!(delay_seconds(&step).is_err());
        }
    }
}
