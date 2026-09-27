//! The site moves of the source's `SiteMoveActionExecutor`: `GetActionState`
//! answers a switch with the cannon (`MoveSiteUseCannonActionState`, among
//! the home, harvest and delivery sites) or one of the three door states
//! (home and rooms, [`door`]); a room and a harvest or delivery site have no
//! state (the source logs an error and returns null). Each move runs its
//! pre-action, core and end action with the game in `GameState.SiteMove`
//! from admission until it returns to Normal. A move can carry the next one:
//! a room to an outdoor site is the source's nested `ChangeSiteEventData`
//! (the door move home, then the cannon, in one `GameState.SiteMove`); an
//! outdoor site to a room, which the source reaches only by two moves, is
//! served as the cannon home then the home-to-room door move, named once as
//! a product composition.
//!
//! The cannon:
//!
//! The source keeps every site at its master `SitePosition` in one world;
//! here the current site is the origin. The Core therefore runs in the old
//! site's frame, where the destination sits at `O_new - O_old`, and the swap
//! re-origins everything the move carries (player, camera, its tween, the
//! cannon, world-space particles) by that offset on the frame the player's
//! flight tween ends. The destination is preloaded in the pre-action (the
//! source awaits `SiteManager.AddSite`; a listed site such as home is still
//! loaded, and its hidden instance is shown again), and it stays hidden until
//! the Core's `SetRenderingEnabled(nextSite, true)` step. The old site stays
//! shown at `O_old - O_new` after the swap
//! ([`crate::site::queue_cannon_swap`]); the end action removes it
//! (`CanRemoveSite`: a harvest or delivery site) or leaves it listed and
//! hidden (home), [`crate::site::end_departure`].
//!
//! Named differences:
//! - The environment follow (`environmentRoot.position = Step.position`
//!   during the flight, then its y restored at landing) has no product
//!   counterpart: the sky dome and the weather sky anchor already follow the
//!   player every frame. The two steps are logged, not applied.
//!
//! Timing conventions are in [`timeline`]: waits are the source's
//! millisecond-rounded delays that skip their creation frame; tweens and
//! animation clocks advance from the frame they start.

pub(crate) mod arrival;
pub(crate) mod camera;
mod cannon;
pub(crate) mod door;
pub(crate) mod door_law;
pub(crate) mod door_state;
pub(crate) mod effects;
pub(crate) mod products;
pub(crate) mod room_door;
mod speed_lines;
pub(crate) mod timeline;
pub(crate) mod wipe;

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
    AvatarDriver, PlayerActionMotion, PlayerActionOwner, PlayerActionToken, IDLE_CLIP, STATE_FADE,
};
use crate::player_state::{PlayerActionState, PlayerAvatarStates};
use crate::site::{SitePreload, SiteSelection};
use crate::ui_layers::{LayerCommand, LayerId, UiLayerStack};
use timeline::{
    core_waits, final_wait, landing, move_time, out_sine, range_int, Delay, Landing, SourceClip,
    Step, TweenClock,
};

/// Queued by `site::read_switch` for a switch between two different sites
/// (`OnChangeSite`); the executor answers it with `GetActionState`.
#[derive(Resource)]
pub(crate) struct SiteMoveRequest {
    pub(crate) from: String,
    pub(crate) next: SiteSelection,
    /// The move admitted when this one ends: the source's nested
    /// `ChangeSiteEventData`, or the second leg of a product composition.
    pub(crate) then: Option<SiteSelection>,
    /// Names the product composition this chain serves (`None` for a source
    /// move or the source's own nesting).
    pub(crate) composition: Option<&'static str>,
}

/// `SiteMoveActionExecutor.GetActionState` over the two sites' kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActionState {
    Cannon,
    Door(door_law::DoorKind),
}

fn action_state(from: &str, to: &str) -> Option<ActionState> {
    use crate::weather_transition::SiteKind::{Delivery, Harvest, Home, Room};
    use door_law::DoorKind;
    let of = crate::weather_transition::SiteKind::of;
    Some(match (of(from), of(to)) {
        (Home, Room) => ActionState::Door(DoorKind::HomeToRoom),
        (Room, Home) => ActionState::Door(DoorKind::RoomToHome),
        (Room, Room) => ActionState::Door(DoorKind::RoomToRoom),
        (Home | Harvest | Delivery, Home | Harvest | Delivery) => ActionState::Cannon,
        (Room, Harvest | Delivery) | (Harvest | Delivery, Room) => return None,
    })
}

/// A door move carried a nested move: its own Normal and `OnFinishEnterAsync`
/// run after the nested move has returned.
#[derive(Resource)]
struct NestedOuter {
    kind: door_law::DoorKind,
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

/// The cannon move's writes to the source's environment root, for the sky
/// host that keeps that root. At the fire the move keeps the root's height
/// (`envDefaultPositionY`); on every update of the follow (`DOVirtual.Float`
/// over the flight time) it sets the root to the player view's `Step`
/// position; at the landing effect it sets the root's height back to the
/// kept one. Points are in the source world frame. Present from the fire to
/// the end of the move; door moves do not write the root. The product tweens
/// the player's root, which the `Step` node sits on.
#[derive(Resource, Clone, Debug)]
pub(crate) struct CannonEnvironmentWrite {
    /// One value per move (its admission frame).
    pub(crate) move_id: u64,
    /// The destination's site type.
    pub(crate) site_type: String,
    /// The follow's last write; `None` before its first update.
    pub(crate) point: Option<Vec3>,
    /// The landing effect has set the kept height back.
    pub(crate) height_restored: bool,
}

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
    /// The site left's `SitePosition` in the source frame.
    from_source_origin: Vec3,
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
    flying_started: bool,
    records: Vec<StepRecord>,
    /// The move admitted when this one ends (see [`SiteMoveRequest`]).
    then: Option<SiteSelection>,
    composition: Option<&'static str>,
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
    effects::install(app);
    speed_lines::install(app);
    wipe::install(app);
    room_door::install(app);
    app.add_systems(
        Update,
        (
            room_door::ensure.before(advance),
            (
                room_door::advance,
                wipe::advance,
                (
                    crate::player_state::update_house_states,
                    door::after_states.run_if(resource_exists::<door::DoorMove>),
                )
                    .chain(),
            )
                .after(advance),
            door::tag_house_entry,
        ),
    );
    app.add_systems(Startup, products::request);
    app.add_systems(Update, products::resolve.run_if(products::pending));
    app.add_observer(on_instance_ready)
        .add_systems(
            Update,
            (
                advance
                    .after(crate::site::read_switch)
                    .before(crate::site::plan)
                    // The NPC side answers the door's await before the door
                    // reads it, so an answer lets the door go on that frame.
                    .after(crate::npc::DoorAnswerSet)
                    .run_if(
                        resource_exists::<SiteMove>
                            .or(resource_exists::<SiteMoveRequest>)
                            .or(resource_exists::<door::DoorMove>),
                    ),
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

/// The move's effects, played from the `EffectManager` pools
/// ([`effects::EffectPools`], built before any move, as `EffectManager.Setup`
/// runs in the field scene's setup, and outliving each move: the landing
/// effect keeps playing after the move has returned to Normal), with the
/// flying copy the camera carries. Present only when the release root lists
/// the site-move products (`products`).
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
        if world.contains_resource::<SiteMove>() || world.contains_resource::<door::DoorMove>() {
            warn!(
                "[site-move] a move is already in progress; request for {} dropped",
                request.next.site_type()
            );
        } else {
            admit(world, request, frame);
        }
    }
    if world.contains_resource::<door::DoorMove>() {
        let done = world.resource_scope(|world, mut session: Mut<door::DoorMove>| {
            session.frame(world, frame, dt);
            session.done()
        });
        if done {
            let mut session = world
                .remove_resource::<door::DoorMove>()
                .expect("present above");
            let from = session.to.clone();
            match session.take_then() {
                Some((next, composition)) => {
                    // ChangeSiteCore: `NextEvent` runs its own OnChangeSite
                    // before this move's Normal (the source's nesting), or
                    // the product composition's second leg follows.
                    let kind = session.kind;
                    session.finish(world, composition.is_none());
                    if composition.is_none() {
                        world.insert_resource(NestedOuter { kind });
                        info!(
                            "[site-move] {}: ChangeSiteEventData.NextEvent: OnChangeSite({from} -> {}) nested, still in GameState SiteMove",
                            kind.name(),
                            next.site_type()
                        );
                    }
                    admit(
                        world,
                        SiteMoveRequest {
                            from,
                            next,
                            then: None,
                            composition,
                        },
                        frame,
                    );
                }
                None => {
                    session.normal(world);
                    session.finish(world, false);
                }
            }
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
        let mut session = world.remove_resource::<SiteMove>().expect("present above");
        let then = session.then.take();
        let composition = session.composition;
        let from = session.to.clone();
        session.finish(world);
        if let Some(outer) = world.remove_resource::<NestedOuter>() {
            info!(
                "[site-move] {} resumes after its nested move: GameState Normal (already Normal), OnFinishEnterAsync of {from} again (no product counterpart)",
                outer.kind.name()
            );
        }
        if let Some(next) = then {
            admit(
                world,
                SiteMoveRequest {
                    from,
                    next,
                    then: None,
                    composition,
                },
                frame,
            );
        }
    }
}

fn advance_effects(world: &mut World) {
    let dt = world.resource::<Time>().delta_secs();
    world.resource_scope(|world, mut effects: Mut<SiteMoveEffects>| effects.0.advance(world, dt));
}

/// `OnChangeSite`: `GameState.SiteMove`, then `ChangeSiteProcessAsync`.
fn admit(world: &mut World, request: SiteMoveRequest, frame: u64) {
    let SiteMoveRequest {
        from,
        next,
        then,
        composition,
    } = request;
    let to = next.site_type().to_owned();
    if let Some(name) = composition {
        info!(
            "[site-move] product composition ({name}): {from} -> {to}{}",
            then.as_ref().map_or(String::new(), |then| format!(
                ", then -> {}",
                then.site_type()
            ))
        );
    }
    match action_state(&from, &to) {
        None => {
            error!("[site-move] GetActionState({from}, {to}): the source has no move state for this pair and returns null; the request is dropped");
            return;
        }
        Some(ActionState::Door(kind)) => {
            door::admit(world, kind, from, next, then, composition, frame);
            return;
        }
        Some(ActionState::Cannon) => {}
    }
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
    let arrival =
        arrival::Arrival::for_destination(&server, &to_place.category, layout, preload.scene_json());
    let delta = to_place.product_origin() - from_place.product_origin();
    world.insert_resource(SiteMoveActive);
    world.insert_resource(EnvironmentHold);
    world.insert_resource(BgmHold);
    // SiteMoveGameState.OnEnter: the gesture layer off, the empty site-move
    // screen replaces the field screen (ChangeUIScreen 653), taps disabled.
    // The site map's screen is removed first: `MysekaiUtility.ChangeSite`
    // calls `RemoveScreen(MysekaiSiteMap)` when the map is in the screen map
    // and active. RemoveScreen runs outside the transition slot and mounts
    // nothing, so the change that follows is not held by a mount's start
    // animation.
    if world.resource::<UiLayerStack>().is_active(LayerId::MysekaiSiteMap) {
        world.write_message(LayerCommand::Remove(LayerId::MysekaiSiteMap));
    }
    world.write_message(LayerCommand::Change(LayerId::MysekaiSiteMove));
    info!(
        "[site-move] admitted {from} -> {to} (cannon): offset {delta:.1}, GameState SiteMove; pre-action: PlayerFootEffect stop, CleanupCurrentSite, AddSite preload"
    );
    crate::footstep::stop_key(world, "cannon pre-action");
    if from_place.category == "housing_home" {
        crate::ui_layers::harvest_summary::save_harvest_point(world, "MoveSiteUseCannonActionState.OnSiteMovePreAction");
    }
    world.insert_resource(SiteMove {
        source_clip: None,
        from,
        to,
        from_category: from_place.category,
        to_category: to_place.category,
        delta,
        from_source_origin: Vec3::from_array(from_place.position),
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
        flying_started: false,
        records: Vec::new(),
        then,
        composition,
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
                    crate::ui_layers::harvest_summary::show_harvest_point_summary(world, "MoveSiteUseCannonActionState.EndAction");
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
        let effects =
            !world.contains_resource::<SiteMoveEffects>() || effects::move_pools_settled(world);
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
        let before = world.resource::<PlayerAvatarStates>().current;
        world
            .resource_mut::<PlayerAvatarStates>()
            .change_status(PlayerActionState::Idle);
        // SetMoveEnabled(false). Entering Idle plays the Idle state's clip;
        // a player already Idle replays nothing (`ChangeStatus` to the
        // current state returns at once) and keeps its clip.
        if before != PlayerActionState::Idle
            && world.resource::<PlayerAvatarStates>().current == PlayerActionState::Idle
        {
            self.play_idle(world);
        } else {
            self.lease_player(world);
        }
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
        world.insert_resource(CannonEnvironmentWrite {
            move_id: self.admitted_frame,
            site_type: self.to.clone(),
            point: None,
            height_restored: false,
        });
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
        match world.get_resource_mut::<CannonEnvironmentWrite>() {
            Some(mut write) if !write.height_restored => {
                write.height_restored = true;
                info!(
                    "[site-move] environment root y restored to the height kept at the fire (move {} to {}, last follow point {:?})",
                    write.move_id, write.site_type, write.point
                );
            }
            Some(_) => error!("[site-move] environment root y restored twice in one move"),
            None => warn!("[site-move] environment root y restore: no cannon write this move"),
        }
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
        // SetMoveEnabled(true): movement and locomotion come back. The
        // player's state is still Idle (set at setup) or an exit state whose
        // closed gate drops the change below, so no idle is replayed: the
        // landing clip holds its last frame until the next state.
        self.hand_back_player(world);
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
        // prev.OnExitSite, then RemoveSite when CanRemoveSite, else it stays
        // listed and next.OnEnterSite's HideOtherSite hides it. Cleanup and
        // ChangeCurrentSiteOnlyData ran at the swap; SetupLockAtCameraBounds
        // ran when the destination settled.
        crate::site::end_departure(world);
        // next.OnEnterSite: a harvest or the delivery site changes the UI
        // screen to its field screen here.
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
            world.write_message(crate::cutscene::HomeScreenStartAnimation {
                caller: "a cannon arrival at home",
            });
        }
    }

    fn finish(self, world: &mut World) {
        world.remove_resource::<CannonEnvironmentWrite>();
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
            if self.environment_follow.is_some() {
                // The follow's update: the environment root takes the player
                // view's point (old-site frame here), in the source world.
                let point = self.from_source_origin + crate::particle_geometry::reflect(position);
                if let Some(mut write) = world.get_resource_mut::<CannonEnvironmentWrite>() {
                    write.point = Some(point);
                }
            }
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
        // `PublishFlyingEffectPlay` is an event of the clip the player is
        // playing: it fires when that clip's own playback time, read from
        // the animator, reaches the event.
        if let Some(event) = self.source_clip.and_then(SourceClip::flying_play_event) {
            let clip_time = self.source_clip.and_then(|clip| player_clip_time(world, clip.name()));
            if !self.flying_started && clip_time.is_some_and(|t| t >= event) {
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
                    "[_s2 animator time {:.4} >= {event:.4}]",
                    clip_time.unwrap_or(f32::NAN)
                ));
                self.record(Step::FlyingStart, frame);
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
            crate::site::queue_cannon_swap(&mut commands, roots, next);
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
            "[site-move] swap: {} left shown at its offset, {} requested from its preload; re-origin by {:.1}",
            self.from, self.to, -delta
        );
    }

    /// `PlayerAvatarIdleState.Initialize` when the move takes the player:
    /// `PlayAnimation("c_000_mov_idle_00")`, fade 0.25.
    fn play_idle(&mut self, world: &mut World) {
        self.play_clip(world, IDLE_CLIP, STATE_FADE);
    }

    /// `ChangeAnimation(name, fade, 1)`: the source clip itself.
    fn change_animation(&mut self, world: &mut World, clip: SourceClip) {
        self.source_clip = Some(clip);
        self.play_clip(world, clip.name(), Duration::from_secs_f32(clip.fade()));
    }

    fn play_clip(&mut self, world: &mut World, clip: &str, blend: Duration) {
        let mut state = SystemState::<(
            ResMut<Assets<AnimationGraph>>,
            Query<&mut AvatarDriver, With<PlayerControlled>>,
            Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
        )>::new(world);
        let (mut graphs, mut drivers, mut animators) = state.get_mut(world);
        let Some(mut driver) = drivers.iter_mut().next() else {
            error!("[site-move] no player driver for {clip}");
            return;
        };
        let Ok((mut player, mut transitions)) = animators.get_mut(driver.player) else {
            error!("[site-move] player animator missing for {clip}");
            return;
        };
        let motion = PlayerActionMotion {
            clip,
            speed: 1.0,
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
            Ok(token) => {
                self.token = Some(token);
                info!(
                    "[site-move] ChangeAnimation({clip}, fade {:.2}s, speed 1)",
                    blend.as_secs_f32()
                );
            }
            Err(error) => error!("[site-move] ChangeAnimation({clip}) refused: {error:?}"),
        }
    }

    fn lease_player(&mut self, world: &mut World) {
        if self.token.is_some() {
            return;
        }
        let mut drivers = world.query_filtered::<&mut AvatarDriver, With<PlayerControlled>>();
        let Some(mut driver) = drivers.iter_mut(world).next() else {
            error!("[site-move] no player driver to lease");
            return;
        };
        match driver.acquire(PlayerActionOwner::Cannon) {
            Ok(token) => {
                self.token = Some(token);
                info!("[site-move] ChangeState(Idle) on an Idle player: no clip replayed, the animator is leased");
            }
            Err(error) => error!("[site-move] the player's animator is not free: {error:?}"),
        }
    }

    fn hand_back_player(&mut self, world: &mut World) {
        let Some(token) = self.token.take() else {
            return;
        };
        let mut drivers = world.query_filtered::<&mut AvatarDriver, With<PlayerControlled>>();
        if let Some(mut driver) = drivers.iter_mut(world).next() {
            driver.hand_back(token);
        }
    }
}

/// The time a clip on the player's animator reaches this frame: the node's
/// own seek time plus the advance the animator applies later in this frame
/// (frame delta times the node's speed), so an event fires on the frame the
/// animator crosses it, as the engine's animation events do.
fn player_clip_time(world: &mut World, clip: &str) -> Option<f32> {
    let dt = world.resource::<Time>().delta_secs();
    let mut drivers = world.query_filtered::<&AvatarDriver, With<PlayerControlled>>();
    let driver = drivers.iter(world).next()?;
    let node = driver.node_of(clip)?;
    let animator = driver.player;
    world
        .get::<AnimationPlayer>(animator)?
        .animation(node)
        .map(|active| active.seek_time() + dt * active.speed())
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
