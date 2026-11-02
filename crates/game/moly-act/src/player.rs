//! The player, driven through the virtual hand: walking along the navigation
//! field, pressing an action button, holding the joystick at a ratio, lifting
//! it, choosing the player's body, tapping the screen, placing the player at
//! a world point at once, and crafting a blueprint on the open craft screen.
//!
//! The hand and the walk law are the hub's ([`moly_features::act::hand`],
//! [`moly_features::act::walk`]); this module is the vocabulary that drives
//! them.

use std::sync::Arc;

use bevy::prelude::*;
use moly_features::act::walk::{self, Walk, WalkPoll};
use moly_features::act::{
    hand, ActionRun, ActionSpec, ActCtx, ActPoll, Fields, FixtureRef, Kinds, Step, Target, TargetKind,
};
use moly_law::action_button::{ButtonType, TargetId, ACTION_BUTTON_INPUT_INTERVAL};

use crate::craft::Craft;
use crate::fields::Gated;
use crate::Requested;

pub fn install(app: &mut App) {
    Kinds::new()
        .step(Step::new("player.walk_to", true, Some(60.0), |fields| Ok(Box::new(read_walk(fields)?))))
        .step(Step::new("player.interact", true, Some(60.0), |fields| Ok(Box::new(read_interact(fields)?))))
        .step(Step::many("player.craft", read_craft))
        .step(Step::new("player.stick", true, None, |fields| Ok(Box::new(read_stick(fields)?))))
        .step(Step::new("player.release", true, None, |_| Ok(Box::new(Release))))
        .step(Step::new("player.wear", false, None, |fields| Ok(Box::new(Wear { unit: Some(fields.req_u32("unit")?) }))))
        .step(Step::new("player.wear_avatar", false, None, |_| Ok(Box::new(Wear { unit: None }))))
        .step(Step::new("player.warp_to", false, Some(20.0), |fields| Ok(Box::new(read_warp(fields)?))))
        .step(Step::new("tap", true, None, |fields| Ok(Box::new(read_tap(fields)?))))
        .step(Step::new("ui.tap", true, None, |fields| Ok(Box::new(read_ui_tap(fields)?))))
        .install(app);
}

/// `player.walk_to`: a walk to a target, ending within `within` metres of it.
#[derive(Debug)]
pub struct WalkTo {
    pub to: Target,
    pub within: f32,
}

fn read_walk(fields: &mut Fields<'_>) -> Result<WalkTo, String> {
    let at = fields.at().to_owned();
    Ok(WalkTo { to: moly_features::act::target(fields.req("to")?, &at)?, within: fields.opt_f32("within")?.unwrap_or(0.3) })
}

impl ActionSpec for WalkTo {
    fn name(&self) -> &str {
        "player.walk_to"
    }

    fn hand(&self) -> bool {
        true
    }

    fn start(&self, _world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        Ok(Box::new(WalkRun { walk: Walk::new(self.to.clone(), self.within) }))
    }
}

/// A `player.walk_to` step in progress.
struct WalkRun {
    walk: Walk,
}

impl ActionRun for WalkRun {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        self.walk.poll(world, ctx.ticket, &ctx.path, ctx.now).into()
    }

    fn stop(&mut self, world: &mut World, ctx: &mut ActCtx) {
        walk::release_hand_of(world, ctx.ticket, &ctx.path);
    }
}

/// `player.interact`: walking to a target until its action button heads the
/// stack, then pressing it.
#[derive(Debug)]
pub struct Interact {
    pub walk: Walk,
    pub button: ButtonType,
}

impl Interact {
    pub fn new(to: Target, stop: f32, button: ButtonType) -> Self {
        Self { walk: Walk::contact(to, stop), button }
    }
}

fn read_interact(fields: &mut Fields<'_>) -> Result<Interact, String> {
    let at = fields.at().to_owned();
    let target_at = format!("{at}.target");
    let to = moly_features::act::target(fields.req("target")?, &at)?;
    let button = moly_features::act::button(fields.req_str("button")?, &target_at)?;
    let stop = fields.opt_f32("stop")?.unwrap_or(0.2);
    Ok(Interact::new(to, stop, button))
}

impl ActionSpec for Interact {
    fn name(&self) -> &str {
        "player.interact"
    }

    fn hand(&self) -> bool {
        true
    }

    fn start(&self, _world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        Ok(Box::new(InteractRun {
            walk: Walk::contact(self.walk.to().clone(), self.walk.within()),
            button: self.button,
            lifted_at: None,
            last_change: f32::NEG_INFINITY,
        }))
    }
}

/// The stack entry an interaction presses: a character's or a placed
/// fixture's own entry when the target names one.
fn wanted(world: &World, target: &TargetKind) -> Option<TargetId> {
    let state = world.get_resource::<moly_features::action_button::ActionButtonState>()?;
    match target {
        TargetKind::Npc(unit) => Some(TargetId::Character(*unit)),
        TargetKind::FixtureUid(uid) => state.fixture_key(uid).map(TargetId::Fixture),
        _ => None,
    }
}

/// An interaction in progress.
struct InteractRun {
    walk: Walk,
    button: ButtonType,
    lifted_at: Option<f32>,
    last_change: f32,
}

impl ActionRun for InteractRun {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        let wanted = wanted(world, self.walk.what());
        let (head, stacked, change_shown) = {
            let Some(state) = world.get_resource::<moly_features::action_button::ActionButtonState>() else {
                return ActPoll::Running("the action buttons are not set up".into());
            };
            let stacked = match wanted {
                Some(target) => state.contains(self.button, target),
                None => state.stacked(self.button),
            };
            (state.current(), stacked, state.change_shown())
        };
        let want = self.button;
        let ours =
            move |(button, target): (ButtonType, TargetId)| button == want && wanted.is_none_or(|wanted| wanted == target);
        let now = ctx.now;
        if head.is_some_and(ours) && now - self.last_change >= ACTION_BUTTON_INPUT_INTERVAL + 0.05 {
            // Lift the joystick finger first and tap on a later frame, once
            // the player stands, if the button still heads the stack.
            if hand::pressing(world, ctx.ticket, &ctx.path) || self.lifted_at.is_none() {
                walk::release_hand_of(world, ctx.ticket, &ctx.path);
                self.lifted_at = Some(now);
                return ActPoll::Running(format!("{:?} heads the stack; standing", self.button));
            }
            let Some(position) = hand::button_position(world, self.button) else {
                return ActPoll::Running(format!("{:?} heads the stack but its view is not laid out", self.button));
            };
            return match hand::tap(world, position) {
                Ok(()) => ActPoll::Done(format!("pressed {:?} at ({:.0}, {:.0})", self.button, position.x, position.y)),
                Err(error) => ActPoll::Running(error.to_owned()),
            };
        }
        self.lifted_at = None;
        if let Some((shown, _)) = head.filter(|head| !ours(*head)) {
            if stacked && change_shown && now - self.last_change >= 1.0 {
                if let Some(position) = hand::change_position(world, shown) {
                    walk::release_hand_of(world, ctx.ticket, &ctx.path);
                    if hand::tap(world, position).is_ok() {
                        self.last_change = now;
                        return ActPoll::Running(format!(
                            "{:?} is stacked behind {shown:?}; tapped the change-target button",
                            self.button
                        ));
                    }
                }
            }
        }
        match self.walk.poll(world, ctx.ticket, &ctx.path, now) {
            WalkPoll::Done(note) => {
                ActPoll::Running(format!("{note}; waiting for {:?} to head the stack (head {head:?})", self.button))
            }
            WalkPoll::Running(note) => ActPoll::Running(format!("{note}; head {head:?}")),
            WalkPoll::Failed(note) => ActPoll::Failed(note),
        }
    }

    fn stop(&mut self, world: &mut World, ctx: &mut ActCtx) {
        walk::release_hand_of(world, ctx.ticket, &ctx.path);
    }
}

/// `player.craft`: the walk to the workbench and its button, then the
/// blueprint on the screen that opens.
fn read_craft(fields: &mut Fields<'_>) -> Result<Vec<Arc<dyn ActionSpec>>, String> {
    let at = fields.at().to_owned();
    let blueprint = fields.req_i32("blueprint")?;
    let mut specs: Vec<Arc<dyn ActionSpec>> = Vec::new();
    if let Some(what) = workbench(fields, &at)? {
        specs.push(Gated::new(Interact::new(Target::at(what), 0.2, ButtonType::OpenCraftTool), true, Some(60.0)).arc());
    }
    specs.push(Gated::new(Craft { blueprint }, true, Some(60.0)).arc());
    Ok(specs)
}

/// The workbench a craft step opens first: the step's `workbench` (a placed
/// fixture, or off), else the nearest craft tool.
fn workbench(fields: &mut Fields<'_>, at: &str) -> Result<Option<TargetKind>, String> {
    Ok(match fields.opt("workbench") {
        Some(serde_json::Value::Bool(false)) => None,
        Some(fixture) => Some(match moly_features::act::fixture_ref(fixture, at)? {
            FixtureRef::Uid(uid) => TargetKind::FixtureUid(uid),
            FixtureRef::Id(id) => TargetKind::FixtureId(id),
        }),
        None => Some(TargetKind::Workbench),
    })
}

/// `player.stick`: the joystick held at a stick ratio (x right, y up, within
/// the unit circle) for `secs`.
#[derive(Debug)]
struct Stick {
    stick: Vec2,
    secs: f32,
}

fn read_stick(fields: &mut Fields<'_>) -> Result<Stick, String> {
    let at = fields.at().to_owned();
    let stick = moly_features::act::pair(fields.req("stick")?, &format!("{at}.stick"))?;
    if stick[0].hypot(stick[1]) > 1.0 + 1e-4 {
        return Err(format!("{at}: stick is a ratio within the unit circle, got {stick:?}"));
    }
    let secs = fields.req_f32("secs")?;
    Ok(Stick { stick: Vec2::from(stick), secs })
}

impl ActionSpec for Stick {
    fn name(&self) -> &str {
        "player.stick"
    }

    fn hand(&self) -> bool {
        true
    }

    fn start(&self, _world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        Ok(Box::new(StickRun { stick: self.stick, until: ctx.now + self.secs }))
    }
}

/// A stick lean until its time.
struct StickRun {
    stick: Vec2,
    until: f32,
}

impl ActionRun for StickRun {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        if ctx.now >= self.until {
            hand::release_of(world, ctx.ticket, &ctx.path);
            return ActPoll::Done(String::new());
        }
        if let Some((owner, _)) = hand::leaning(world).filter(|(owner, _)| *owner != ctx.ticket) {
            return ActPoll::Failed(format!("the joystick finger is walking for {owner}"));
        }
        match hand::lean(world, ctx.ticket, &ctx.path, self.stick) {
            Ok(()) => ActPoll::Running(format!("{:.1} s left", self.until - ctx.now)),
            Err(error) => ActPoll::Running(error.to_owned()),
        }
    }

    fn stop(&mut self, world: &mut World, ctx: &mut ActCtx) {
        hand::release_of(world, ctx.ticket, &ctx.path);
    }
}

/// `player.release`: the joystick finger lifted.
#[derive(Debug)]
struct Release;

impl ActionSpec for Release {
    fn name(&self) -> &str {
        "player.release"
    }

    fn hand(&self) -> bool {
        true
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        hand::release(world);
        Ok(Box::new(Requested { note: String::new() }))
    }
}

/// `player.wear`: the player's body, a character's or the avatar's.
#[derive(Debug)]
struct Wear {
    unit: Option<u32>,
}

impl ActionSpec for Wear {
    fn name(&self) -> &str {
        "player.wear"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        use moly_features::player_avatar::oc::{CharacterChoice, OcRequest};
        let (request, note) = match self.unit {
            Some(unit) => (OcRequest::Character(CharacterChoice::Unit(unit)), format!("character body {unit} requested")),
            None => (OcRequest::Avatar, "avatar body requested".to_owned()),
        };
        world.write_message(request);
        Ok(Box::new(Requested { note }))
    }
}

/// `player.warp_to`: the player stood at a world point at once.
#[derive(Debug)]
struct Warp {
    to: [f32; 3],
}

fn read_warp(fields: &mut Fields<'_>) -> Result<Warp, String> {
    let at = fields.at().to_owned();
    Ok(Warp { to: moly_features::act::triple(fields.req("to")?, &format!("{at}.to"))? })
}

impl ActionSpec for Warp {
    fn name(&self) -> &str {
        "player.warp_to"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let to = self.to;
        let mut query = world.query_filtered::<&mut Transform, With<moly_features::player::PlayerControlled>>();
        match query.iter_mut(world).next() {
            Some(mut transform) => {
                transform.translation = Vec3::from(to);
                Ok(Box::new(Requested { note: format!("the player warped to {to:?}") }))
            }
            None => Err("the player is not on the site".to_owned()),
        }
    }
}

/// `tap`: a tap of the screen at logical pixels, or at a fraction of the
/// window.
#[derive(Debug)]
struct Tap {
    at: [f32; 2],
    fraction: bool,
}

fn read_tap(fields: &mut Fields<'_>) -> Result<Tap, String> {
    let at = fields.at().to_owned();
    let (at_value, fraction) = match (fields.opt("at"), fields.opt("at_fraction")) {
        (Some(value), None) => (value, false),
        (None, Some(value)) => (value, true),
        _ => return Err(format!("{at}: give exactly one of at (logical pixels) or at_fraction (0..1 of the window)")),
    };
    Ok(Tap { at: moly_features::act::pair(at_value, &format!("{at}.at"))?, fraction })
}

impl ActionSpec for Tap {
    fn name(&self) -> &str {
        "tap"
    }

    fn hand(&self) -> bool {
        true
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let position = if self.fraction {
            match hand::window_size(world) {
                Some(size) => Vec2::new(self.at[0], self.at[1]) * size,
                None => return Err("no primary window".into()),
            }
        } else {
            Vec2::new(self.at[0], self.at[1])
        };
        hand::tap(world, position)
            .map(|()| Box::new(Requested { note: format!("tapped ({:.0}, {:.0})", position.x, position.y) }) as Box<dyn ActionRun>)
            .map_err(str::to_owned)
    }
}

/// `ui.tap`: a tap at the centre of a node of a shown UI view.
#[derive(Debug)]
struct UiTap {
    view: String,
    path: String,
}

fn read_ui_tap(fields: &mut Fields<'_>) -> Result<UiTap, String> {
    Ok(UiTap { view: fields.req_str("view")?.to_owned(), path: fields.req_str("path")?.to_owned() })
}

impl ActionSpec for UiTap {
    fn name(&self) -> &str {
        "ui.tap"
    }

    fn hand(&self) -> bool {
        true
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let (view, node) = (self.view.as_str(), self.path.as_str());
        let position = hand::ui_point(world, view, node)
            .and_then(|position| hand::tap(world, position).map(|()| position).map_err(str::to_owned))?;
        Ok(Box::new(Requested { note: format!("tapped {view} {node} at ({:.0}, {:.0})", position.x, position.y) }))
    }
}
