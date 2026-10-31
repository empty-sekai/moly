//! The layout steps of the director vocabulary: switches, clear, put and
//! remove, on the editor's own draft and save path.
//!
//! - `layout.free {on}` writes the free layout mode of the site into the
//!   local document and ends once the game holds it.
//! - `layout.clear {keep?}` stores the site's fixtures and saves once.
//! - `layout.put {fixture, texture?, at, y?, face?}`, `layout.put_many {items}`
//!   and `layout.remove {uid | at}` place several fixtures or store one, and
//!   save once.
//!
//! A put is a transaction on the editor's draft in the same form
//! `moly.layout.apply` runs ([`moly_layout::LayoutOp`]), so it meets the same
//! put rules. The draft is opened without the editor's screen; its decided
//! rows go through the editor's own save: the save checks, the housing
//! layout call, the saved copy written on its reply, and the normal layout
//! reload. One save is out at a time.
//!
//! A step is made of the source steps the editor's own actions are: the
//! draft opened, its decision, the save begun, the answer's write and the
//! reload ([`open`], [`decide`], [`save`]). [`LayoutSave`] holds the call in
//! flight and what it writes; [`settled`] says when it ended.

use bevy::prelude::*;
use moly_law::fixture::{Direction, GridPosition};
use moly_layout::{Anchor, Apply, Cell, LayoutOp, Rot};
use moly_protocol::{Call, CallKind};
use serde_json::{json, Value};

use moly_features::fixture::EditableFixture;
use crate::session;
use moly_features::fixture::FixturePlacements;

/// What a layout step does.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutStep {
    /// Set the free layout mode of the site.
    SetFree { on: bool },
    /// Store the site's fixtures but the kept ones, and save once.
    Clear { keep: Vec<Keep> },
    /// Apply the operations to the draft, then save once.
    Edit { ops: Vec<LayoutOp> },
}

/// A fixture a clear keeps placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    House,
    Gate,
}

impl Keep {
    pub fn word(self) -> &'static str {
        match self {
            Self::House => "house",
            Self::Gate => "gate",
        }
    }
}

/// The rotation a `face` names: `front` is the master's own facing, `back`
/// the quarter turn that puts a door toward +z, the side the camera starts
/// from.
pub fn face(word: &str) -> Result<Rot, String> {
    Ok(match word {
        "front" => Rot::Deg0,
        "left" => Rot::Deg90,
        "back" => Rot::Deg180,
        "right" => Rot::Deg270,
        other => return Err(format!("face is front, back, left or right, not {other:?}")),
    })
}

/// The name of a direction, for a step's note.
pub fn face_word(direction: Direction) -> &'static str {
    match direction {
        Direction::Front => "front",
        Direction::Left => "left",
        Direction::Back => "back",
        Direction::Right => "right",
    }
}

/// One `layout.put` item: the master row, the texture, the cell its minimum
/// corner goes to and the rotation.
#[derive(Clone, Debug, PartialEq)]
pub struct PutItem {
    pub fixture: i32,
    pub texture: Option<u32>,
    pub at: [i8; 2],
    pub y: i8,
    pub face: Rot,
}

/// The transaction that places the items, in order.
pub fn puts(items: &[PutItem]) -> LayoutStep {
    LayoutStep::Edit {
        ops: items
            .iter()
            .map(|item| LayoutOp::Place {
                fixture: item.fixture,
                texture: item.texture,
                at: Anchor::Cell {
                    x: item.at[0],
                    z: item.at[1],
                    layer: None,
                },
                rotation: item.face,
            })
            .collect(),
    }
}

/// The step as the run records it.
pub fn wire(kind: &LayoutStep) -> Value {
    match kind {
        LayoutStep::SetFree { on } => json!({ "layout.free": on }),
        LayoutStep::Clear { keep } => json!({
            "layout.clear": keep.iter().map(|keep| keep.word()).collect::<Vec<_>>()
        }),
        LayoutStep::Edit { ops } => json!({ "layout.edit": ops.len() }),
    }
}

// ---------------------------------------------------------------------------
// The save in flight
// ---------------------------------------------------------------------------

/// The save a step has begun and the reply of which is not yet written.
struct Sent {
    seq: u32,
    next: FixturePlacements,
    returned: Vec<EditableFixture>,
    storage: moly_features::fixture::layouts::EditStorageSnapshot,
}

/// The step's save in flight, and how the last one ended.
#[derive(Resource, Default)]
pub struct LayoutSave {
    sent: Option<Sent>,
    ended: Option<Result<(), String>>,
    /// The revision the reload of the last save began from.
    reload_from: Option<u64>,
}

/// Whether a program's save is out: the editor's screen opens no draft and
/// starts no save until it is done.
pub fn saving(world: &World) -> bool {
    world
        .get_resource::<LayoutSave>()
        .is_some_and(|save| save.sent.is_some() || save.reload_from.is_some())
}

/// Forgets how the last program's save ended, so the step that starts next
/// waits for its own save.
pub fn reset(world: &mut World) {
    if let Some(mut save) = world.get_resource_mut::<LayoutSave>() {
        save.ended = None;
    }
}

/// Whether the last program's save has ended, and how; `None` while it runs
/// or when the layout it started has not loaded yet.
pub fn settled(world: &World) -> Option<Result<(), String>> {
    let save = world.get_resource::<LayoutSave>()?;
    if save.sent.is_some() || save.reload_from.is_some() {
        return None;
    }
    Some(save.ended.clone().unwrap_or(Ok(())))
}

/// The layout's revision.
fn revision(world: &World) -> u64 {
    world
        .get_resource::<moly_features::fixture::FixtureLayoutRevision>()
        .map_or(0, |revision| revision.0)
}

/// The site's rows as the layout holds them.
fn rows_of(world: &World) -> Vec<EditableFixture> {
    world
        .get_resource::<FixturePlacements>()
        .map_or_else(Vec::new, FixturePlacements::editor_rows)
}

/// The fixture holding a cell, by the site's rows.
fn holder(rows: &[EditableFixture], cell: GridPosition) -> Option<String> {
    rows.iter().find_map(|row| {
        let (min, max) = row.footprint().ok()?;
        ((min.x..=max.x).contains(&cell.x) && (min.z..=max.z).contains(&cell.z))
            .then(|| row.uid.clone())
    })
}

/// A transaction's refusal as a step names it. The refused operation is one
/// of the step's items, in item order.
fn refusal(rows: &[EditableFixture], outcome: &moly_layout::ApplyOutcome, items: usize) -> Option<String> {
    let Err(rejected) = outcome else {
        return None;
    };
    let detail = rejected.refusal.detail.as_deref().unwrap_or_default();
    let reason = match rejected.refusal.code {
        moly_layout::RefusalCode::OutOfBounds => "out of the floor".to_owned(),
        moly_layout::RefusalCode::Overlap => match rejected
            .refusal
            .cell
            .and_then(|cell: Cell| holder(rows, GridPosition::new(cell.x, 0, cell.z)))
        {
            Some(uid) => format!("overlaps {uid}"),
            None => "overlaps another fixture".to_owned(),
        },
        moly_layout::RefusalCode::NotOwned => "not owned".to_owned(),
        moly_layout::RefusalCode::WrongLayer => "wrong layer".to_owned(),
        moly_layout::RefusalCode::UnavailableZone => "unavailable zone".to_owned(),
        moly_layout::RefusalCode::StackNotAllowed => "no room above".to_owned(),
        moly_layout::RefusalCode::MasterMissing => "the fixture is not in the master".to_owned(),
        moly_layout::RefusalCode::StoredFootprintInvalid
        | moly_layout::RefusalCode::ResultFootprintInvalid => "the fixture's box leaves the grid".to_owned(),
        moly_layout::RefusalCode::InputMissing => format!("an input is missing ({detail})"),
        other => format!("{other:?}"),
    };
    Some(if items > 1 {
        format!("item {}: {reason}", rejected.index)
    } else {
        reason
    })
}

/// The step's decided layout, and its note.
#[derive(Resource, Default)]
struct StepLayout {
    decided: Option<FixturePlacements>,
    returned: Vec<EditableFixture>,
    note: Option<String>,
}

/// The placement nonce the last save began from: the rows whose nonce is at
/// or above it are the ones the save wrote.
#[derive(Resource, Default)]
pub struct LayoutNonce(Option<u64>);

impl LayoutNonce {
    /// The base of the last write, for the step's note.
    pub fn base(&self) -> Option<u64> {
        self.0
    }
}

/// The nonce a row of `uid` was saved with (the same suffix the editor gives
/// the rows it saves).
fn nonce_of(uid: &str) -> Option<u64> {
    uid.rsplit('-').next()?.parse().ok()
}

/// The `layout.put` note: each fixture this save created, by its uid and its
/// cells, in row order. A fixture the site held before the save (its nonce
/// below the loaded one) is not named.
pub fn put_notes(world: &World) -> String {
    let Some(base) = world
        .get_resource::<LayoutNonce>()
        .and_then(LayoutNonce::base)
    else {
        return "the site holds the saved layout".to_owned();
    };
    let Some(placements) = world.get_resource::<FixturePlacements>() else {
        return "the site layout is not installed".to_owned();
    };
    let mut notes = Vec::new();
    for row in placements.editor_rows() {
        if nonce_of(&row.uid).is_some_and(|nonce| nonce < base) {
            continue;
        }
        let Ok((min, max)) = row.footprint() else {
            continue;
        };
        notes.push(format!(
            "fixture {} placed as {} at ({},{})..({},{}) face {}",
            row.fixture_id,
            row.uid,
            min.x,
            min.z,
            max.x,
            max.z,
            face_word(row.direction)
        ));
    }
    notes.join("; ")
}

pub fn install(app: &mut App) {
    app.init_resource::<LayoutSave>()
        .init_resource::<StepLayout>()
        .init_resource::<LayoutNonce>()
        .add_systems(Update, answer_saves);
}

/// Keeps `layout`'s nonce as the base the next save's rows are above.
fn keep_base(world: &mut World, layout: &FixturePlacements) {
    let nonce = layout.next_edit_uid();
    world.insert_resource(LayoutNonce(Some(nonce)));
}

// ---------------------------------------------------------------------------
// The steps' parts
// ---------------------------------------------------------------------------

/// Opens the draft of a step, or writes the switch.
pub fn open(world: &mut World, kind: &LayoutStep) -> Result<String, String> {
    world.insert_resource(StepLayout::default());
    if let LayoutStep::SetFree { on } = kind {
        let site = world
            .get_resource::<moly_features::site::SiteActive>()
            .ok_or_else(|| "no site is active".to_owned())?
            .site_id;
        moly_features::free_layout::set(world, site, *on)?;
        return Ok(format!("free layout {}", if *on { "on" } else { "off" }));
    }
    session::ready(world)?;
    Ok("the draft is open".into())
}

/// The step's decision: the transaction on the draft, or the clear.
pub fn decide(world: &mut World, kind: &LayoutStep) -> Result<String, String> {
    match kind {
        LayoutStep::SetFree { on } => Ok(format!("free layout {}", if *on { "on" } else { "off" })),
        LayoutStep::Edit { ops } => edit(world, ops),
        LayoutStep::Clear { keep } => clear(world, keep),
    }
}

fn edit(world: &mut World, ops: &[LayoutOp]) -> Result<String, String> {
    let mut draft = session::open(world)?;
    let rows = rows_of(world);
    let outcome = session::apply(world, &mut draft, &Apply::new(ops.to_vec()))?;
    let (next, returned) = draft.decided()?;
    if let Some(reason) = refusal(&rows, &outcome, ops.len()) {
        return Err(reason);
    }
    let note = format!("{} fixtures on the site", next.total());
    world.insert_resource(StepLayout {
        decided: Some(next),
        returned,
        note: Some(note.clone()),
    });
    Ok(note)
}

/// `layout.clear`: every row of the site but the kept ones.
fn clear(world: &mut World, keep: &[Keep]) -> Result<String, String> {
    let mut draft = session::open(world)?;
    let site_free = world
        .get_resource::<moly_features::free_layout::FreeLayout>()
        .is_some_and(moly_features::free_layout::FreeLayout::is_on);
    let homes = world.get_resource::<moly_features::house::HomeFixtures>();
    let rows = rows_of(world);
    let is_home = |row: &EditableFixture| homes.is_some_and(|homes| homes.is_home(&row.package).unwrap_or(false));
    let is_gate =
        |row: &EditableFixture| homes.is_some_and(|homes| !homes.can_clean_up(&row.package)) && !is_home(row);
    let keep_house = keep.contains(&Keep::House) || !site_free;
    let keep_gate = keep.contains(&Keep::Gate) || !site_free;
    let kept: Vec<String> = rows
        .iter()
        .filter(|row| (is_home(row) && keep_house) || (is_gate(row) && keep_gate))
        .map(|row| row.uid.clone())
        .collect();
    let (mut next, returned) = draft.decided()?;
    next.retain_by_uid(|uid| kept.iter().any(|kept| kept == uid));
    let note = format!(
        "stored {} fixtures, {} kept{}",
        rows.len() - kept.len(),
        kept.len(),
        if site_free { "" } else { " (the house and the gates, as the game keeps them)" }
    );
    world.insert_resource(StepLayout {
        decided: Some(next),
        returned,
        note: Some(note.clone()),
    });
    Ok(note)
}

/// The save: the editor's checks, then the call.
pub fn save(world: &mut World) -> Result<String, String> {
    let step = world.remove_resource::<StepLayout>().unwrap_or_default();
    let Some(next) = step.decided else {
        return Ok("nothing to save".into());
    };
    let note = step.note.unwrap_or_else(|| "the layout is saved".into());
    let (floor, rules) = session::rules(world)?;
    session::check_save(world, &next.editor_rows(), floor, &rules)?;
    // Outside the free layout mode the site's put cost may not pass its
    // limit: the same budget the editor's road plan is held to.
    if !rules.free_layout {
        let budget = moly_features::fixture_put_limit::budget(world, &next.site_type(), &next.editor_rows())?;
        if budget.remaining < 0 {
            return Err(format!(
                "the site's put cost would be {} over its limit of {}",
                -budget.remaining,
                budget.limit
            ));
        }
    }
    let storage = world
        .get_resource_mut::<moly_features::fixture::layouts::SiteFixtureLayouts>()
        .ok_or_else(|| "the saved layouts are not ready".to_owned())?
        .snapshot(next.site_id())?;
    let Some(storage) = storage else {
        return Err("the site's saved layout was not loaded".into());
    };
    // The server's own extension goes in the same document change as the
    // layout: a server that reads `molyExtensions.freeLayout` lays the site
    // out as the game does.
    let site_id = next.site_id();
    let free = world
        .get_resource::<moly_features::free_layout::FreeLayout>()
        .filter(|free| free.site_id() == site_id)
        .is_some_and(moly_features::free_layout::FreeLayout::is_on);
    moly_features::free_layout::set_server_document(site_id, free)?;
    let call = Call::HousingLayout(moly_features::fixture::layouts::housing_layout_request(&next));
    let seq = world.resource_mut::<moly_link::ServerLink>().call(call);
    info!(
        "[layout] PostUserMysekaiHousingLayoutApi sent as seq {seq} by a layout step: {} rows",
        next.total()
    );
    world.insert_resource(LayoutSave {
        sent: Some(Sent {
            seq,
            next,
            returned: step.returned,
            storage,
        }),
        ended: None,
        reload_from: None,
    });
    Ok(note)
}

/// The reply's write: the saved copy and its reload for an accepted answer,
/// the end of the save either way.
fn write(world: &mut World, sent: Sent, accepted: Result<(), String>) {
    keep_base(world, &sent.next);
    let ended = accepted.and_then(|()| {
        moly_features::fixture::layouts::persist_edit(&sent.next, &sent.returned, &sent.storage)
            .map(|receipt| {
                world.insert_resource(sent.next.clone());
                if let Some(mut layouts) =
                    world.get_resource_mut::<moly_features::fixture::layouts::SiteFixtureLayouts>()
                {
                    layouts.did_save(&sent.next, receipt);
                }
                let before = revision(world);
                moly_features::fixture::reload_after_save(world);
                world.write_message(crate::LayoutSaved);
                before
            })
            .map_err(|error| format!("the saved copy was not written: {error}"))
    });
    if let Some(mut save) = world.get_resource_mut::<LayoutSave>() {
        match ended {
            Ok(before) => {
                save.reload_from = Some(before);
                save.ended = Some(Ok(()));
            }
            Err(error) => {
                warn!("[layout] {error}");
                save.reload_from = None;
                save.ended = Some(Err(error));
            }
        }
    }
}

/// Update: the housing layout answers. The answers are read, not taken: the
/// editor's own save and a field build read the same messages.
fn answer_saves(mut answers: MessageReader<moly_features::userdata::Answered>, mut commands: Commands) {
    let answers: Vec<(u32, Result<(), String>)> = answers
        .read()
        .filter(|answer| answer.call == CallKind::HousingLayout)
        .map(|answer| {
            let accepted = match &answer.outcome {
                Ok(moly_protocol::Reply::SuiteUser(reply)) if reply.success => Ok(()),
                Ok(moly_protocol::Reply::SuiteUser(reply)) => Err(reply
                    .refusal
                    .clone()
                    .unwrap_or_else(|| "the server refused the layout".into())),
                Err(refusal) => Err(refusal.reason.clone()),
                other => Err(format!("the server answered {other:?}")),
            };
            (answer.seq, accepted)
        })
        .collect();
    commands.queue(move |world: &mut World| settle_saves(world, answers));
}

/// The program's save an answer names is written; a reload that has landed
/// ends the wait for its save.
fn settle_saves(world: &mut World, answers: Vec<(u32, Result<(), String>)>) {
    for (seq, accepted) in answers {
        let Some(sent) = world.get_resource_mut::<LayoutSave>().and_then(|mut save| {
            if save.sent.as_ref().is_some_and(|sent| sent.seq == seq) {
                save.sent.take()
            } else {
                None
            }
        }) else {
            continue;
        };
        write(world, sent, accepted);
    }
    // The reload of a save has landed: nothing waits for the save any more.
    let current = revision(world);
    let ready = world.contains_resource::<moly_features::fixture::FixtureScenesReady>();
    if let Some(mut save) = world.get_resource_mut::<LayoutSave>() {
        if let Some(from) = save.reload_from {
            if current != from && ready {
                save.reload_from = None;
            }
        }
    }
}
