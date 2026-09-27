//! Source-qualified, single-character actions for browsing and exact playback.
//! NoTalk actions and a story's pre-action are different source identities.
//! Both can borrow the normal NPC fixture owner without starting a dialogue.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ActivityOrigin {
    NoTalk(i32),
    PreAction(i32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ActivityKey {
    pub origin: ActivityOrigin,
    pub timeline_id: i32,
}
impl ActivityKey {
    pub(crate) fn source_id(self) -> i32 {
        match self.origin {
            ActivityOrigin::NoTalk(id) | ActivityOrigin::PreAction(id) => id,
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct ActivitySpec {
    pub key: ActivityKey,
    pub unit: u32,
    pub fixture_id: i32,
    pub point: i32,
    pub timeline: ActivityTimeline,
    pub tweet: Option<TweetRef>,
    pub related_talk: Option<i32>,
    pub variant: usize,
    pub variants: usize,
}

impl FixtureActivityTables {
    /// Join only authored, coherent relationships. A timeline filename is not
    /// evidence that an arbitrary character can use an arbitrary fixture.
    pub(crate) fn catalogue_activities(&self, store: &TalkStore) -> Vec<ActivitySpec> {
        let mut result = Vec::new();
        for action in self.no_talk_rows() {
            if action.unit == 0 || !self.fixtures.contains_key(&action.fixture_id) {
                continue;
            }
            let pool: Vec<_> = self.timeline_rows(action.timeline_group_id).collect();
            let Some(first) = pool.first() else {
                continue;
            };
            // CreateNoneTalkData assigns the locator from the FIRST timeline,
            // independently of the later chosen animation in that group.
            let Ok(point) = self.no_talk_point(action.unit, first) else {
                continue;
            };
            for (index, timeline) in pool.iter().enumerate() {
                result.push(ActivitySpec {
                    key: ActivityKey {
                        origin: ActivityOrigin::NoTalk(action.id),
                        timeline_id: timeline.id,
                    },
                    unit: action.unit,
                    fixture_id: action.fixture_id,
                    point,
                    timeline: (*timeline).clone(),
                    tweet: None,
                    related_talk: None,
                    variant: index + 1,
                    variants: pool.len(),
                });
            }
        }
        for master in &self.talks.rows {
            let Some(pre) = self
                .pre_action_by_talk
                .get(&master.id)
                .map(|index| &self.pre_actions[*index])
            else {
                continue;
            };
            let Some(group_id) = pre.timeline_group_id.filter(|id| *id > 0) else {
                continue;
            };
            if pre
                .fixture_together_communication_id
                .is_some_and(|id| id != 0)
            {
                continue;
            }
            let Some(units) = self.units.get(master.unit_group_id) else {
                continue;
            };
            let members: Vec<_> = units
                .units
                .iter()
                .enumerate()
                .filter_map(|(slot, unit)| unit.filter(|unit| *unit > 0).map(|unit| (slot, unit)))
                .collect();
            let [(unit_slot, unit)] = members.as_slice() else {
                continue;
            };
            let Some(row) = store.row(master.id) else {
                continue;
            };
            if row.lua != master.lua
                || row.unit_ids.as_slice() != [*unit]
                || row.condition_group_id != master.condition_group_id
            {
                continue;
            }
            let [fixture_id] = row.fixture_ids.as_slice() else {
                continue;
            };
            if !self.fixtures.contains_key(fixture_id) {
                continue;
            }
            // Pre-action tweet identity comes from the same script and source
            // table. Never substitute a greeting or the first line of dialogue.
            if pre.tweet_id > 0 && row.tweet.id != pre.tweet_id {
                continue;
            }
            let tweet =
                (pre.tweet_id > 0 && !row.tweet.text.trim().is_empty()).then(|| row.tweet.clone());
            let pool: Vec<_> = self.timeline_rows(group_id).collect();
            for (index, timeline) in pool.iter().enumerate() {
                let Ok(Some(point)) =
                    self.action_point_value(timeline.action_point_definition, *unit_slot)
                else {
                    continue;
                };
                if point <= 0 {
                    continue;
                }
                result.push(ActivitySpec {
                    key: ActivityKey {
                        origin: ActivityOrigin::PreAction(pre.id),
                        timeline_id: timeline.id,
                    },
                    unit: *unit as u32,
                    fixture_id: *fixture_id,
                    point,
                    timeline: (*timeline).clone(),
                    tweet: tweet.clone(),
                    related_talk: Some(master.id),
                    variant: index + 1,
                    variants: pool.len(),
                });
            }
        }
        result
    }

    pub(crate) fn resolve_activity(
        &self,
        key: ActivityKey,
        store: &TalkStore,
    ) -> Option<ActivitySpec> {
        self.catalogue_activities(store)
            .into_iter()
            .find(|row| row.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_tables_and_timeline_variants_never_collide() {
        let a = ActivityKey {
            origin: ActivityOrigin::NoTalk(42),
            timeline_id: 7,
        };
        assert_ne!(
            a,
            ActivityKey {
                origin: ActivityOrigin::PreAction(42),
                timeline_id: 7
            }
        );
        assert_ne!(
            a,
            ActivityKey {
                origin: ActivityOrigin::NoTalk(42),
                timeline_id: 8
            }
        );
    }
}
