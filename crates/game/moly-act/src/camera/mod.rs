//! The camera: a field camera held at a pose (`camera.free`, the talk lens),
//! a finger drag and a wheel turn through the gesture layer, and the talk
//! lens that follows the speaker of a conversation.
//!
//! A pose is written on the camera lease: whoever holds it writes
//! [`moly_features::camera_hold::CameraHold`], and the hold is the one place
//! the pose reaches the camera. The talk lens holds the lease while a
//! conversation is framed, and turns the view's placed fixtures aside while
//! its `clear` is on.

pub mod lens;

use bevy::input::touch::TouchPhase;
use bevy::prelude::*;
use moly_features::act::hand;
use moly_features::act::{ActionRun, ActionSpec, ActCtx, ActPoll, Fields, Kinds, Step};
use moly_features::camera_hold::{CameraHold, HeldPose};
use moly_features::lease::{Holder, Leases};

use crate::Requested;

pub fn install(app: &mut App) {
    Kinds::new()
        .step(Step::new("camera.free", true, None, |fields| Ok(Box::new(read_free(fields)?))))
        .step(Step::new("camera.drag", true, None, |fields| Ok(Box::new(read_drag(fields)?))))
        .step(Step::new("camera.zoom", true, None, |fields| Ok(Box::new(read_zoom(fields)?))))
        .step(Step::new("camera.talk_lens", false, None, |fields| Ok(Box::new(read_lens(fields)?))))
        .step(Step::new("camera.talk_lens_off", false, None, |_| Ok(Box::new(LensOff))))
        .install(app);
    lens::install(app);
}

/// Holds the camera at `pose` for `holder`: the lease is taken before the
/// pose is written, and the hold is cleared when the lease is let go.
pub fn hold(world: &mut World, holder: &Holder, pose: HeldPose) -> Result<(), String> {
    let mut hold = world.remove_resource::<CameraHold>().unwrap_or_default();
    let taken = hold.take(&mut world.resource_mut::<Leases>(), holder, pose);
    world.insert_resource(hold);
    taken.map_err(|refusal| format!("the camera is held: {refusal}"))
}

/// Holds the camera for `holder` without a pose, which it writes frame by
/// frame; refused by name when another holder at the same or a higher rank
/// has it.
fn claim(world: &mut World, holder: &Holder) -> Result<(), String> {
    let mut hold = world.remove_resource::<CameraHold>().unwrap_or_default();
    let claimed = hold.claim(&mut world.resource_mut::<Leases>(), holder);
    world.insert_resource(hold);
    claimed.map_err(|refusal| format!("the camera is held: {refusal}"))
}

/// Lets the camera go for `holder`, clearing the hold when it was its.
fn release(world: &mut World, holder: &str) {
    let mut hold = world.remove_resource::<CameraHold>().unwrap_or_default();
    hold.release(&mut world.resource_mut::<Leases>(), holder);
    world.insert_resource(hold);
}

/// The holder of a step's camera lease: the step's own program, at the
/// runner's rank.
fn camera_holder(ctx: &ActCtx) -> Holder {
    Holder::new(ctx.holder.id.clone(), ctx.holder.rank)
}

/// `camera.free`: the field camera held at a world pose.
#[derive(Debug)]
struct Free {
    position: [f32; 3],
    target: [f32; 3],
    fov: f32,
}

fn read_free(fields: &mut Fields<'_>) -> Result<Free, String> {
    let at = fields.at().to_owned();
    let position = moly_features::act::triple(fields.req("position")?, &format!("{at}.position"))?;
    let target = moly_features::act::triple(fields.req("target")?, &format!("{at}.target"))?;
    let fov = fields.opt_f32("fov")?.unwrap_or(45.0);
    if !(5.0..=120.0).contains(&fov) {
        return Err(format!("{at}: fov is degrees in 5..=120, got {fov}"));
    }
    Ok(Free { position, target, fov })
}

impl ActionSpec for Free {
    fn name(&self) -> &str {
        "camera.free"
    }

    fn hand(&self) -> bool {
        true
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let holder = camera_holder(ctx);
        hold(
            world,
            &holder,
            HeldPose { eye: Vec3::from(self.position), target: Vec3::from(self.target), up: Vec3::Y, fov_degrees: self.fov },
        )?;
        Ok(Box::new(Freed { owner: holder.id }))
    }
}

/// A free camera: its pose is held until the step ends, or another program's
/// pose takes the camera from it.
struct Freed {
    owner: String,
}

impl ActionRun for Freed {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        let held = world.get_resource::<CameraHold>().is_some_and(|hold| hold.holder.as_deref() == Some(self.owner.as_str()));
        if held {
            ActPoll::Done("the free camera holds the pose".to_owned())
        } else {
            ActPoll::Done("the free camera is done".to_owned())
        }
    }

    fn stop(&mut self, world: &mut World, _ctx: &mut ActCtx) {
        release(world, &self.owner);
    }
}

/// `camera.drag`: a one-finger drag of `by` logical pixels over `secs`,
/// starting at `from` (fractions of the window).
#[derive(Debug)]
struct Drag {
    by: [f32; 2],
    secs: f32,
    from: [f32; 2],
}

fn read_drag(fields: &mut Fields<'_>) -> Result<Drag, String> {
    let at = fields.at().to_owned();
    let by = moly_features::act::pair(fields.req("by")?, &format!("{at}.by"))?;
    let secs = fields.opt_f32("secs")?.unwrap_or(0.5);
    let from = match fields.opt("from") {
        Some(from) => moly_features::act::pair(from, &format!("{at}.from"))?,
        None => [0.5, 0.35],
    };
    if from.iter().any(|f| !(0.0..=1.0).contains(f)) {
        return Err(format!("{at}: from is a fraction of the window, got {from:?}"));
    }
    Ok(Drag { by, secs, from })
}

impl ActionSpec for Drag {
    fn name(&self) -> &str {
        "camera.drag"
    }

    fn hand(&self) -> bool {
        true
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let Some(size) = hand::window_size(world) else {
            return Err("no primary window".into());
        };
        Ok(Box::new(DragRun {
            from: Vec2::from(self.from) * size,
            by: Vec2::from(self.by),
            start: ctx.now,
            secs: self.secs,
            down: false,
        }))
    }
}

/// A drag in progress; `down` is false until the finger has touched down at
/// `from`.
struct DragRun {
    from: Vec2,
    by: Vec2,
    start: f32,
    secs: f32,
    down: bool,
}

impl ActionRun for DragRun {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        if !self.down {
            if let Err(error) = hand::drag_touch(world, TouchPhase::Started, self.from) {
                return ActPoll::Failed(error.into());
            }
            self.down = true;
            self.start = ctx.now;
            return ActPoll::Running("finger down".into());
        }
        let done = (ctx.now - self.start) / self.secs.max(f32::EPSILON);
        let at = self.from + self.by * done.min(1.0);
        if let Err(error) = hand::drag_touch(world, TouchPhase::Moved, at) {
            return ActPoll::Failed(error.into());
        }
        if done < 1.0 {
            return ActPoll::Running(format!("{:.0} %", done * 100.0));
        }
        // The last move lands on the end point before the finger lifts.
        self.down = false;
        match hand::drag_touch(world, TouchPhase::Ended, at) {
            Ok(()) => ActPoll::Done(format!("dragged ({:.0}, {:.0}) px", self.by.x, self.by.y)),
            Err(error) => ActPoll::Failed(error.into()),
        }
    }

    fn stop(&mut self, world: &mut World, _ctx: &mut ActCtx) {
        if self.down {
            let _ = hand::drag_touch(world, TouchPhase::Ended, self.from + self.by);
        }
    }
}

/// `camera.zoom`: a wheel turn of `by` pixels (positive zooms in) spread over
/// `secs`.
#[derive(Debug)]
struct Zoom {
    by: f32,
    secs: f32,
}

fn read_zoom(fields: &mut Fields<'_>) -> Result<Zoom, String> {
    let at = fields.at().to_owned();
    let by = fields
        .req("by")?
        .as_f64()
        .filter(|by| by.is_finite())
        .ok_or_else(|| format!("{at}: by must be a number of pixels"))? as f32;
    let secs = fields.opt_f32("secs")?.unwrap_or(0.5);
    Ok(Zoom { by, secs })
}

impl ActionSpec for Zoom {
    fn name(&self) -> &str {
        "camera.zoom"
    }

    fn hand(&self) -> bool {
        true
    }

    fn start(&self, _world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        Ok(Box::new(ZoomRun {
            left: self.by,
            per_sec: if self.secs > 0.0 { self.by / self.secs } else { f32::INFINITY },
            until: ctx.now + self.secs,
        }))
    }
}

/// A wheel turn in progress.
struct ZoomRun {
    left: f32,
    per_sec: f32,
    until: f32,
}

impl ActionRun for ZoomRun {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        let step = if ctx.now >= self.until {
            self.left
        } else {
            let delta = world.resource::<Time>().delta_secs();
            (self.per_sec * delta).clamp(-self.left.abs(), self.left.abs())
        };
        if step != 0.0 {
            if let Err(error) = hand::wheel(world, step) {
                return ActPoll::Failed(error.into());
            }
            self.left -= step;
        }
        if ctx.now >= self.until || self.left == 0.0 {
            ActPoll::Done(String::new())
        } else {
            ActPoll::Running(format!("{:.0} px left", self.left))
        }
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}

/// `camera.talk_lens`: the lens that frames the speaker of a conversation.
#[derive(Debug)]
struct Lens {
    lens: lens::TalkLens,
}

fn read_lens(fields: &mut Fields<'_>) -> Result<Lens, String> {
    let at = fields.at().to_owned();
    let fov = fields.opt_f32("fov")?.unwrap_or(35.0);
    if !(5.0..=120.0).contains(&fov) {
        return Err(format!("{at}: fov is degrees in 5..=120, got {fov}"));
    }
    let glide = fields.opt_f32("glide")?.unwrap_or(2.5);
    if glide < 0.0 {
        return Err(format!("{at}: glide is a rate per second, 0 or more, got {glide}"));
    }
    let push = fields.opt_f32("push")?.unwrap_or(0.12);
    if !(0.0..1.0).contains(&push) {
        return Err(format!("{at}: push is a fraction of the distance in 0..1, got {push}"));
    }
    let reach = fields.opt_f32("reach")?;
    if let Some(reach) = reach.filter(|reach| reach.is_nan() || *reach <= 0.0) {
        return Err(format!("{at}: reach is metres above 0, got {reach}"));
    }
    Ok(Lens {
        lens: lens::TalkLens {
            distance: fields.opt_f32("distance")?.unwrap_or(1.9),
            height: fields.opt_f32("height")?.unwrap_or(0.8),
            target: fields.opt_f32("target")?.unwrap_or(0.47),
            fov,
            swing: fields.opt_f32("swing")?.unwrap_or(0.5),
            idle_unit: fields.opt_u32("unit")?,
            wide: fields.opt_bool("wide")?.unwrap_or(true),
            wide_hold: fields.opt_f32("wide_hold")?.unwrap_or(2.5),
            glide,
            clear: fields.opt_bool("clear")?.unwrap_or(true),
            push,
            reach,
        },
    })
}

impl ActionSpec for Lens {
    fn name(&self) -> &str {
        "camera.talk_lens"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        // The lens frames the conversation every frame: its pose is written
        // on the camera lease, which the lens holds while it is set.
        let holder = camera_holder(ctx);
        claim(world, &holder)?;
        let owner = holder.id.clone();
        world.insert_resource(lens::TalkLensState { lens: self.lens.clone(), holder: owner });
        Ok(Box::new(LensOn { holder }))
    }
}

/// A lens set: it holds the camera until the lens is set again or taken away,
/// and ends on the frame it starts.
struct LensOn {
    holder: Holder,
}

impl ActionRun for LensOn {
    fn poll(&mut self, _world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        ActPoll::Done("the talk lens follows the speaker".to_owned())
    }

    fn stop(&mut self, world: &mut World, _ctx: &mut ActCtx) {
        // A lens set and left unheld is taken away with the step that set it.
        release(world, &self.holder.id);
    }
}

/// `camera.talk_lens_off`: the lens taken away.
#[derive(Debug)]
struct LensOff;

impl ActionSpec for LensOff {
    fn name(&self) -> &str {
        "camera.talk_lens_off"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        // The lens belongs to the program that set it: another program's lens
        // is not taken away, and the camera it holds is not let go.
        let holder = camera_holder(ctx);
        let mine = world.get_resource::<lens::TalkLensState>().is_some_and(|lens| lens.holder == holder.id);
        if mine {
            world.remove_resource::<lens::TalkLensState>();
            release(world, &holder.id);
        }
        Ok(Box::new(Requested { note: "the talk lens is off".to_owned() }))
    }
}
