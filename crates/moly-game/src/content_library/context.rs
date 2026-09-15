//! A read-only view of the real world; no catalog definition becomes an entity.
use super::*;

pub(crate) fn refresh_context(
    time: Res<Time>,
    catalog: Res<LibraryCatalog>,
    mut state: ResMut<ContentLibrary>,
    mut context: ResMut<LibraryContext>,
    runtime: Res<PlayerFixtureRuntime>,
    npcs: Query<(&CharacterUnitId, &NpcActions)>,
    fixtures: Query<(Entity, &FixtureActivityIdentity, Option<&GlobalTransform>)>,
    players: Query<&GlobalTransform, With<crate::player::PlayerControlled>>,
    mut elapsed: Local<f32>,
    mut was_open: Local<bool>,
) {
    *elapsed += time.delta_secs();
    let opened = state.open && !*was_open;
    *was_open = state.open;
    if context.revision != 0 && !opened && *elapsed < 0.2 {
        state.rebuild_results(&catalog, &context);
        return;
    }
    *elapsed = 0.;
    let actors: HashMap<u32, bool> = npcs
        .iter()
        .map(|(unit, walk)| (unit.0, walk.ready()))
        .collect();
    let mut instances: HashMap<i32, Vec<InstanceView>> = HashMap::new();
    for (entity, identity, transform) in &fixtures {
        let target = FixtureTarget {
            entity,
            uid: identity.uid.clone(),
        };
        let (ready, reason) = match runtime.availability(&target) {
            Some(
                PlayerFixtureAvailability::Ready { .. } | PlayerFixtureAvailability::GimmickReady,
            ) => (true, "可以体验".into()),
            Some(PlayerFixtureAvailability::Pending(error)) => {
                (false, human_preparation_error(error))
            }
            Some(PlayerFixtureAvailability::Unavailable(reason)) => {
                (false, human_reason(reason).into())
            }
            None => (false, "互动资源正在准备".into()),
        };
        let can_stage = matches!(runtime.availability(&target), Some(PlayerFixtureAvailability::Pending(error)) if positional_preparation(error));
        instances
            .entry(identity.master_id)
            .or_default()
            .push(InstanceView {
                target,
                position: transform
                    .map(GlobalTransform::translation)
                    .unwrap_or_default(),
                ready,
                can_stage,
                reason,
            });
    }
    for rows in instances.values_mut() {
        rows.sort_by(|a, b| a.target.uid.cmp(&b.target.uid));
    }
    let changed = context.actors != actors || !same_instances(&context.instances, &instances);
    context.actors = actors;
    context.instances = instances;
    context.player = players.iter().next().map(GlobalTransform::translation);
    if changed || context.revision == 0 {
        context.revision = context.revision.wrapping_add(1);
    }
    state.rebuild_results(&catalog, &context);
    if state.selected_uid.is_none() {
        if let Some(key) = state.selected {
            if let Some(instance) = instances_for(key, &catalog, &context).first() {
                state.selected_uid = Some(instance.target.uid.clone());
            }
        }
    }
}
/// These checks depend on the player's current location. The library may
/// arrange a real nearby position first, then must pass the SAME runtime
/// preparation again. No missing binding/asset/identity is excused here.
fn positional_preparation(
    error: &crate::player_fixture_action::PlayerFixturePreparationError,
) -> bool {
    use crate::player_fixture_action::PlayerFixturePreparationError as Error;
    matches!(
        error,
        Error::Missing("locator radius tile identities")
            | Error::Rejected("no unoccupied reachable player locator")
    )
}
fn same_instances(
    a: &HashMap<i32, Vec<InstanceView>>,
    b: &HashMap<i32, Vec<InstanceView>>,
) -> bool {
    a.len() == b.len()
        && a.iter().all(|(id, rows)| {
            b.get(id).is_some_and(|other| {
                rows.len() == other.len()
                    && rows.iter().zip(other).all(|(left, right)| {
                        left.target.entity == right.target.entity
                            && left.target.uid == right.target.uid
                            && left.ready == right.ready
                            && left.can_stage == right.can_stage
                            && left.reason == right.reason
                    })
            })
        })
}
pub(super) fn talk_here(row: &LibraryTalk, world: &LibraryContext) -> bool {
    !row.units.is_empty()
        && row.units.iter().all(|unit| world.actors.contains_key(unit))
        && (row.fixture_ids.is_empty()
            || row.fixture_ids.iter().all(|id| {
                world
                    .instances
                    .get(id)
                    .is_some_and(|items| !items.is_empty())
            }))
}
pub(super) fn talk_reason(
    row: &LibraryTalk,
    catalog: &LibraryCatalog,
    world: &LibraryContext,
) -> Option<String> {
    if row.units.is_empty() {
        return Some("这段内容没有有效的登场角色，暂时只能阅读台词".into());
    }
    let missing: Vec<_> = row
        .units
        .iter()
        .filter(|unit| !world.actors.contains_key(unit))
        .map(|unit| catalog.character(*unit))
        .collect();
    if !missing.is_empty() {
        return Some(format!("需要 {} 来到当前场景", missing.join("、")));
    }
    let loading: Vec<_> = row
        .units
        .iter()
        .filter(|unit| world.actors.get(unit) != Some(&true))
        .map(|unit| catalog.character(*unit))
        .collect();
    if !loading.is_empty() {
        return Some(format!("{} 还在准备，请稍候", loading.join("、")));
    }
    if !row.fixture_ids.is_empty()
        && !row
            .fixture_ids
            .iter()
            .all(|id| world.instances.get(id).is_some_and(|rows| !rows.is_empty()))
    {
        let names = row
            .fixture_ids
            .iter()
            .map(|id| catalog.fixture_name(*id))
            .collect::<Vec<_>>();
        return Some(format!("需要在当前场景摆放 {}", names.join(" / ")));
    }
    None
}
pub(super) fn instances_for<'a>(
    key: EntryKey,
    catalog: &LibraryCatalog,
    world: &'a LibraryContext,
) -> Vec<&'a InstanceView> {
    let ids = match key {
        EntryKey::Fixture(id) => vec![id],
        _ => catalog
            .talk(key)
            .map(|row| row.fixture_ids.clone())
            .unwrap_or_default(),
    };
    let mut seen = HashSet::new();
    let mut result: Vec<_> = ids
        .iter()
        .filter_map(|id| world.instances.get(id))
        .flatten()
        .filter(|row| seen.insert(row.target.entity))
        .collect();
    result.sort_by(|a, b| {
        if let Some(player) = world.player {
            a.position
                .distance_squared(player)
                .total_cmp(&b.position.distance_squared(player))
                .then(a.target.uid.cmp(&b.target.uid))
        } else {
            a.target.uid.cmp(&b.target.uid)
        }
    });
    result
}
pub(super) fn selected_instance<'a>(
    state: &ContentLibrary,
    catalog: &LibraryCatalog,
    world: &'a LibraryContext,
) -> Option<&'a InstanceView> {
    let key = state.selected?;
    let instances = instances_for(key, catalog, world);
    match state.selected_uid.as_deref() {
        Some(uid) => instances.into_iter().find(|row| row.target.uid == uid),
        None => instances.into_iter().next(),
    }
}
pub(super) fn selected_reason(
    state: &ContentLibrary,
    catalog: &LibraryCatalog,
    world: &LibraryContext,
) -> Option<String> {
    let Some(key) = state.selected else {
        return Some("先选一段内容，或换个关键词试试".into());
    };
    if state.mode == ExperienceMode::Independent {
        return independent_reason(key, catalog);
    }
    match key {
        EntryKey::Fixture(id) => {
            let Some(row) = catalog.fixture(id) else {
                return Some("这件家具已不在图鉴中".into());
            };
            if !row.interactive() {
                return Some("这是一件陈设家具，没有角色互动；相关故事仍可浏览".into());
            }
            let Some(instance) = selected_instance(state, catalog, world) else {
                return Some(if state.selected_uid.is_some() {
                    "刚才选择的家具已移除，请重新选择实例".into()
                } else {
                    "在当前场景摆放这件家具后，就能在这里体验".into()
                });
            };
            if !instance.ready
                && !instance.can_stage
                && !(state.active.is_some() && instance.reason == "另一段家具互动正在进行")
            {
                return Some(instance.reason.clone());
            }
        }
        _ => {
            let Some(row) = catalog.talk(key) else {
                return Some("这段内容已不在图鉴中".into());
            };
            if let Some(reason) = talk_reason(row, catalog, world) {
                return Some(reason);
            }
            if !row.fixture_ids.is_empty() && selected_instance(state, catalog, world).is_none() {
                return Some("刚才选择的家具已移除，请重新选择实例".into());
            }
        }
    }
    None
}

pub(super) fn independent_reason(key: EntryKey, catalog: &LibraryCatalog) -> Option<String> {
    let fixture_reason = |id| match catalog.fixture(id) {
        None => Some(format!("家具 {id} 已不在当前来源的图鉴中")),
        Some(row) if row.source.as_ref().is_none_or(|source| !source.exported) => Some(format!(
            "{} 的原始模型尚未完整导出，暂时只能查看图鉴",
            row.name
        )),
        Some(_) => None,
    };
    match key {
        EntryKey::Fixture(id) => fixture_reason(id),
        _ => {
            let row = catalog.talk(key)?;
            if row.units.is_empty() {
                return Some("这段内容没有有效的登场角色，暂时只能阅读台词".into());
            }
            row.fixture_ids.iter().find_map(|id| fixture_reason(*id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row() -> LibraryTalk {
        LibraryTalk {
            content: TalkContent {
                master_id: 6998,
                backend: TalkBackend::Fixture,
                is_general: None,
            },
            units: vec![1],
            fixture_ids: vec![837],
            title: "测试".into(),
            preview: String::new(),
            lines: vec![],
            search: String::new(),
            furniture_related: true,
            drives_fixture: true,
        }
    }
    #[test]
    fn a_definition_is_not_a_placed_instance() {
        let mut world = LibraryContext::default();
        world.actors.insert(1, true);
        assert!(!talk_here(&row(), &world));
        assert!(talk_reason(&row(), &LibraryCatalog::default(), &world)
            .unwrap()
            .contains("摆放"));
    }
    #[test]
    fn missing_cast_is_reported_before_playback() {
        assert!(talk_reason(
            &row(),
            &LibraryCatalog::default(),
            &LibraryContext::default()
        )
        .unwrap()
        .contains("来到"));
    }
    #[test]
    fn source_backed_static_furniture_is_inspectable_in_independent_mode() {
        let mut catalog = LibraryCatalog::default();
        catalog.fixtures.push(LibraryFixture {
            id: 7,
            name: "陈设".into(),
            description: String::new(),
            action: "no_action".into(),
            search: String::new(),
            thumbnail: None,
            source: Some(FixtureSource {
                package: "mysekai__fixture__static".into(),
                grid_size: moly_law::fixture::Vector3Int::new(1, 1, 1),
                exported: true,
                layout: moly_law::fixture::position::layout_type::FLOOR,
                center_y: 0,
            }),
        });
        assert_eq!(independent_reason(EntryKey::Fixture(7), &catalog), None);
    }
}
