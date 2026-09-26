//! The three door moves of `SiteMoveActionExecutor`: `HomeToMyRoomActionState`,
//! `MyRoomToHomeActionState` and `MyRoomToMyRoomActionState`, each with its
//! pre-action, core and end action, run by the same executor as the cannon
//! (`super::advance`), with the game in `GameState.SiteMove` from admission
//! until the executor returns it to Normal.
//!
//! One site is loaded at a time. The source keeps home loaded while the
//! player is in a room (`SiteManager` never removes the home site; the rooms
//! sit 400 m away) and switches its visibility: home to room hides home in
//! the end action, after the wipe has opened on the room; room to home shows
//! home in the pre-action, before the wipe closes. Here the site is swapped
//! under the closed wipe instead: `ChangeSite` tears the old site down and
//! waits until the destination stands (scenes, materials, fixtures and, for
//! home, the house; for a room, its appearance and its door), which is where
//! the source's `ChangeSite` would wait for a site it had not loaded. Nothing
//! of either site shows through: the wipe covers every pixel from the end of
//! its close to the start of its open.
//!
//! Timing conventions are the cannon's ([`super::timeline`]): waits are the
//! source's millisecond-rounded delays that skip their creation frame; the
//! wipe tween advances from the frame it starts; a `WaitUntil` on a callback
//! continues on the frame after the callback. Every step is logged with its
//! anchor (admission, the arrival at the door, the destination standing),
//! the source offset from that anchor, the measured one and the frames.
//!
//! Named differences:
//! - `HomeSiteController.ExecuteNPCRandomFixtureAction` (room to home, the
//!   NPCs take random fixture actions while the wipe opens) belongs to the
//!   NPC domain. The core requests it by inserting
//!   `npc::RandomFixtureActionPending` and awaits it until the NPC side
//!   removes it, but only while the NPC side's reader marker
//!   (`npc::RandomFixtureActionReader`) exists; without the marker the await
//!   returns at once with one WARN naming the missing reader.
//! - `Home.OnEnterSite` returns the game to Normal inside the room-to-home
//!   core; here the site-move input lock holds until the executor's Normal
//!   (in the source the player's closed intercept gate and the HouseEntry
//!   camera hold the same span).
//! - `MysekaiTransitioner.SafeFinish` does nothing in solo play: no white
//!   transitioner exists (`UIUtility.PlayMysekaiTransition` has one caller,
//!   the multiplay layout update). Logged at its step.
//! - `CullingWallFixture` (home to room, 0.03 s after HouseEntry) and the
//!   player's render layer have no product counterpart; each is logged at
//!   its step. The room's expansion
//!   performance in `OnFinishEnterAsync` is `site_expansion`.
//! - Normal's private camera model is written on its exit only while no
//!   camera tween is in flight: the source clears that flag only during its
//!   own transfer tween.
//! - A room exit's `FadeOut` callback (`player.Hide`) runs on the frame after
//!   the tween completes, when the executor reads it; the wipe covers the
//!   screen on both frames.

use bevy::prelude::*;

use super::door_law::{self, Anchor, DoorKind, DoorStep};
use super::door_state::{self, HouseState};
use super::timeline::Delay;
use super::wipe::{self, Fade, Init};
use crate::camera::{
    CameraSetting, CameraStateType, CameraTween, FieldCameraModel, FieldCameraState,
};
use crate::entry::law::{self as entry_law, NormalPrivate};
use crate::fixture_gimmick::house_door::{self, HouseBinding, HouseLookup, HouseTrigger};
use crate::player::PlayerControlled;
use crate::player_avatar::PlayerActionToken;
use crate::player_fixture_action::PlayerFixtureNavigation;
use crate::player_state::{PlayerActionState, PlayerAvatarStates};
use crate::site::{SitePreload, SiteSelection};
use crate::ui_layers::{LayerCommand, LayerId, UiLayerStack};

/// Product guard, not a source value: a room without a player navigation
/// snapshot after this long walks to its door in a straight line (WARN).
const NAVIGATION_WAIT_REAL: f64 = 5.0;
/// Product guard, not a source value: an AutoMove that has not moved for
/// this long is placed on its target (ERROR).
const WALK_STALL: f32 = 0.5;

/// Normal's private model (`NormalCameraState._model`): `OnExit` copies the
/// owner's look-at, yaw, pitch and distance into it; its FOV stays the
/// camera setting's.
#[derive(Clone, Copy, Debug)]
struct PrivateModel {
    look_at: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
    fov: f32,
}

#[derive(Clone, Copy, Debug)]
enum Stage {
    /// The inputs of the move (house, navigation, room door, wipe) bind.
    PreAction,
    Walking,
    /// Home to room: `PlayEnterMyRoomAction`'s 1.1 s.
    EnterHouseWait(Delay),
    /// `DoorTransitionerManager.Init` before a FadeOut (home to room).
    WipeInit,
    /// Home to room: `WaitUntil` on FadeOut's callback.
    FadeOutWait,
    /// Room exits: 0.2 s after `se_door_open`.
    ExitSeWait(Delay),
    /// Room exits: 1.0 s after `ShowFadeOutAnimation`.
    AfterFadeOutWait(Delay),
    /// Room to room: `MoveMyRoom`'s 1.0 s before `ChangeSite`.
    RoomWait(Delay),
    Loading,
    /// Home to room: 1.0 s after the room door starts opening.
    EnterRoomWait(Delay),
    /// Room to home: the await on
    /// `HomeSiteController.ExecuteNPCRandomFixtureAction` (the NPC side
    /// removes `npc::RandomFixtureActionPending`).
    NpcFixtureActionWait,
    /// Room to home: `StartMysekaiTransition`'s 0.1 s.
    StartTransitionWait(Delay),
    /// Room to home: `PlayExitMyRoomAction`'s 1.0 s.
    ExitHouseWait(Delay),
    /// Home to room and room to room: 1.0 s after `FadeIn`.
    AfterFadeInWait(Delay),
    Done,
}

/// `PlayerAvatarAutoMoveState` toward one point.
struct Walk {
    target: Vec3,
    corners: Vec<Vec3>,
    cursor: usize,
    last: Vec3,
    unchanged: f32,
}

struct Record {
    step: &'static str,
    anchor: Option<Anchor>,
    source: Option<f64>,
    measured: f64,
    frames: u64,
    note: String,
}

/// One door move (`ChangeSiteProcessAsync` with a door state).
#[derive(Resource)]
pub(crate) struct DoorMove {
    pub(crate) kind: DoorKind,
    pub(crate) from: String,
    pub(crate) to: String,
    next: Option<SiteSelection>,
    /// The nested `ChangeSiteEventData` or the second leg of a product
    /// composition: the move the executor admits when this one ends.
    then: Option<SiteSelection>,
    composition: Option<&'static str>,
    preload: Option<SitePreload>,
    stage: Stage,
    admitted_frame: u64,
    now: f64,
    frame: u64,
    anchors: [Option<(f64, u64)>; 3],
    records: Vec<Record>,
    token: Option<PlayerActionToken>,
    walk: Option<Walk>,
    house: Option<HouseBinding>,
    private: Option<PrivateModel>,
    fade_out_pending: bool,
    fade_out_done: bool,
    fade_in_pending: bool,
    /// The wipe was refused: its callbacks complete at once.
    no_wipe: bool,
    /// The state whose `UpdateState` returns to Idle next; Idle's
    /// `Initialize` then plays the idle loop.
    idle_watch: Option<(PlayerActionState, DoorStep)>,
    wait_logged: bool,
    started_real: f64,
    stage_real: f64,
}

fn anchor_index(anchor: Anchor) -> usize {
    match anchor {
        Anchor::Admission => 0,
        Anchor::Arrival => 1,
        Anchor::SiteReady => 2,
    }
}

impl DoorMove {
    #[allow(clippy::too_many_arguments)]
    fn new(
        kind: DoorKind,
        from: String,
        next: SiteSelection,
        then: Option<SiteSelection>,
        composition: Option<&'static str>,
        preload: Option<SitePreload>,
        frame: u64,
        real: f64,
    ) -> Self {
        let to = next.site_type().to_owned();
        let mut session = Self {
            kind,
            from,
            to,
            next: Some(next),
            then,
            composition,
            preload,
            stage: Stage::PreAction,
            admitted_frame: frame,
            now: 0.0,
            frame,
            anchors: [Some((0.0, frame)), None, None],
            records: Vec::new(),
            token: None,
            walk: None,
            house: None,
            private: None,
            fade_out_pending: false,
            fade_out_done: false,
            fade_in_pending: false,
            no_wipe: false,
            idle_watch: None,
            wait_logged: false,
            started_real: real,
            stage_real: real,
        };
        session.record_step(
            DoorStep::PreAction,
            "[OnChangeSite: GameState SiteMove]".into(),
        );
        session
    }

    pub(crate) fn done(&self) -> bool {
        matches!(self.stage, Stage::Done)
    }

    /// The move the executor admits after this one.
    pub(crate) fn take_then(&mut self) -> Option<(SiteSelection, Option<&'static str>)> {
        self.then.take().map(|then| (then, self.composition))
    }

    fn set_anchor(&mut self, anchor: Anchor) {
        self.anchors[anchor_index(anchor)] = Some((self.now, self.frame));
    }

    /// One step line: its anchor, the source offset, the measured one.
    fn record(&mut self, step: &'static str, source: Option<(Anchor, f64)>, note: String) {
        let anchor = source.map(|(anchor, _)| anchor);
        let (base, base_frame) = anchor
            .and_then(|anchor| self.anchors[anchor_index(anchor)])
            .unwrap_or((0.0, self.admitted_frame));
        let measured = self.now - base;
        let frames = self.frame.saturating_sub(base_frame);
        match source {
            Some((anchor, offset)) => info!(
                "[site-move] {} step {:<26} {:<10} source +{:.4}s measured +{:.4}s (delta {:+.4}s) frame +{} {}",
                self.kind.name(),
                step,
                anchor.name(),
                offset,
                measured,
                measured - offset,
                frames,
                note
            ),
            None => info!(
                "[site-move] {} step {:<26} since admission {:.4}s frame +{} {}",
                self.kind.name(),
                step,
                self.now,
                frames,
                note
            ),
        }
        self.records.push(Record {
            step,
            anchor,
            source: source.map(|(_, offset)| offset),
            measured: if source.is_some() { measured } else { self.now },
            frames,
            note,
        });
    }

    fn record_step(&mut self, step: DoorStep, note: String) {
        let source = door_law::source_offset(self.kind, step);
        self.record(step.name(), source, note);
    }

    fn due(delay: &Delay, dt: f32) -> String {
        format!(
            "[wait {:.3} elapsed {:.4} late {:+.4} < frame-dt {:.4}]",
            delay.target,
            delay.elapsed,
            delay.elapsed - delay.target,
            dt
        )
    }

    /// One executor frame: the continuations, then the AutoMove's update.
    pub(crate) fn frame(&mut self, world: &mut World, frame: u64, dt: f32) {
        if frame != self.admitted_frame {
            self.now += dt as f64;
        }
        self.frame = frame;
        self.callbacks(world);
        self.run(world, frame, dt);
        self.walk_step(world, dt);
    }

    /// The wipe callbacks, as the continuations see them (the frame after
    /// the tween completed).
    fn callbacks(&mut self, world: &mut World) {
        if self.fade_out_pending {
            let done = if self.no_wipe {
                Some((self.frame, 0.0))
            } else {
                wipe::take_finished(world, Fade::Out)
            };
            if let Some((at, clock)) = done {
                self.fade_out_pending = false;
                self.fade_out_done = true;
                self.record_step(
                    DoorStep::FadeOutDone,
                    format!("[tween clock {clock:.4} >= 1.0 on frame {at}]"),
                );
                if matches!(self.kind, DoorKind::RoomToHome | DoorKind::RoomToRoom) {
                    set_player_visible(world, false);
                    info!("[site-move] ShowFadeOutAnimation callback: player.Hide()");
                }
            }
        }
        if self.fade_in_pending {
            let done = if self.no_wipe {
                Some((self.frame, 0.0))
            } else {
                wipe::take_finished(world, Fade::In)
            };
            if let Some((at, clock)) = done {
                self.fade_in_pending = false;
                self.record_step(
                    DoorStep::FadeInDone,
                    format!("[tween clock {clock:.4} >= 1.0 on frame {at}; wipe hidden]"),
                );
            }
        }
    }

    /// After the player states' `UpdateState` this frame: a state that went
    /// Idle runs Idle's `Initialize` (the idle loop, 0.25 s).
    pub(crate) fn after_states(&mut self, world: &mut World) {
        let Some((from, step)) = self.idle_watch else {
            return;
        };
        let current = world.resource::<PlayerAvatarStates>().current;
        if current == from {
            return;
        }
        self.idle_watch = None;
        if current != PlayerActionState::Idle {
            return;
        }
        self.record_step(
            step,
            format!("[{from:?} UpdateState: gate open, Idle; Idle plays the idle loop]"),
        );
        self.token = door_state::play_idle_loop(world, self.token);
    }

    fn run(&mut self, world: &mut World, frame: u64, dt: f32) {
        loop {
            match &mut self.stage {
                Stage::PreAction => {
                    if !self.pre_action_ready(world) {
                        return;
                    }
                    self.pre_action(world);
                    if matches!(self.stage, Stage::PreAction) {
                        return;
                    }
                }
                Stage::Walking => {
                    // WaitUntil(state == Idle): the AutoMove's arrival.
                    let arrived = self.walk.is_none()
                        && world.resource::<PlayerAvatarStates>().current
                            == PlayerActionState::Idle;
                    if !arrived {
                        return;
                    }
                    self.set_anchor(Anchor::Arrival);
                    self.record_step(DoorStep::Arrival, "[WaitUntil(Idle) continued]".into());
                    self.arrival(world, frame);
                }
                Stage::EnterHouseWait(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let note = Self::due(delay, dt);
                    info!("[site-move] PlayEnterMyRoomAction returned {note}");
                    self.stage = Stage::WipeInit;
                }
                Stage::WipeInit => {
                    match wipe::init(world) {
                        Init::Pending => return,
                        Init::Ready => {}
                        Init::Refused => self.no_wipe = true,
                    }
                    self.fade_out(world, String::new());
                    self.stage = Stage::FadeOutWait;
                }
                Stage::FadeOutWait => {
                    if !self.fade_out_done {
                        return;
                    }
                    self.change_site(world);
                }
                Stage::ExitSeWait(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let note = Self::due(delay, dt);
                    // OnSiteMovePreAction: ShowFadeOutAnimation, then 1.0 s.
                    self.fade_out(world, note);
                    self.stage = Stage::AfterFadeOutWait(Delay::new(
                        door_law::wait(door_law::AFTER_FADE_OUT_WAIT),
                        frame,
                    ));
                }
                Stage::AfterFadeOutWait(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let note = Self::due(delay, dt);
                    self.record_step(DoorStep::Core, note);
                    if self.kind == DoorKind::RoomToHome {
                        self.change_site(world);
                    } else {
                        info!("[site-move] MoveMyRoom: ChangeSubState(34) (multiplay notification, solo no-op); MysekaiTransitioner.SafeFinish: no instance in solo play, nothing to finish");
                        self.stage = Stage::RoomWait(Delay::new(
                            door_law::wait(door_law::ROOM_TO_ROOM_WAIT),
                            frame,
                        ));
                    }
                }
                Stage::RoomWait(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    info!(
                        "[site-move] MoveMyRoom wait returned {}",
                        Self::due(delay, dt)
                    );
                    self.change_site(world);
                }
                Stage::Loading => {
                    if !self.destination_ready(world) {
                        return;
                    }
                    self.set_anchor(Anchor::SiteReady);
                    let arrival = self.anchors[anchor_index(Anchor::Arrival)].map_or(0.0, |a| a.0);
                    self.record_step(
                        DoorStep::SiteReady,
                        format!(
                            "[ChangeSite returned: {} stands, {:.4}s after the arrival]",
                            self.to,
                            self.now - arrival
                        ),
                    );
                    self.site_ready(world, frame);
                }
                Stage::EnterRoomWait(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let note = Self::due(delay, dt);
                    self.enter_room(world, frame, note);
                }
                Stage::NpcFixtureActionWait => {
                    if world.contains_resource::<crate::npc::RandomFixtureActionPending>() {
                        if world.contains_resource::<crate::npc::RandomFixtureActionReader>() {
                            self.waiting(
                                world,
                                "ExecuteNPCRandomFixtureAction",
                                "the NPC side has not removed RandomFixtureActionPending",
                            );
                            return;
                        }
                        warn!("[site-move] MyRoomToHome core: npc::RandomFixtureActionReader went away while the await was pending; RandomFixtureActionPending removed, the await returns");
                        world.remove_resource::<crate::npc::RandomFixtureActionPending>();
                    }
                    info!("[site-move] HomeSiteController.ExecuteNPCRandomFixtureAction returned");
                    self.move_to_entrance(world, frame);
                }
                Stage::StartTransitionWait(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let note = Self::due(delay, dt);
                    self.exit_house(world, frame, note);
                }
                Stage::ExitHouseWait(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let note = Self::due(delay, dt);
                    self.finish_room_to_home(world, note);
                }
                Stage::AfterFadeInWait(delay) => {
                    if !delay.tick(frame, dt) {
                        return;
                    }
                    let note = Self::due(delay, dt);
                    self.finish_into_room(world, note);
                }
                Stage::Done => return,
            }
        }
    }

    fn waiting(&mut self, world: &World, what: &str, reason: &str) {
        let real = world.resource::<Time<Real>>().elapsed_secs_f64();
        if !self.wait_logged && real - self.stage_real > 1.0 {
            self.wait_logged = true;
            info!("[site-move] {} {what} waits: {reason}", self.kind.name());
        }
    }

    // --- Pre-action ----------------------------------------------------

    fn pre_action_ready(&mut self, world: &mut World) -> bool {
        let server = world.resource::<AssetServer>().clone();
        match self.preload.as_ref().map(|preload| preload.ready(&server)) {
            Some(Ok(false)) => {
                self.waiting(world, "pre-action", "destination preload");
                return false;
            }
            Some(Err(error)) => {
                error!("[site-move] destination preload: {error}");
                self.preload = None;
            }
            _ => {}
        }
        let blocker = match self.kind {
            DoorKind::HomeToRoom => match house_door::find_house(world) {
                HouseLookup::Pending(reason) => Some(reason),
                _ => navigation_ready(world).err(),
            },
            DoorKind::RoomToHome | DoorKind::RoomToRoom => {
                if !super::room_door::settled(world) {
                    Some("the room door is binding".to_owned())
                } else if wipe::init(world) == Init::Pending {
                    Some("the door transitioner is loading".to_owned())
                } else {
                    navigation_ready(world).err()
                }
            }
        };
        match blocker {
            None => true,
            Some(reason) => {
                let real = world.resource::<Time<Real>>().elapsed_secs_f64();
                if reason == NAVIGATION_PENDING
                    && self.kind != DoorKind::HomeToRoom
                    && real - self.started_real > NAVIGATION_WAIT_REAL
                {
                    warn!(
                        "[site-move] {} {}: no player navigation for this room after {NAVIGATION_WAIT_REAL} s; the walk to the door goes straight (product gap)",
                        self.kind.name(),
                        self.from
                    );
                    return true;
                }
                self.waiting(world, "pre-action", &reason);
                false
            }
        }
    }

    fn pre_action(&mut self, world: &mut World) {
        self.wait_logged = false;
        match self.kind {
            DoorKind::HomeToRoom => {
                info!("[site-move] HomeToMyRoom pre-action: CleanupCurrentSite (no product counterpart); core: GetHouseView");
                let house = match house_door::find_house(world) {
                    HouseLookup::Found(house) => house,
                    HouseLookup::Absent => {
                        return self.abort(world, "no placed fixture carries a HouseView")
                    }
                    HouseLookup::Failed(reason) | HouseLookup::Pending(reason) => {
                        return self.abort(world, &reason)
                    }
                };
                let inside = house
                    .inside_door
                    .and_then(|e| world.get::<GlobalTransform>(e).copied());
                self.house = Some(house);
                let Some(inside) = inside else {
                    return self.abort(world, "HouseView.InsideDoorActionPoint is null");
                };
                info!("[site-move] ChangeSubState(34) (multiplay notification, solo no-op); AutoMoveEntrance to the inside door");
                self.start_walk(world, inside.translation());
            }
            DoorKind::RoomToHome | DoorKind::RoomToRoom => {
                if self.kind == DoorKind::RoomToHome {
                    info!("[site-move] MyRoomToHome pre-action: ChangeSubState(34) (solo no-op); ShowSite(home): home is loaded at ChangeSite here (one site at a time); CleanupCurrentSite (no product counterpart)");
                } else {
                    info!("[site-move] MyRoomToMyRoom pre-action: CleanupCurrentSite (no product counterpart)");
                }
                if wipe::init(world) == Init::Refused {
                    self.no_wipe = true;
                }
                if self.kind == DoorKind::RoomToRoom {
                    // OpenDoorAsync().Forget() before the walk.
                    super::room_door::open(world);
                }
                let Some(inside) = super::room_door::inside_point(world)
                    .and_then(|e| world.get::<GlobalTransform>(e).copied())
                else {
                    return self.abort(world, "the room has no door action point");
                };
                self.start_walk(world, inside.translation());
            }
        }
    }

    /// `ChangeStateAutoMove(target)`, then `SetInterceptFlag(false)`.
    fn start_walk(&mut self, world: &mut World, target: Vec3) {
        let Some(player) = super::player_entity(world) else {
            return self.abort(world, "no player");
        };
        let from = world
            .get::<Transform>(player)
            .map_or(target, |t| t.translation);
        let corners = match agent_path(world, from, target) {
            Ok(corners) => corners,
            Err(reason) => {
                error!("[site-move] AutoMove to {target:.2}: {reason}; walking straight");
                vec![target]
            }
        };
        {
            let mut states = world.resource_mut::<PlayerAvatarStates>();
            states.change_status(PlayerActionState::AutoMove);
            states.can_intercept = false;
        }
        self.token = door_state::play_run_loop(world, self.token);
        let length: f32 = std::iter::once(from)
            .chain(corners.iter().copied())
            .collect::<Vec<_>>()
            .windows(2)
            .map(|pair| pair[0].distance(pair[1]))
            .sum();
        self.record_step(
            DoorStep::AutoMove,
            format!(
                "[ChangeStateAutoMove to {target:.2}: {} corners, {length:.2} m; gate closed]",
                corners.len()
            ),
        );
        self.walk = Some(Walk {
            target,
            corners,
            cursor: 0,
            last: from,
            unchanged: 0.0,
        });
        self.stage = Stage::Walking;
    }

    /// The AutoMove state's `UpdateState` (after the continuations).
    fn walk_step(&mut self, world: &mut World, dt: f32) {
        if self.walk.is_none() || dt <= 0.0 {
            return;
        }
        let Some(player) = super::player_entity(world) else {
            return;
        };
        let Some(position) = world.get::<Transform>(player).map(|t| t.translation) else {
            return;
        };
        let navigation = current_navigation(world);
        let speed = navigation.as_ref().map_or(0.0, |nav| nav.agent_speed);
        let walk = self.walk.as_mut().expect("checked above");
        while walk.cursor < walk.corners.len()
            && position.distance(walk.corners[walk.cursor]) <= door_law::AUTO_MOVE_STEER_EPSILON
        {
            walk.cursor += 1;
        }
        let steering = walk
            .corners
            .get(walk.cursor)
            .or(walk.corners.last())
            .copied()
            .unwrap_or(walk.target);
        let velocity = door_law::auto_move_velocity(position, steering, speed);
        let requested = door_law::auto_move_request(position, steering, velocity, dt);
        let accepted = navigation
            .as_ref()
            .and_then(|nav| nav.world.constrain_move(position, requested))
            .filter(|accepted| accepted.is_finite())
            .unwrap_or(requested);
        let actual = (accepted - position) / dt;
        let target = walk.target;
        let mut arrived = door_law::auto_move_arrived(accepted, target, actual);
        if accepted.distance(walk.last) < 1.0e-4 {
            walk.unchanged += dt;
        } else {
            walk.unchanged = 0.0;
        }
        walk.last = accepted;
        let mut placed = accepted;
        if !arrived && (walk.unchanged >= WALK_STALL || speed <= 0.0) {
            error!(
                "[site-move] AutoMove stalled {:.3} m from {target:.2} (agent speed {speed}); placed on the target (product guard)",
                accepted.distance(target)
            );
            placed = target;
            arrived = true;
        }
        if let Some(mut transform) = world.get_mut::<Transform>(player) {
            transform.translation = placed;
            if actual.x != 0.0 || actual.z != 0.0 {
                // The avatar faces its velocity, as the move state writes it.
                transform.rotation = Quat::from_rotation_y(actual.x.atan2(actual.z));
            }
        }
        if arrived {
            self.walk = None;
            {
                let mut states = world.resource_mut::<PlayerAvatarStates>();
                states.can_intercept = true;
                states.change_status(PlayerActionState::Idle);
            }
            info!(
                "[site-move] AutoMove arrived {:.3} m from {target:.2}: gate open, Idle",
                placed.distance(target)
            );
            self.token = door_state::play_idle_loop(world, self.token);
        }
    }

    // --- Arrival at the door --------------------------------------------

    fn arrival(&mut self, world: &mut World, frame: u64) {
        match self.kind {
            DoorKind::HomeToRoom => {
                // PlayEnterMyRoomAction(insideDoor.forward).
                if let Some(forward) = self
                    .house
                    .as_ref()
                    .and_then(|house| house.inside_door)
                    .and_then(|e| world.get::<GlobalTransform>(e))
                    .map(|g| g.rotation() * Vec3::Z)
                {
                    face(world, forward);
                }
                if let Some(house) = self.house.take() {
                    if let Err(reason) =
                        house_door::set_trigger(world, &house, HouseTrigger::PlayerOn)
                    {
                        error!(
                            "[site-move] HouseView.SetAnimationTrigger(PlayerOn) refused: {reason}"
                        );
                    }
                    self.house = Some(house);
                }
                self.token =
                    door_state::change_state(world, HouseState::EnterMoveHouse, self.token);
                self.idle_watch = Some((PlayerActionState::EnterMoveHouse, DoorStep::StateTimer));
                info!("[site-move] PlayEnterMyRoomAction: facing the inside door's forward, HouseView PlayerOn, ChangeState(EnterMoveHouse)");
                self.stage = Stage::EnterHouseWait(Delay::new(
                    door_law::wait(door_law::ENTER_HOUSE_WAIT),
                    frame,
                ));
            }
            DoorKind::RoomToHome | DoorKind::RoomToRoom => {
                if self.kind == DoorKind::RoomToHome {
                    // OpenDoorAsync().Forget() after the walk.
                    super::room_door::open(world);
                }
                // FieldCamera.ChangeState(None): Normal's exit.
                if world.resource::<FieldCameraState>().0 == CameraStateType::Normal {
                    self.normal_exit(world);
                }
                world.resource_mut::<FieldCameraState>().0 = CameraStateType::None;
                info!("[site-move] FieldCamera.ChangeState(None): the camera stays where it is; SetLookAtTarget(player) has no reader in the None state");
                if let Some(forward) = super::room_door::inside_point(world)
                    .and_then(|e| world.get::<GlobalTransform>(e))
                    .map(|g| g.rotation() * Vec3::Z)
                {
                    // LookRotation(-forward): out through the door.
                    face(world, -forward);
                }
                self.token =
                    door_state::change_state(world, HouseState::ExitMoveMyRoom, self.token);
                world.resource_mut::<PlayerAvatarStates>().can_intercept = false;
                self.idle_watch = Some((PlayerActionState::ExitMoveMyRoom, DoorStep::StateTimer));
                super::push_se(world, "se_door_open");
                info!("[site-move] facing out of the room, ChangeState(ExitMoveMyRoom), SetInterceptFlag(false), se_door_open");
                self.stage = Stage::ExitSeWait(Delay::new(
                    door_law::wait(door_law::EXIT_ROOM_SE_WAIT),
                    frame,
                ));
            }
        }
    }

    fn fade_out(&mut self, world: &mut World, note: String) {
        if self.no_wipe || !wipe::fade_out(world) {
            self.no_wipe = true;
            info!("[site-move] FadeOut without a transitioner: its callback completes at once");
        }
        self.fade_out_pending = true;
        self.record_step(DoorStep::FadeOut, note);
    }

    fn fade_in(&mut self, world: &mut World) {
        if self.no_wipe || !wipe::fade_in(world) {
            self.no_wipe = true;
            info!("[site-move] FadeIn without a transitioner: its callback completes at once");
        }
        self.fade_in_pending = true;
    }

    // --- ChangeSite ------------------------------------------------------

    fn change_site(&mut self, world: &mut World) {
        self.record_step(
            DoorStep::ChangeSite,
            format!("[SiteManager.ChangeSite({})]", self.to),
        );
        let Some(next) = self.next.take() else {
            return;
        };
        if let Some(house) = self.house.take() {
            house_door::release(world, house.root);
        }
        let roots = super::site_roots(world);
        {
            let mut commands = world.commands();
            crate::site::queue_transition(&mut commands, roots, next);
        }
        world.flush();
        info!(
            "[site-move] {} torn down under the closed wipe; {} built from its preload",
            self.from, self.to
        );
        self.wait_logged = false;
        self.stage_real = world.resource::<Time<Real>>().elapsed_secs_f64();
        self.stage = Stage::Loading;
    }

    fn destination_ready(&mut self, world: &mut World) -> bool {
        match destination_blocker(world, &self.to, self.kind) {
            None => {
                self.preload = None;
                true
            }
            Some(reason) => {
                self.waiting(world, "ChangeSite", &reason);
                false
            }
        }
    }

    // --- The destination stands -----------------------------------------

    fn site_ready(&mut self, world: &mut World, frame: u64) {
        match self.kind {
            DoorKind::HomeToRoom => {
                info!("[site-move] GetSite, SiteManager.SetupRoom (room and door set up, ShowDoor); SetPlayerLayer: no product counterpart; MoveMyRoom: MysekaiTransitioner.SafeFinish: no instance in solo play; OpenDoorAsync().Forget()");
                super::room_door::open(world);
                self.stage = Stage::EnterRoomWait(Delay::new(
                    door_law::wait(door_law::ENTER_ROOM_DOOR_WAIT),
                    frame,
                ));
            }
            DoorKind::RoomToRoom => {
                info!("[site-move] GetSite, SiteManager.SetupRoom (room and door set up, ShowDoor); OpenDoorAsync().Forget()");
                super::room_door::open(world);
                world.resource_mut::<PlayerAvatarStates>().can_intercept = true;
                self.place_in_room(world);
                self.token =
                    door_state::change_state(world, HouseState::EnterMoveMyRoom, self.token);
                self.idle_watch =
                    Some((PlayerActionState::EnterMoveMyRoom, DoorStep::EnterRoomIdle));
                set_player_visible(world, true);
                world.resource_mut::<PlayerAvatarStates>().can_intercept = false;
                // Both floors share one environment site: no CrossFade.
                world.remove_resource::<super::EnvironmentHold>();
                world.remove_resource::<super::BgmHold>();
                info!("[site-move] ChangeState(EnterMoveMyRoom), player.Show, ChangeSubState(35) (solo no-op); PlayBGMAsync released; no ChangeEnvironment (the rebuilt environment is taken at once)");
                self.fade_in(world);
                self.house_entry_camera(world);
                self.record_step(
                    DoorStep::EnterRoom,
                    "[place, EnterMoveMyRoom, FadeIn, camera HouseEntry]".into(),
                );
                self.stage = Stage::AfterFadeInWait(Delay::new(
                    door_law::wait(door_law::AFTER_FADE_IN_WAIT),
                    frame,
                ));
            }
            DoorKind::RoomToHome => {
                // Home.OnEnterSite: GameState Normal, camera Normal from None.
                info!("[site-move] Home.OnEnterSite: GameState Normal (the site-move input lock holds until the executor's Normal here)");
                self.transfer_from_none(world);
                let house = match house_door::find_house(world) {
                    HouseLookup::Found(house) => Some(house),
                    HouseLookup::Absent => {
                        error!("[site-move] MyRoomToHome core: GetHouseView: no placed fixture carries a HouseView; the exit plays without the house");
                        None
                    }
                    HouseLookup::Failed(reason) | HouseLookup::Pending(reason) => {
                        error!("[site-move] MyRoomToHome core: GetHouseView failed ({reason}); the exit plays without the house");
                        None
                    }
                };
                set_player_visible(world, false);
                self.fade_in(world);
                self.house = house;
                if world.contains_resource::<crate::npc::RandomFixtureActionReader>() {
                    let requested_frame = world.resource::<bevy::diagnostic::FrameCount>().0;
                    world.insert_resource(crate::npc::RandomFixtureActionPending { requested_frame });
                    info!("[site-move] player.Hide, ShowFadeInAnimation; await HomeSiteController.ExecuteNPCRandomFixtureAction (RandomFixtureActionPending inserted on frame {requested_frame})");
                    self.wait_logged = false;
                    self.stage_real = world.resource::<Time<Real>>().elapsed_secs_f64();
                    self.stage = Stage::NpcFixtureActionWait;
                } else {
                    warn!("[site-move] player.Hide, ShowFadeInAnimation; HomeSiteController.ExecuteNPCRandomFixtureAction: no reader of npc::RandomFixtureActionPending is installed (npc::RandomFixtureActionReader is missing); the await returns at once");
                    self.move_to_entrance(world, frame);
                }
            }
        }
    }

    /// Room to home, after the await on
    /// `HomeSiteController.ExecuteNPCRandomFixtureAction`: `MoveToEntrance`,
    /// the HouseEntry camera, then `StartMysekaiTransition`.
    fn move_to_entrance(&mut self, world: &mut World, frame: u64) {
        if let Some(outside) = self
            .house
            .as_ref()
            .and_then(|house| house.outside_door)
            .and_then(|e| world.get::<GlobalTransform>(e).copied())
        {
            // MoveToEntrance: ForceSetPosition, agent off, LookRotation(forward).
            place(world, outside.translation(), outside.rotation() * Vec3::Z);
            info!(
                "[site-move] MoveToEntrance: player at the outside door {:.2}",
                outside.translation()
            );
        }
        self.house_entry_camera(world);
        world.remove_resource::<super::BgmHold>();
        info!("[site-move] PlayBGMAsync released; StartMysekaiTransition: MysekaiTransitioner.SafeFinish (no instance in solo play), LiveTransitioner.SafeFinish (no cover up after the entry)");
        self.stage = Stage::StartTransitionWait(Delay::new(
            door_law::wait(door_law::START_TRANSITION_WAIT),
            frame,
        ));
    }

    /// Home to room: `MoveMyRoom` after the door wait.
    fn enter_room(&mut self, world: &mut World, frame: u64, note: String) {
        self.place_in_room(world);
        self.token = door_state::change_state(world, HouseState::EnterMoveMyRoom, self.token);
        self.idle_watch = Some((PlayerActionState::EnterMoveMyRoom, DoorStep::EnterRoomIdle));
        world.resource_mut::<PlayerAvatarStates>().can_intercept = false;
        world.remove_resource::<super::EnvironmentHold>();
        world.remove_resource::<super::BgmHold>();
        info!("[site-move] ChangeState(EnterMoveMyRoom), ChangeSubState(35) (solo no-op); ChangeEnvironment: CrossFade with FadeAnimationDuration 0 released; PlayBGMAsync released");
        set_player_visible(world, true);
        self.house_entry_camera(world);
        info!(
            "[site-move] player.Show; DelayCall({}, CullingWallFixture): no product counterpart",
            door_law::CULLING_WALL_DELAY
        );
        self.fade_in(world);
        self.record_step(DoorStep::EnterRoom, note);
        self.stage = Stage::AfterFadeInWait(Delay::new(
            door_law::wait(door_law::AFTER_FADE_IN_WAIT),
            frame,
        ));
    }

    /// Room to home, after `StartMysekaiTransition`.
    fn exit_house(&mut self, world: &mut World, frame: u64, note: String) {
        world.remove_resource::<super::EnvironmentHold>();
        self.record_step(
            DoorStep::ChangeEnvironment,
            format!("{note} [CrossFade with FadeAnimationDuration 0 released]"),
        );
        // PlayExitMyRoomAction.
        world.resource_mut::<PlayerAvatarStates>().can_intercept = true;
        if let Some(house) = &self.house {
            if let Err(reason) = house_door::set_trigger(world, house, HouseTrigger::PlayerOff) {
                error!("[site-move] HouseView.SetAnimationTrigger(PlayerOff) refused: {reason}");
            }
        }
        set_player_visible(world, true);
        self.token = door_state::change_state(world, HouseState::ExitMoveHouse, self.token);
        world.resource_mut::<PlayerAvatarStates>().can_intercept = false;
        if let Some(house) = &self.house {
            house_door::add_ignore_se(world, house, "se_door_open");
        }
        self.record_step(
            DoorStep::ExitHouse,
            "[PlayerOff, player.Show, ChangeState(ExitMoveHouse), ChangeSubState(35), gate closed, AddIgnoreSe(se_door_open)]".into(),
        );
        self.stage =
            Stage::ExitHouseWait(Delay::new(door_law::wait(door_law::EXIT_HOUSE_WAIT), frame));
    }

    /// Room to home: `ClearIgnoreSe`, camera Normal, `Finish`, end action.
    fn finish_room_to_home(&mut self, world: &mut World, note: String) {
        if let Some(house) = self.house.take() {
            house_door::clear_ignore_se(world, &house);
        }
        self.normal_from_house_entry(world);
        self.record_step(DoorStep::CameraNormal, note);
        // Finish: agent on, gate open, idle 0.1 s on the view, ChangeState(Idle).
        world.resource_mut::<PlayerAvatarStates>().can_intercept = true;
        door_state::finish_idle(world, self.token.take(), door_law::FINISH_IDLE_FADE);
        world
            .resource_mut::<PlayerAvatarStates>()
            .change_status(PlayerActionState::Idle);
        self.idle_watch = None;
        info!("[site-move] Finish: NavMeshAgent on, gate open, PlayAnimation(c_000_mov_idle_00, 0.1), ChangeState(Idle)");
        self.record_step(
            DoorStep::EndAction,
            "[OnSiteMoveEndAction: SetupLockAtCameraBounds ran when home settled]".into(),
        );
        self.stage = Stage::Done;
    }

    /// Home to room and room to room, 1.0 s after FadeIn.
    fn finish_into_room(&mut self, world: &mut World, note: String) {
        match self.kind {
            DoorKind::HomeToRoom => {
                world.resource_mut::<PlayerAvatarStates>().can_intercept = true;
                super::room_door::close(world);
                self.record_step(DoorStep::CloseDoor, note);
                // OnSiteMoveEndAction: camera Normal, HideSite(home), PrepareForNextSite.
                self.normal_from_house_entry(world);
                self.record_step(
                    DoorStep::EndAction,
                    "[camera Normal; HideSite(home): home was torn down at ChangeSite here; PrepareForNextSite]".into(),
                );
            }
            _ => {
                self.normal_from_house_entry(world);
                world.resource_mut::<PlayerAvatarStates>().can_intercept = true;
                super::room_door::close(world);
                self.record_step(DoorStep::CloseDoor, note);
                self.record_step(DoorStep::EndAction, "[PrepareForNextSite]".into());
            }
        }
        // The state timer already changed to Idle, whose clip is playing;
        // the move's end plays nothing more.
        door_state::hand_back(world, self.token.take());
        self.stage = Stage::Done;
    }

    /// `GameState.Normal` and the destination's `OnFinishEnterAsync`.
    pub(crate) fn normal(&mut self, world: &mut World) {
        self.record_step(
            DoorStep::Normal,
            "[GameState Normal: FieldCamera.ChangeState(Normal) returns at once]".into(),
        );
        world.remove_resource::<super::SiteMoveActive>();
        world.write_message(LayerCommand::Change(LayerId::HomeField));
        match self.kind {
            DoorKind::RoomToHome => {
                info!(
                    "[site-move] HomeSiteController.OnFinishEnterAsync: SetupScreenLayerMysekaiHome"
                );
                world.write_message(crate::cutscene::HomeScreenStartAnimation {
                    caller: "a door arrival at home",
                });
            }
            _ => {
                info!("[site-move] MyRoomSiteController.OnFinishEnterAsync: the field screen; PlayRoomSiteExpansionPerformAsync");
                crate::site_expansion::room_finish_enter(world);
            }
        }
    }

    /// The step table, once, when the move ends. `chained`: the executor
    /// admits the nested move next, still in `GameState.SiteMove`.
    pub(crate) fn finish(mut self, world: &mut World, chained: bool) {
        if world
            .remove_resource::<crate::npc::RandomFixtureActionPending>()
            .is_some()
        {
            warn!(
                "[site-move] {} {} -> {} ended with npc::RandomFixtureActionPending still present: the NPC side never answered the await; removed",
                self.kind.name(),
                self.from,
                self.to
            );
        }
        if !chained {
            world.remove_resource::<super::SiteMoveActive>();
            world.remove_resource::<super::EnvironmentHold>();
            world.remove_resource::<super::BgmHold>();
        }
        wipe::disarm(world);
        if let Some(house) = self.house.take() {
            house_door::clear_ignore_se(world, &house);
        }
        if self.token.is_some() {
            door_state::finish_idle(world, self.token.take(), door_law::STATE_CLIP_FADE);
        }
        let rows: Vec<String> = self
            .records
            .iter()
            .map(|r| match (r.anchor, r.source) {
                (Some(anchor), Some(source)) => format!(
                    "{}|{}|{:.4}|{:.4}|{:+.4}|{}|{}",
                    r.step,
                    anchor.name(),
                    source,
                    r.measured,
                    r.measured - source,
                    r.frames,
                    r.note
                ),
                _ => format!("{}|-|-|{:.4}|-|{}|{}", r.step, r.measured, r.frames, r.note),
            })
            .collect();
        info!(
            "[site-move] {} {} -> {} done; step table (step|anchor|source|measured|delta|frames|note): {}",
            self.kind.name(),
            self.from,
            self.to,
            rows.join(" ; ")
        );
    }

    fn abort(&mut self, world: &mut World, reason: &str) {
        error!(
            "[site-move] {} {} -> {}: {reason}; the move switches without the door action",
            self.kind.name(),
            self.from,
            self.to
        );
        self.walk = None;
        self.idle_watch = None;
        {
            let mut states = world.resource_mut::<PlayerAvatarStates>();
            states.can_intercept = true;
            states.change_status(PlayerActionState::Idle);
        }
        if let Some(house) = self.house.take() {
            house_door::release(world, house.root);
        }
        if let Some(next) = self.next.take() {
            let roots = super::site_roots(world);
            let mut commands = world.commands();
            crate::site::queue_transition(&mut commands, roots, next);
            world.flush();
        }
        set_player_visible(world, true);
        world.resource_mut::<FieldCameraState>().0 = CameraStateType::Normal;
        self.stage = Stage::Done;
    }

    // --- Camera -----------------------------------------------------------

    /// `NormalCameraState.OnExit`.
    fn normal_exit(&mut self, world: &mut World) {
        let site = world
            .get_resource::<crate::site::SiteActive>()
            .map(|site| site.site_type.clone())
            .unwrap_or_else(|| self.from.clone());
        let flag = !world.contains_resource::<CameraTween>();
        let Some(model) = world.get_resource::<FieldCameraModel>().cloned() else {
            return;
        };
        let fov = world
            .get_resource::<CameraSetting>()
            .map_or(model.fov, |s| s.fov);
        if flag {
            self.private = Some(PrivateModel {
                look_at: model.look_at,
                yaw: model.yaw,
                pitch: model.pitch,
                distance: model.distance,
                fov,
            });
        }
        super::camera::record_normal_exit(world, &site);
        info!(
            "[site-move] NormalCameraState.OnExit at {site}: private model {}; transfer data for {site} written",
            if flag {
                "written"
            } else {
                "kept (its transfer tween is in flight)"
            }
        );
    }

    /// `FieldCamera.ChangeState(HouseEntry)` on the player as placed.
    fn house_entry_camera(&mut self, world: &mut World) {
        if world.resource::<FieldCameraState>().0 == CameraStateType::Normal {
            self.normal_exit(world);
        }
        let pose =
            super::player_entity(world).and_then(|player| world.get::<Transform>(player).copied());
        let setting = world.get_resource::<CameraSetting>().copied();
        let (Some(pose), Some(setting)) = (pose, setting) else {
            warn!(
                "[site-move] camera HouseEntry without its tween: no player pose or camera setting"
            );
            world.resource_mut::<FieldCameraState>().0 = CameraStateType::HouseEntry;
            return;
        };
        let fov = camera_fov(world);
        let tween = world
            .get_resource_mut::<FieldCameraModel>()
            .map(|mut model| {
                entry_law::enter_house_entry(
                    &mut model,
                    &setting,
                    pose.translation,
                    pose.rotation,
                    fov,
                )
            });
        if let Some(tween) = tween {
            info!(
                "[site-move] FieldCamera.ChangeState(HouseEntry): tween {:.2}s pitch {:.1}->{:.1} yaw {:.1}->{:.1} distance {:.2}->{:.2}",
                tween.duration,
                tween.pitch.0,
                tween.pitch.1,
                tween.yaw.0,
                tween.yaw.1,
                tween.distance.0,
                tween.distance.1
            );
            world.insert_resource(tween);
        }
        world.resource_mut::<FieldCameraState>().0 = CameraStateType::HouseEntry;
    }

    /// `ChangeState(Normal)` from HouseEntry: `TransferCameraSettings` case 8.
    fn normal_from_house_entry(&mut self, world: &mut World) {
        if world.resource::<FieldCameraState>().0 != CameraStateType::HouseEntry {
            return;
        }
        let Some(setting) = world.get_resource::<CameraSetting>().copied() else {
            world.resource_mut::<FieldCameraState>().0 = CameraStateType::Normal;
            return;
        };
        // A private model never written keeps the constructor's copy of the
        // camera setting.
        let private = self.private.unwrap_or(PrivateModel {
            look_at: Vec3::ZERO,
            yaw: setting.init_yaw,
            pitch: setting.init_pitch,
            distance: setting.distance,
            fov: setting.fov,
        });
        let fov = camera_fov(world);
        let tween = world
            .get_resource_mut::<FieldCameraModel>()
            .map(|mut model| {
                entry_law::transfer_from_house_entry(
                    &mut model,
                    &setting,
                    NormalPrivate {
                        pitch: private.pitch,
                        distance: private.distance,
                        fov: private.fov,
                    },
                    fov,
                )
            });
        if let Some(tween) = tween {
            info!(
                "[site-move] FieldCamera.ChangeState(Normal): TransferCameraSettings from HouseEntry {:.2}s pitch {:.1}->{:.1} distance {:.2}->{:.2}",
                tween.duration, tween.pitch.0, tween.pitch.1, tween.distance.0, tween.distance.1
            );
            world.insert_resource(tween);
        }
        world.resource_mut::<FieldCameraState>().0 = CameraStateType::Normal;
    }

    /// `ChangeState(Normal)` from None: `TransferCameraSettings` has no case
    /// for it, so one 1.0 s tween goes to the whole private model.
    fn transfer_from_none(&mut self, world: &mut World) {
        let setting = world.get_resource::<CameraSetting>().copied();
        let fov = camera_fov(world);
        let private = self.private;
        let tween = match (setting, private) {
            (Some(setting), Some(private)) => {
                world
                    .get_resource_mut::<FieldCameraModel>()
                    .map(|mut model| {
                        model.min_distance = setting.min_distance;
                        model.max_distance = setting.max_distance;
                        model.min_pitch = setting.min_pitch;
                        model.max_pitch = setting.max_pitch;
                        model.offset = setting.offset;
                        entry_law::camera_setting_tween(
                            &model,
                            fov,
                            private.look_at,
                            private.pitch,
                            private.yaw,
                            private.fov,
                            private.distance,
                            entry_law::NORMAL_TRANSFER_TWEEN_SECONDS,
                        )
                    })
            }
            _ => None,
        };
        match tween {
            Some(tween) => {
                info!(
                    "[site-move] FieldCamera.ChangeState(Normal) from None: {:.2}s tween to the private model (the room's view)",
                    tween.duration
                );
                world.insert_resource(tween);
            }
            None => warn!("[site-move] camera Normal from None without a private model or camera setting: no tween"),
        }
        world.resource_mut::<FieldCameraState>().0 = CameraStateType::Normal;
    }

    fn place_in_room(&mut self, world: &mut World) {
        let Some(inside) = super::room_door::inside_point(world)
            .and_then(|e| world.get::<GlobalTransform>(e).copied())
        else {
            error!("[site-move] the room has no door action point: the player is not placed");
            return;
        };
        place(world, inside.translation(), inside.rotation() * Vec3::Z);
        info!(
            "[site-move] ForceSetPosition(door action point {:.2}), LookRotation(forward)",
            inside.translation()
        );
    }
}

fn camera_fov(world: &mut World) -> f32 {
    let mut cameras = world.query_filtered::<&Projection, With<Camera3d>>();
    match cameras.single(world) {
        Ok(projection) => crate::camera::perspective_fov_deg(projection),
        Err(_) => world
            .get_resource::<CameraSetting>()
            .map_or(0.0, |setting| setting.fov),
    }
}

fn set_player_visible(world: &mut World, visible: bool) {
    let mut players = world.query_filtered::<&mut Visibility, With<PlayerControlled>>();
    for mut visibility in players.iter_mut(world) {
        *visibility = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// `Quaternion.LookRotation(direction)` about the up axis (the avatar faces
/// its local +Z).
fn look_rotation(direction: Vec3) -> Option<Quat> {
    let forward = direction.normalize_or_zero();
    let right = Vec3::Y.cross(forward).normalize_or_zero();
    if forward == Vec3::ZERO || right == Vec3::ZERO {
        return None;
    }
    Some(Quat::from_mat3(&Mat3::from_cols(
        right,
        forward.cross(right),
        forward,
    )))
}

fn face(world: &mut World, direction: Vec3) {
    let Some(rotation) = look_rotation(direction) else {
        return;
    };
    if let Some(player) = super::player_entity(world) {
        if let Some(mut transform) = world.get_mut::<Transform>(player) {
            transform.rotation = rotation;
        }
    }
}

/// `ForceSetPosition(position)` and `LookRotation(forward)`.
fn place(world: &mut World, position: Vec3, forward: Vec3) {
    let rotation = look_rotation(forward);
    if let Some(player) = super::player_entity(world) {
        if let Some(mut transform) = world.get_mut::<Transform>(player) {
            transform.translation = position;
            if let Some(rotation) = rotation {
                transform.rotation = rotation;
            }
        }
    }
}

const NAVIGATION_PENDING: &str = "the player navigation for this site is binding";

/// `NavMeshAgent.SetDestination(target)`: the agent's path query maps both
/// ends onto the walk field within the agent's query extents, so a target
/// just off the field (the house's inside-door point sits on the edge of the
/// house footprint) ends at its projection; the arrival test still reads the
/// target itself. Corner heights are the navigation surface's.
fn agent_path(world: &World, from: Vec3, target: Vec3) -> Result<Vec<Vec3>, String> {
    let field = world
        .get_resource::<crate::walk_face::WalkFace>()
        .map(|walk| walk.field.clone())
        .ok_or("no walk field for this site")?;
    let path = field
        .calculate_path(
            [from.x, from.z],
            [target.x, target.z],
            moly_law::carve::AGENT_QUERY_HALF_EXTENT,
        )
        .ok_or("the agent path query found no walk cell within its extents")?;
    let surface = world.get_resource::<crate::npc_objective::ObjectiveFace>();
    let corners: Vec<Vec3> = path
        .corners
        .iter()
        .map(|c| {
            let y = surface
                .and_then(|face| face.navigation_point_at(*c))
                .map_or(target.y, |point| point[1]);
            Vec3::new(c[0], y, c[1])
        })
        .collect();
    let Some(last) = corners.last() else {
        return Err("the agent path has no corner".into());
    };
    let projected = Vec2::new(last.x - target.x, last.z - target.z).length();
    info!(
        "[site-move] AutoMove path: {} corners, {}, last corner {projected:.3} m from the target",
        corners.len(),
        if path.complete { "complete" } else { "partial" }
    );
    Ok(corners)
}

/// The installed navigation, when it belongs to the current site.
fn current_navigation(world: &World) -> Option<PlayerFixtureNavigation> {
    let epoch = world.get_resource::<crate::site::GroundEpoch>()?.0;
    world
        .get_resource::<PlayerFixtureNavigation>()
        .filter(|navigation| navigation.site_generation == epoch)
        .cloned()
}

fn navigation_ready(world: &World) -> Result<(), String> {
    current_navigation(world)
        .map(|_| ())
        .ok_or_else(|| NAVIGATION_PENDING.to_owned())
}

/// What the destination still lacks before `ChangeSite` returns.
pub(crate) fn destination_blocker(world: &mut World, to: &str, kind: DoorKind) -> Option<String> {
    let active = world
        .get_resource::<crate::site::SiteActive>()
        .map(|site| site.site_type.clone());
    if active.as_deref() != Some(to) {
        return Some("site not active".into());
    }
    if !world.contains_resource::<crate::site::SiteScenesReady>() {
        return Some("scenes expanding".into());
    }
    if !world.contains_resource::<crate::site_material::SiteMaterialsSwapped>() {
        return Some("materials swapping".into());
    }
    if !world.contains_resource::<crate::fixture::FixtureScenesReady>()
        || !crate::fixture::placements_resolved(world)
    {
        return Some("fixture layout binding".into());
    }
    let epoch = world
        .get_resource::<crate::site::GroundEpoch>()
        .map_or(0, |epoch| epoch.0);
    match kind {
        DoorKind::RoomToHome => {
            if let HouseLookup::Pending(reason) = house_door::find_house(world) {
                return Some(reason);
            }
        }
        DoorKind::HomeToRoom | DoorKind::RoomToRoom => {
            if !world
                .get_resource::<crate::room_appearance::RoomAppearanceState>()
                .is_some_and(|state| state.settled_for(epoch))
            {
                return Some("room appearance loading".into());
            }
            if !super::room_door::settled(world) {
                return Some("room door binding".into());
            }
        }
    }
    None
}

/// `OnChangeSite` for a door state: `GameState.SiteMove`, the holds, the
/// site-move screen and the destination's preload.
pub(crate) fn admit(
    world: &mut World,
    kind: DoorKind,
    from: String,
    next: SiteSelection,
    then: Option<SiteSelection>,
    composition: Option<&'static str>,
    frame: u64,
) {
    let server = world.resource::<AssetServer>().clone();
    let preload = match world.get_resource::<crate::site::Sites>() {
        Some(sites) => SitePreload::request(&server, sites, &next),
        None => Err("site table missing".to_owned()),
    };
    let preload = match preload {
        Ok(preload) => Some(preload),
        Err(error) => {
            warn!("[site-move] destination preload refused ({error}); ChangeSite loads it");
            None
        }
    };
    world.insert_resource(super::SiteMoveActive);
    world.insert_resource(super::EnvironmentHold);
    world.insert_resource(super::BgmHold);
    if world.resource::<UiLayerStack>().current() == LayerId::MysekaiSiteMap {
        world.write_message(LayerCommand::Pop);
    }
    world.write_message(LayerCommand::Change(LayerId::MysekaiSiteMove));
    let real = world.resource::<Time<Real>>().elapsed_secs_f64();
    info!(
        "[site-move] admitted {from} -> {} ({}): GameState SiteMove, site-move screen, destination preload",
        next.site_type(),
        kind.name()
    );
    world.insert_resource(DoorMove::new(
        kind,
        from,
        next,
        then,
        composition,
        preload,
        frame,
        real,
    ));
    wipe::arm(world);
}

/// Update, after the player states: see [`DoorMove::after_states`].
pub(crate) fn after_states(world: &mut World) {
    world.resource_scope(|world, mut session: Mut<DoorMove>| session.after_states(world));
}

/// `HouseView.InsideDoorActionPoint` of the placed house, on its fixture
/// root: the house-entry button's availability reads it.
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct HouseEntryPoint(pub(crate) Entity);

/// The site generation the house-entry point was looked up for.
#[derive(Resource)]
pub(crate) struct HouseEntryTagged(u64);

/// Update: tag the placed house with its inside-door point once per site.
pub(crate) fn tag_house_entry(world: &mut World) {
    let Some(epoch) = world
        .get_resource::<crate::site::GroundEpoch>()
        .map(|epoch| epoch.0)
    else {
        return;
    };
    if world
        .get_resource::<HouseEntryTagged>()
        .is_some_and(|tagged| tagged.0 == epoch)
    {
        return;
    }
    let home = world
        .get_resource::<crate::site::SiteActive>()
        .is_some_and(|site| site.site_type == "home_site");
    if !home || !world.contains_resource::<crate::fixture::FixtureScenesReady>() {
        return;
    }
    match house_door::find_house(world) {
        HouseLookup::Pending(_) => {}
        HouseLookup::Found(house) => {
            world.insert_resource(HouseEntryTagged(epoch));
            match house.inside_door {
                Some(point) => {
                    world.entity_mut(house.root).insert(HouseEntryPoint(point));
                    // CanMoveDoorActionPoint measured from the outside door,
                    // the entry's standing point in front of the house.
                    let at = |e: Option<Entity>| {
                        e.and_then(|e| world.get::<GlobalTransform>(e))
                            .map(GlobalTransform::translation)
                    };
                    let reach = match (
                        at(house.outside_door),
                        at(Some(point)),
                        world.get_resource::<crate::walk_face::WalkFace>(),
                    ) {
                        (Some(outside), Some(inside), Some(walk)) => {
                            let last = walk
                                .field
                                .calculate_path(
                                    [outside.x, outside.z],
                                    [inside.x, inside.z],
                                    moly_law::carve::STATIC_QUERY_HALF_EXTENT,
                                )
                                .and_then(|path| path.corners.last().copied())
                                .map(|c| Vec2::new(c[0] - inside.x, c[1] - inside.z).length());
                            format!(
                                "from the outside door the static path ends {last:.3?} m from it (reach {})",
                                door_law::HOUSE_ENTRY_REACH
                            )
                        }
                        _ => "not measured (door points or walk field missing)".to_owned(),
                    };
                    info!(
                        "[site-move] house {}: InsideDoorActionPoint {point:?} for the house-entry button; {reach}",
                        house.uid
                    );
                }
                None => warn!(
                    "[site-move] house {}: HouseView.InsideDoorActionPoint is null; no house-entry button",
                    house.uid
                ),
            }
        }
        HouseLookup::Absent => {
            world.insert_resource(HouseEntryTagged(epoch));
            info!("[site-move] home has no placed HouseView: no house-entry button");
        }
        HouseLookup::Failed(reason) => {
            world.insert_resource(HouseEntryTagged(epoch));
            warn!("[site-move] house-entry point not found: {reason}");
        }
    }
}
