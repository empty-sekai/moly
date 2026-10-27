//! Ordinary harvest and gather logs share one authoritative server map.
//! Consecutive hits merge by fixture and tool; harvest requests precede
//! their gathers, one at a time. Bulk waits for this queue to finish.
//! Replies update user maps before Answered is published; the final normal
//! reply settles player prediction after all earlier hit logs are sent.
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
use super::server_mock::{GatherRequest, HarvestRequest, UserDrop, UserHarvestMap, DROP_BEFORE};
use super::HarvestObject;
use crate::site_move::timeline::Delay;
use crate::userdata::{Answered, ServerPending};

/// One hit as the log records it.
#[derive(Clone, Debug)]
pub struct HarvestStack {
    pub site_id: u32,
    pub position_x: i32,
    pub position_z: i32,
    pub fixture_id: i32,
    /// The object's hp after the hit.
    pub hp: i32,
    pub is_last_attack: bool,
    /// (tool id, durability after the hit, the tool broke on this hit).
    pub tool: Option<(i64, i32, bool)>,
    pub stamina_after: Stamina,
}

#[derive(Clone, Debug)]
pub enum Stack {
    Harvest(HarvestStack),
    Gather { site_id: u32, drop: UserDrop },
}

#[derive(Resource, Default)]
pub struct HarvestLogQueue {
    pub stacks: Vec<Stack>,
    /// Objects whose last attack is already logged (`IsDestroyedByLastAttack`).
    pub destroyed: HashSet<u64>,
    timer: Option<(u32, Delay)>,
    pub harvest_requests: usize,
    pub gather_requests: usize,
    /// The harvest call whose answer has not arrived.
    awaiting: Option<ServerPending>,
    /// The request the waiting call carries, for the map's hit on its answer.
    in_flight: Option<HarvestRequest>,
    /// Merged requests of the flush after the one in flight.
    pending: VecDeque<HarvestRequest>,
    gathers: VecDeque<GatherRequest>,
    gather_in_flight: Option<GatherRequest>,
    force_flush: bool,
    refused: bool,
}

impl HarvestLogQueue {
    /// Gather rows already sent or waiting behind a hit still occupy capacity
    /// until their authoritative reply updates the possession count.
    pub fn pending_gather_quantity(&self) -> i64 {
        self.gathers
            .iter()
            .chain(self.gather_in_flight.iter())
            .flat_map(|request| &request.drops)
            .map(|row| i64::from(row.quantity))
            .sum()
    }

    pub fn has_failed(&self) -> bool {
        self.refused
    }
    /// A fresh authoritative arrival restores any prediction refused before
    /// leaving the old site. Uncollected server drops will be placed again.
    pub fn reset_refused_on_arrival(&mut self) {
        if self.refused {
            self.stacks.clear();
            self.pending.clear();
            self.gathers.clear();
            self.in_flight = None;
            self.gather_in_flight = None;
            self.awaiting = None;
            self.refused = false;
            self.destroyed.clear();
        }
    }
    pub fn request_flush(&mut self) {
        self.force_flush = true;
    }
    pub fn is_idle(&self) -> bool {
        self.stacks.is_empty()
            && self.awaiting.is_none()
            && self.pending.is_empty()
            && self.gathers.is_empty()
            && !self.refused
    }

    /// The server call this loop waits for, for the director.
    #[allow(dead_code)] // Read by the director's wait.
    pub fn server_pending(&self) -> Option<&ServerPending> {
        self.awaiting.as_ref()
    }
}

/// `CreateHarvestRequests`: merge consecutive stacks of one fixture while
/// the tool neither changes nor breaks.
pub fn merge_harvest(stacks: &[HarvestStack]) -> Vec<HarvestRequest> {
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
pub fn advance(
    frames: Res<FrameCount>,
    time: Res<Time>,
    mut answers: MessageReader<Answered>,
    mut link: ResMut<ServerLink>,
    arrival: Res<super::arrival::HarvestArrival>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    mut queue: ResMut<HarvestLogQueue>,
    mut user: Option<ResMut<HarvestUserData>>,
    mut model: Option<ResMut<HarvestPlayerModel>>,
    catalog: Option<Res<HarvestCatalog>>,
    mut objects: Query<&mut HarvestObject>,
) {
    let (Some(configs), Some(user), Some(model), Some(catalog)) =
        (configs, user.as_deref_mut(), model.as_deref_mut(), catalog)
    else {
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
                user,
                model,
                &catalog,
                &mut objects,
                queue.pending.is_empty() && queue.stacks.is_empty(),
            );
        }
        if let Some(request) = queue.gather_in_flight.take() {
            match &answer.outcome {
                Ok(Reply::Gather(_)) => {
                    if let Some(map) = user.maps.get(&request.site_id).cloned() {
                        apply_map(&map, request.site_id, user, &catalog, &mut objects);
                    }
                }
                Err(error) => {
                    queue.refused = true;
                    error!("[harvest-api] gather refused: {error}");
                }
                _ => {
                    queue.refused = true;
                    error!("[harvest-api] unexpected gather reply");
                }
            }
        }
        if answer.outcome.is_err() {
            queue.refused = true;
        }
        if !queue.refused {
            if let Some(next) = queue.pending.pop_front() {
                send_harvest(&mut queue, &mut link, next);
            } else if let Some(next) = queue.gathers.pop_front() {
                send_gather(&mut queue, &mut link, next);
            }
        }
    }
    let queued_site = queue.stacks.first().map(|stack| match stack {
        Stack::Harvest(row) => row.site_id,
        Stack::Gather { site_id, .. } => *site_id,
    });
    let Some(site_id) = arrival
        .site_id()
        .or(queued_site.filter(|_| queue.force_flush))
    else {
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
    if !due && !queue.force_flush {
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
    if queue.refused {
        return;
    }
    queue.force_flush = false;
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
        queue.gathers.push_back(request);
    }
    if queue.awaiting.is_none() {
        if let Some(next) = queue.gathers.pop_front() {
            send_gather(&mut queue, &mut link, next);
        }
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
        mysekai_tool_used: request
            .tool
            .map(|(tool_id, use_count, durability)| MysekaiToolUsed {
                mysekai_tool_id: tool_id as i32,
                use_count,
                durability,
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
    user: &mut HarvestUserData,
    model: &mut HarvestPlayerModel,
    catalog: &HarvestCatalog,
    objects: &mut Query<&mut HarvestObject>,
    settle_prediction: bool,
) {
    match outcome {
        Ok(Reply::Harvest(stamina)) => {
            let stamina = Stamina {
                normal: stamina.normal,
                enhance: stamina.enhance,
                boost: stamina.boost,
            };
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
            user.stamina = stamina;
            if settle_prediction {
                apply_stamina(stamina, model, user);
            }
            if let Some(map) = user.maps.get(&request.site_id).cloned() {
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
            error!("[harvest-api] PostUserMysekaiHarvestApi (seq {seq}) answered with {other:?}");
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

fn send_gather(queue: &mut HarvestLogQueue, link: &mut ServerLink, request: GatherRequest) {
    let drops: Option<Vec<_>> = request
        .drops
        .iter()
        .map(moly_protocol::api::MysekaiSiteHarvestResourceDrop::from_row)
        .collect();
    let Some(drops) = drops else {
        queue.refused = true;
        error!("[harvest-api] unsupported gather resource");
        return;
    };
    let call = Call::Gather(moly_protocol::api::UserMysekaiGatherRequest {
        mysekai_site_id: request.site_id as i32,
        gather_mysekai_site_harvest_resource_drops: drops,
    });
    let kind = call.kind();
    let seq = link.call(call);
    queue.awaiting = Some(ServerPending {
        seq: Some(seq),
        call: kind,
    });
    queue.gather_in_flight = Some(request);
    queue.gather_requests += 1;
}

#[cfg(test)]
mod pending_capacity_tests {
    use super::*;
    #[test]
    fn sent_gather_rows_still_reserve_capacity_until_reply() {
        let drop = UserDrop {
            resource_type: 41,
            resource_id: 1,
            position_x: 0,
            position_z: 0,
            hp: 0,
            seq: 1,
            status: 1,
            quantity: 3,
            group_id: 0,
        };
        let mut queue = HarvestLogQueue::default();
        queue.gathers.push_back(GatherRequest {
            site_id: 5,
            drops: vec![drop.clone()],
        });
        queue.gather_in_flight = Some(GatherRequest {
            site_id: 5,
            drops: vec![drop],
        });
        assert_eq!(queue.pending_gather_quantity(), 6);
        queue.gather_in_flight = None;
        assert_eq!(queue.pending_gather_quantity(), 3);
    }
}
