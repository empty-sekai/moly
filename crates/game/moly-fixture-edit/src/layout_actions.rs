//! The layout steps, registered with the hub's action registry: switches,
//! clear, put and remove, on the editor's own draft and save path
//! ([`crate::layout_director`]).
//!
//! - `layout.free {on}` writes the free layout mode of the site into the
//!   local document and ends once the game holds it.
//! - `layout.clear {keep?}` stores the site's fixtures and saves once.
//! - `layout.put {fixture, texture?, at, y?, face?}`, `layout.put_many
//!   {items}` and `layout.remove {uid | at}` place several fixtures or store
//!   one, and save once: the step ends once the layout it saved has loaded.
//! - `layout.place {fixture, texture, center, rotate, decide, save}` puts one
//!   owned fixture through the editor's own commands: enter, select its
//!   catalog row, turn it, move it to the tile, decide it and save.

use bevy::prelude::*;
use moly_features::act::{ActionRun, ActionSpec, ActCtx, ActPoll, Fields, Kinds, Step};
use moly_features::act::hand;
use moly_layout::Rot;
use serde_json::{json, Value};

use crate::layout_director::{self, Keep, LayoutStep, PutItem};

/// How long one layout command may take to show its effect (seconds).
const LAYOUT_ANSWER_SECS: f32 = 15.0;
/// How many times a move is sent while the reported center stands off the
/// wanted tile.
const LAYOUT_MOVES: u32 = 3;

pub fn install(app: &mut App) {
    Kinds::new()
        .step(Step::new("layout.free", false, Some(30.0), |fields| Ok(Box::new(read_free(fields)?))))
        .step(Step::new("layout.clear", false, Some(120.0), |fields| Ok(Box::new(read_clear(fields)?))))
        .step(Step::new("layout.put", false, Some(120.0), |fields| Ok(Box::new(read_put(fields)?))))
        .step(Step::new("layout.put_many", false, Some(180.0), |fields| Ok(Box::new(read_put_many(fields)?))))
        .step(Step::new("layout.remove", false, Some(120.0), |fields| Ok(Box::new(read_remove(fields)?))))
        .step(Step::new("layout.place", false, Some(120.0), |fields| Ok(Box::new(read_place(fields)?))))
        .install(app);
}

/// `layout.free`: the site's free layout mode.
#[derive(Debug)]
struct Free {
    on: bool,
}

fn read_free(fields: &mut Fields<'_>) -> Result<Free, String> {
    Ok(Free { on: fields.req_bool("on")? })
}

impl ActionSpec for Free {
    fn name(&self) -> &str {
        "layout.free"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        // The switch decides itself; the note names the mode it left.
        layout_director::open(world, &LayoutStep::SetFree { on: self.on })
            .map(|note| Box::new(Done { note }) as Box<dyn ActionRun>)
    }
}

/// `layout.clear`: every fixture of the site but the kept ones, saved once.
#[derive(Debug)]
struct Clear {
    keep: Vec<Keep>,
}

fn read_clear(fields: &mut Fields<'_>) -> Result<Clear, String> {
    let at = fields.at().to_owned();
    let keep = match fields.opt("keep") {
        None => Vec::new(),
        Some(list) => list
            .as_array()
            .ok_or_else(|| format!("{at}: keep must be a list of house and gate"))?
            .iter()
            .map(|item| match item.as_str() {
                Some("house") => Ok(Keep::House),
                Some("gate") => Ok(Keep::Gate),
                other => Err(format!("{at}: keep is house or gate, not {other:?}")),
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    let mut keep: Vec<Keep> = keep;
    keep.sort_by_key(|keep| *keep as u8);
    keep.dedup();
    Ok(Clear { keep })
}

impl ActionSpec for Clear {
    fn name(&self) -> &str {
        "layout.clear"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        plan(world, LayoutStep::Clear { keep: self.keep.clone() })
    }
}

/// `layout.put`: one fixture, its minimum corner on a cell, saved once.
#[derive(Debug)]
struct Put {
    fixture: i32,
    texture: Option<u32>,
    at: [i8; 2],
    y: i8,
    face: Rot,
}

fn read_put(fields: &mut Fields<'_>) -> Result<Put, String> {
    let at = fields.at().to_owned();
    let item = put(fields, &at)?;
    Ok(Put { fixture: item.fixture, texture: item.texture, at: item.at, y: item.y, face: item.face })
}

impl ActionSpec for Put {
    fn name(&self) -> &str {
        "layout.put"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        plan(
            world,
            layout_director::puts(&[PutItem {
                fixture: self.fixture,
                texture: self.texture,
                at: self.at,
                y: self.y,
                face: self.face,
            }]),
        )
    }
}

/// `layout.put_many`: several fixtures, all or nothing, in one save.
#[derive(Debug)]
struct PutMany {
    items: Vec<PutItem>,
}

fn read_put_many(fields: &mut Fields<'_>) -> Result<PutMany, String> {
    let at = fields.at().to_owned();
    let list = fields
        .req("items")?
        .as_array()
        .ok_or_else(|| format!("{at}: items must be a list of fixtures"))?;
    if list.is_empty() {
        return Err(format!("{at}: items is empty"));
    }
    let items = list
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let at = format!("{at}.items[{index}]");
            let object = value.as_object().ok_or_else(|| format!("{at}: expected an object"))?;
            let mut item = Fields::new(object, &at);
            let placed = put(&mut item, &at)?;
            item.finish()?;
            Ok(placed)
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(PutMany { items })
}

impl ActionSpec for PutMany {
    fn name(&self) -> &str {
        "layout.put_many"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        plan(world, layout_director::puts(&self.items))
    }
}

/// `layout.put`'s item, read from `fields`; the `at` names it.
fn put(fields: &mut Fields<'_>, at: &str) -> Result<PutItem, String> {
    let fixture = fields.req_i32("fixture")?;
    let texture = fields.opt_u32("texture")?;
    if texture == Some(0) {
        return Err(format!("{at}: texture counts from 1"));
    }
    let cell = fields.req("at").and_then(|value| moly_features::act::cell_pair(value, at))?;
    let y = fields.opt_u32("y")?.map_or(Ok(0), |y| {
        i8::try_from(y).map_err(|_| format!("{at}: y is a grid height below 128, not {y}"))
    })?;
    let face = fields.opt_str("face")?.unwrap_or("front");
    let face = layout_director::face(face).map_err(|error| format!("{at}: {error}"))?;
    Ok(PutItem { fixture, texture, at: cell, y, face })
}

/// `layout.remove`: one placed fixture stored, by its uid or a cell it
/// covers, saved once.
#[derive(Debug)]
struct Remove {
    uid: Option<String>,
    at: Option<[i8; 2]>,
}

fn read_remove(fields: &mut Fields<'_>) -> Result<Remove, String> {
    let at = fields.at().to_owned();
    let uid = fields.opt_str("uid")?.map(str::to_owned);
    let cell = fields
        .opt("at")
        .map(|cell| moly_features::act::cell_pair(cell, &format!("{at}.at")))
        .transpose()?;
    if uid.is_some() == cell.is_some() {
        return Err(format!("{at}: give exactly one of uid and at"));
    }
    Ok(Remove { uid, at: cell })
}

impl ActionSpec for Remove {
    fn name(&self) -> &str {
        "layout.remove"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let rows = world
            .get_resource::<moly_features::fixture::FixturePlacements>()
            .map_or_else(Vec::new, moly_features::fixture::FixturePlacements::editor_rows);
        let removed = match &self.uid {
            Some(uid) => rows.iter().position(|row| row.uid == *uid),
            None => self.at.and_then(|cell| {
                rows.iter().position(|row| {
                    row.footprint()
                        .is_ok_and(|(min, max)| (min.x..=max.x).contains(&cell[0]) && (min.z..=max.z).contains(&cell[1]))
                })
            }),
        };
        let Some(index) = removed else {
            let named = match (self.uid.as_deref(), self.at) {
                (Some(uid), _) => format!("uid {uid}"),
                (None, Some(cell)) => format!("the cell ({}, {})", cell[0], cell[1]),
                (None, None) => "the site has no fixture to remove".to_owned(),
            };
            return Err(format!("no fixture of the site is {named}"));
        };
        plan(
            world,
            LayoutStep::Edit {
                ops: vec![moly_layout::LayoutOp::Remove { piece: moly_layout::Piece(index as u32 + 1) }],
            },
        )
    }
}

/// A layout step's draft, its decision and its save, in order: the step
/// starts with its save in flight and waits for it; the refusal is the
/// step's own (a put rule, a save check).
fn plan(world: &mut World, step: LayoutStep) -> Result<Box<dyn ActionRun>, String> {
    layout_director::open(world, &step)?;
    layout_director::reset(world);
    layout_director::decide(world, &step)?;
    layout_director::save(world)?;
    Ok(Box::new(Waiting { step }))
}

/// A layout step that ended on the frame it started, with its note.
struct Done {
    note: String,
}

impl ActionRun for Done {
    fn poll(&mut self, _world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        ActPoll::Done(self.note.clone())
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}

/// A layout step's save, until the layout it saved has loaded.
struct Waiting {
    step: LayoutStep,
}

impl ActionRun for Waiting {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        match layout_director::settled(world) {
            Some(Ok(())) => ActPoll::Done(layout_note(world, &self.step)),
            Some(Err(error)) => ActPoll::Failed(error),
            None => ActPoll::Running(format!("the layout step {} is waiting for its save", layout_director::wire(&self.step))),
        }
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}

/// The note a finished layout step ends with: the fixtures the save
/// created, by uid and cells, or the mode the switch left.
fn layout_note(world: &World, step: &LayoutStep) -> String {
    match step {
        LayoutStep::SetFree { on } => format!("free layout {}", if *on { "on" } else { "off" }),
        LayoutStep::Clear { .. } => "the site is cleared".to_owned(),
        LayoutStep::Edit { .. } => layout_director::put_notes(world),
    }
}

/// `layout.place`: one owned fixture through the layout editor's own
/// commands. Each command object is the one `moly.layout.command` takes and
/// is read by that method's own reader, so a step and the editor's buttons,
/// list cells and drags go through the same checks; every phase reads the
/// editor's view to see what its command did. The editor stays open
/// afterwards, as it does after the same sequence of buttons.
#[derive(Debug)]
struct Place {
    fixture: i32,
    texture: Option<u32>,
    center: [i8; 3],
    rotate: u32,
    decide: bool,
    save: bool,
}

fn read_place(fields: &mut Fields<'_>) -> Result<Place, String> {
    let at = fields.at().to_owned();
    let fixture = fields.req_i32("fixture")?;
    let texture = fields.opt_u32("texture")?;
    let center = moly_features::act::tile(fields.req("center")?, &format!("{at}.center"))?;
    let rotate = fields.opt_u32("rotate")?.unwrap_or(0);
    if rotate > 3 {
        return Err(format!("{at}: rotate is 0 to 3 quarter turns, got {rotate}"));
    }
    let decide = fields.opt_bool("decide")?.unwrap_or(true);
    let save = fields.opt_bool("save")?.unwrap_or(true);
    Ok(Place { fixture, texture, center, rotate, decide, save })
}

impl ActionSpec for Place {
    fn name(&self) -> &str {
        "layout.place"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, _world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        Ok(Box::new(PlaceRun {
            fixture: self.fixture,
            texture: self.texture,
            center: self.center,
            rotate: self.rotate,
            decide: self.decide,
            save: self.save,
            phase: PlacePhase::Enter,
            row: None,
            turns: 0,
            moves: 0,
            target: [self.center[0] as i32, self.center[1] as i32, self.center[2] as i32],
            uid: String::new(),
            direction: None,
            sent: false,
            sent_at: 0.0,
            save_sent: false,
            tapped_at: f32::NEG_INFINITY,
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlacePhase {
    /// Waiting for the open editor.
    Enter,
    /// Choosing the owned catalog row.
    Select,
    /// Turning the selection a quarter turn at a time.
    Rotate,
    /// Moving the selection onto the wanted tile.
    Move,
    /// Deciding the placement.
    Decide,
    /// Saving the draft.
    Save,
}

/// A placement in progress.
struct PlaceRun {
    fixture: i32,
    texture: Option<u32>,
    center: [i8; 3],
    rotate: u32,
    decide: bool,
    save: bool,
    phase: PlacePhase,
    /// The catalog row chosen, the turns and the moves sent.
    row: Option<usize>,
    turns: u32,
    moves: u32,
    /// The tile the next move names: `move_to` takes the touched tile, and
    /// the reported center can stand off it by a tile.
    target: [i32; 3],
    /// The selection the moves belong to.
    uid: String,
    /// The direction the turn in flight started from.
    direction: Option<String>,
    /// A command of this phase is in flight, and when it went out.
    sent: bool,
    sent_at: f32,
    save_sent: bool,
    /// When the save confirmation was last tapped.
    tapped_at: f32,
}

impl PlaceRun {
    /// The command the current phase waits on.
    fn word(&self) -> &'static str {
        match self.phase {
            PlacePhase::Enter => "enter",
            PlacePhase::Select => "select_catalog",
            PlacePhase::Rotate => "rotate",
            PlacePhase::Move => "move_to",
            PlacePhase::Decide => "decide",
            PlacePhase::Save => "save",
        }
    }

    fn begin(&mut self, phase: PlacePhase, now: f32) {
        self.phase = phase;
        self.sent = false;
        self.sent_at = now;
    }

    /// Sends one command through the control surface's own reader and marks
    /// this phase as waiting for its effect.
    fn send(&mut self, world: &mut World, params: Value, now: f32) -> ActPoll {
        let queued = match layout_command(world, params) {
            Ok(queued) => queued,
            Err(error) => return ActPoll::Failed(error),
        };
        self.sent = true;
        self.sent_at = now;
        ActPoll::Running(format!("sent {queued}"))
    }

    /// What the view shows now, for a wait that has to report itself.
    fn reading(&self, view: &crate::EditView) -> String {
        let selected = match &view.selected {
            Some(selected) => format!(
                "{} fixture {} at [{}, {}, {}] {:?}",
                selected.item.uid,
                selected.item.fixture_id,
                selected.item.center.x,
                selected.item.center.y,
                selected.item.center.z,
                selected.put_status
            ),
            None => "no selection".to_owned(),
        };
        format!("{selected}; {} placed rows; feedback {}", view.placed_rows.len(), view.feedback)
    }

    fn poll(&mut self, world: &mut World, now: f32) -> ActPoll {
        let Some(view) = world.get_resource::<crate::EditView>().cloned() else {
            return ActPoll::Failed("the layout editor is not set up".into());
        };
        if self.phase != PlacePhase::Enter && !view.active {
            return ActPoll::Failed("the layout editor closed during the placement".into());
        }
        // The editor answers in the frame after a command arrives. While its
        // own save confirmation is up, the save has an answer to wait for.
        if self.sent && self.phase != PlacePhase::Save && now - self.sent_at > LAYOUT_ANSWER_SECS {
            return ActPoll::Failed(format!(
                "the {} command was not answered in {LAYOUT_ANSWER_SECS:.0} s ({})",
                self.word(),
                self.reading(&view)
            ));
        }
        match self.phase {
            PlacePhase::Enter => {
                if view.active {
                    self.begin(PlacePhase::Select, now);
                    return ActPoll::Running("the layout editor is open".into());
                }
                if self.sent {
                    return ActPoll::Running("waiting for the layout editor to open".into());
                }
                self.send(world, json!({ "command": "enter" }), now)
            }
            PlacePhase::Select => {
                if let Some(selected) = view.selected.as_ref() {
                    if selected.item.fixture_id != self.fixture
                        || self.texture.is_some_and(|texture| selected.item.texture_id != texture)
                    {
                        // The editor's selection is another fixture or texture.
                        return ActPoll::Failed(format!(
                            "the editor selected fixture {} texture {} instead of fixture {}{}",
                            selected.item.fixture_id,
                            selected.item.texture_id,
                            self.fixture,
                            self.texture.map_or(String::new(), |texture| format!(" texture {texture}"))
                        ));
                    }
                    self.uid = selected.item.uid.clone();
                    self.begin(PlacePhase::Rotate, now);
                    return ActPoll::Running(format!("selected {} of the owned catalog", self.uid));
                }
                if self.sent {
                    return ActPoll::Running(format!("waiting for catalog row {} to be selected", self.row.unwrap_or_default()));
                }
                let row = view.catalog.iter().find(|row| {
                    row.fixture_id == self.fixture
                        && self.texture.is_none_or(|texture| row.texture_id == texture)
                        && row.placeable > 0
                });
                // The first owned row of the fixture (and texture) with a copy left.
                let Some(row) = row else {
                    return ActPoll::Failed(self.no_row(&view));
                };
                self.row = Some(row.index);
                self.send(world, json!({ "command": "select_catalog", "index": row.index }), now)
            }
            PlacePhase::Rotate => {
                let Some(selected) = view.selected.as_ref() else {
                    return ActPoll::Failed(format!("the selection was dropped before the turns ({})", view.feedback));
                };
                if self.turns == self.rotate {
                    self.begin(PlacePhase::Move, now);
                    return ActPoll::Running(format!("turned {} quarter turn(s)", self.turns));
                }
                let direction = format!("{:?}", selected.item.direction);
                if self.sent {
                    if self.direction.as_deref() != Some(direction.as_str()) {
                        self.turns += 1;
                        self.direction = None;
                        self.sent = false;
                        return ActPoll::Running(format!("turn {} of {} done, now {direction}", self.turns, self.rotate));
                    }
                    return ActPoll::Running(format!("waiting for the turn; the direction is {direction}"));
                }
                self.direction = Some(direction);
                self.send(world, json!({ "command": "rotate" }), now)
            }
            PlacePhase::Move => {
                let Some(selected) = view.selected.as_ref() else {
                    return ActPoll::Failed(format!("the selection was dropped before the move ({})", view.feedback));
                };
                let at = selected.item.center;
                if at.x == self.center[0] && at.z == self.center[2] {
                    let note = format!("the fixture is on tile [{}, {}, {}]", at.x, at.y, at.z);
                    if !self.decide {
                        return ActPoll::Done(format!("{note}; left selected, not decided"));
                    }
                    self.begin(PlacePhase::Decide, now);
                    return ActPoll::Running(note);
                }
                if self.sent {
                    return ActPoll::Running(format!("waiting for the move; the center is [{}, {}, {}]", at.x, at.y, at.z));
                }
                if self.moves == LAYOUT_MOVES {
                    return ActPoll::Failed(format!(
                        "the editor kept the fixture's center at [{}, {}, {}] instead of tile [{}, ?, {}] ({})",
                        at.x, at.y, at.z, self.center[0], self.center[2], view.feedback
                    ));
                }
                // The first move names the requested tile itself, as the
                // proxy's sequence does; later moves add the difference
                // between the last touched tile and the centre the editor
                // reports, as a drag correcting itself does.
                let target = if self.moves == 0 {
                    [self.center[0] as i32, self.center[1] as i32, self.center[2] as i32]
                } else {
                    [
                        self.target[0] + self.center[0] as i32 - at.x as i32,
                        self.center[1] as i32,
                        self.target[2] + self.center[2] as i32 - at.z as i32,
                    ]
                };
                let (Ok(x), Ok(y), Ok(z)) = (i8::try_from(target[0]), i8::try_from(target[1]), i8::try_from(target[2])) else {
                    return ActPoll::Failed(format!("the corrected tile {target:?} is outside the layout grid"));
                };
                self.target = target;
                self.moves += 1;
                self.send(world, json!({ "command": "move_to", "center": [x, y, z] }), now)
            }
            PlacePhase::Decide => {
                if view.selected.is_none() {
                    let placed = view.placed_rows.iter().find(|row| row.uid == self.uid);
                    let Some(placed) = placed else {
                        return ActPoll::Failed(format!(
                            "the editor lists no fixture with uid {} after the decision ({})",
                            self.uid, view.feedback
                        ));
                    };
                    let at = placed.center;
                    let note = format!("placed fixture {} ({}) at [{}, {}, {}]", self.fixture, self.uid, at.x, at.y, at.z);
                    if !self.save {
                        return ActPoll::Done(note);
                    }
                    self.begin(PlacePhase::Save, now);
                    return ActPoll::Running(format!("{note}; saving"));
                }
                let selected = view.selected.as_ref().expect("checked above");
                if let crate::PutStatus::Refused(refusal) = selected.put_status {
                    return ActPoll::Failed(format!(
                        "the editor refuses to put fixture {} on tile [{}, {}, {}]: {refusal:?}",
                        self.fixture,
                        selected.item.center.x,
                        selected.item.center.y,
                        selected.item.center.z
                    ));
                }
                if self.sent {
                    return ActPoll::Running("waiting for the decision".into());
                }
                self.send(world, json!({ "command": "decide" }), now)
            }
            PlacePhase::Save => {
                if view.save_dialog {
                    // The editor asks before it saves over a cutscene area or
                    // with invalid fixtures listed; its OK is the answer a
                    // player gives.
                    if now - self.tapped_at < 1.0 {
                        return ActPoll::Running("waiting for the save confirmation to close".into());
                    }
                    let (_, caller, key, node) = match moly_features::common_dialog::shown_button(world, true) {
                        Ok(button) => button,
                        Err(error) => return ActPoll::Running(error),
                    };
                    let position = match hand::ui_point(world, key, &node) {
                        Ok(position) => position,
                        Err(error) => return ActPoll::Running(error),
                    };
                    return match hand::tap(world, position) {
                        Ok(()) => {
                            self.tapped_at = now;
                            ActPoll::Running(format!("tapped OK on {key} from {caller}"))
                        }
                        Err(error) => ActPoll::Running(error.to_owned()),
                    };
                }
                if self.save_sent {
                    if !view.dirty {
                        let at = view.placed_rows.iter().find(|row| row.uid == self.uid).map(|row| row.center);
                        return ActPoll::Done(match at {
                            Some(at) => format!("placed fixture {} ({}) at [{}, {}, {}] and saved", self.fixture, self.uid, at.x, at.y, at.z),
                            None => format!("placed fixture {} ({}) and saved", self.fixture, self.uid),
                        });
                    }
                    if now - self.sent_at > LAYOUT_ANSWER_SECS {
                        return ActPoll::Failed(format!(
                            "the save left the draft dirty for {LAYOUT_ANSWER_SECS:.0} s (selected {}, save dialog {})",
                            view.selected.is_some(),
                            view.save_dialog
                        ));
                    }
                    return ActPoll::Running("waiting for the save".into());
                }
                if !view.can_save {
                    return ActPoll::Running(format!(
                        "waiting to save (selected {}, dialog {})",
                        view.selected.is_some(),
                        view.save_dialog
                    ));
                }
                self.save_sent = true;
                self.send(world, json!({ "command": "save" }), now)
            }
        }
    }

    /// Why no catalog row can be selected.
    fn no_row(&self, view: &crate::EditView) -> String {
        let rows: Vec<usize> = view.catalog.iter().filter(|row| row.fixture_id == self.fixture).map(|row| row.index).collect();
        let texture = self.texture.map_or(String::new(), |texture| format!(" texture {texture}"));
        if rows.is_empty() {
            format!("fixture {}{texture} is not in the owned catalog", self.fixture)
        } else {
            format!("fixture {}{texture}: every owned copy is placed (catalog rows {rows:?})", self.fixture)
        }
    }
}

impl ActionRun for PlaceRun {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        self.poll(world, ctx.now)
    }

    fn stop(&mut self, world: &mut World, _ctx: &mut ActCtx) {
        // A selection left in the editor is cancelled: the editor's own
        // cancel restores the pose the fixture was selected at.
        world.write_message(crate::EditCommand::Cancel);
    }
}

/// One layout editor command built the way `moly.layout.command` builds it.
fn layout_command(world: &mut World, params: Value) -> Result<String, String> {
    let Value::Object(params) = params else {
        return Err("a layout command is an object".to_owned());
    };
    let command = crate::layout_command::command(&params)?;
    let queued = format!("{command:?}");
    world.write_message(command);
    Ok(queued)
}
