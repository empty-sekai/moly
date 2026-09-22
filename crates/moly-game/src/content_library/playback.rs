//! Admission and transport state. Script execution remains in its original owner.
use super::*;
use crate::npc_fixture_activity::preview::{PreviewPhase, PreviewRecord};
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
    balloon_art: Option<Res<crate::balloon::BalloonArt>>,
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
                    EntryKey::Activity(id) => catalog
                        .activity(id)
                        .is_some_and(|row| row.spec.fixture_id == identity.master_id),
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
            completed: false,
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
        EntryKey::Activity(key) => {
            let Some(target) = choice.target.clone() else {
                fail(&mut state, "所选角色互动需要一个真实家具实例");
                return;
            };
            let ticket = choice.ticket;
            commands.queue(move |world: &mut World| {
                crate::npc_fixture_activity::preview::start(world, key, target, ticket);
            });
        }
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
            if choice.preview {
                let Some(tweet) = row.preview_tweet.clone() else {
                    fail(&mut state, "这段对话没有原始前置气泡数据");
                    return;
                };
                if balloon_art.is_none() {
                    return;
                }
                let ticket = choice.ticket;
                commands.queue(move |world: &mut World| {
                    crate::balloon::show_talk_preview(world, entity, unit, &tweet, ticket);
                });
                state.active = Some(ActiveChoice {
                    title: catalog.title(choice.key),
                    choice,
                    started: true,
                    completed: false,
                    elapsed: 0.,
                    effect_owner: None,
                    static_view: false,
                });
                state.pending = None;
                state.open = false;
                state.watching = true;
                state.release_guard = 2;
                state.status = "前置气泡预览；进入对话后播放完整内容".into();
                state.changed();
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
                    if choice.mode == ExperienceMode::Independent {
                        state.status =
                            format!("正在准备家具互动：{}", human_preparation_error(error));
                        state.last_error = Some(human_preparation_error(error));
                        return;
                    }
                    fail(&mut state, &human_preparation_error(error));
                    return;
                }
                Some(PlayerFixtureAvailability::Unavailable(reason)) => {
                    fail(&mut state, human_reason(reason));
                    return;
                }
                None => {
                    if choice.mode == ExperienceMode::Independent {
                        state.status = "正在等待家具互动资源就绪…".into();
                        return;
                    }
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
    state.last_error = None;
    state.active = Some(ActiveChoice {
        choice,
        title,
        started: false,
        completed: false,
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
    activity: Option<Res<PreviewRecord>>,
) {
    let effect_running = state
        .active
        .as_ref()
        .and_then(|active| active.effect_owner)
        .is_some_and(|owner| crate::fixture_gimmick::session::active(&gimmicks, owner));
    let busy = player_session.is_some()
        || pair_session.is_some()
        || runtime.active()
        || effect_running
        || activity.as_ref().is_some_and(|record| record.active());
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
    if active.completed {
        return;
    }
    active.elapsed += time.delta_secs();
    if active.static_view || active.choice.preview {
        state.active = Some(active);
        return;
    }
    if let EntryKey::Activity(key) = active.choice.key {
        if let Some(record) = activity
            .as_ref()
            .filter(|record| record.ticket == active.choice.ticket && record.key == key)
        {
            if record.phase == PreviewPhase::Failed {
                let reason = record.error.as_deref().unwrap_or("原始角色互动资源不可用");
                fail(&mut state, &format!("角色互动无法完成：{reason}"));
                return;
            }
            if record.active() {
                state.status = record.label().into();
            }
        }
    }
    let running = match active.choice.key {
        EntryKey::Activity(key) => activity.as_ref().is_some_and(|record| {
            record.ticket == active.choice.ticket && record.key == key && record.active()
        }),
        EntryKey::Talk(TalkBackend::General, id) => player_session
            .as_ref()
            .is_some_and(|session| session.talk_id() == id),
        EntryKey::Talk(TalkBackend::Fixture, id) => pair_session
            .as_ref()
            // Body admission starts playback; the same owner stays active
            // while its window is closed and the authored ending still runs.
            .is_some_and(|session| {
                session.talk_id() == id && (active.started || session.body_ready())
            }),
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
        } else if active.elapsed >= 20.
            && !matches!(active.choice.key, EntryKey::Talk(TalkBackend::Fixture, id)
                if pair_session.as_ref().is_some_and(|session| session.talk_id() == id))
        {
            // Once the exact fixture talk owns its preparation, that owner
            // times loading, navigation and the authored pre-action. A second
            // 20-second admission clock must not cancel a healthy long start.
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
        // The action owners have completed their normal cleanup. Keep the
        // preview ticket alive so staging retains its actors, furniture and
        // return snapshot until an explicit stop/close or replacement intent.
        active.completed = true;
        state.active = Some(active.clone());
        state.status = "播放完毕，场景已保留；可重新播放、选择下一项或返回原场景".into();
        state.changed();
        info!("[content-library] finished {:?}", active.choice.key);
        return;
    }
    state.active = Some(active);
}
pub(super) fn fail(state: &mut ContentLibrary, reason: &str) {
    warn!("[content-library] preparation failed: {reason}");
    let message = human_playback_failure(reason);
    state.last_error = Some(message.clone());
    state.pending = None;
    state.active = None;
    state.stopping = false;
    state.open = true;
    state.watching = false;
    state.release_guard = 2;
    state.status = message;
    state.changed();
}
/// Runtime diagnostics stay in logs; both native and browser chrome present
/// an actionable sentence instead of package paths, source keys or ECS types.
pub(super) fn human_playback_failure(reason: &str) -> String {
    let lower = reason.to_ascii_lowercase();
    let contains = |tokens: &[&str]| tokens.iter().any(|token| lower.contains(token));
    if contains(&["failed to fetch", "http", "network", "timed out", "timeout"])
        || reason.contains("限定时间")
        || reason.contains("准备未完成")
    {
        "所需资源暂时未能载入，请稍候重试。场景会自动恢复。".into()
    } else if contains(&[
        "actor-source-library",
        "actor source clip",
        "source-routed animation",
        "actor animation",
        "source animation",
        "sourceclip",
    ]) {
        "这段互动所需的角色动作尚不可用，可以先欣赏其他内容。".into()
    } else if contains(&["source se", "source-sounds", "audio", "sound"]) {
        "这段演出的声音资源暂时不可用，请稍候重试。".into()
    } else if contains(&["occupied", "reservation", "another", "replaced", "lease"])
        || reason.contains("占用")
    {
        "角色或家具正在进行其他互动，请停止当前体验后重试。".into()
    } else if contains(&[
        "locator",
        "endloc",
        "approach",
        "navigation",
        "path slot",
        "route owner",
    ]) {
        "角色暂时无法到达家具的互动位置，请重新选择家具后重试。".into()
    } else if contains(&[
        "timeline",
        "prefab",
        "json-",
        "gltf",
        "glb",
        "source-",
        "binding",
        "animator",
        "entity/uid",
        "mat_wall_",
        "mat_floor_",
    ]) || reason.contains("assets/")
        || reason.contains("moly://")
    {
        "这项内容的演出资源尚不完整，暂时无法播放。".into()
    } else if reason
        .chars()
        .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
    {
        reason.to_owned()
    } else {
        "这项内容暂时无法播放，请重新选择后再试。".into()
    }
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
            preview: false,
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
    let retained = {
        let state = world.resource::<ContentLibrary>();
        if state.stopping || state.pending.is_some() {
            None
        } else {
            state
                .active
                .as_ref()
                .filter(|active| matches!(active.choice.key, EntryKey::Activity(_)))
                .map(|active| active.choice.ticket)
        }
    };
    crate::npc_fixture_activity::preview::reap(world, retained);
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

#[cfg(test)]
mod user_failure_tests {
    use super::*;
    #[test]
    fn technical_actor_failure_never_enters_product_status() {
        let raw = "actor-source-library: unit 29, prefab assets/sekai/surprisebox.prefab: actor source clip is missing or ambiguous";
        let mut state = ContentLibrary::default();
        fail(&mut state, raw);
        assert!(state.status.contains("角色动作"));
        assert!(!state.status.contains("prefab"));
        assert!(!state.last_error.as_ref().unwrap().contains("unit 29"));
        assert!(state.active.is_none() && state.pending.is_none());
    }
    #[test]
    fn user_context_errors_are_preserved_but_unknown_internal_errors_are_not() {
        assert_eq!(
            human_playback_failure("请先退出家具布局编辑，再开始独立体验"),
            "请先退出家具布局编辑，再开始独立体验"
        );
        assert!(!human_playback_failure("fixture model matrix invariant X99").contains("X99"));
        assert!(
            human_playback_failure("Asset HTTP request failed for effects.json: TypeError")
                .contains("载入")
        );
    }
}
