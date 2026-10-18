//! The field's own markers for an embedded stage's transient exploration: only
//! a successfully validated, explicitly entered transient import lets an
//! embedded stage restore player layouts. The stage itself is
//! `crate::browser_stage`; the world's layout owner reads these two.

use bevy::prelude::*;

/// Only a successfully validated, explicitly entered transient import lets an
/// embedded stage restore player layouts. Its absence keeps the safe empty
/// stage and prevents accidental reads of a different region's saved preset.
#[derive(Resource)]
pub(crate) struct TransientExploration;

/// Whether the stage must start from an empty layout: a temporary site always,
/// an embedded stage without a transient import.
pub(crate) fn use_empty_stage_layout(temporary: bool, stage: bool, exploring: bool) -> bool {
    temporary || (stage && !exploring)
}
