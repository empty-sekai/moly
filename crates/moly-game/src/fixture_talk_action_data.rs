//! Immutable source join for an already-admitted fixture-talk cast.
//!
//! Unlike NPC candidate selection, this does not draw another conversation or
//! rewrite the cast. Unit-group positions select action-point fields exactly;
//! one timeline-row draw is retained across all asynchronous asset preparation.

use super::*;

#[derive(Debug, Clone)]
pub(crate) struct CastLocate {
    pub unit: u32,
    pub actor: Entity,
    pub locate: ActivityLocate,
}

#[derive(Debug, Clone)]
pub(crate) struct FixtureTalkActionPlan {
    pub talk_id: i32,
    pub fixture: Entity,
    pub fixture_uid: String,
    pub package: String,
    pub prefab: String,
    pub timeline: ActivityTimeline,
    pub cast: Vec<CastLocate>,
}

pub(crate) struct FixtureTalkActionInput<'a> {
    pub talk_id: i32,
    pub participants: &'a [(u32, Entity)],
    pub fixture: Entity,
    pub fixture_id: i32,
    pub fixture_uid: &'a str,
    pub fixture_world: GlobalTransform,
    pub fixture_view_local_y: Option<f32>,
}

/// Field positions must not be compacted when a middle member is absent.
fn cast_slots(group: &UnitGroup, participants: &[(u32, Entity)]) -> Result<Vec<(usize, u32, Entity)>, PrepareError> {
    let mut seen = HashSet::new();
    let mut result = Vec::with_capacity(participants.len());
    for &(unit, actor) in participants {
        if !seen.insert(unit) {
            return Err(PrepareError::InvalidData("duplicate admitted cast unit".into()));
        }
        let slot = group.units.iter().position(|candidate| *candidate == Some(unit as i32))
            .ok_or_else(|| PrepareError::InvalidData(format!("unit {unit} is outside the source unit group")))?;
        if slot >= 4 {
            return Err(PrepareError::InvalidData("fifth unit has no source action-point field".into()));
        }
        result.push((slot, unit, actor));
    }
    if result.is_empty() { return Err(PrepareError::Rejected("empty fixture-talk cast")); }
    result.sort_by_key(|(slot, _, _)| *slot);
    Ok(result)
}

impl FixtureActivityTables {
    /// An authored furniture Director is distinct from ordinary talking,
    /// facial changes and idle character gestures in the conversation script.
    pub(crate) fn has_talk_timeline(&self, talk_id: i32) -> bool {
        self.pre_action_by_talk.get(&talk_id)
            .and_then(|index| self.pre_actions[*index].timeline_group_id)
            .filter(|group| *group > 0)
            .is_some_and(|group| self.timelines.rows.iter().any(|row| row.group_id == group))
    }

    pub(crate) fn prepare_fixture_talk_action(
        &self,
        input: FixtureTalkActionInput<'_>,
        attach: &AttachPoints,
        rng: &mut impl UniformDraw,
    ) -> Result<Option<FixtureTalkActionPlan>, PrepareError> {
        let Some(pre) = self.pre_action_by_talk.get(&input.talk_id).map(|index| &self.pre_actions[*index]) else { return Ok(None); };
        let Some(group_id) = pre.timeline_group_id.filter(|id| *id > 0) else {
            // Together-communication actions have another source controller;
            // no fabricated Timeline is substituted for that independent path.
            return Ok(None);
        };
        let master = self.talks.get(input.talk_id).ok_or_else(|| PrepareError::DataMissing(format!("talk master {}", input.talk_id)))?;
        let units = self.units.get(master.unit_group_id).ok_or_else(|| PrepareError::DataMissing(format!("unit group {}", master.unit_group_id)))?;
        let slots = cast_slots(units, input.participants)?;
        let rows: Vec<_> = self.timelines.rows.iter().filter(|row| row.group_id == group_id).collect();
        if rows.is_empty() { return Err(PrepareError::DataMissing(format!("fixture timeline group {group_id}"))); }
        let index = rng.draw(rows.len());
        let timeline = (**rows.get(index).ok_or_else(|| PrepareError::InvalidData("timeline draw outside source pool".into()))?).clone();
        let points = self.action_points.get(timeline.action_point_definition)
            .ok_or_else(|| PrepareError::DataMissing(format!("action point definition {}", timeline.action_point_definition)))?;
        let fixture = self.fixture_master(input.fixture_id).ok_or_else(|| PrepareError::DataMissing(format!("fixture master {}", input.fixture_id)))?;
        let view_y = input.fixture_view_local_y.ok_or_else(|| PrepareError::DataMissing("fixture view-local Y".into()))?;
        if input.fixture_uid.is_empty() { return Err(PrepareError::InvalidData("placed fixture has no UID".into())); }
        let model_package = format!("mysekai__fixture__{}", fixture.model_name);
        let mut cast = Vec::with_capacity(slots.len());
        for (slot, unit, actor) in slots {
            let value = points.points[slot].ok_or_else(|| PrepareError::DataMissing(format!("source action-point field {}", slot + 1)))?;
            let index = attach.instance_index(&model_package, value)
                .ok_or_else(|| PrepareError::DataMissing(format!("fixture {model_package} source locate {value}")))?;
            let pair = attach.instance_poses(&model_package, value, &input.fixture_world)
                .ok_or_else(|| PrepareError::DataMissing(format!("fixture {model_package} instance locate {value}")))?;
            let finite_pose = |pose: crate::fixture_attach::AttachPose| {
                pose.position.into_iter().all(f32::is_finite) && pose.rotation.is_finite()
            };
            if !finite_pose(pair.start) || pair.end.is_some_and(|pose| !finite_pose(pose)) {
                return Err(PrepareError::InvalidData("non-finite source action-point pose".into()));
            }
            cast.push(CastLocate { unit, actor, locate: ActivityLocate {
                unit, unit_slot: slot, action_point_value: value, action_point_index: index,
                start: ActivityPose { position: pair.start.position, rotation: pair.start.rotation },
                end: pair.end.map(|pose| ActivityPose { position: pose.position, rotation: pose.rotation }),
            }});
        }
        let (package, prefab) = timeline_asset(&timeline.asset_name, fixture.put_type, view_y)?;
        Ok(Some(FixtureTalkActionPlan { talk_id: input.talk_id, fixture: input.fixture,
            fixture_uid: input.fixture_uid.to_owned(), package, prefab, timeline, cast }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_cast_slots_preserve_holes_and_ignore_runtime_iteration_order() {
        let mut world = World::new();
        let a = world.spawn_empty().id();
        let b = world.spawn_empty().id();
        let group = UnitGroup { id: 1, units: [Some(14), None, Some(15), None, None] };
        let slots = cast_slots(&group, &[(15, b), (14, a)]).unwrap();
        assert_eq!(slots, vec![(0, 14, a), (2, 15, b)]);
    }

    #[test]
    fn unknown_duplicate_and_unrepresentable_actors_fail_closed() {
        let mut world = World::new();
        let a = world.spawn_empty().id();
        let group = UnitGroup { id: 1, units: [Some(14), Some(15), None, None, Some(16)] };
        assert!(cast_slots(&group, &[(13, a)]).is_err());
        assert!(cast_slots(&group, &[(14, a), (14, a)]).is_err());
        assert!(cast_slots(&group, &[(16, a)]).is_err());
        assert!(cast_slots(&group, &[]).is_err());
    }
}
