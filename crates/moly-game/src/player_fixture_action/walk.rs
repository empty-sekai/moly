//! Instrument (off by default): walk the player to a furniture seat, tap its
//! timeline button, let the timeline play, then end it with the joystick.
//!
//! `MOLY_FIXTURE_WALK_SECS` arms it until that many seconds of app time,
//! from `MOLY_FIXTURE_WALK_AFTER` seconds. The target is the placed fixture
//! whose id is `MOLY_FIXTURE_WALK_ID`, or else the nearest placed fixture
//! with a player timeline row. The walk goes through the joystick's touch
//! stream, as the action button's own walks do; once the button stack head
//! is a timeline fixture button, the tap goes to the window's event stream at
//! `MOLY_FIXTURE_WALK_TAP=x,y` (the button's window position), through the
//! gesture and click path. `MOLY_FIXTURE_WALK_HOLD` seconds (default 8) after
//! the session starts, a short joystick push asks the session to end
//! (`request_end_from_input`). `MOLY_FIXTURE_WALK_VIA=x,z;x,z` first walks
//! through those points (to go round other furniture). One walk per site
//! generation.

use bevy::{
    input::touch::{TouchInput, TouchPhase},
    prelude::*,
    window::{PrimaryWindow, WindowEvent},
};
use moly_law::action_button::ButtonType;

use super::PlayerFixtureRuntime;
use crate::{
    action_button::ActionButtonState,
    canvas::RootCanvas,
    fixture::{FixturePlacement, FixtureRoot},
    fixture_activity_data::FixtureActivityTables,
    joystick::{JoystickState, HANDLE_SIZE},
    player::PlayerControlled,
};

const FINGER: u64 = 99021;
const TAP_FINGER: u64 = 99022;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Stage {
    #[default]
    Walking,
    /// The button was tapped at this time.
    Tapped(f32),
    /// The session started at this time.
    Playing(f32),
    /// The end push began at this time.
    Ending(f32),
    Done,
}

#[derive(Default)]
pub(crate) struct FixtureWalk {
    epoch: Option<u64>,
    stage: Stage,
    target: Option<(i32, Vec3)>,
    /// Waypoints still ahead of the target.
    via: Vec<Vec3>,
    since: f32,
    pressing: bool,
    /// The tap finger went down at this time and is still down.
    tap_down: Option<f32>,
    /// The placed fixtures of this generation were logged.
    listed: bool,
    last_log: f32,
}

fn env_secs(name: &str) -> Option<f32> {
    std::env::var(name).ok()?.trim().parse::<f32>().ok()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn smoke_fixture_walk(
    mut touches: MessageWriter<TouchInput>,
    mut window_events: MessageWriter<WindowEvent>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    root: Option<Res<RootCanvas>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    players: Query<&Transform, With<PlayerControlled>>,
    fixtures: Query<(&FixturePlacement, &GlobalTransform), With<FixtureRoot>>,
    tables: Option<Res<FixtureActivityTables>>,
    joystick: Res<JoystickState>,
    button_state: Res<ActionButtonState>,
    runtime: Res<PlayerFixtureRuntime>,
    epoch: Option<Res<crate::site::GroundEpoch>>,
    time: Res<Time>,
    mut walk: Local<FixtureWalk>,
) {
    let Some(armed) = env_secs("MOLY_FIXTURE_WALK_SECS").filter(|secs| *secs > 0.0) else {
        return;
    };
    let Some(tap) = tap_position() else {
        if walk.stage != Stage::Done {
            warn!("[fixture-walk] MOLY_FIXTURE_WALK_TAP=x,y (the button's window position) is not set");
            walk.stage = Stage::Done;
        }
        return;
    };
    let now = time.elapsed_secs();
    let Ok((window_entity, window)) = windows.single() else {
        return;
    };
    let base = Vec2::new(window.width() * 0.15, window.height() * 0.75);
    let mut touch = |id: u64, phase: TouchPhase, position: Vec2| {
        let input = TouchInput {
            phase,
            position,
            window: window_entity,
            force: None,
            id,
        };
        if id == TAP_FINGER {
            window_events.write(WindowEvent::TouchInput(input));
        } else {
            touches.write(input);
        }
    };
    let walk = &mut *walk;
    if let Some(down) = walk.tap_down {
        if now - down >= 0.1 {
            touch(TAP_FINGER, TouchPhase::Ended, tap);
            walk.tap_down = None;
        }
    }
    let release = |walk: &mut FixtureWalk, touch: &mut dyn FnMut(u64, TouchPhase, Vec2)| {
        if walk.pressing {
            touch(FINGER, TouchPhase::Ended, base);
            walk.pressing = false;
        }
    };
    if now >= armed || now < env_secs("MOLY_FIXTURE_WALK_AFTER").unwrap_or(0.0) {
        release(walk, &mut touch);
        return;
    }
    let Some(epoch) = epoch.map(|epoch| epoch.0) else {
        return;
    };
    if walk.epoch != Some(epoch) {
        release(walk, &mut touch);
        *walk = FixtureWalk {
            epoch: Some(epoch),
            since: now,
            tap_down: walk.tap_down,
            ..Default::default()
        };
    }
    let (Ok(player), Some(tables)) = (players.single(), tables) else {
        return;
    };
    let head = button_state.current();
    let session = runtime.session.is_some();
    if walk.stage != Stage::Done && now - walk.last_log >= 3.0 {
        walk.last_log = now;
        info!(
            "[fixture-walk] stage {:?} player ({:.2},{:.2},{:.2}) head {head:?} session {session} joystick enabled {}",
            walk.stage, player.translation.x, player.translation.y, player.translation.z, joystick.enabled
        );
    }
    match walk.stage {
        Stage::Done => {}
        Stage::Tapped(at) => {
            if session {
                walk.stage = Stage::Playing(now);
                info!("[fixture-walk] the tap started a player fixture session");
            } else if now - at > 10.0 {
                warn!("[fixture-walk] no session 10 s after the tap; head {head:?}");
                walk.stage = Stage::Done;
            }
        }
        Stage::Playing(since) => {
            if !session {
                info!("[fixture-walk] the session ended before the end push; head {head:?}");
                walk.stage = Stage::Done;
            } else if now - since >= env_secs("MOLY_FIXTURE_WALK_HOLD").unwrap_or(8.0) {
                if joystick.enabled {
                    touch(FINGER, TouchPhase::Started, base);
                    walk.pressing = true;
                    info!("[fixture-walk] joystick push to end the session");
                } else {
                    touch(TAP_FINGER, TouchPhase::Started, tap);
                    walk.tap_down = Some(now);
                    info!("[fixture-walk] joystick disabled; tapping to end the session");
                }
                walk.stage = Stage::Ending(now);
            }
        }
        Stage::Ending(since) => {
            let elapsed = now - since;
            if walk.pressing && elapsed < 0.3 {
                touch(FINGER, TouchPhase::Moved, base + Vec2::new(0.0, -30.0));
            } else if walk.pressing {
                release(walk, &mut touch);
            }
            if !session {
                info!("[fixture-walk] session finished {elapsed:.2} s after the end request; head {head:?}");
                walk.stage = Stage::Done;
            } else if elapsed > 30.0 {
                warn!("[fixture-walk] session still running 30 s after the end request");
                walk.stage = Stage::Done;
            }
        }
        Stage::Walking => {
            if walk.target.is_none() {
                if fixtures.is_empty() {
                    return;
                }
                let mut placed: Vec<_> = fixtures
                    .iter()
                    .map(|(placement, global)| {
                        let at = global.translation();
                        let rows = tables.player_timelines(placement.fixture_id).count();
                        format!(
                            "{}@({:.2},{:.2}) rows {rows}",
                            placement.fixture_id, at.x, at.z
                        )
                    })
                    .collect();
                placed.sort();
                if !walk.listed {
                    walk.listed = true;
                    info!("[fixture-walk] placed fixtures: {}", placed.join("; "));
                }
                walk.target = pick_target(&fixtures, &tables, player.translation);
                walk.via = std::env::var("MOLY_FIXTURE_WALK_VIA")
                    .ok()
                    .map(|raw| {
                        raw.split(';')
                            .filter_map(|point| {
                                let (x, z) = point.split_once(',')?;
                                Some(Vec3::new(
                                    x.trim().parse().ok()?,
                                    0.0,
                                    z.trim().parse().ok()?,
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if walk.target.is_none() && now - walk.since < 20.0 {
                    return;
                }
                if let Some((id, at)) = walk.target {
                    info!(
                        "[fixture-walk] site generation {epoch}: walking to fixture {id} at ({:.2},{:.2},{:.2}); its player rows {:?}",
                        at.x,
                        at.y,
                        at.z,
                        tables.player_timelines(id).map(|row| row.id).collect::<Vec<_>>()
                    );
                } else {
                    warn!("[fixture-walk] site generation {epoch}: no placed fixture with a player timeline row");
                    walk.stage = Stage::Done;
                    return;
                }
            }
            let (_, mut target) = walk.target.expect("picked above");
            if let Some(point) = walk.via.first().copied() {
                let delta = point - player.translation;
                if Vec2::new(delta.x, delta.z).length() < 0.3 {
                    info!(
                        "[fixture-walk] passed waypoint ({:.2},{:.2})",
                        point.x, point.z
                    );
                    walk.via.remove(0);
                } else {
                    target = point.with_y(player.translation.y);
                }
            }
            if head.is_some_and(|(button, _)| button == ButtonType::TimelineFixture) {
                release(walk, &mut touch);
                touch(TAP_FINGER, TouchPhase::Started, tap);
                walk.tap_down = Some(now);
                walk.stage = Stage::Tapped(now);
                info!(
                    "[fixture-walk] head {head:?}; tapping the button at ({:.0},{:.0})",
                    tap.x, tap.y
                );
                return;
            }
            let delta = target - player.translation;
            let distance = Vec2::new(delta.x, delta.z).length();
            if now - walk.since > 40.0 {
                release(walk, &mut touch);
                warn!("[fixture-walk] no timeline button after 40 s at distance {distance:.2} m; head {head:?}");
                walk.stage = Stage::Done;
                return;
            }
            if distance < 0.3 || !joystick.enabled {
                release(walk, &mut touch);
                return;
            }
            let (Ok(camera), Some(root)) = (cameras.single(), root) else {
                return;
            };
            // The joystick's inverse: world direction onto the camera's
            // flattened right and forward axes.
            let direction = Vec2::new(delta.x, delta.z).normalize_or(Vec2::X);
            let forward = camera.forward();
            let forward = Vec2::new(forward.x, forward.z).normalize_or_zero();
            let right = camera.right();
            let joy = Vec2::new(
                direction.dot(Vec2::new(right.x, right.z)),
                direction.dot(forward),
            );
            let position = base + Vec2::new(joy.x, -joy.y) * HANDLE_SIZE * root.scale(window);
            if !walk.pressing {
                touch(FINGER, TouchPhase::Started, base);
                walk.pressing = true;
                return;
            }
            touch(FINGER, TouchPhase::Moved, position);
        }
    }
}

/// The button's window position (`MOLY_FIXTURE_WALK_TAP=x,y`).
fn tap_position() -> Option<Vec2> {
    let raw = std::env::var("MOLY_FIXTURE_WALK_TAP").ok()?;
    let (x, y) = raw.split_once(',')?;
    Some(Vec2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
}

fn pick_target(
    fixtures: &Query<(&FixturePlacement, &GlobalTransform), With<FixtureRoot>>,
    tables: &FixtureActivityTables,
    player: Vec3,
) -> Option<(i32, Vec3)> {
    let wanted = std::env::var("MOLY_FIXTURE_WALK_ID")
        .ok()
        .and_then(|raw| raw.trim().parse::<i32>().ok());
    fixtures
        .iter()
        .map(|(placement, global)| (placement.fixture_id, global.translation()))
        .filter(|(id, _)| match wanted {
            Some(wanted) => *id == wanted,
            None => tables.player_timelines(*id).next().is_some(),
        })
        .min_by(|a, b| {
            a.1.distance_squared(player)
                .total_cmp(&b.1.distance_squared(player))
        })
}
