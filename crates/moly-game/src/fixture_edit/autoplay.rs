//! Instrument `MOLY_EDIT_AUTOPLAY` (off by default; the value is the delay
//! in seconds after the joystick opens, default 3): one layout edit at the
//! current site, sent as the same commands the menu and the edit screen
//! send. The menu's edit button (`Enter`), the camera rotate button once and
//! the change-look button twice (`RotateCamera`, `ChangeLookCamera`), a
//! placed fixture picked from the screen (`SelectPlaced`), a one-cell drag
//! (`MoveTo`), the decide button (`Decide`); the same fixture again, dragged
//! back to its start and decided; then the save button (`SaveAndExit`). A drag whose cell the
//! screen would not accept is retried in the next direction; after four the
//! screen's cancel button (`Cancel`) and the next fixture. With
//! `MOLY_EDIT_AUTOPLAY_WAIT_TWEET` set, the edit button waits (up to 120 s)
//! until a tweet balloon is visible, so the edit's tweet hiding has
//! something to hide.

use bevy::prelude::*;
use moly_law::fixture::GridPosition;

use super::{EditCommand, EditView, PutStatus};

const DIRECTIONS: [(i8, i8); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
/// Wait after entering: the edit camera's 0.5 s tween, and some margin.
const AFTER_ENTER: f32 = 1.5;
/// Wait after each camera button (its rotation tween is 0.5 s).
const AFTER_CAMERA: f32 = 0.8;
/// Wait after each pick (the focus tween is 0.5 s).
const AFTER_PICK: f32 = 1.0;
/// Wait after each decide: the put effect's samples run 3.5 s.
const AFTER_DECIDE: f32 = 3.8;

#[derive(Default, Clone, Copy, PartialEq, Debug)]
enum Step {
    #[default]
    Waiting,
    Entering,
    Entered,
    Rotated,
    Looked,
    LookedBack,
    Picked,
    Moved,
    Decided,
    Repicked,
    MovedBack,
    DecidedBack,
    Saving,
    Done,
}

#[derive(Default)]
pub(super) struct Run {
    warned: bool,
    since: Option<f32>,
    step: Step,
    at: f32,
    uid: String,
    start: Option<GridPosition>,
    direction: usize,
    tried: Vec<String>,
    rotated: bool,
    waiting_logged: bool,
}

fn delay() -> Option<f32> {
    let raw = std::env::var("MOLY_EDIT_AUTOPLAY").ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(3.0);
    }
    Some(
        raw.parse::<f32>()
            .unwrap_or_else(|_| panic!("MOLY_EDIT_AUTOPLAY is not a delay in seconds: {raw:?}")),
    )
}

fn offset(center: GridPosition, (x, z): (i8, i8)) -> Option<GridPosition> {
    Some(GridPosition::new(
        center.x.checked_add(x)?,
        0,
        center.z.checked_add(z)?,
    ))
}

pub(super) fn autoplay(
    time: Res<Time>,
    joystick: Res<crate::joystick::JoystickState>,
    view: Res<EditView>,
    balloons: Query<&InheritedVisibility, With<crate::balloon::BalloonAnchor>>,
    mut out: MessageWriter<EditCommand>,
    mut run: Local<Run>,
) {
    let Some(after) = delay() else {
        return;
    };
    if !run.warned {
        run.warned = true;
        warn!("[edit-autoplay] instrument MOLY_EDIT_AUTOPLAY is on: one layout edit is sent as the menu's and the edit screen's commands");
    }
    let now = time.elapsed_secs();
    let elapsed = now - run.at;
    match run.step {
        Step::Waiting => {
            if !joystick.enabled {
                return;
            }
            let since = *run.since.get_or_insert(now);
            if now - since < after {
                return;
            }
            if std::env::var("MOLY_EDIT_AUTOPLAY_WAIT_TWEET").is_ok() {
                let visible = balloons.iter().filter(|v| v.get()).count();
                if visible == 0 && now - since < after + 120.0 {
                    if !run.waiting_logged {
                        run.waiting_logged = true;
                        info!("[edit-autoplay] waiting for a visible tweet balloon before the edit button");
                    }
                    return;
                }
                info!(
                    "[edit-autoplay] {visible} tweet balloons visible {:.1}s after the delay",
                    now - since - after
                );
            }
            info!("[edit-autoplay] the menu's edit button: Enter");
            out.write(EditCommand::Enter);
            run.step = Step::Entering;
            run.at = now;
        }
        Step::Entering => {
            if view.active {
                info!("[edit-autoplay] edit session active {elapsed:.2}s after Enter");
                run.step = Step::Entered;
                run.at = now;
                run.rotated = false;
            } else if elapsed > 10.0 {
                warn!(
                    "[edit-autoplay] the edit session did not start within 10 s ({}); stopped",
                    view.feedback
                );
                run.step = Step::Done;
            }
        }
        Step::Entered if !run.rotated => {
            if elapsed < AFTER_ENTER {
                return;
            }
            info!("[edit-autoplay] the camera rotate button: RotateCamera");
            out.write(EditCommand::RotateCamera);
            run.rotated = true;
            run.step = Step::Rotated;
            run.at = now;
        }
        Step::Rotated | Step::Looked => {
            if elapsed < AFTER_CAMERA {
                return;
            }
            info!("[edit-autoplay] the change-look button: ChangeLookCamera");
            out.write(EditCommand::ChangeLookCamera);
            run.step = if run.step == Step::Rotated {
                Step::Looked
            } else {
                Step::LookedBack
            };
            run.at = now;
        }
        Step::LookedBack => {
            if elapsed < AFTER_CAMERA {
                return;
            }
            run.step = Step::Entered;
            run.at = now - AFTER_ENTER;
        }
        Step::Entered => {
            if elapsed < AFTER_ENTER {
                return;
            }
            let tried = run.tried.clone();
            let candidates: Vec<_> = view
                .placed_rows
                .iter()
                .filter(|row| row.editable && !tried.contains(&row.uid))
                .collect();
            // Prefer a footprint whose x and z differ, so the effect's
            // scale shows which axis is which.
            let pick = candidates
                .iter()
                .find(|row| row.grid_size.x != row.grid_size.z)
                .or_else(|| candidates.first())
                .map(|row| {
                    (
                        row.uid.clone(),
                        row.center,
                        row.fixture_id,
                        row.grid_size,
                        row.direction,
                    )
                });
            let Some((uid, center, fixture_id, size, direction)) = pick else {
                warn!("[edit-autoplay] no editable placed fixture left to pick ({} placed rows); stopped", view.placed_rows.len());
                run.step = Step::Done;
                return;
            };
            info!(
                "[edit-autoplay] the screen picks placed fixture {uid} (fixture {fixture_id}, grid ({}, {}, {}), direction {direction:?}, center ({}, {}, {})): SelectPlaced",
                size.x, size.y, size.z, center.x, center.y, center.z
            );
            out.write(EditCommand::SelectPlaced { uid: uid.clone() });
            run.tried.push(uid.clone());
            run.uid = uid;
            run.start = Some(center);
            run.direction = 0;
            run.step = Step::Picked;
            run.at = now;
        }
        Step::Picked => {
            if elapsed < AFTER_PICK {
                return;
            }
            let Some(target) = run
                .start
                .and_then(|start| offset(start, DIRECTIONS[run.direction]))
            else {
                run.step = Step::Done;
                return;
            };
            info!(
                "[edit-autoplay] one-cell drag to ({}, 0, {}): MoveTo",
                target.x, target.z
            );
            out.write(EditCommand::MoveTo { center: target });
            run.step = Step::Moved;
            run.at = now;
        }
        Step::Moved | Step::MovedBack => {
            if elapsed < 0.2 {
                return;
            }
            let status = view.selected.as_ref().map(|selected| selected.put_status);
            if status == Some(PutStatus::Ok) {
                info!("[edit-autoplay] the decide button: Decide ({:?})", run.step);
                out.write(EditCommand::Decide);
                run.step = if run.step == Step::Moved {
                    Step::Decided
                } else {
                    Step::DecidedBack
                };
                run.at = now;
                return;
            }
            if run.step == Step::MovedBack {
                warn!("[edit-autoplay] the start cell is not accepted back ({status:?}); the screen's cancel button");
                out.write(EditCommand::Cancel);
                run.step = Step::DecidedBack;
                run.at = now;
                return;
            }
            run.direction += 1;
            if run.direction < DIRECTIONS.len() {
                let Some(target) = run
                    .start
                    .and_then(|start| offset(start, DIRECTIONS[run.direction]))
                else {
                    run.step = Step::Done;
                    return;
                };
                info!("[edit-autoplay] cell not accepted ({status:?}); drag to ({}, 0, {}) instead: MoveTo", target.x, target.z);
                out.write(EditCommand::MoveTo { center: target });
                run.at = now;
            } else {
                info!("[edit-autoplay] no neighbouring cell accepted for {}; the screen's cancel button, next fixture", run.uid);
                out.write(EditCommand::Cancel);
                run.step = Step::Entered;
                run.at = now - AFTER_ENTER;
            }
        }
        Step::Decided => {
            if elapsed < AFTER_DECIDE {
                return;
            }
            info!(
                "[edit-autoplay] the screen picks {} again: SelectPlaced",
                run.uid
            );
            out.write(EditCommand::SelectPlaced {
                uid: run.uid.clone(),
            });
            run.step = Step::Repicked;
            run.at = now;
        }
        Step::Repicked => {
            if elapsed < AFTER_PICK {
                return;
            }
            let Some(start) = run.start else {
                run.step = Step::Done;
                return;
            };
            info!(
                "[edit-autoplay] drag back to the start cell ({}, 0, {}): MoveTo",
                start.x, start.z
            );
            out.write(EditCommand::MoveTo { center: start });
            run.step = Step::MovedBack;
            run.at = now;
        }
        Step::DecidedBack => {
            if elapsed < AFTER_DECIDE {
                return;
            }
            info!("[edit-autoplay] the save button: SaveAndExit");
            out.write(EditCommand::SaveAndExit);
            run.step = Step::Saving;
            run.at = now;
        }
        Step::Saving => {
            if !view.active {
                info!("[edit-autoplay] edit session left {elapsed:.2}s after SaveAndExit");
                run.step = Step::Done;
            } else if elapsed > 30.0 {
                warn!(
                    "[edit-autoplay] the edit session did not end within 30 s ({}); stopped",
                    view.feedback
                );
                run.step = Step::Done;
            }
        }
        Step::Done => {}
    }
}
