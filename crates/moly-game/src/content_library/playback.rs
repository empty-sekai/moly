//! Admission and transport state. Script execution remains in its original owner.
use super::*;
use crate::player_talk::{PlayerTalkLedger, PlayerTalkOutcome};

#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch(
    mut commands: Commands,
    mut state: ResMut<ContentLibrary>,
    catalog: Res<LibraryCatalog>,
    player_session: Option<Res<crate::player_talk::PlayerTalkSession>>,
    pair_session: Option<Res<crate::talk::ActiveTalk>>,
    mut runtime: ResMut<PlayerFixtureRuntime>,
    npcs: Query<(Entity, &CharacterUnitId, &NpcActions)>,
    instances: Query<&FixtureActivityIdentity>,
    mut talks: MessageWriter<PlayerTalkRequest>,
    mut fixtures: MessageWriter<PlayerFixtureRequest>,
    ledger: Option<ResMut<PlayerTalkLedger>>,
    preview: Option<Res<staging::ScenePreview>>,
    staged: Query<(&Transform, &GlobalTransform)>,
    epoch: Option<Res<crate::site::GroundEpoch>>,
) {
    if state.cleanup_frames != 0
        || player_session.is_some()
        || pair_session.is_some()
        || runtime.active()
    {
        return;
    }
    let Some(choice) = state.pending.clone() else {
        return;
    };
    if preview.as_ref().is_none_or(|preview| {
        epoch.as_ref().is_none_or(|epoch| epoch.0 != preview.epoch)
            || !preview.ready(&choice, &staged)
    }) {
        return;
    }
    if let Some(target) = &choice.target {
        let valid = instances.get(target.entity).is_ok_and(|identity| {
            identity.uid == target.uid
                && match choice.key {
                    EntryKey::Fixture(id) => identity.master_id == id,
                    _ => catalog
                        .talk(choice.key)
                        .is_some_and(|row| row.fixture_ids.contains(&identity.master_id)),
                }
        });
        if !valid {
            fail(&mut state, "所选家具已移动或移除，请重新选择场景中的实例");
            return;
        }
    }
    let mut effect_owner = None;
    let static_view = matches!(choice.key, EntryKey::Fixture(id)
        if choice.mode == ExperienceMode::Independent && catalog.fixture(id).is_some_and(|row| !row.interactive()));
    if static_view {
        let title = catalog.title(choice.key);
        state.active = Some(ActiveChoice {
            choice,
            title,
            started: true,
            elapsed: 0.,
            effect_owner: None,
            static_view: true,
        });
        state.pending = None;
        state.open = false;
        state.watching = true;
        state.search_focus = false;
        state.release_guard = 2;
        state.status = "正在查看陈设家具；返回时会恢复原场景".into();
        state.changed();
        return;
    }
    match choice.key {
        EntryKey::Talk(_, _) => {
            let Some(row) = catalog.talk(choice.key) else {
                fail(&mut state, "所选对话已不在图鉴中");
                return;
            };
            let mut cast = Vec::new();
            for unit in &row.units {
                let Some((entity, _, _)) = npcs
                    .iter()
                    .find(|(_, member, action)| member.0 == *unit && action.ready())
                else {
                    fail(
                        &mut state,
                        &format!("{} 还没有准备好，请稍候再试", catalog.character(*unit)),
                    );
                    return;
                };
                cast.push((*unit, entity));
            }
            let Some((unit, entity)) = cast.first().copied() else {
                fail(&mut state, "这段对话缺少有效的登场角色");
                return;
            };
            if !row.fixture_ids.is_empty() && choice.target.is_none() {
                fail(&mut state, "这段故事需要场景中真实摆放的家具");
                return;
            }
            // A prior rejection of the same ID must not become this request's
            // result. Admission publishes a fresh outcome in the shared ledger.
            if let Some(mut ledger) = ledger {
                ledger.last_outcome = None;
            }
            talks.write(PlayerTalkRequest {
                entity,
                unit,
                exact: Some(row.content.clone()),
                target_fixture: choice.target.as_ref().map(|target| target.entity),
            });
        }
        EntryKey::Fixture(id) => {
            let Some(row) = catalog.fixture(id) else {
                fail(&mut state, "所选家具已不在图鉴中");
                return;
            };
            if !row.interactive() {
                fail(&mut state, "这是一件陈设家具，没有角色互动");
                return;
            }
            let Some(target) = choice.target.clone() else {
                fail(&mut state, "请先在当前场景摆放这件家具");
                return;
            };
            match runtime.availability(&target) {
                Some(
                    PlayerFixtureAvailability::Ready { .. }
                    | PlayerFixtureAvailability::GimmickReady,
                ) => {
                    runtime.last_outcome = None;
                    let request = if row.action == "timeline" {
                        PlayerFixtureRequest::Timeline(target)
                    } else {
                        let owner = commands.spawn(LibraryPreviewOwner).id();
                        effect_owner = Some(owner);
                        PlayerFixtureRequest::PreviewGimmick { target, owner }
                    };
                    fixtures.write(request);
                }
                Some(PlayerFixtureAvailability::Pending(error)) => {
                    fail(&mut state, &human_preparation_error(error));
                    return;
                }
                Some(PlayerFixtureAvailability::Unavailable(reason)) => {
                    fail(&mut state, human_reason(reason));
                    return;
                }
                None => {
                    fail(&mut state, "家具互动资源正在准备，请稍候");
                    return;
                }
            }
        }
    }
    let title = catalog.title(choice.key);
    info!(
        "[content-library] requested {:?}, target={:?}",
        choice.key,
        choice.target.as_ref().map(|target| &target.uid)
    );
    state.active = Some(ActiveChoice {
        choice,
        title,
        started: false,
        elapsed: 0.,
        effect_owner,
        static_view: false,
    });
    state.pending = None;
    state.status = "正在准备所选内容…".into();
    state.changed();
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn observe_start(
    time: Res<Time>,
    mut state: ResMut<ContentLibrary>,
    player_session: Option<Res<crate::player_talk::PlayerTalkSession>>,
    pair_session: Option<Res<crate::talk::ActiveTalk>>,
    mut runtime: ResMut<PlayerFixtureRuntime>,
    ledger: Option<Res<PlayerTalkLedger>>,
    mut cancel: MessageWriter<TalkCancelRequest>,
    mut fixtures: MessageWriter<PlayerFixtureRequest>,
    gimmicks: Res<crate::fixture_gimmick::Gimmicks>,
) {
    let effect_running = state
        .active
        .as_ref()
        .and_then(|active| active.effect_owner)
        .is_some_and(|owner| crate::fixture_gimmick::session::active(&gimmicks, owner));
    let busy =
        player_session.is_some() || pair_session.is_some() || runtime.active() || effect_running;
    if state.stopping {
        if !busy && state.cleanup_frames == 0 {
            state.active = None;
            state.stopping = false;
            state.status = "已停止，场景已恢复".into();
            state.changed();
            info!("[content-library] stopped; talk owners and player fixture owner are idle");
        }
        return;
    }
    let Some(mut active) = state.active.clone() else {
        return;
    };
    active.elapsed += time.delta_secs();
    if active.static_view {
        state.active = Some(active);
        return;
    }
    let running = match active.choice.key {
        EntryKey::Talk(TalkBackend::General, id) => player_session
            .as_ref()
            .is_some_and(|session| session.talk_id() == id),
        EntryKey::Talk(TalkBackend::Fixture, id) => pair_session
            .as_ref()
            .is_some_and(|session| session.talk_id() == id),
        EntryKey::Fixture(_) => {
            effect_running
                || active.choice.target.as_ref().is_some_and(|chosen| {
                    runtime.target().is_some_and(|target| {
                        target.entity == chosen.entity && target.uid == chosen.uid
                    })
                })
        }
    };
    if !active.started {
        if matches!(active.choice.key, EntryKey::Fixture(_)) && !running {
            use crate::player_fixture_action::PlayerFixtureOutcome;
            let reason = match &runtime.last_outcome {
                Some(PlayerFixtureOutcome::GimmickNotPrepared { target, reason })
                    if active.choice.target.as_ref() == Some(target) =>
                {
                    Some(rejection_message(reason).to_owned())
                }
                Some(PlayerFixtureOutcome::NotPrepared(error)) => {
                    Some(human_preparation_error(error))
                }
                _ => None,
            };
            if let Some(reason) = reason {
                fail(&mut state, &reason);
                return;
            }
        }
        if let Some(reason) = ledger
            .as_ref()
            .and_then(|ledger| ledger.last_outcome.as_ref())
            .and_then(|outcome| match outcome {
                PlayerTalkOutcome::Rejected {
                    requested: Some(content),
                    reason,
                } if active.choice.key == EntryKey::Talk(content.backend, content.master_id) => {
                    Some(reason.clone())
                }
                _ => None,
            })
        {
            warn!(
                "[content-library] admission rejected {:?}: {reason}",
                active.choice.key
            );
            fail(&mut state, rejection_message(&reason));
            return;
        }
        if running {
            active.started = true;
            active.elapsed = 0.;
            state.open = false;
            state.watching = true;
            state.search_focus = false;
            state.release_guard = 2;
            state.status = "播放中".into();
            state.changed();
            info!("[content-library] started {:?}", active.choice.key);
        } else if active.elapsed >= 8. {
            cancel.write(TalkCancelRequest);
            fixtures.write(PlayerFixtureRequest::Cancel(
                PlayerFixtureCancelReason::User,
            ));
            fail(
                &mut state,
                "未能开始播放。场景条件或资源可能已变化，请重新选择后再试",
            );
            return;
        }
    } else if !running && state.pending.is_none() {
        state.active = None;
        state.status = "播放结束，可以再看一遍或返回对话与互动".into();
        state.changed();
        info!("[content-library] finished {:?}", active.choice.key);
        return;
    }
    state.active = Some(active);
}
pub(super) fn fail(state: &mut ContentLibrary, reason: &str) {
    state.pending = None;
    state.active = None;
    state.stopping = false;
    state.open = true;
    state.watching = false;
    state.release_guard = 2;
    state.status = reason.to_owned();
    state.changed();
}
fn rejection_message(reason: &str) -> &str {
    if reason.contains("throttl") {
        "切换得有点快，请稍候再试"
    } else if reason.contains("another") || reason.contains("active") || reason.contains("busy") {
        "角色正在进行其他互动，请稍候再试"
    } else if reason.contains("coher")
        || reason.contains("distance")
        || reason.contains("far")
        || reason.contains("anchor")
    {
        "角色与所需家具暂时不在一起，等他们靠近后再试"
    } else if reason.contains("fixture") {
        "所需家具当前不可用，请重新选择场景实例"
    } else if reason.contains("actor") || reason.contains("cast") || reason.contains("NPC") {
        "所需角色还没有准备好，请稍候再试"
    } else if reason.contains("loaded") || reason.contains("catalog") {
        "这段内容的资源暂不可用，台词仍可阅读"
    } else if reason.contains("player") {
        "玩家角色还在准备，请稍候再试"
    } else {
        "当前场景不满足这段内容的播放条件，台词仍可阅读"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_clears_pending_and_returns_to_browsing() {
        let mut state = ContentLibrary::default();
        state.watching = true;
        state.pending = Some(PlaybackChoice {
            key: EntryKey::Fixture(157),
            target: None,
            ticket: 1,
            mode: ExperienceMode::CurrentScene,
        });
        fail(&mut state, "不可用");
        assert!(state.open);
        assert!(!state.watching);
        assert!(state.pending.is_none());
        assert_eq!(state.status, "不可用");
    }
    #[test]
    fn user_messages_do_not_leak_runtime_fields() {
        assert_eq!(
            rejection_message("requested NPC entity/unit identity changed"),
            "所需角色还没有准备好，请稍候再试"
        );
    }
}

/// An entity generation is the lease identity: repeating the same furniture or
/// talk ID can never make stale cleanup target its successor.
#[derive(Component)]
struct LibraryPreviewOwner;
pub(crate) fn reap_preview_owners(world: &mut World) {
    let current = world
        .resource::<ContentLibrary>()
        .active
        .as_ref()
        .and_then(|active| active.effect_owner);
    let retired: Vec<_> = world
        .query_filtered::<Entity, With<LibraryPreviewOwner>>()
        .iter(world)
        .filter(|owner| Some(*owner) != current)
        .collect();
    for owner in retired {
        crate::fixture_gimmick::session::finish_owner(world, owner);
        if let Ok(entity) = world.get_entity_mut(owner) {
            entity.despawn();
        }
    }
}
