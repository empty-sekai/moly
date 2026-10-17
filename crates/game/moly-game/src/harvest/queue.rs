//! The harvest log loop (`HarvestUserDataManager.ScheduleExecuteHarvestLogData`).
//!
//! Each hit enqueues a harvest stack (`HarvestStackData`, event 32) and each
//! collected drop a gather stack (`GatherStackData`). While a harvest site is
//! loaded, a loop waits `HarvestAPIInterval` (FloatConfigs 81) and flushes
//! the queue: consecutive harvest stacks on the same fixture (key id, x, z)
//! merge into one request unless the tool changed or broke in between
//! (`GatheringActionLogToAPIConverter`); gather stacks become one gather
//! request. One harvest request at a time goes to the server as
//! `Call::Harvest` (the hit, the run's tool and the stamina that is left);
//! its answer updates the stamina and the tools, and this client applies its
//! own hit to the map, from which each object's pending drop rows are rebuilt.
//! The gather requests go to the `HarvestServerMock` panel; its replies update
//! the user data (event 30: stamina and tools; event 31: the harvest map).

use std::collections::{HashSet, VecDeque};

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use moly_link::ServerLink;
use moly_protocol::api::{
    HarvestRequest as HarvestCall, MysekaiSiteHarvestFixtureTarget, MysekaiStaminaUsed,
    MysekaiToolUsed,
};
use moly_protocol::{Call, Refusal, Reply};

use super::action::HarvestPlayerModel;
use super::catalog::{HarvestCatalog, HarvestUserData};
use super::law::Stamina;
use super::server_mock::{
    GatherRequest, HarvestRequest, HarvestServerMock, SuiteUserReply, UserDrop, UserHarvestMap,
    DROP_BEFORE,
};
use super::HarvestObject;
use crate::site_move::timeline::Delay;
use crate::userdata::{Answered, ServerPending};

/// One hit as the log records it.
#[derive(Clone, Debug)]
pub(crate) struct HarvestStack {
    pub(crate) site_id: u32,
    pub(crate) position_x: i32,
    pub(crate) position_z: i32,
    pub(crate) fixture_id: i32,
    /// The object's hp after the hit.
    pub(crate) hp: i32,
    pub(crate) is_last_attack: bool,
    /// (tool id, durability after the hit, the tool broke on this hit).
    pub(crate) tool: Option<(i64, i32, bool)>,
    pub(crate) stamina_after: Stamina,
}

#[derive(Clone, Debug)]
pub(crate) enum Stack {
    Harvest(HarvestStack),
    Gather { site_id: u32, drop: UserDrop },
}

#[derive(Resource, Default)]
pub(crate) struct HarvestLogQueue {
    pub(crate) stacks: Vec<Stack>,
    /// Objects whose last attack is already logged (`IsDestroyedByLastAttack`).
    pub(crate) destroyed: HashSet<u64>,
    timer: Option<(u32, Delay)>,
    pub(crate) harvest_requests: usize,
    pub(crate) gather_requests: usize,
    /// The harvest call whose answer has not arrived.
    awaiting: Option<ServerPending>,
    /// The request the waiting call carries, for the map's hit on its answer.
    in_flight: Option<HarvestRequest>,
    /// Merged requests of the flush after the one in flight.
    pending: VecDeque<HarvestRequest>,
}

impl HarvestLogQueue {
    /// The server call this loop waits for, for the director.
    #[allow(dead_code)] // Read by the director's wait.
    pub(crate) fn server_pending(&self) -> Option<&ServerPending> {
        self.awaiting.as_ref()
    }
}

/// `CreateHarvestRequests`: merge consecutive stacks of one fixture while
/// the tool neither changes nor breaks.
pub(crate) fn merge_harvest(stacks: &[HarvestStack]) -> Vec<HarvestRequest> {
    let mut requests: Vec<HarvestRequest> = Vec::new();
    let mut previous: Option<&HarvestStack> = None;
    for stack in stacks {
        let same = previous.is_some_and(|prev| {
            prev.site_id == stack.site_id
                && prev.fixture_id == stack.fixture_id
                && prev.position_x == stack.position_x
                && prev.position_z == stack.position_z
                && prev.tool.map(|t| t.0) == stack.tool.map(|t| t.0)
                && !prev.tool.is_some_and(|t| t.2)
        });
        let rests = [
            stack.stamina_after.normal,
            stack.stamina_after.enhance,
            stack.stamina_after.boost,
        ];
        if same {
            let request = requests.last_mut().expect("merged into a request");
            request.hp = stack.hp;
            request.is_last_attack = stack.is_last_attack;
            request.stamina_rests = rests;
            if let (Some(tool), Some((id, durability, _))) = (request.tool.as_mut(), stack.tool) {
                *tool = (id, tool.1 + 1, durability);
            }
        } else {
            requests.push(HarvestRequest {
                site_id: stack.site_id,
                position_x: stack.position_x,
                position_z: stack.position_z,
                fixture_id: stack.fixture_id,
                hp: stack.hp,
                is_last_attack: stack.is_last_attack,
                tool: stack.tool.map(|(id, durability, _)| (id, 1, durability)),
                stamina_rests: rests,
            });
        }
        previous = Some(stack);
    }
    requests
}

/// Update: the loop and the flush.
#[allow(clippy::too_many_arguments)]
pub(crate) fn advance(
    frames: Res<FrameCount>,
    time: Res<Time>,
    mut answers: MessageReader<Answered>,
    mut link: ResMut<ServerLink>,
    arrival: Res<super::arrival::HarvestArrival>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    mut queue: ResMut<HarvestLogQueue>,
    mut server: Option<ResMut<HarvestServerMock>>,
    mut user: Option<ResMut<HarvestUserData>>,
    mut model: Option<ResMut<HarvestPlayerModel>>,
    catalog: Option<Res<HarvestCatalog>>,
    mut objects: Query<&mut HarvestObject>,
    mut possession: ResMut<super::possession::PossessionMock>,
) {
    let (Some(configs), Some(server), Some(user), Some(model), Some(catalog)) = (
        configs,
        server.as_deref_mut(),
        user.as_deref_mut(),
        model.as_deref_mut(),
        catalog,
    ) else {
        return;
    };
    // The harvest call's answer: the server's stamina is applied here, and the
    // map's hit is this client's own effect of the swing. The tool rows ride
    // the sections and reach the client copy through the user data owner.
    for answer in answers.read() {
        let Some(seq) = queue.awaiting.as_ref().and_then(|pending| pending.seq) else {
            continue;
        };
        if seq != answer.seq {
            continue;
        }
        queue.awaiting = None;
        if let Some(request) = queue.in_flight.take() {
            finish_harvest(
                &request,
                seq,
                &answer.outcome,
                server,
                user,
                model,
                &catalog,
                &mut objects,
            );
        }
        if let Some(next) = queue.pending.pop_front() {
            send_harvest(&mut queue, &mut link, next);
        }
    }
    let Some(site_id) = arrival.site_id() else {
        queue.timer = None;
        return;
    };
    let interval = configs.float(crate::client_config::KEY_HARVEST_API_INTERVAL);
    let frame = u64::from(frames.0);
    let due = match queue.timer.as_mut() {
        Some((site, delay)) if *site == site_id => delay.tick(frame, time.delta_secs()),
        _ => {
            queue.timer = Some((
                site_id,
                Delay::new(super::law::delay_seconds(interval as f64), frame),
            ));
            false
        }
    };
    if !due {
        return;
    }
    queue.timer = Some((
        site_id,
        Delay::new(super::law::delay_seconds(interval as f64), frame),
    ));
    if queue.awaiting.is_some() {
        return;
    }
    if queue.stacks.is_empty() {
        return;
    }
    let stacks = std::mem::take(&mut queue.stacks);
    let harvests: Vec<HarvestStack> = stacks
        .iter()
        .filter_map(|stack| match stack {
            Stack::Harvest(stack) => Some(stack.clone()),
            Stack::Gather { .. } => None,
        })
        .collect();
    let mut gathers: Vec<(u32, UserDrop)> = stacks
        .iter()
        .filter_map(|stack| match stack {
            Stack::Gather { site_id, drop } => Some((*site_id, drop.clone())),
            Stack::Harvest(_) => None,
        })
        .collect();
    let mut requests = merge_harvest(&harvests).into_iter();
    if let Some(first) = requests.next() {
        send_harvest(&mut queue, &mut link, first);
        queue.pending.extend(requests);
    }
    gathers.sort_by_key(|(site, _)| *site);
    let mut index = 0;
    while index < gathers.len() {
        let site = gathers[index].0;
        let drops: Vec<UserDrop> = gathers[index..]
            .iter()
            .take_while(|(s, _)| *s == site)
            .map(|(_, drop)| drop.clone())
            .collect();
        index += drops.len();
        let request = GatherRequest {
            site_id: site,
            drops,
        };
        let reply = server.gather(&request);
        possession.on_gather(&request.drops, &catalog);
        queue.gather_requests += 1;
        info!(
            "[harvest-api] PostUserMysekaiGatherApi (mock) site {site}: {} drops -> materials {:?}; PossessionMock material quantity {}, fixtures {}",
            request.drops.len(),
            reply.materials,
            possession.material_quantity,
            possession.fixtures.values().sum::<i64>(),
        );
        apply_reply(&reply, site, user, model, &catalog, &mut objects);
    }
}

/// Sends one merged harvest request; only one call is in flight at a time.
fn send_harvest(queue: &mut HarvestLogQueue, link: &mut ServerLink, request: HarvestRequest) {
    let call = Call::Harvest(HarvestCall {
        mysekai_site_id: request.site_id as i32,
        mysekai_site_harvest_fixture_target: MysekaiSiteHarvestFixtureTarget {
            position_x: request.position_x,
            position_z: request.position_z,
            mysekai_site_harvest_fixture_id: request.fixture_id,
            hp: request.hp,
            is_last_attack: request.is_last_attack,
        },
        mysekai_tool_used: request.tool.map(|(tool_id, use_count, durability)| {
            MysekaiToolUsed {
                mysekai_tool_id: tool_id as i32,
                use_count,
                durability,
            }
        }),
        mysekai_stamina_useds: vec![
            MysekaiStaminaUsed {
                mysekai_stamina_type: "normal".to_owned(),
                rest: request.stamina_rests[0],
            },
            MysekaiStaminaUsed {
                mysekai_stamina_type: "enhance".to_owned(),
                rest: request.stamina_rests[1],
            },
            MysekaiStaminaUsed {
                mysekai_stamina_type: "boost".to_owned(),
                rest: request.stamina_rests[2],
            },
        ],
    });
    let kind = call.kind();
    let seq = link.call(call);
    info!(
        "[harvest-api] PostUserMysekaiHarvestApi site {} target ({}, {}) fixture {} hp {} last {} tool {:?} stamina rests {:?} sent as seq {seq}",
        request.site_id,
        request.position_x,
        request.position_z,
        request.fixture_id,
        request.hp,
        request.is_last_attack,
        request.tool,
        request.stamina_rests,
    );
    queue.harvest_requests += 1;
    queue.awaiting = Some(ServerPending {
        seq: Some(seq),
        call: kind,
    });
    queue.in_flight = Some(request);
}

/// The harvest answer: the server's stamina, and the map's hit as this client
/// predicts it.
#[allow(clippy::too_many_arguments)]
fn finish_harvest(
    request: &HarvestRequest,
    seq: u32,
    outcome: &Result<Reply, Refusal>,
    server: &mut HarvestServerMock,
    user: &mut HarvestUserData,
    model: &mut HarvestPlayerModel,
    catalog: &HarvestCatalog,
    objects: &mut Query<&mut HarvestObject>,
) {
    match outcome {
        Ok(Reply::Harvest(stamina)) => {
            let stamina = Stamina {
                normal: stamina.normal,
                enhance: stamina.enhance,
                boost: stamina.boost,
            };
            server.set_stamina(stamina);
            info!(
                "[harvest-api] PostUserMysekaiHarvestApi site {} target ({}, {}) fixture {} hp {} last {} tool {:?} stamina rests {:?} -> stamina {:?} (seq {seq})",
                request.site_id,
                request.position_x,
                request.position_z,
                request.fixture_id,
                request.hp,
                request.is_last_attack,
                request.tool,
                request.stamina_rests,
                stamina,
            );
            apply_stamina(stamina, model, user);
            if let Some(map) = server.hit(request) {
                apply_map(&map, request.site_id, user, catalog, objects);
            }
        }
        Err(refusal) => {
            error!(
                "[harvest-api] PostUserMysekaiHarvestApi (seq {seq}) refused: {} ({})",
                refusal.reason, refusal.code
            );
        }
        Ok(other) => {
            error!(
                "[harvest-api] PostUserMysekaiHarvestApi (seq {seq}) answered with {other:?}"
            );
        }
    }
}

/// Event 30: the stamina. Every stack of the flush was sent before the reply,
/// so nothing unsynchronised is left to subtract.
fn apply_stamina(stamina: Stamina, model: &mut HarvestPlayerModel, user: &mut HarvestUserData) {
    if model.stamina != stamina {
        info!(
            "[harvest-api] event 30: stamina {:?} -> server {:?}",
            model.stamina, stamina
        );
    }
    model.stamina = stamina;
    user.stamina = stamina;
}

/// Event 31: the harvest map; the pending drop rows of the site's objects are
/// rebuilt from it (`SynchronizedTargetBeforeDropItems`).
fn apply_map(
    map: &UserHarvestMap,
    site_id: u32,
    user: &mut HarvestUserData,
    catalog: &HarvestCatalog,
    objects: &mut Query<&mut HarvestObject>,
) {
    user.maps.insert(site_id, map.clone());
    let mut rebuilt = 0usize;
    for mut object in objects.iter_mut() {
        if object.site_id != site_id {
            continue;
        }
        let hp = object.hp;
        let rows: Vec<_> = map
            .drops
            .iter()
            .filter(|drop| {
                drop.position_x == object.position_x
                    && drop.position_z == object.position_z
                    && drop.status == DROP_BEFORE
                    && drop.hp <= hp
            })
            .map(|drop| super::pending_drop(catalog, drop))
            .collect();
        if rows.len() != object.pending_drops.len() {
            rebuilt += 1;
        }
        object.pending_drops = rows;
    }
    if rebuilt > 0 {
        info!("[harvest-api] event 31: pending drop rows rebuilt on {rebuilt} objects");
    }
}

/// A `GatherApiMock` reply's user-data events.
fn apply_reply(
    reply: &SuiteUserReply,
    site_id: u32,
    user: &mut HarvestUserData,
    model: &mut HarvestPlayerModel,
    catalog: &HarvestCatalog,
    objects: &mut Query<&mut HarvestObject>,
) {
    apply_stamina(reply.stamina, model, user);
    if let Some(tools) = &reply.tools {
        model.refresh_tools(tools);
        user.tools = tools.clone();
    }
    user.materials = reply.materials.clone();
    if let Some(map) = reply.map.as_ref() {
        apply_map(map, site_id, user, catalog, objects);
    }
}

#[cfg(test)]
mod value_checks {
    use super::*;

    fn stack(fixture_id: i32, hp: i32, tool: Option<(i64, i32, bool)>) -> HarvestStack {
        HarvestStack {
            site_id: 5,
            position_x: 3,
            position_z: -4,
            fixture_id,
            hp,
            is_last_attack: hp == 0,
            tool,
            stamina_after: Stamina {
                normal: 100 - hp,
                enhance: 0,
                boost: 0,
            },
        }
    }

    /// The merge rule: same fixture key and tool merge (use count grows,
    /// the last hp, durability and rests stand); a broken tool or another
    /// fixture starts a new request.
    #[test]
    fn consecutive_stacks_merge_by_fixture_and_tool() {
        let requests = merge_harvest(&[
            stack(7, 70, Some((2, 99, false))),
            stack(7, 50, Some((2, 98, false))),
            stack(7, 30, Some((2, 97, true))),
            stack(7, 10, Some((2, 100, false))),
            stack(8, 0, None),
        ]);
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].tool, Some((2, 3, 97)));
        assert_eq!(requests[0].hp, 30);
        assert_eq!(requests[1].tool, Some((2, 1, 100)));
        assert_eq!(requests[2].tool, None);
        assert!(requests[2].is_last_attack);
    }
}
