//! The conditions an action waits on (`wait_until`), asserts, or leaves a
//! macro's next step to test: they are read from the world each frame, and a
//! condition that cannot be read yet names what is missing.

use bevy::prelude::*;
use moly_law::action_button::ButtonType;
use serde_json::Value;

use super::targets::{flat, player, resolve};
use super::{Target, TargetKind};

/// What a condition names and how it is read.
#[derive(Clone, Debug, PartialEq)]
pub enum Cond {
    All(Vec<Cond>),
    Any(Vec<Cond>),
    Not(Box<Cond>),
    /// The active site's type (`home_site`, `first_floor`, ...).
    SiteIs(String),
    SiteKind(SiteKindName),
    /// A site move (the house's door, a gate) is in progress.
    SiteMoving,
    /// The entry sequence has ended, including the stamina refresh clip when the join reports one.
    EntryDone,
    /// The entry has opened movement and gestures, which a site move waits
    /// for.
    ControlOpen,
    DialogShown,
    /// A player or NPC conversation is playing.
    TalkActive,
    /// The camera has left its entry shot and no transition tween is in
    /// flight, so a drag or a wheel turn moves it.
    CameraSettled,
    ButtonHead(ButtonType),
    ButtonStacked(ButtonType),
    PlayerNear { to: Target, within: f32 },
    NpcNear { unit: u32, to: Target, within: f32 },
    PhenomenonIs(i32),
    /// The character's action state (`Idle`, `Walk`, `Rest`, `Talk`, ...).
    NpcAction { unit: u32, is: String },
    /// The character is on this site with its AI (a roster character, not a
    /// cut-scene stand-in).
    NpcHere(u32),
    /// The node at `path` of the UI view `view` is shown and active.
    UiShown { view: String, path: String },
}

/// The kinds of site a condition names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteKindName {
    Home,
    Room,
    Harvest,
    Delivery,
}

/// A condition outside a script, as the control surface sends one.
pub fn condition(value: &Value) -> Result<Cond, String> {
    parse(value, "condition")
}

/// A condition: a bare name, or an object with one key.
pub fn parse(value: &Value, at: &str) -> Result<Cond, String> {
    if let Some(name) = value.as_str() {
        return Ok(match name {
            "site_moving" => Cond::SiteMoving,
            "entry_done" => Cond::EntryDone,
            "control_open" => Cond::ControlOpen,
            "dialog_shown" => Cond::DialogShown,
            "talk_active" => Cond::TalkActive,
            "camera_settled" => Cond::CameraSettled,
            other => return Err(format!("{at}: unknown condition {other}")),
        });
    }
    let object = value
        .as_object()
        .ok_or_else(|| format!("{at}: a condition is a name or an object with one key"))?;
    let mut entries = object.iter();
    let (Some((key, inner)), None) = (entries.next(), entries.next()) else {
        return Err(format!("{at}: a condition object has exactly one key"));
    };
    let at = format!("{at}.{key}");
    Ok(match key.as_str() {
        "all" | "any" => {
            let list = inner.as_array().ok_or_else(|| format!("{at}: expected a list of conditions"))?;
            let conds = list.iter().map(|item| parse(item, &at)).collect::<Result<Vec<_>, _>>()?;
            if key == "all" {
                Cond::All(conds)
            } else {
                Cond::Any(conds)
            }
        }
        "not" => Cond::Not(Box::new(parse(inner, &at)?)),
        "site_is" => Cond::SiteIs(inner.as_str().ok_or_else(|| format!("{at}: expected a site type"))?.to_owned()),
        "site_kind" => Cond::SiteKind(match inner.as_str() {
            Some("home") => SiteKindName::Home,
            Some("room") => SiteKindName::Room,
            Some("harvest") => SiteKindName::Harvest,
            Some("delivery") => SiteKindName::Delivery,
            _ => return Err(format!("{at}: expected home, room, harvest or delivery")),
        }),
        "button_head" => Cond::ButtonHead(super::button(inner.as_str().unwrap_or_default(), &at)?),
        "button_stacked" => Cond::ButtonStacked(super::button(inner.as_str().unwrap_or_default(), &at)?),
        "phenomenon_is" => Cond::PhenomenonIs(
            inner
                .as_i64()
                .and_then(|v| i32::try_from(v).ok())
                .ok_or_else(|| format!("{at}: expected a phenomenon id"))?,
        ),
        "player_near" => {
            let mut fields = super::Fields::new(inner.as_object().ok_or_else(|| format!("{at}: expected an object"))?, &at);
            let near = Cond::PlayerNear { to: super::target(fields.req("to")?, &at)?, within: fields.opt_f32("within")?.unwrap_or(0.5) };
            fields.finish()?;
            near
        }
        "ui_shown" => {
            let mut fields = super::Fields::new(inner.as_object().ok_or_else(|| format!("{at}: expected an object"))?, &at);
            let shown = Cond::UiShown { view: fields.req_str("view")?.to_owned(), path: fields.req_str("path")?.to_owned() };
            fields.finish()?;
            shown
        }
        "npc_here" => Cond::NpcHere(
            inner
                .as_u64()
                .and_then(|unit| u32::try_from(unit).ok())
                .ok_or_else(|| format!("{at}: expected a character unit"))?,
        ),
        "npc_action" => {
            let mut fields = super::Fields::new(inner.as_object().ok_or_else(|| format!("{at}: expected an object"))?, &at);
            let unit = fields.req_u32("unit")?;
            let is = fields.req_str("is")?.to_owned();
            if !super::NPC_ACTIONS.contains(&is.as_str()) {
                return Err(format!("{at}: unknown action state {is}; the states are {}", super::NPC_ACTIONS.join(", ")));
            }
            fields.finish()?;
            Cond::NpcAction { unit, is }
        }
        "npc_near" => {
            let mut fields = super::Fields::new(inner.as_object().ok_or_else(|| format!("{at}: expected an object"))?, &at);
            let near = Cond::NpcNear {
                unit: fields.req_u32("unit")?,
                to: super::target(fields.req("to")?, &at)?,
                within: fields.opt_f32("within")?.unwrap_or(0.5),
            };
            fields.finish()?;
            near
        }
        other => return Err(format!("{at}: unknown condition {other}")),
    })
}

fn site_type(world: &World) -> Option<String> {
    world.get_resource::<crate::site::SiteActive>().map(|site| site.site_type.clone())
}

fn kind_of(site_type: &str) -> Option<SiteKindName> {
    Some(match site_type {
        "home_site" => SiteKindName::Home,
        "first_floor" | "second_floor" | "third_floor" => SiteKindName::Room,
        "grassland" | "shore" | "flower_garden" | "memorial_place" => SiteKindName::Harvest,
        "festival_garden" => SiteKindName::Delivery,
        _ => return None,
    })
}

/// Whether `cond` holds, and what was read (for a timeout's report).
pub fn eval(world: &mut World, cond: &Cond) -> (bool, String) {
    match cond {
        Cond::All(list) => {
            let mut notes = Vec::new();
            for cond in list {
                let (holds, note) = eval(world, cond);
                notes.push(note);
                if !holds {
                    return (false, notes.join("; "));
                }
            }
            (true, notes.join("; "))
        }
        Cond::Any(list) => {
            let mut notes = Vec::new();
            for cond in list {
                let (holds, note) = eval(world, cond);
                notes.push(note);
                if holds {
                    return (true, notes.join("; "));
                }
            }
            (false, notes.join("; "))
        }
        Cond::Not(inner) => {
            let (holds, note) = eval(world, inner);
            (!holds, format!("not ({note})"))
        }
        Cond::SiteIs(want) => {
            let site = site_type(world);
            (site.as_deref() == Some(want.as_str()), format!("site {site:?}"))
        }
        Cond::SiteKind(want) => {
            let site = site_type(world);
            (site.as_deref().and_then(kind_of) == Some(*want), format!("site {site:?}"))
        }
        Cond::SiteMoving => {
            let moving = world.contains_resource::<crate::site_move::SiteMoveActive>();
            (moving, format!("site moving {moving}"))
        }
        Cond::EntryDone => {
            let done = crate::entry::entry_settled(world.get_resource::<crate::entry::EntrySequence>());
            (done, format!("entry {}", if done { "settled" } else { "in progress" }))
        }
        Cond::ControlOpen => {
            let open = crate::entry::control_open(world.get_resource::<crate::entry::EntrySequence>());
            (open, format!("entry control {}", if open { "open" } else { "closed" }))
        }
        Cond::DialogShown => {
            let shown = world
                .get_resource::<crate::ui_layers::ScreenManager>()
                .is_some_and(crate::ui_layers::ScreenManager::exists_dialog);
            (shown, format!("dialog shown {shown}"))
        }
        Cond::CameraSettled => {
            let state = world.get_resource::<crate::camera::FieldCameraState>().map(|state| state.0);
            let tween = world.contains_resource::<crate::camera::CameraTween>();
            let settled = state.is_some_and(|state| state != crate::camera::CameraStateType::HouseEntry) && !tween;
            (settled, format!("camera state {state:?}, tween in flight {tween}"))
        }
        Cond::TalkActive => {
            let playing = world.contains_resource::<crate::talk::ActiveTalk>();
            let talk = playing || world.contains_resource::<crate::player_talk::PlayerTalkSession>();
            (talk, format!("talk active {talk}"))
        }
        Cond::ButtonHead(button) => {
            let head = world
                .get_resource::<crate::action_button::ActionButtonState>()
                .and_then(|state| state.current());
            (head.is_some_and(|(shown, _)| shown == *button), format!("button head {head:?}"))
        }
        Cond::ButtonStacked(button) => {
            let stacked = world
                .get_resource::<crate::action_button::ActionButtonState>()
                .is_some_and(|state| state.stacked(*button));
            (stacked, format!("{button:?} stacked {stacked}"))
        }
        Cond::PlayerNear { to, within } => near(world, None, to, *within),
        Cond::NpcNear { unit, to, within } => near(world, Some(*unit), to, *within),
        Cond::NpcAction { unit, is } => match crate::act::Taken::body(world, *unit) {
            Ok(entity) => {
                let current = world
                    .get::<crate::npc::NpcActions>(entity)
                    .map(|actions| format!("{:?}", actions.current));
                (current.as_deref() == Some(is.as_str()), format!("character {unit} action {current:?}"))
            }
            Err(error) => (false, error),
        },
        Cond::UiShown { view, path } => match crate::act::ui_view::point(world, view, path) {
            Ok(at) => (true, format!("{view} {path} shown at ({:.0}, {:.0})", at.x, at.y)),
            Err(error) => (false, error),
        },
        Cond::NpcHere(unit) => match crate::act::Taken::body(world, *unit) {
            Ok(_) => (true, format!("character {unit} is here")),
            Err(error) => (false, error),
        },
        Cond::PhenomenonIs(want) => {
            let current = world
                .get_resource::<crate::weather::CurrentPhenomenonId>()
                .map(|id| id.0);
            (current == Some(*want), format!("phenomenon {current:?}"))
        }
    }
}

fn near(world: &mut World, unit: Option<u32>, to: &Target, within: f32) -> (bool, String) {
    let from = match unit {
        None => player(world),
        Some(unit) => resolve(world, &Target::at(TargetKind::Npc(unit))),
    };
    match (from, resolve(world, to)) {
        (Ok(from), Ok(to)) => {
            let distance = flat(from, to);
            (distance <= within, format!("distance {distance:.2} m (within {within} m)"))
        }
        (Err(error), _) | (_, Err(error)) => (false, error),
    }
}


