//! JP 7.0.0 HarvestPresenter's bulk action. The gesture owns no inventory:
//! only an accepted server reply applies maps, stamina, tools and rewards.

use std::collections::BTreeMap;

use bevy::prelude::*;
use moly_law::bulk_harvest::{self as law, Budget, Gates, Plan, Tool, UnavailableReason};
use moly_masters::bulk_harvest::BulkHarvestMasters;
use moly_protocol::{api, Call, Reply};

use super::{action::HarvestPlayerModel, catalog::HarvestUserData, queue::HarvestLogQueue};
use crate::player::{PlayerControlled, PlayerInput};
use crate::player_avatar::item_timeline::PlayerStepItem;

pub use super::bulk_dialog::WORDINGS;
pub const CHARGE_TIME: i32 = 222;
const CHARGE_SPEED: i32 = 220;
const REWARD_DELAY: i32 = 223;
const COMPLETE_SOUND_BUNDLE: i32 = 224;
const COMPLETE_SOUND_CUE: i32 = 225;
const COMPLETE_SOUND_DELAY: i32 = 226;
const TIMELINE_BUNDLE: &str = "mysekai/player/motion/bulk_harvest";
const TIMELINE_PREFAB: &str = "tl_bulk_harvest_main";
const FADE_SECONDS: f32 = 0.3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    None,
    Charging,
    Canceling,
    CancelingImmediately,
    Executing,
}

/// Source realtime hold: becoming full does not execute. Only a matching
/// pointer's release completes, and exit/cancel always cancels the hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    pub pointer: i32,
    pub started: f64,
    pub duration: f64,
}

impl Press {
    pub fn progress(self, now: f64) -> f32 {
        ((now - self.started) / self.duration).clamp(0.0, 1.0) as f32
    }
    pub fn release(self, pointer: i32, now: f64) -> Option<bool> {
        (self.pointer == pointer).then(|| now - self.started >= self.duration)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Input {
    Down(i32),
    Up(i32),
    Cancel(i32),
    Cannot,
}

enum Execution {
    Sync,
    WaitSignal(Plan),
    Waiting {
        seq: u32,
        plan: Plan,
    },
    Respawn {
        plan: Plan,
        reply: api::UserMysekaiSiteBulkHarvestResponse,
    },
    Complete {
        at: f64,
        plan: Plan,
        reply: api::UserMysekaiSiteBulkHarvestResponse,
        effect: Option<Entity>,
        sound_played: bool,
    },
}

#[derive(Resource, Default)]
pub struct BulkHarvest {
    pub phase: Phase,
    pub press: Option<Press>,
    pub visible: bool,
    pub available: UnavailableReason,
    pub input: Vec<Input>,
    pub balloon: Option<(UnavailableReason, f64)>,
    pub masters: Option<BulkHarvestMasters>,
    pub settings: BTreeMap<i32, Vec<i32>>,
    pub settings_error: Option<String>,
    site: Option<u32>,
    pass_observed: bool,
    settings_loaded: bool,
    returning: bool,
    before_stamina: Option<super::law::Stamina>,
    before_tools: Vec<super::server_mock::UserTool>,
    execution: Option<Execution>,
}

impl BulkHarvest {
    fn show_unavailable_balloon(&mut self, now: f64) {
        // OnBulkHarvestCannotAsync has no message for a treasure-box sync.
        // Leave any existing balloon alone instead of showing prefab text.
        if super::bulk_ui::reason_key(self.available).is_some() {
            self.balloon = Some((self.available, now));
        }
    }

    pub fn blocks_normal(&self) -> bool {
        self.phase != Phase::None
    }
    pub fn progress(&self, now: f64) -> f32 {
        self.press.map_or(0.0, |press| press.progress(now))
    }
    pub fn settings_for(&self, site: i32) -> Option<&[i32]> {
        self.settings.get(&site).map(Vec::as_slice)
    }
    /// Save on OK, merge only this source member, and publish after storage
    /// commits. An explicit empty vector must survive a reload.
    pub fn save_settings(&mut self, values: BTreeMap<i32, Vec<i32>>) -> Result<Vec<i32>, String> {
        let changed = values
            .iter()
            .filter_map(|(site, ids)| (self.settings.get(site) != Some(ids)).then_some(*site))
            .collect();
        crate::settings_store::save_sections(&[(
            "MysekaiOptionSettingData",
            serde_json::json!({"BulkHarvestCheckedTargetIds": values}),
        )])?;
        self.settings = values;
        self.settings_error = None;
        Ok(changed)
    }
}

fn read_settings(value: &serde_json::Value) -> Result<BTreeMap<i32, Vec<i32>>, String> {
    let Some(raw) = value.pointer("/MysekaiOptionSettingData/BulkHarvestCheckedTargetIds") else {
        return Ok(BTreeMap::new());
    };
    if raw.is_null() {
        return Ok(BTreeMap::new());
    }
    let mut out = BTreeMap::new();
    for (site, ids) in raw
        .as_object()
        .ok_or("BulkHarvestCheckedTargetIds is not an object")?
    {
        let site: i32 = site
            .parse()
            .map_err(|_| "bulk harvest site is not an integer")?;
        let ids = ids
            .as_array()
            .ok_or("bulk harvest checked IDs are not an array")?
            .iter()
            .map(|id| {
                id.as_i64()
                    .and_then(|id| i32::try_from(id).ok())
                    .filter(|id| *id > 0)
                    .ok_or_else(|| "bulk harvest target is not a positive integer".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        out.insert(site, ids);
    }
    Ok(out)
}

fn now(world: &World) -> f64 {
    world.resource::<Time<Real>>().elapsed_secs_f64()
}

fn tutorial(world: &World) -> bool {
    world
        .get_resource::<crate::server_panel::ServerPanel>()
        .is_some_and(|p| p.is_tutorial())
}

fn current_site(world: &World) -> Option<u32> {
    world
        .get_resource::<crate::site::SiteActive>()
        .filter(|s| s.category == "harvest")
        .map(|s| s.site_id)
}

/// Read current views as well as map rows: a not-yet-created fixture is not
/// eligible. The authoritative server validates the same selection again.
fn evaluate(world: &mut World, state: &BulkHarvest) -> Option<(UnavailableReason, Plan)> {
    let site = current_site(world)?;
    let masters = state.masters.as_ref()?;
    let player = world
        .query_filtered::<&Transform, With<PlayerControlled>>()
        .single(world)
        .ok()?
        .translation;
    let views = world
        .query::<&super::HarvestObject>()
        .iter(world)
        .filter(|o| o.site_id == site)
        .map(|o| ((o.fixture_id, o.position_x, o.position_z), (o.hp, o.status)))
        .collect::<BTreeMap<_, _>>();
    let user = world.get_resource::<HarvestUserData>()?;
    let map = user.maps.get(&site)?;
    let mut candidates = masters.candidates(map, -player.x, player.z);
    for row in &mut candidates {
        let view = views.get(&(row.key.fixture_id, row.key.position_x, row.key.position_z));
        row.view_exists = view.is_some();
        if let Some((hp, status)) = view {
            row.hp = *hp;
            row.spawned = *status == super::server_mock::FIXTURE_SPAWNED;
        }
    }
    let model = world.get_resource::<HarvestPlayerModel>()?;
    let catalog = world.get_resource::<super::catalog::HarvestCatalog>()?;
    let tools = model
        .tools
        .iter()
        .filter_map(|tool| {
            let def = catalog.tool(tool.tool_id)?;
            Some(Tool {
                id: i32::try_from(tool.tool_id).ok()?,
                quantity: tool.quantity,
                durability: tool.durability,
                max_durability: def.max_durability,
                attack_power: def.attack_power,
            })
        })
        .collect::<Vec<_>>();
    let budget = Budget {
        normal: model.stamina.normal,
        enhance: model.stamina.enhance,
        boost: model.stamina.boost,
    };
    let limits = world.get_resource::<super::possession::PossessionLimits>()?;
    let possession = world.resource::<super::possession::PossessionMock>();
    let possession_limit = limits
        .material
        .get(&possession.material_level)
        .is_none_or(|limit| possession.material_quantity >= *limit);
    let pass_valid = world
        .get_resource::<crate::userdata::ClientUserData>()
        .is_some_and(|user| user.has_mysekai_colorful_pass(now(world) as f32));
    let gates =
        Gates {
            pass_valid,
            possession_limit,
            treasure_box_not_synced: user.boxes.iter().any(|row| {
                row.site_id == site && row.status == super::server_mock::BOX_BEFORE_SPAWNED
            }) || world
                .resource::<super::airplane::PaperAirplanes>()
                .bulk_unsynced(site),
        };
    let checked = state.settings_for(site as i32);
    Some((
        law::judge_available(gates, &candidates, checked, &tools, budget),
        law::calculate(&candidates, checked, &tools, budget),
    ))
}

fn request(
    world: &World,
    site: u32,
    plan: &Plan,
) -> Result<api::UserMysekaiSiteBulkHarvestRequest, String> {
    let map = world
        .resource::<HarvestUserData>()
        .maps
        .get(&site)
        .ok_or("current harvest map is absent")?;
    let drops = map
        .drops
        .iter()
        .filter(|drop| {
            drop.status == super::server_mock::DROP_BEFORE
                && plan.targets.iter().any(|key| {
                    key.position_x == drop.position_x && key.position_z == drop.position_z
                })
        })
        .map(|drop| {
            api::MysekaiSiteHarvestResourceDrop::from_row(drop)
                .ok_or_else(|| "bulk harvest drop has an unsupported resource identity".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(api::UserMysekaiSiteBulkHarvestRequest {
        mysekai_site_id: site as i32,
        mysekai_site_bulk_harvest_target_ids: plan.target_ids.clone(),
        target_mysekai_site_harvest_fixtures: plan
            .targets
            .iter()
            .map(|key| api::BulkHarvestFixture {
                position_x: key.position_x,
                position_z: key.position_z,
                mysekai_site_harvest_fixture_id: key.fixture_id,
            })
            .collect(),
        gather_mysekai_site_harvest_resource_drops: drops,
        mysekai_stamina_useds: [
            ("normal", plan.stamina_after.normal),
            ("enhance", plan.stamina_after.enhance),
            ("boost", plan.stamina_after.boost),
        ]
        .into_iter()
        .map(|(kind, rest)| api::MysekaiStaminaUsed {
            mysekai_stamina_type: kind.to_owned(),
            rest,
        })
        .collect(),
    })
}

fn cleanup(world: &mut World, state: &mut BulkHarvest) {
    state.press = None;
    state.phase = Phase::CancelingImmediately;
    state.returning = true;
    state.execution = None;
    if let Some(mut step) = world.get_resource_mut::<PlayerStepItem>() {
        step.clear_step_item_object();
        step.bulk_camera_return();
    }
    world
        .resource_mut::<super::tool_model::ToolModelRequests>()
        .0
        .push(super::tool_model::ToolModelRequest::Hide);
}

fn cancel(world: &mut World, state: &mut BulkHarvest, immediate: bool) {
    if state.phase != Phase::Charging {
        return;
    }
    state.press = None;
    if immediate {
        state.phase = Phase::CancelingImmediately;
        world.resource_mut::<PlayerStepItem>().stop();
        world.resource_mut::<PlayerStepItem>().bulk_camera_inherit();
        cleanup(world, state);
    } else {
        state.phase = Phase::Canceling;
        let mut step = world.resource_mut::<PlayerStepItem>();
        step.jump_to_section("Cancel");
    }
}

fn fail(world: &mut World, state: &mut BulkHarvest, reason: String) {
    error!("[bulk-harvest] {reason}");
    cleanup(world, state);
    crate::screen_fade::fade_in(world, 0.0, FADE_SECONDS, "bulk-harvest-failed");
    let message = crate::common_dialog::wording(world, "BulkHarvest", "MSG_API_SERVER_ERROR");
    crate::common_dialog::open_detached(
        world,
        "BulkHarvest",
        message,
        crate::common_dialog::Buttons::One { ok: "WORD_OK" },
    );
}

pub fn tick(world: &mut World) {
    let mut state = world
        .remove_resource::<BulkHarvest>()
        .expect("bulk state installed");
    tick_inner(world, &mut state);
    world.insert_resource(state);
}

fn tick_inner(world: &mut World, state: &mut BulkHarvest) {
    let now = now(world);
    if !state.settings_loaded {
        state.settings_loaded = true;
        match crate::settings_store::read_document().and_then(|doc| read_settings(&doc)) {
            Ok(values) => state.settings = values,
            Err(reason) => {
                error!("[bulk-harvest] {reason}");
                state.settings_error = Some(reason);
            }
        }
    }
    if state.masters.is_none() {
        if let Some(tables) = world.get_resource::<moly_masters::load::MasterTables>() {
            if [
                "mysekaiSiteBulkHarvests",
                "mysekaiSiteBulkHarvestTargets",
                "mysekaiSiteBulkHarvestTargetGroups",
                "mysekaiSiteHarvestFixtures",
            ]
            .into_iter()
            .all(|name| tables.rows.contains_key(name))
            {
                match BulkHarvestMasters::parse(&tables.rows) {
                    Ok(masters) => state.masters = Some(masters),
                    Err(reason) => {
                        error!("[bulk-harvest] master join refused: {reason}");
                    }
                }
            }
        }
    }
    let site = current_site(world);
    if state.site != site {
        if state.phase != Phase::None {
            cleanup(world, state);
            state.returning = false;
            state.phase = Phase::None;
            world.remove_resource::<crate::field_modes::BulkHarvestActive>();
        }
        state.site = site;
        state.pass_observed = false;
        state.balloon = None;
    }
    if state.returning {
        state.input.clear();
        if world
            .get_resource::<PlayerStepItem>()
            .is_none_or(PlayerStepItem::bulk_camera_finished)
        {
            state.returning = false;
            state.phase = Phase::None;
            world.remove_resource::<crate::field_modes::BulkHarvestActive>();
            let model = world.get_resource::<HarvestPlayerModel>();
            let restore = model.and_then(|m| {
                m.selected.and_then(|id| {
                    world
                        .get_resource::<super::catalog::HarvestCatalog>()?
                        .tool(id)
                        .map(|t| (id, t.assetbundle.clone()))
                })
            });
            if let Some((tool_id, assetbundle)) = restore {
                world
                    .resource_mut::<super::tool_model::ToolModelRequests>()
                    .0
                    .push(super::tool_model::ToolModelRequest::Show {
                        tool_id,
                        assetbundle,
                    });
            }
        }
    }
    let pass = world
        .get_resource::<crate::userdata::ClientUserData>()
        .is_some_and(|user| user.has_mysekai_colorful_pass(now as f32));
    state.pass_observed |= pass;
    state.visible = site.is_some() && state.pass_observed && !tutorial(world);
    if let Some((reason, _)) = evaluate(world, state) {
        state.available = reason;
    } else {
        state.available = UnavailableReason::NoTarget;
    }

    let moving = world
        .query_filtered::<&PlayerInput, With<PlayerControlled>>()
        .iter(world)
        .any(|input| input.active);
    if state.phase == Phase::Charging
        && (!state.visible
            || moving
            || world.contains_resource::<crate::field_modes::SiteMoveActive>())
    {
        cancel(world, state, true);
    }
    for input in std::mem::take(&mut state.input) {
        match input {
            Input::Cannot => state.show_unavailable_balloon(now),
            Input::Down(pointer)
                if state.phase == Phase::None
                    && state.visible
                    && state.available == UnavailableReason::None =>
            {
                if world
                    .resource::<super::action::HarvestAction>()
                    .in_progress()
                    || !super::bulk_effect::ready(world)
                {
                    continue;
                }
                let Some(config) = world.get_resource::<crate::client_config::ClientConfigs>()
                else {
                    continue;
                };
                let duration = f64::from(config.float(CHARGE_TIME).max(0.01));
                let speed = f64::from(config.float(CHARGE_SPEED));
                state.phase = Phase::Charging;
                state.press = Some(Press {
                    pointer,
                    started: now,
                    duration,
                });
                world.write_message(crate::joystick::ForceResetJoystick {
                    reason: "BulkHarvest.OnLongPressStart",
                });
                world.insert_resource(crate::field_modes::BulkHarvestActive);
                world
                    .resource_mut::<super::tool_model::ToolModelRequests>()
                    .0
                    .push(super::tool_model::ToolModelRequest::Hide);
                let mut step = world.resource_mut::<PlayerStepItem>();
                step.update_step_item_object(TIMELINE_BUNDLE, TIMELINE_PREFAB);
                step.setup(None);
                step.set_speed(speed);
                step.play();
            }
            Input::Up(pointer) if state.phase == Phase::Charging => {
                match state.press.and_then(|press| press.release(pointer, now)) {
                    Some(true) => {
                        state.press = None;
                        state.phase = Phase::Executing;
                        state.execution = Some(Execution::Sync);
                        world.resource_mut::<HarvestLogQueue>().request_flush();
                    }
                    Some(false) => cancel(world, state, false),
                    None => {}
                }
            }
            Input::Cancel(pointer) if state.press.is_some_and(|press| press.pointer == pointer) => {
                cancel(world, state, false)
            }
            _ => {}
        }
    }
    if state.phase == Phase::Canceling {
        let playing = world
            .get_resource::<PlayerStepItem>()
            .is_some_and(|step| step.is_playing(world));
        if !playing {
            cleanup(world, state);
        }
    }
    if state.phase == Phase::Charging
        && world
            .get_resource::<PlayerStepItem>()
            .is_none_or(|step| !step.is_playing(world))
    {
        cancel(world, state, true);
    }
    let Some(execution) = state.execution.take() else {
        return;
    };
    let Some(site) = state.site else {
        cleanup(world, state);
        return;
    };
    match execution {
        Execution::Sync => {
            if world.resource::<HarvestLogQueue>().has_failed() {
                fail(world, state, "The previous harvest could not be synchronized. Re-enter this site to refresh its map.".into());
                return;
            }
            if !world.resource::<HarvestLogQueue>().is_idle() {
                state.execution = Some(Execution::Sync);
                return;
            }
            let Some((reason, plan)) = evaluate(world, state) else {
                fail(world, state, "Bulk harvest data is not ready".into());
                return;
            };
            if reason != UnavailableReason::None {
                state.balloon = Some((reason, now));
                cleanup(world, state);
                return;
            }
            let model = world.resource::<HarvestPlayerModel>();
            state.before_stamina = Some(model.stamina);
            state.before_tools = model.tools.clone();
            world
                .resource_mut::<PlayerStepItem>()
                .jump_to_section("Finish");
            state.execution = Some(Execution::WaitSignal(plan));
        }
        Execution::WaitSignal(plan) => {
            let step = world.resource::<PlayerStepItem>();
            if step.first_signal_fired_on(
                world.get_resource::<crate::fixture_activity_timeline::FixtureActivityTimelines>(),
            ) {
                crate::screen_fade::fade_out(world, 0.0, FADE_SECONDS, "bulk-harvest-execute");
                match request(world, site, &plan) {
                    Ok(request) => {
                        let seq = world
                            .resource_mut::<moly_link::ServerLink>()
                            .call(Call::BulkHarvest(request));
                        state.execution = Some(Execution::Waiting { seq, plan });
                    }
                    Err(reason) => fail(world, state, reason),
                }
            } else if !step.is_playing(world) {
                fail(
                    world,
                    state,
                    "Bulk harvest timeline ended before its execute signal".into(),
                );
            } else {
                state.execution = Some(Execution::WaitSignal(plan));
            }
        }
        Execution::Waiting { seq, plan } => {
            let answer = world
                .resource::<crate::userdata::AnsweredLog>()
                .get(seq)
                .cloned();
            let Some(answer) = answer else {
                state.execution = Some(Execution::Waiting { seq, plan });
                return;
            };
            match answer.outcome {
                Ok(Reply::BulkHarvest(reply)) => {
                    if current_site(world) != Some(site) {
                        cleanup(world, state);
                        return;
                    }
                    let stamina = world.resource::<HarvestUserData>().stamina;
                    let tools = super::server_mock::user_tools(
                        world
                            .resource::<crate::userdata::tools::ClientMysekaiTools>()
                            .rows(),
                    );
                    world.resource_mut::<HarvestUserData>().tools = tools.clone();
                    let mut model = world.resource_mut::<HarvestPlayerModel>();
                    model.stamina = stamina;
                    model.refresh_tools(&tools);
                    super::arrival::refresh_from_authoritative(world, site);
                    state.execution = Some(Execution::Respawn { plan, reply });
                }
                Err(refusal) => fail(world, state, refusal.reason),
                Ok(other) => fail(
                    world,
                    state,
                    format!("BulkHarvest received unexpected reply {other:?}"),
                ),
            }
        }
        Execution::Respawn { plan, reply } => {
            let arrival = world.resource::<super::arrival::HarvestArrival>();
            let ready = arrival.site_id() == Some(site)
                && !arrival.in_progress()
                && world.resource::<super::HarvestScenesReadyCount>().0
                    == world.resource::<super::HarvestSpawnedCount>().0;
            if !ready
                || !world
                    .resource::<crate::screen_fade::ScreenFader>()
                    .finished()
            {
                state.execution = Some(Execution::Respawn { plan, reply });
                return;
            }
            world.resource_mut::<PlayerStepItem>().bulk_camera_return();
            crate::screen_fade::fade_in(world, 0.0, FADE_SECONDS, "bulk-harvest-complete");
            let effect = super::bulk_effect::emit(world);
            state.execution = Some(Execution::Complete {
                at: now,
                plan,
                reply,
                effect,
                sound_played: false,
            });
        }
        Execution::Complete {
            at,
            plan,
            reply,
            effect,
            mut sound_played,
        } => {
            if current_site(world) != Some(site) {
                cleanup(world, state);
                return;
            }
            let config = world.resource::<crate::client_config::ClientConfigs>();
            let reward_delay = f64::from(config.float(REWARD_DELAY));
            let sound_delay = f64::from(config.float(COMPLETE_SOUND_DELAY));
            if !sound_played && now - at >= sound_delay {
                let bundle = config.string(COMPLETE_SOUND_BUNDLE).to_owned();
                let cue = config.string(COMPLETE_SOUND_CUE).to_owned();
                super::bulk_effect::play_sound(world, &bundle, &cue);
                sound_played = true;
            }
            if now - at >= reward_delay {
                world
                    .resource_mut::<super::bulk_dialog::BulkDialogs>()
                    .rewards
                    .push_back(super::bulk_dialog::RewardResult {
                        plan,
                        reply,
                        special_started: false,
                        site_id: site,
                        before_stamina: state.before_stamina.expect("execution snapshot"),
                        before_tools: std::mem::take(&mut state.before_tools),
                    });
                cleanup(world, state);
            } else {
                state.execution = Some(Execution::Complete {
                    at,
                    plan,
                    reply,
                    effect,
                    sound_played,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn treasure_box_sync_does_not_show_a_pass_expired_balloon() {
        let mut state = BulkHarvest {
            available: UnavailableReason::TreasureBoxNotSynced,
            ..Default::default()
        };
        state.show_unavailable_balloon(10.0);
        assert_eq!(state.balloon, None);
        state.available = UnavailableReason::PassExpired;
        state.show_unavailable_balloon(11.0);
        assert_eq!(state.balloon, Some((UnavailableReason::PassExpired, 11.0)));
        state.available = UnavailableReason::TreasureBoxNotSynced;
        state.show_unavailable_balloon(12.0);
        assert_eq!(state.balloon, Some((UnavailableReason::PassExpired, 11.0)));
    }

    #[test]
    fn full_charge_still_requires_same_pointer_release() {
        let press = Press {
            pointer: 7,
            started: 10.0,
            duration: 2.0,
        };
        assert_eq!(press.progress(99.0), 1.0);
        assert_eq!(press.release(8, 99.0), None);
        assert_eq!(press.release(7, 11.999), Some(false));
        assert_eq!(press.release(7, 12.0), Some(true));
    }
    #[test]
    fn absent_and_explicit_empty_preferences_remain_distinct() {
        assert!(read_settings(&serde_json::json!({}))
            .unwrap()
            .get(&5)
            .is_none());
        let settings = read_settings(&serde_json::json!({"MysekaiOptionSettingData":{"BulkHarvestCheckedTargetIds":{"5":[]}}})).unwrap();
        assert_eq!(settings.get(&5), Some(&Vec::new()));
        assert!(read_settings(&serde_json::json!({"MysekaiOptionSettingData":{"BulkHarvestCheckedTargetIds":{"5":["bad"]}}})).is_err());
    }
}
