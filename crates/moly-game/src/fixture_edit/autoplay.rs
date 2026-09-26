//! Instrument `MOLY_EDIT_AUTOPLAY` (off by default; the value is the delay
//! in seconds after the joystick opens, default 3): one layout edit at the
//! current site, sent as the same commands the menu and the edit screen
//! send. The menu's edit button (`Enter`), the camera rotate button once and
//! the change-look button twice (`RotateCamera`, `ChangeLookCamera`); then,
//! for each catalog index in `MOLY_EDIT_AUTOPLAY_CATALOG` (comma separated,
//! default `2,0`), the catalog cell (`SelectCatalog`), which puts the new
//! fixture on the tile the spiral finds, and after the put effect the
//! screen's cancel button (`Cancel`, which removes the pre-placement), or,
//! for an index written with a trailing `d`, the decide button (`Decide`,
//! which places it, so the next put searches around it); a
//! placed fixture picked from the screen (`SelectPlaced`), a one-cell drag
//! (`MoveTo`: the tile the finger touches, one tile beside the one under the
//! fixture's source center), the decide button (`Decide`); the same fixture
//! again, dragged back to its start and decided; then the save button
//! (`SaveAndExit`). A drag whose cell the
//! screen would not accept is retried in the next direction; after four the
//! screen's cancel button (`Cancel`) and the next fixture. With
//! `MOLY_EDIT_AUTOPLAY_WAIT_TWEET` set, the edit button waits (up to 120 s)
//! until a tweet balloon is visible, so the edit's tweet hiding has
//! something to hide. With `MOLY_EDIT_AUTOPLAY_STACK=<uid>,<uid>` the first
//! pick is the first fixture and its drag goes to the tile under the second
//! fixture's source center (a drag onto it), then the same decide and drag
//! back; then the second fixture is picked (it carries what stands on it), a
//! one-cell drag, the fixture rotate button (`Rotate`), the decide button,
//! and the same drag back and decide. With `MOLY_EDIT_AUTOPLAY_HOLD=<secs>`
//! the run waits that long after the camera buttons, with nothing selected
//! and no command sent, so a drag given from outside (a real pointer drag
//! on the ground) reaches the edit camera's drag. With
//! `MOLY_EDIT_AUTOPLAY_CLEAN_UP` set, before the save button the run presses
//! the remove-all button (`RequestCleanUp`) and the confirmation's clean-up
//! button (`CleanUpAll`). With `MOLY_EDIT_AUTOPLAY_RETURN_BASE` set, the
//! base of the stack run is picked once more and sent to storage with the
//! delete button (`ReturnToInventory`) before that.

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
    CatalogPut,
    CatalogCancelled,
    Picked,
    Moved,
    Decided,
    Repicked,
    MovedBack,
    DecidedBack,
    Returning,
    CleanUpAsked,
    CleanedUp,
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
    catalog: Option<Vec<(usize, bool)>>,
    decide_put: bool,
    stack: Option<(String, String)>,
    stack_read: bool,
    /// The first drag's tile when it goes onto another fixture.
    onto: Option<GridPosition>,
    /// The base picked after the drag onto it.
    base_next: Option<String>,
    /// The fixture rotate button is pressed once before the next decide.
    rotate_pending: bool,
    /// The hold after the camera buttons was taken.
    held: bool,
    /// The remove-all buttons were pressed.
    cleaned: bool,
    /// The stack run's base, for the delete button.
    base_uid: Option<String>,
}

/// The catalog indices, each with whether its put is decided (a trailing
/// `d`) rather than cancelled.
fn catalog_indices() -> Vec<(usize, bool)> {
    let raw = std::env::var("MOLY_EDIT_AUTOPLAY_CATALOG").unwrap_or_else(|_| "2,0".into());
    raw.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (digits, decide) = match part.strip_suffix('d') {
                Some(digits) => (digits, true),
                None => (part, false),
            };
            let index = digits.parse::<usize>().unwrap_or_else(|_| {
                panic!("MOLY_EDIT_AUTOPLAY_CATALOG is not a list of catalog indices: {raw:?}")
            });
            (index, decide)
        })
        .collect()
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

/// The ground tile under a placed fixture's source center: the tile a drag
/// that starts on the fixture touches (product frame).
fn touch_tile(row: &super::EditItemView) -> Option<GridPosition> {
    let (center, _, _) = moly_assets::player_data::mirror_fixture_layout(
        row.center,
        row.grid_size,
        row.direction,
        row.layout,
    )
    .ok()?;
    Some(GridPosition::new(
        i8::try_from(-i16::from(center.x) - 1).ok()?,
        0,
        center.z,
    ))
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
        Step::LookedBack | Step::CatalogCancelled => {
            if elapsed < AFTER_CAMERA {
                return;
            }
            if run.step == Step::LookedBack && !run.held {
                run.held = true;
                let hold = std::env::var("MOLY_EDIT_AUTOPLAY_HOLD")
                    .ok()
                    .map(|raw| {
                        raw.trim().parse::<f32>().unwrap_or_else(|_| {
                            panic!("MOLY_EDIT_AUTOPLAY_HOLD is not seconds: {raw:?}")
                        })
                    })
                    .unwrap_or(0.0);
                if hold > 0.0 {
                    info!(
                        "[edit-autoplay] holding {hold:.1}s with nothing selected ({}): no command is sent",
                        if view.selected.is_none() { "selection none" } else { "a selection is open" }
                    );
                    run.at = now + hold;
                    return;
                }
            }
            let queue = run.catalog.get_or_insert_with(catalog_indices);
            if queue.is_empty() {
                run.step = Step::Entered;
                run.at = now - AFTER_ENTER;
                return;
            }
            let (index, decide) = queue.remove(0);
            info!("[edit-autoplay] the catalog cell {index}: SelectCatalog");
            out.write(EditCommand::SelectCatalog { index });
            run.decide_put = decide;
            run.step = Step::CatalogPut;
            run.at = now;
        }
        Step::CatalogPut => {
            if elapsed < AFTER_DECIDE {
                return;
            }
            let (button, command) = if run.decide_put && view.selected.is_some() {
                ("the decide button: Decide", EditCommand::Decide)
            } else {
                ("the screen's cancel button: Cancel", EditCommand::Cancel)
            };
            match view.selected.as_ref() {
                Some(selected) => info!(
                    "[edit-autoplay] catalog put selected {} at center ({}, {}, {}) direction {:?}, put status {:?}; {button}",
                    selected.item.uid,
                    selected.item.center.x,
                    selected.item.center.y,
                    selected.item.center.z,
                    selected.item.direction,
                    selected.put_status
                ),
                None => info!(
                    "[edit-autoplay] catalog put left nothing selected ({}); {button}",
                    view.feedback
                ),
            }
            out.write(command);
            // After a decide, the put effect's samples (3.5 s) before the
            // next button.
            run.step = Step::CatalogCancelled;
            run.at = if run.decide_put {
                now + AFTER_DECIDE - AFTER_CAMERA
            } else {
                now
            };
        }
        Step::Entered => {
            if elapsed < AFTER_ENTER {
                return;
            }
            if !run.stack_read {
                run.stack_read = true;
                run.stack = std::env::var("MOLY_EDIT_AUTOPLAY_STACK")
                    .ok()
                    .and_then(|raw| {
                        let (a, b) = raw.split_once(',')?;
                        Some((a.trim().to_owned(), b.trim().to_owned()))
                    });
            }
            if let Some((target, base)) = run.stack.take() {
                let find = |uid: &str| view.placed_rows.iter().find(|row| row.uid == uid);
                match (find(&target), find(&base)) {
                    (Some(row), Some(under)) => {
                        let (touch, onto) = (touch_tile(row), touch_tile(under));
                        info!(
                            "[edit-autoplay] the screen picks {target} (fixture {}, center ({}, {}, {}), touch tile {touch:?}) to drag onto {base} (fixture {}, touch tile {onto:?}): SelectPlaced",
                            row.fixture_id, row.center.x, row.center.y, row.center.z, under.fixture_id
                        );
                        out.write(EditCommand::SelectPlaced { uid: target.clone() });
                        run.tried.push(target.clone());
                        run.uid = target;
                        run.start = touch;
                        run.onto = onto;
                        run.base_next = Some(base);
                        run.direction = 0;
                        run.step = Step::Picked;
                        run.at = now;
                        return;
                    }
                    _ => warn!("[edit-autoplay] MOLY_EDIT_AUTOPLAY_STACK names {target} or {base}, not a placed row; the usual pick"),
                }
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
                        touch_tile(row),
                    )
                });
            let Some((uid, center, fixture_id, size, direction, touch)) = pick else {
                warn!("[edit-autoplay] no editable placed fixture left to pick ({} placed rows); stopped", view.placed_rows.len());
                run.step = Step::Done;
                return;
            };
            info!(
                "[edit-autoplay] the screen picks placed fixture {uid} (fixture {fixture_id}, grid ({}, {}, {}), direction {direction:?}, center ({}, {}, {}), touch tile {touch:?}): SelectPlaced",
                size.x, size.y, size.z, center.x, center.y, center.z
            );
            out.write(EditCommand::SelectPlaced { uid: uid.clone() });
            run.tried.push(uid.clone());
            run.uid = uid;
            run.start = touch;
            run.direction = 0;
            run.step = Step::Picked;
            run.at = now;
        }
        Step::Picked => {
            if elapsed < AFTER_PICK {
                return;
            }
            let target = match run.onto {
                Some(onto) => Some(onto),
                None => run
                    .start
                    .and_then(|start| offset(start, DIRECTIONS[run.direction])),
            };
            let Some(target) = target else {
                run.step = Step::Done;
                return;
            };
            info!(
                "[edit-autoplay] {} drag to ({}, 0, {}): MoveTo",
                if run.onto.is_some() { "a" } else { "one-cell" },
                target.x,
                target.z
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
            if status == Some(PutStatus::Ok) && run.rotate_pending && run.step == Step::Moved {
                info!("[edit-autoplay] the fixture rotate button: Rotate");
                out.write(EditCommand::Rotate);
                run.rotate_pending = false;
                run.at = now;
                return;
            }
            if status == Some(PutStatus::Ok) {
                info!("[edit-autoplay] the decide button: Decide ({:?})", run.step);
                out.write(EditCommand::Decide);
                run.onto = None;
                run.step = if run.step == Step::Moved {
                    Step::Decided
                } else {
                    Step::DecidedBack
                };
                run.at = now;
                return;
            }
            if run.onto.take().is_some() {
                info!("[edit-autoplay] the drag onto the other fixture is not accepted ({status:?}); the screen's cancel button, next fixture");
                out.write(EditCommand::Cancel);
                run.step = Step::Entered;
                run.at = now - AFTER_ENTER;
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
            if let Some(base) = run.base_next.take() {
                if let Some(row) = view.placed_rows.iter().find(|row| row.uid == base) {
                    let touch = touch_tile(row);
                    info!(
                        "[edit-autoplay] the screen picks the base {base} (fixture {}, center ({}, {}, {}), touch tile {touch:?}): SelectPlaced",
                        row.fixture_id, row.center.x, row.center.y, row.center.z
                    );
                    out.write(EditCommand::SelectPlaced { uid: base.clone() });
                    run.base_uid = Some(base.clone());
                    run.uid = base;
                    run.start = touch;
                    run.onto = None;
                    run.direction = 0;
                    run.rotate_pending = true;
                    run.step = Step::Picked;
                    run.at = now;
                    return;
                }
                warn!("[edit-autoplay] the base {base} is not a placed row any more");
            }
            if std::env::var("MOLY_EDIT_AUTOPLAY_RETURN_BASE").is_ok() {
                if let Some(base) = run.base_uid.take() {
                    info!("[edit-autoplay] the screen picks the base {base} again for the delete button: SelectPlaced");
                    out.write(EditCommand::SelectPlaced { uid: base });
                    run.step = Step::Returning;
                    run.at = now;
                    return;
                }
            }
            if !run.cleaned && std::env::var("MOLY_EDIT_AUTOPLAY_CLEAN_UP").is_ok() {
                run.cleaned = true;
                info!(
                    "[edit-autoplay] the remove-all button: RequestCleanUp ({} placed rows, {} in storage)",
                    view.placed_rows.len(),
                    view.inventory.len()
                );
                out.write(EditCommand::RequestCleanUp);
                run.step = Step::CleanUpAsked;
                run.at = now;
                return;
            }
            info!("[edit-autoplay] the save button: SaveAndExit");
            out.write(EditCommand::SaveAndExit);
            run.step = Step::Saving;
            run.at = now;
        }
        Step::Returning => {
            if elapsed < AFTER_PICK {
                return;
            }
            info!(
                "[edit-autoplay] the delete button: ReturnToInventory ({} placed rows, {} in storage)",
                view.placed_rows.len(),
                view.inventory.len()
            );
            out.write(EditCommand::ReturnToInventory);
            run.step = Step::DecidedBack;
            run.at = now;
        }
        Step::CleanUpAsked => {
            if elapsed < AFTER_CAMERA {
                return;
            }
            info!(
                "[edit-autoplay] the confirmation is {}; its clean-up button: CleanUpAll",
                if view.clean_up_dialog {
                    "open"
                } else {
                    "not open"
                }
            );
            out.write(EditCommand::CleanUpAll);
            run.step = Step::CleanedUp;
            run.at = now;
        }
        Step::CleanedUp => {
            if elapsed < AFTER_DECIDE {
                return;
            }
            info!(
                "[edit-autoplay] after the clean-up: {} placed rows, {} in storage, confirmation {}; the save button: SaveAndExit",
                view.placed_rows.len(),
                view.inventory.len(),
                if view.clean_up_dialog { "open" } else { "closed" }
            );
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
