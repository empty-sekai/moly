//! The cannon site move between the home, harvest and delivery sites: the
//! source's `SiteMoveActionExecutor` running `MoveSiteUseCannonActionState`
//! (pre-action, the ordered Core timeline, the end action), with the game in
//! `GameState.SiteMove` from admission until it returns to Normal.
//!
//! One site is loaded at a time. The source keeps every site at its master
//! `SitePosition` in one world; here the loaded site is the origin. The Core
//! therefore runs in the old site's frame, where the destination sits at
//! `O_new - O_old`, and the swap re-origins everything the move carries
//! (player, camera, its tween, the cannon, world-space particles) by that
//! offset on the frame the player's flight tween ends. The destination is
//! preloaded in the pre-action (the source awaits `SiteManager.AddSite`), so
//! the swap only instantiates it, and it stays hidden until the Core's
//! `SetRenderingEnabled(nextSite, true)` step.
//!
//! Named differences, each a consequence of the one-site structure:
//! - The old site is torn down at the swap, not in the end action, so it is
//!   not visible behind the player during the last part of the flight and
//!   the landing (the source removes a harvest site in the end action and
//!   never removes home).
//! - The environment follow (`environmentRoot.position = Step.position`
//!   during the flight, then its y restored at landing) has no product
//!   counterpart: the sky dome and the weather sky anchor already follow the
//!   player every frame. The two steps are logged, not applied.
//!
//! Timing conventions are in [`timeline`]: waits are the source's
//! millisecond-rounded delays that skip their creation frame; tweens and
//! animation clocks advance from the frame they start.

mod arrival;
pub(crate) mod camera;
mod cannon;
pub(crate) mod effects;
pub(crate) mod products;
mod speed_lines;
pub(crate) mod timeline;

use bevy::diagnostic::FrameCount;
use bevy::ecs::observer::On;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use bevy::scene::SceneInstanceReady;
use std::collections::HashMap;
use std::time::Duration;

use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::camera::{CameraTween, FieldCameraModel};
use crate::player::PlayerControlled;
use crate::player_avatar::{
    AvatarDriver, PlayerActionMotion, PlayerActionOwner, PlayerActionToken, PlayerVisualClips,
};
use crate::player_state::{PlayerActionState, PlayerAvatarStates};
use crate::site::{SitePreload, SiteSelection};
use crate::ui_layers::{LayerCommand, LayerId, UiLayerStack};
use timeline::{
    core_waits, final_wait, landing, move_time, out_sine, range_int, Delay, Landing, SourceClip,
    Step, TweenClock,
};

/// Queued by `site::read_switch` for a switch that `GetActionState` answers
/// with the cannon state.
#[derive(Resource)]
pub(crate) struct SiteMoveRequest {
    pub(crate) from: String,
    pub(crate) next: SiteSelection,
}

/// `GameState.SiteMove`: present from the move's admission until the
/// executor sets Normal. Its readers stop what the source stops in this
/// state (manual movement and input, the action-button scan, site commands).
#[derive(Resource)]
pub struct SiteMoveActive;

/// The Core's `ChangeEnvironment` has not run yet: the destination
/// environment is requested but its cross-fade does not start.
#[derive(Resource)]
pub(crate) struct EnvironmentHold;

/// `MysekaiBGMManager.PlayBGMAsync` for the destination has not run yet.
#[derive(Resource)]
pub(crate) struct BgmHold;

/// `SetRenderingEnabled(nextSite, false)` is in force.
#[derive(Resource)]
pub(crate) struct RevealHold;

/// The visibility a held destination entity had when the hold caught it.
#[derive(Component)]
pub(crate) struct RevealHeld(Visibility);

/// An entity the move spawned (the cannon and the pooled effects); never
/// part of the destination's rendering hold.
#[derive(Component)]
pub(crate) struct SiteMoveOwned;

#[derive(Component)]
pub(crate) struct PendingInstance;

#[derive(Component)]
pub(crate) struct InstanceReady;

pub(crate) enum AssetState {
    Pending,
    Ready,
    Failed(String),
}

#[derive(Clone, Copy, Debug)]
enum Stage {
    PreAction,
    WaitFirst(Delay),
    CreateCannon,
    WaitInCannon(Delay),
    WaitReady(Delay),
    WaitOutMinusReady(Delay),
    WaitFly(Delay),
    RevealPending,
    WaitLanding(Delay),
    WaitFinal(Delay),
    WaitHomeSummary(Delay),
    Done,
}

struct PlayerTween {
    from: Vec3,
    to: Vec3,
    clock: TweenClock,
}

/// A second SD segment to start when the first reaches its length (the
/// stumble stand-in plays its start then its end).
struct PendingSegment {
    at: f32,
    clip: String,
    speed: f32,
}

struct StepRecord {
    step: Step,
    source: Option<f64>,
    measured: f64,
    frame: u64,
    note: String,
}

/// The session: `ChangeSiteProcessAsync` for one cannon move.
#[derive(Resource)]
pub(crate) struct SiteMove {
    /// The player's current animation by its source name, as
    /// `GetCurrentAnimationName` reports it; the camera branches on it.
    pub(crate) source_clip: Option<SourceClip>,
    from: String,
    to: String,
    from_category: String,
    to_category: String,
    delta: Vec3,
    next: Option<SiteSelection>,
    preload: Option<SitePreload>,
    cannon_assets: cannon::CannonAssets,
    cannon: Option<cannon::Cannon>,
    arrival: arrival::Arrival,
    stage: Stage,
    admitted_frame: u64,
    since_admission: f64,
    core_start: Option<(f64, u64)>,
    t0: Option<f64>,
    token: Option<PlayerActionToken>,
    tween: Option<PlayerTween>,
    environment_follow: Option<TweenClock>,
    swapped: bool,
    landing: Option<Landing>,
    reveal_wait_logged: bool,
    clip_clock: f32,
    flying_started: bool,
    pending_segment: Option<PendingSegment>,
    records: Vec<StepRecord>,
    /// An exit state closed the intercept gate at the landing step.
    exit_gate_watch: bool,
    /// How the step being recorded became due (a delay's target, its
    /// accumulated elapsed time and the completing frame's delta, or a
    /// clock value).
    due: Option<String>,
    fly_due: Option<String>,
    frame_dt: f32,
}

pub(crate) fn install(app: &mut App) {
    speed_lines::install(app);
    app.add_systems(Startup, products::request);
    app.add_systems(Update, products::resolve.run_if(products::pending));
    app.add_observer(on_instance_ready)
        .add_systems(
            Update,
            (
                advance
                    .after(crate::site::read_switch)
                    .before(crate::site::plan)
                    .run_if(resource_exists::<SiteMove>.or(resource_exists::<SiteMoveRequest>)),
                advance_effects.run_if(resource_exists::<SiteMoveEffects>),
                crate::player_state::update_site_exit_states.before(advance),
            ),
        )
        .add_systems(
            PostUpdate,
            (
                hold_reveal
                    .run_if(resource_exists::<RevealHold>)
                    .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
                (camera::update, follow_flying)
                    .chain()
                    .after(crate::camera::follow_avatar)
                    .before(crate::sky::follow_player_view)
                    .before(crate::uber_particle::advance)
                    .before(crate::uber_particle::advance_fixture_particles)
                    .before(crate::character_material::write_frame_state)
                    .before(crate::balloon::place)
                    .before(crate::emoticon::advance),
            ),
        );
}

/// The effect pools: built before any move, as `EffectManager.Setup` runs
/// in the field scene's setup, and outliving each move (the landing effect
/// keeps playing after the move has returned to Normal). Present only when
/// the release root lists the site-move products (`products`).
#[derive(Resource)]
pub(crate) struct SiteMoveEffects(effects::Effects);

fn on_instance_ready(
    trigger: On<SceneInstanceReady>,
    pending: Query<(), With<PendingInstance>>,
    mut commands: Commands,
) {
    let entity = trigger.event().entity;
    if pending.get(entity).is_ok() {
        commands
            .entity(entity)
            .remove::<PendingInstance>()
            .insert(InstanceReady);
    }
}

/// Every named descendant of `root` by its name path (`a/b/c`), the key
/// shape of the particle documents' node paths.
pub(crate) fn node_paths(world: &mut World, root: Entity) -> HashMap<String, Vec<Entity>> {
    let mut state = SystemState::<(Query<&Name>, Query<&Children>)>::new(world);
    let (names, children) = state.get(world);
    let mut out = HashMap::new();
    if let Ok(kids) = children.get(root) {
        for kid in kids {
            crate::inactive_nodes::collect(*kid, &mut Vec::new(), &names, &children, &mut out);
        }
    }
    out
}

pub(crate) fn descendants(world: &World, root: Entity) -> Vec<Entity> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        out.push(entity);
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    out
}

fn advance(world: &mut World) {
    let frame = u64::from(world.resource::<FrameCount>().0);
    let dt = world.resource::<Time>().delta_secs();
    if let Some(request) = world.remove_resource::<SiteMoveRequest>() {
        if world.contains_resource::<SiteMove>() {
            warn!(
                "[site-move] a move is already in progress; request for {} dropped",
                request.next.site_type()
            );
        } else {
            admit(world, request, frame);
        }
    }
    if !world.contains_resource::<SiteMove>() {
        return;
    }
    let done = world.resource_scope(|world, mut session: Mut<SiteMove>| {
        session.frame(world, frame, dt);
        matches!(session.stage, Stage::Done)
    });
    if done {
        let session = world.remove_resource::<SiteMove>().expect("present above");
        session.finish(world);
    }
}

fn advance_effects(world: &mut World) {
    let dt = world.resource::<Time>().delta_secs();
    world.resource_scope(|world, mut effects: Mut<SiteMoveEffects>| effects.0.advance(world, dt));
}

/// `OnChangeSite`: `GameState.SiteMove`, then `ChangeSiteProcessAsync`.
fn admit(world: &mut World, request: SiteMoveRequest, frame: u64) {
    let SiteMoveRequest { from, next } = request;
    let to = next.site_type().to_owned();
    let Some(sites) = world.get_resource::<crate::site::Sites>() else {
        error!("[site-move] site table missing at admission; switching immediately");
        immediate(world, next);
        return;
    };
    let (Some(from_place), Some(to_place)) = (sites.placement(&from), sites.placement(&to)) else {
        error!("[site-move] {from} or {to} missing from the site table; switching immediately");
        immediate(world, next);
        return;
    };
    let server = world.resource::<AssetServer>().clone();
    let preload = match SitePreload::request(&server, sites, &next) {
        Ok(preload) => preload,
        Err(error) => {
            error!("[site-move] destination preload refused ({error}); switching immediately");
            immediate(world, next);
            return;
        }
    };
    let layout = (to_place.category == "housing_home").then(|| {
        let layouts = world.resource::<crate::fixture::layouts::SiteFixtureLayouts>();
        let region = world
            .get_resource::<crate::site::NavMeshSourceRegion>()
            .copied();
        // The loader completes a home layout with the player's house, so the
        // arrival reads the same completed layout.
        let homes = world.get_resource::<crate::entry::house::HomeFixtures>();
        next.restored_layout(sites, layouts, region, homes)
    });
    let arrival = arrival::Arrival::for_destination(&server, &to_place.category, layout);
    let delta = to_place.product_origin() - from_place.product_origin();
    world.insert_resource(SiteMoveActive);
    world.insert_resource(EnvironmentHold);
    world.insert_resource(BgmHold);
    // SiteMoveGameState.OnEnter: the gesture layer off, the empty site-move
    // screen replaces the field screen (ChangeUIScreen 653), taps disabled.
    // The site map's screen is removed first (ChangeSiteProcessAsync).
    if world.resource::<UiLayerStack>().current() == LayerId::MysekaiSiteMap {
        world.write_message(LayerCommand::Pop);
    }
    world.write_message(LayerCommand::Change(LayerId::MysekaiSiteMove));
    info!(
        "[site-move] admitted {from} -> {to} (cannon): offset {delta:.1}, GameState SiteMove; pre-action: PlayerFootEffect stop, CleanupCurrentSite, AddSite preload"
    );
    crate::footstep::stop_key(world, "cannon pre-action");
    world.insert_resource(SiteMove {
        source_clip: None,
        from,
        to,
        from_category: from_place.category,
        to_category: to_place.category,
        delta,
        next: Some(next),
        preload: Some(preload),
        cannon_assets: cannon::CannonAssets::request(&server),
        cannon: None,
        arrival,
        stage: Stage::PreAction,
        admitted_frame: frame,
        since_admission: 0.0,
        core_start: None,
        t0: None,
        token: None,
        tween: None,
        environment_follow: None,
        swapped: false,
        landing: None,
        reveal_wait_logged: false,
        clip_clock: 0.0,
        flying_started: false,
        pending_segment: None,
        records: Vec::new(),
        exit_gate_watch: false,
        due: None,
        fly_due: None,
        frame_dt: 0.0,
    });
    speed_lines::arm(world);
}

/// Tour instrument, not a source value: `MOLY_SITE_MOVE_LANDING_DRAW=<n>`
/// replaces the landing draw with `n` (the failed landing is 3 of the 100
/// draws, so a run otherwise rarely reaches it). A value outside the draw's
/// range is refused.
const LANDING_DRAW_ENV: &str = "MOLY_SITE_MOVE_LANDING_DRAW";

fn forced_landing_draw() -> Option<i32> {
    let raw = std::env::var(LANDING_DRAW_ENV).ok()?;
    let (min, max) = timeline::LANDING_RANGE;
    let roll = raw
        .trim()
        .parse::<i32>()
        .ok()
        .filter(|roll| (min..max).contains(roll))
        .unwrap_or_else(|| {
            panic!("{LANDING_DRAW_ENV}={raw}: not a draw of Random.Range({min}, {max})")
        });
    warn!("[site-move] landing draw forced to {roll} by the {LANDING_DRAW_ENV} instrument");
    Some(roll)
}

fn immediate(world: &mut World, next: SiteSelection) {
    let roots = site_roots(world);
    let mut commands = world.commands();
    crate::site::queue_transition(&mut commands, roots, next);
    world.flush();
}

fn site_roots(world: &mut World) -> Vec<Entity> {
    let mut roots = world.query_filtered::<Entity, With<crate::site::SiteRoot>>();
    roots.iter(world).collect()
}

fn player_entity(world: &mut World) -> Option<Entity> {
    let mut players = world.query_filtered::<Entity, With<PlayerControlled>>();
    players.iter(world).next()
}

impl SiteMove {
    fn frame(&mut self, world: &mut World, frame: u64, dt: f32) {
        if frame != self.admitted_frame {
            self.since_admission += dt as f64;
        }
        self.frame_dt = dt;
        self.run_steps(world, frame, dt);
        self.advance_clocks(world, dt);
    }

    fn record(&mut self, step: Step, frame: u64) {
        let (core, core_frame) = self.core_start.unwrap_or((self.since_admission, frame));
        let measured = self.since_admission - core;
        let source = match step {
            Step::Setup => Some(0.0),
            Step::CreateCannon => Some(core_waits().first as f64),
            _ => timeline::source_offset_from_t0(step, self.landing)
                .zip(self.t0)
                .map(|(offset, t0)| t0 - core + offset),
        };
        let frames = frame.saturating_sub(core_frame);
        let note = self.due.take().unwrap_or_default();
        match source {
            Some(source) => info!(
                "[site-move] step {:<28} source {:>7.4}s measured {:>7.4}s (delta {:+.4}s) frame {} {}",
                step.name(), source, measured, measured - source, frames, note
            ),
            None => info!(
                "[site-move] step {:<28} measured {:>7.4}s frame {} {}",
                step.name(), measured, frames, note
            ),
        }
        self.records.push(StepRecord {
            step,
            source,
            measured,
            frame: frames,
            note,
        });
    }

    /// A delay completed this frame: note how (the source completes it on
    /// the first frame whose accumulated delta reaches the target, so the
    /// lateness is below that frame's delta).
    fn due_by(&mut self, delay: Delay) {
        self.due = Some(format!(
            "[wait {:.3} elapsed {:.4} late {:+.4} < frame-dt {:.4}]",
            delay.target,
            delay.elapsed,
            delay.elapsed - delay.target,
            self.frame_dt
        ));
    }

    fn run_steps(&mut self, world: &mut World, frame: u64, dt: f32) {
        let waits = core_waits();
        loop {
            match &mut self.stage {
                Stage::PreAction => {
                    if !self.pre_action_ready(world) {
                        return;
                    }
                    self.setup(world, frame);
                    self.stage = Stage::WaitFirst(Delay::new(waits.first, frame));
                }
                Stage::WaitFirst(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let delay = *delay;
                    self.due_by(delay);
                    self.create_cannon(world, frame);
                    self.stage = Stage::CreateCannon;
                }
                Stage::CreateCannon => {
                    // `await CreateCannon`: the load wait takes at least one
                    // frame; the instance exists on a later frame.
                    let ready = self
                        .cannon
                        .as_ref()
                        .is_none_or(|cannon| cannon.instance_ready(world));
                    if !ready {
                        return;
                    }
                    self.cannon_start(world, frame);
                    self.stage = Stage::WaitInCannon(Delay::new(waits.in_cannon, frame));
                }
                Stage::WaitInCannon(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let delay = *delay;
                    self.due_by(delay);
                    self.change_animation(world, SourceClip::CannonS2);
                    self.record(Step::Clip2, frame);
                    self.stage = Stage::WaitReady(Delay::new(waits.ready, frame));
                }
                Stage::WaitReady(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let delay = *delay;
                    self.due_by(delay);
                    self.fire(world, frame);
                    self.stage = Stage::WaitOutMinusReady(Delay::new(waits.out_minus_ready, frame));
                }
                Stage::WaitOutMinusReady(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let delay = *delay;
                    self.due_by(delay);
                    self.change_environment(world, frame);
                    self.stage = Stage::WaitFly(Delay::new(waits.fly, frame));
                }
                Stage::WaitFly(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let delay = *delay;
                    self.due_by(delay);
                    self.fly_due = self.due.take();
                    self.stage = Stage::RevealPending;
                }
                Stage::RevealPending => {
                    if !self.swapped && self.tween.is_none() {
                        // No flight ran (no player to move): swap here.
                        self.swap(world);
                    }
                    let ready = self.swapped
                        && world.contains_resource::<crate::site_material::SiteMaterialsSwapped>();
                    if !ready {
                        if !self.reveal_wait_logged {
                            info!("[site-move] reveal step reached before the destination's materials are installed: holding the timeline here");
                            self.reveal_wait_logged = true;
                        }
                        return;
                    }
                    self.due = self.fly_due.take();
                    if self.reveal_wait_logged {
                        let held = self.due.take().unwrap_or_default();
                        self.due = Some(format!("{held} [held for the destination]"));
                    }
                    self.reveal(world, frame);
                    self.stage = Stage::WaitLanding(Delay::new(waits.landing, frame));
                }
                Stage::WaitLanding(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let delay = *delay;
                    self.due_by(delay);
                    let length = self.landing_effect(world, frame);
                    self.stage = Stage::WaitFinal(Delay::new(final_wait(length), frame));
                }
                Stage::WaitFinal(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let delay = *delay;
                    self.due_by(delay);
                    self.core_end(world, frame);
                    if self.end_action(world, frame) {
                        self.stage = Stage::WaitHomeSummary(Delay::new(
                            timeline::delay_seconds(timeline::HOME_SUMMARY_DELAY),
                            frame,
                        ));
                    } else {
                        self.normal(world, frame);
                        self.stage = Stage::Done;
                    }
                }
                Stage::WaitHomeSummary(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let delay = *delay;
                    self.due_by(delay);
                    info!("[site-move] HarvestUtility.ShowHarvestPointSummary: the harvest result popup is not ported (named gap)");
                    self.normal(world, frame);
                    self.stage = Stage::Done;
                }
                Stage::Done => return,
            }
        }
    }

    fn pre_action_ready(&mut self, world: &mut World) -> bool {
        let server = world.resource::<AssetServer>().clone();
        let site = match self.preload.as_ref().map(|preload| preload.ready(&server)) {
            Some(Ok(ready)) => ready,
            Some(Err(error)) => {
                error!("[site-move] {error}");
                false
            }
            None => true,
        };
        let listed = world
            .get_resource::<products::SiteMoveProducts>()
            .map_or(Some(false), products::SiteMoveProducts::present);
        self.cannon_assets.decide(&server, listed);
        let cannon = match self.cannon_assets.state(&server) {
            AssetState::Ready => true,
            AssetState::Pending => false,
            AssetState::Failed(error) => {
                // No cannon prop; the move still runs.
                error!("[site-move] {error}: the move runs without the cannon prop");
                true
            }
        };
        let effects = world
            .get_resource::<SiteMoveEffects>()
            .is_none_or(|effects| effects.0.settled(&server));
        let arrival = self.arrival.poll(world);
        site && cannon && effects && arrival
    }

    /// The Core's synchronous head, before its first wait.
    fn setup(&mut self, world: &mut World, frame: u64) {
        self.core_start = Some((self.since_admission, frame));
        info!(
            "[site-move] pre-action complete after {:.4}s ({} frames): destination, cannon and effects loaded",
            self.since_admission,
            frame.saturating_sub(self.admitted_frame)
        );
        self.record(Step::Setup, frame);
        // GetEnterSiteCharacterState: entering a harvest or the delivery
        // site from another category.
        let enter = if self.from_category != "harvest" && self.to_category == "harvest" {
            Some(PlayerActionState::EnterHarvestSite)
        } else if self.from_category != "delivery" && self.to_category == "delivery" {
            Some(PlayerActionState::EnterDeliverySite)
        } else {
            None
        };
        if let Some(state) = enter {
            world
                .resource_mut::<PlayerAvatarStates>()
                .change_status(state);
        }
        camera::enter_pre_site_move(world, &self.from);
        // SetNameTextVisible(false) and HideMysekaiMultiplayUI: the product
        // has no player name plate or multiplay HUD (no-ops).
        if let Some(player) = player_entity(world) {
            if let Some(mut transform) = world.get_mut::<Transform>(player) {
                // player.LookAt(nextSite.SitePosition): a full 3D LookAt.
                let direction = self.delta - transform.translation;
                if direction.length_squared() > 0.0 {
                    let position = transform.translation;
                    *transform = transform.with_rotation(
                        Transform::from_translation(position)
                            .looking_to(-direction, Vec3::Y)
                            .rotation,
                    );
                }
            }
        }
        push_se(world, "se_move_site_set");
        world
            .resource_mut::<PlayerAvatarStates>()
            .change_status(PlayerActionState::Idle);
        // SetMoveEnabled(false), with the Idle state's own motion.
        self.play_idle(world);
        camera::enter_site_move(world);
        // SetRenderingEnabled(nextSite, false): the destination does not
        // exist here until the swap, which starts the hold.
    }

    fn create_cannon(&mut self, world: &mut World, frame: u64) {
        self.record(Step::CreateCannon, frame);
        let pose = player_entity(world)
            .and_then(|player| world.get::<GlobalTransform>(player).copied())
            .map(|global| global.compute_transform().with_scale(Vec3::ONE));
        let Some(pose) = pose else {
            error!("[site-move] no player view transform for the cannon");
            return;
        };
        self.cannon = cannon::spawn(world, &self.cannon_assets, pose);
    }

    fn cannon_start(&mut self, world: &mut World, frame: u64) {
        self.t0 = Some(self.since_admission);
        if let Some(cannon) = self.cannon.as_mut() {
            cannon.start(world, &mut self.cannon_assets);
        }
        self.change_animation(world, SourceClip::CannonS);
        self.record(Step::CannonStart, frame);
    }

    fn fire(&mut self, world: &mut World, frame: u64) {
        self.record(Step::Fire, frame);
        let arrive = self.arrival.point() + self.delta;
        if let Some(player) = player_entity(world) {
            if let Some(transform) = world.get::<Transform>(player) {
                self.tween = Some(PlayerTween {
                    from: transform.translation,
                    to: arrive,
                    clock: TweenClock::new(move_time()),
                });
            }
        }
        // envDefaultPositionY = environmentRoot.position.y; DOVirtual.Float
        // over the same 0.4 s drags the environment root along with Step.
        self.environment_follow = Some(TweenClock::new(move_time()));
        push_se(world, "se_move_site_firing");
        speed_lines::set_active(world, true);
        info!(
            "[site-move] flight {:.2}s OutSine to {arrive:.2} (old-site frame)",
            move_time()
        );
    }

    fn change_environment(&mut self, world: &mut World, frame: u64) {
        self.record(Step::ChangeEnvironment, frame);
        info!(
            "[site-move] ChangeEnvironment: cross-fade {}s released; PlayBGMAsync for {} released",
            timeline::ENVIRONMENT_FADE,
            self.to
        );
        world.remove_resource::<EnvironmentHold>();
        world.remove_resource::<BgmHold>();
        self.change_animation(world, SourceClip::CannonL);
    }

    fn reveal(&mut self, world: &mut World, frame: u64) {
        // The loader holds the destination's handles by now.
        self.preload = None;
        if let Some(cannon) = self.cannon.as_mut() {
            cannon.end();
        }
        speed_lines::set_active(world, false);
        release_reveal(world);
        // FixtureManager.HideAllTransparentBlock: the product has no
        // transparent blocks (no-op).
        let roll = forced_landing_draw().unwrap_or_else(|| {
            let mut word = [0_u8; 4];
            getrandom::getrandom(&mut word).expect("landing draw needs OS entropy");
            range_int(
                u32::from_le_bytes(word),
                timeline::LANDING_RANGE.0,
                timeline::LANDING_RANGE.1,
            )
        });
        let outcome = landing(roll);
        self.landing = Some(outcome);
        self.record(Step::Reveal, frame);
        info!(
            "[site-move] landing draw {roll}: {} ({})",
            if outcome.failure {
                "failure"
            } else {
                "success"
            },
            outcome.clip.name()
        );
        self.change_animation(world, outcome.clip);
        // PublishFlyingEffectEnd sits at 0 of both landing clips.
        if outcome.clip.flying_end_event() && world.contains_resource::<SiteMoveEffects>() {
            let stopped = world.resource_scope(|world, mut effects: Mut<SiteMoveEffects>| {
                effects.0.stop_flying(world)
            });
            if stopped {
                info!("[site-move] EffectManager.Stop(Flying) at the landing clip's first frame");
            }
        }
    }

    /// Returns the landing clip's source length for the last wait.
    fn landing_effect(&mut self, world: &mut World, frame: u64) -> f32 {
        self.record(Step::LandingEffect, frame);
        info!("[site-move] environment root y restored: no product environment root (logged only)");
        let outcome = self.landing.expect("drawn at the reveal");
        let position = player_entity(world)
            .and_then(|player| world.get::<GlobalTransform>(player).copied())
            .map(|global| global.translation())
            .unwrap_or(Vec3::ZERO);
        let (kind, cue) = if outcome.failure {
            (
                effects::EffectType::SiteMoveFailedPlayer,
                "se_move_site_failed",
            )
        } else {
            (
                effects::EffectType::SiteMoveEndPlayer,
                "se_move_site_landing",
            )
        };
        emit_effect(world, kind, Transform::from_translation(position));
        push_se(world, cue);
        // GetExitSiteCharacterState: "current" is still the site left.
        let exit = if self.from_category == "harvest" && self.to_category != "harvest" {
            Some(PlayerActionState::ExitHarvestSite)
        } else if self.from_category == "delivery" && self.to_category != "delivery" {
            Some(PlayerActionState::ExitDeliverySite)
        } else {
            None
        };
        if let Some(state) = exit {
            world
                .resource_mut::<PlayerAvatarStates>()
                .change_status(state);
            self.exit_gate_watch = !world.resource::<PlayerAvatarStates>().can_intercept;
        }
        outcome.clip.length()
    }

    fn core_end(&mut self, world: &mut World, frame: u64) {
        self.record(Step::CoreEnd, frame);
        // SetMoveEnabled(true): movement and locomotion come back.
        self.release_player(world);
        self.source_clip = None;
        // SetNameTextVisible(true): no-op. OnMoveFinish = ChangeStatus(Idle),
        // which an exit state's closed intercept gate drops.
        world
            .resource_mut::<PlayerAvatarStates>()
            .change_status(PlayerActionState::Idle);
    }

    /// `OnSiteMoveEndAction`. Returns true when the home summary wait follows.
    fn end_action(&mut self, world: &mut World, frame: u64) -> bool {
        self.record(Step::EndAction, frame);
        // Cleanup, OnExitSite, RemoveSite and ChangeCurrentSiteOnlyData ran
        // at the swap; SetupLockAtCameraBounds ran when the destination
        // settled. next.OnEnterSite: a harvest or the delivery site changes
        // the UI screen to its field screen here.
        if matches!(self.to_category.as_str(), "harvest" | "delivery") {
            world.write_message(LayerCommand::Change(LayerId::HomeField));
        }
        self.to_category == "housing_home"
    }

    /// GameState Normal, then the destination's `OnFinishEnterAsync`.
    fn normal(&mut self, world: &mut World, frame: u64) {
        self.record(Step::Normal, frame);
        world.remove_resource::<SiteMoveActive>();
        let prev = world.resource::<crate::camera::PrevSiteType>().0.clone();
        camera::enter_normal(world, &self.to, &self.to_category, &prev);
        if self.to_category == "housing_home" {
            // SetupScreenLayerMysekaiHome: ChangeUIScreen(MysekaiHome).
            world.write_message(LayerCommand::Change(LayerId::HomeField));
        }
    }

    fn finish(self, world: &mut World) {
        world.remove_resource::<SiteMoveActive>();
        world.remove_resource::<EnvironmentHold>();
        world.remove_resource::<BgmHold>();
        release_reveal(world);
        speed_lines::disarm(world);
        if let Some(cannon) = &self.cannon {
            cannon.despawn(world);
        }
        let rows: Vec<String> = self
            .records
            .iter()
            .map(|r| match r.source {
                Some(source) => format!(
                    "{}|{:.4}|{:.4}|{:+.4}|{}|{}",
                    r.step.name(),
                    source,
                    r.measured,
                    r.measured - source,
                    r.frame,
                    r.note
                ),
                None => format!(
                    "{}|-|{:.4}|-|{}|{}",
                    r.step.name(),
                    r.measured,
                    r.frame,
                    r.note
                ),
            })
            .collect();
        info!(
            "[site-move] move {} -> {} done; step table (step|source|measured|delta|frame|due): {}",
            self.from,
            self.to,
            rows.join(" ; ")
        );
    }

    fn advance_clocks(&mut self, world: &mut World, dt: f32) {
        // Player DOMove, then the swap on its final frame.
        if let Some(tween) = &mut self.tween {
            let e = tween.clock.advance(dt);
            let position = tween.from.lerp(tween.to, out_sine(e));
            let done = tween.clock.done();
            if let Some(player) = player_entity(world) {
                if let Some(mut transform) = world.get_mut::<Transform>(player) {
                    transform.translation = position;
                }
            }
            if done {
                self.due = Some(format!(
                    "[DOMove clock {:.4} >= {:.4}]",
                    tween.clock.elapsed, tween.clock.duration
                ));
                self.tween = None;
                self.swap(world);
            }
        }
        if let Some(follow) = &mut self.environment_follow {
            follow.advance(dt);
            if follow.done() {
                self.due = Some(format!(
                    "[DOVirtual clock {:.4} >= {:.4}]",
                    follow.elapsed, follow.duration
                ));
                self.environment_follow = None;
                let frame = u64::from(world.resource::<FrameCount>().0);
                self.record(Step::EnvironmentFollowEnd, frame);
            }
        }
        let frame = u64::from(world.resource::<FrameCount>().0);
        let (mut play_effect, mut destroyed) = (None, false);
        if let Some(cannon) = self.cannon.as_mut() {
            if cannon.started() {
                if cannon.advance(world, dt) {
                    play_effect = Some(cannon.clock());
                }
                if cannon.destroyed() {
                    cannon.despawn(world);
                    destroyed = true;
                }
            }
        }
        if let Some(clock) = play_effect {
            info!(
                "[site-move] cannon PlayEffect at clip {clock:.4}s: _fireEffect.Play() on its already playing group (no restart)"
            );
            self.due = Some(format!(
                "[cannon clip clock {clock:.4} >= {:.4}]",
                timeline::CANNON_PLAY_EFFECT_AT
            ));
            self.record(Step::CannonPlayEffect, frame);
        }
        if destroyed {
            self.cannon = None;
            self.due = Some(format!(
                "[DOScale clock >= {:.3}]",
                timeline::CANNON_SHRINK_TIME
            ));
            self.record(Step::CannonDestroyed, frame);
        }
        if self.exit_gate_watch && world.resource::<PlayerAvatarStates>().can_intercept {
            self.exit_gate_watch = false;
            self.due = Some(format!(
                "[ElapsedTime {:.4} >= 1.0]",
                world.resource::<PlayerAvatarStates>().state_elapsed
            ));
            self.record(Step::ExitStateGateOpen, frame);
        }
        // Player clip clock: animation events and the second SD segment.
        self.clip_clock += dt;
        if let Some(event) = self.source_clip.and_then(SourceClip::flying_play_event) {
            if !self.flying_started && self.clip_clock >= event {
                self.flying_started = true;
                let pose = {
                    let mut cameras = world.query_filtered::<&GlobalTransform, With<Camera3d>>();
                    cameras
                        .single(world)
                        .map(|global| global.compute_transform())
                        .ok()
                };
                if let Some(pose) = pose {
                    emit_effect(world, effects::EffectType::Flying, pose);
                }
                self.due = Some(format!(
                    "[_s2 clip clock {:.4} >= {event:.4}]",
                    self.clip_clock
                ));
                self.record(Step::FlyingStart, frame);
            }
        }
        if let Some(segment) = &self.pending_segment {
            if self.clip_clock >= segment.at {
                let segment = self.pending_segment.take().expect("checked");
                self.play_sd(world, &segment.clip, false, segment.speed, Duration::ZERO);
            }
        }
    }

    /// The end of the player's flight: the destination replaces the old
    /// site and everything the move carries is re-origined.
    fn swap(&mut self, world: &mut World) {
        let frame = u64::from(world.resource::<FrameCount>().0);
        let Some(next) = self.next.take() else {
            return;
        };
        let roots = site_roots(world);
        {
            let mut commands = world.commands();
            crate::site::queue_transition(&mut commands, roots, next);
        }
        world.flush();
        world.insert_resource(RevealHold);
        let delta = self.delta;
        if let Some(player) = player_entity(world) {
            if let Some(mut transform) = world.get_mut::<Transform>(player) {
                transform.translation -= delta;
            }
        }
        if let Some(mut model) = world.get_resource_mut::<FieldCameraModel>() {
            model.look_at -= delta;
        }
        if let Some(mut tween) = world.get_resource_mut::<CameraTween>() {
            tween.look_at.0 -= delta;
            tween.look_at.1 -= delta;
        }
        {
            let mut cameras =
                world.query_filtered::<(&mut Transform, &mut GlobalTransform), With<Camera3d>>();
            if let Ok((mut transform, mut global)) = cameras.single_mut(world) {
                transform.translation -= delta;
                *global = GlobalTransform::from(*transform);
            }
        }
        let mut carried = Vec::new();
        if let Some(cannon) = &self.cannon {
            if let Some(mut transform) = world.get_mut::<Transform>(cannon.root) {
                transform.translation -= delta;
            }
            carried.push(cannon.root);
        }
        if let Some(root) = world
            .get_resource::<SiteMoveEffects>()
            .and_then(|effects| effects.0.flying_root())
        {
            carried.push(root);
        }
        for root in carried {
            shift_world_particles(world, root, delta);
        }
        self.swapped = true;
        self.record(Step::PlayerArrive, frame);
        info!(
            "[site-move] swap: {} torn down, {} requested from its preload; re-origin by {:.1}",
            self.from, self.to, -delta
        );
    }

    /// The Idle state's motion when the move takes the player.
    fn play_idle(&mut self, world: &mut World) {
        let Some(clips) = visual_clips(world) else {
            error!("[site-move] player has no SD clip set");
            return;
        };
        self.play_sd(
            world,
            &format!("{}_L", clips.idle),
            true,
            1.0,
            Duration::from_secs_f32(0.25),
        );
    }

    /// `ChangeAnimation(name, fade, 1)`, through the SD stand-in table.
    fn change_animation(&mut self, world: &mut World, clip: SourceClip) {
        self.source_clip = Some(clip);
        self.clip_clock = 0.0;
        self.pending_segment = None;
        let Some(clips) = visual_clips(world) else {
            error!("[site-move] player has no SD clip set");
            return;
        };
        let fade = Duration::from_secs_f32(clip.fade());
        match sd_stand_in(clip, &clips) {
            Ok(SdStandIn::Single {
                clip: name,
                looping,
            }) => self.play_sd(world, &name, looping, 1.0, fade),
            Ok(SdStandIn::Pair { first, second }) => {
                // The source clip lasts `clip.length()`; the stand-in plays
                // its start at 1x, then its end sped up to finish with it.
                let lengths = self.sd_lengths(world, &[&first, &second]);
                match lengths {
                    Some([first_len, second_len]) if clip.length() > first_len => {
                        let speed = second_len / (clip.length() - first_len);
                        self.play_sd(world, &first, false, 1.0, fade);
                        self.pending_segment = Some(PendingSegment {
                            at: first_len,
                            clip: second,
                            speed,
                        });
                        info!(
                            "[site-move] {} stand-in: {first} {first_len:.3}s then {} at {speed:.3}x (ends at the source length {:.4}s)",
                            clip.name(), self.pending_segment.as_ref().unwrap().clip, clip.length()
                        );
                    }
                    _ => {
                        error!(
                            "[site-move] {} stand-in clips unavailable or too long",
                            clip.name()
                        );
                    }
                }
            }
            Err(error) => error!("[site-move] {error}"),
        }
    }

    fn sd_lengths(&self, world: &mut World, names: &[&String; 2]) -> Option<[f32; 2]> {
        let mut drivers = world.query_filtered::<&AvatarDriver, With<PlayerControlled>>();
        let driver = drivers.iter(world).next()?;
        let clips = world.resource::<Assets<AnimationClip>>();
        Some([
            driver.sd_clip_duration(names[0], clips)?,
            driver.sd_clip_duration(names[1], clips)?,
        ])
    }

    fn play_sd(
        &mut self,
        world: &mut World,
        clip: &str,
        looping: bool,
        speed: f32,
        blend: Duration,
    ) {
        let mut state = SystemState::<(
            ResMut<Assets<AnimationGraph>>,
            Query<&mut AvatarDriver, With<PlayerControlled>>,
            Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
        )>::new(world);
        let (mut graphs, mut drivers, mut animators) = state.get_mut(world);
        let Some(mut driver) = drivers.iter_mut().next() else {
            error!("[site-move] no SD player driver for {clip}");
            return;
        };
        let Ok((mut player, mut transitions)) = animators.get_mut(driver.player) else {
            error!("[site-move] SD player animator missing for {clip}");
            return;
        };
        let motion = PlayerActionMotion {
            clip,
            looping,
            speed,
            blend,
            blocks_manual_movement: true,
        };
        let result = match self.token {
            Some(token) => driver
                .play_owned_action(token, motion, &mut graphs, &mut player, &mut transitions)
                .map(|()| token),
            None => driver.start_action(
                PlayerActionOwner::Cannon,
                motion,
                &mut graphs,
                &mut player,
                &mut transitions,
            ),
        };
        match result {
            Ok(token) => self.token = Some(token),
            Err(error) => error!("[site-move] SD motion {clip} refused: {error:?}"),
        }
    }

    fn release_player(&mut self, world: &mut World) {
        let Some(token) = self.token.take() else {
            return;
        };
        let mut state = SystemState::<(
            Query<&mut AvatarDriver, With<PlayerControlled>>,
            Query<&mut AnimationPlayer>,
        )>::new(world);
        let (mut drivers, mut animators) = state.get_mut(world);
        let Some(mut driver) = drivers.iter_mut().next() else {
            return;
        };
        let animator = driver.player;
        if let Ok(mut player) = animators.get_mut(animator) {
            driver.release_action(token, &mut player);
        }
    }
}

fn visual_clips(world: &mut World) -> Option<PlayerVisualClips> {
    let mut players = world.query_filtered::<&PlayerVisualClips, With<PlayerControlled>>();
    players.iter(world).next().cloned()
}

enum SdStandIn {
    Single { clip: String, looping: bool },
    Pair { first: String, second: String },
}

/// The one table of simple SD adaptations (owner decision: the visible
/// player is the SD body, which has no cannon clips). Waits keep the source
/// clip lengths; only the playable motion is substituted.
///
/// | source clip                 | SD stand-in                               |
/// |-----------------------------|-------------------------------------------|
/// | `mov_u000_site_cannon01_s`  | the player's idle loop                    |
/// | `mov_u000_site_cannon01_s2` | the player's run start (`_S`)             |
/// | `mov_u000_site_cannon01_l`  | the player's run loop (`_L`)              |
/// | `mov_u000_site_cannon01_e`  | the player's run end (`_E`)               |
/// | `mov_u000_site_cannon02_e`  | `mov_c{w,m}_normal_stumble001` start, end |
///
/// The stumble family (w or m) is the one of the player's idle and walk
/// clips; the stumble loop segment is skipped because start plus end already
/// exceed the source clip's 2.8 s.
fn sd_stand_in(clip: SourceClip, clips: &PlayerVisualClips) -> Result<SdStandIn, String> {
    let family = |name: &str| name.get(4..6).map(str::to_owned);
    let (Some(idle_family), Some(walk_family)) = (family(&clips.idle), family(&clips.walk)) else {
        return Err(format!(
            "SD clip names {} / {} carry no family",
            clips.idle, clips.walk
        ));
    };
    if idle_family != walk_family || !matches!(idle_family.as_str(), "cw" | "cm") {
        return Err(format!(
            "SD idle/walk families disagree or are unknown: {idle_family} / {walk_family}"
        ));
    }
    Ok(match clip {
        SourceClip::CannonS => SdStandIn::Single {
            clip: format!("{}_L", clips.idle),
            looping: true,
        },
        SourceClip::CannonS2 => SdStandIn::Single {
            clip: format!("{}_S", clips.run),
            looping: false,
        },
        SourceClip::CannonL => SdStandIn::Single {
            clip: format!("{}_L", clips.run),
            looping: true,
        },
        SourceClip::CannonE => SdStandIn::Single {
            clip: format!("{}_E", clips.run),
            looping: false,
        },
        SourceClip::Cannon02E => SdStandIn::Pair {
            first: format!("mov_{idle_family}_normal_stumble001_S"),
            second: format!("mov_{idle_family}_normal_stumble001_E"),
        },
    })
}

fn push_se(world: &mut World, cue: &str) {
    world.resource_mut::<SeRequests>().0.push(SeRequest {
        owner: None,
        cue: cue.to_owned(),
        class: SeClass::Ingame,
        source: "site-move",
    });
}

fn emit_effect(world: &mut World, kind: effects::EffectType, pose: Transform) {
    if !world.contains_resource::<SiteMoveEffects>() {
        return;
    }
    world.resource_scope(|world, mut effects: Mut<SiteMoveEffects>| {
        effects.0.emit(world, kind, pose);
    });
}

/// World-space particles already emitted keep their world positions; the
/// re-origin moves them with everything else.
fn shift_world_particles(world: &mut World, root: Entity, delta: Vec3) {
    let draws: Vec<Entity> = world
        .get::<Children>(root)
        .map(|children| children.iter().collect())
        .unwrap_or_default();
    for draw in draws {
        if let Some(mut live) = world.get_mut::<crate::uber_particle::FixtureParticleLive>(draw) {
            let system = &mut live.0;
            if system.emitter.simulation_space == moly_law::particle::schema::SimulationSpace::World
            {
                for particle in &mut system.pool {
                    particle.position[0] -= delta.x;
                    particle.position[1] -= delta.y;
                    particle.position[2] -= delta.z;
                }
            }
        }
    }
}

/// PostUpdate while the hold is in force: destination content that would
/// render is kept hidden (its visibility is kept to restore). The source
/// disables the destination's renderers except SD character bodies; NPCs,
/// which are not site content here, stay visible as those bodies do.
fn hold_reveal(
    mut commands: Commands,
    mut fresh: Query<
        (Entity, &mut Visibility),
        (
            Or<(
                With<crate::site::SiteRoot>,
                With<crate::fixture::FixtureRoot>,
                With<crate::harvest::HarvestRoot>,
                With<crate::uber_particle::UberParticleDraw>,
            )>,
            Without<RevealHeld>,
            Without<SiteMoveOwned>,
        ),
    >,
    mut held: Query<(&mut Visibility, &mut RevealHeld), Changed<Visibility>>,
) {
    for (entity, mut visibility) in &mut fresh {
        if *visibility == Visibility::Hidden {
            continue;
        }
        commands.entity(entity).insert(RevealHeld(*visibility));
        *visibility = Visibility::Hidden;
    }
    // A writer changed a held entity again (a material reveal, a harvest
    // state): keep its latest wish for the release.
    for (mut visibility, mut wish) in &mut held {
        wish.0 = *visibility;
        *visibility = Visibility::Hidden;
    }
}

fn release_reveal(world: &mut World) {
    if world.remove_resource::<RevealHold>().is_none() {
        return;
    }
    let mut held = world.query::<(Entity, &RevealHeld)>();
    let rows: Vec<(Entity, Visibility)> = held
        .iter(world)
        .map(|(entity, wish)| (entity, wish.0))
        .collect();
    for (entity, visibility) in &rows {
        world
            .entity_mut(*entity)
            .insert(*visibility)
            .remove::<RevealHeld>();
    }
    info!(
        "[site-move] SetRenderingEnabled(next, true): {} held destination roots restored",
        rows.len()
    );
}

/// PostUpdate after the camera: the flying effect follows it.
fn follow_flying(
    effects: Option<Res<SiteMoveEffects>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut nodes: Query<(&mut Transform, &mut GlobalTransform), Without<Camera3d>>,
    children: Query<&Children>,
) {
    let Some(root) = effects.and_then(|effects| effects.0.flying_root()) else {
        return;
    };
    let Ok(camera) = cameras.single() else {
        return;
    };
    effects::follow_camera(root, *camera, &mut nodes, &children);
}
