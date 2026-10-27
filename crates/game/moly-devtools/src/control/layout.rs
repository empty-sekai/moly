//! `moly.layout.view` and `moly.layout.command`: the layout editor's state
//! and the editor's own commands, the ones its buttons, list cells and drags
//! send. A command runs in the next frame under the editor's checks; read
//! the view again to see what it did (`feedback` names a refusal).

use bevy::prelude::*;
use moly_law::fixture::GridPosition;
use serde_json::{json, Map, Value};

use crate::fixture_edit::{
    EditCatalogView, EditCommand, EditItemView, EditSelectionView, EditView, PutStatus,
};

pub(super) const COMMANDS: &[&str] = &[
    "enter",
    "select_content",
    "select_environment",
    "select_catalog",
    "select_placed",
    "select_inventory",
    "move_to",
    "nudge",
    "rotate",
    "decide",
    "cancel",
    "store",
    "save",
    "save_and_exit",
    "request_exit",
    "keep_editing",
    "discard_and_exit",
    "rotate_camera",
    "change_look",
    "request_clean_up",
    "clean_up_all",
    "cancel_clean_up",
];

pub(super) fn view(world: &World) -> Value {
    let Some(view) = world.get_resource::<EditView>() else {
        return json!({ "active": false });
    };
    json!({
        "revision": view.revision,
        "active": view.active,
        "content": format!("{:?}", view.content),
        "environment": view.environment,
        "environment_choices": view.environment_choices.iter().map(|row| json!({
            "phenomena_id": row.phenomena_id, "name": row.name,
        })).collect::<Vec<_>>(),
        "road": {
            "planned": view.road.planned.iter().map(item).collect::<Vec<_>>(),
            "target": view.road.target.as_ref().map(item),
            "can_decide": view.road.can_decide,
            "remaining": view.road.remaining,
            "select_mode": view.road.select_mode,
            "show_fixtures": view.road.show_fixtures,
            "moving": view.road.moving,
            "dragging": view.road.dragging,
        },
        "phase": format!("{:?}", view.phase),
        "site_id": view.site_id,
        "site_type": view.site_type,
        "dirty": view.dirty,
        "exit_dialog": view.exit_dialog,
        "clean_up_dialog": view.clean_up_dialog,
        "save_dialog": view.save_dialog,
        "selected": view.selected.as_ref().map(selection),
        "placed": view.placed_rows.iter().map(item).collect::<Vec<_>>(),
        "inventory": view.inventory.iter().map(item).collect::<Vec<_>>(),
        "catalog": view.catalog.iter().map(catalog).collect::<Vec<_>>(),
        "can_save": view.can_save,
        "feedback": view.feedback,
    })
}

fn grid(position: GridPosition) -> Value {
    json!([position.x, position.y, position.z])
}

fn item(item: &EditItemView) -> Value {
    json!({
        "uid": item.uid,
        "piece": item.piece,
        "package": item.package,
        "fixture_id": item.fixture_id,
        "texture_id": item.texture_id,
        "center": grid(item.center),
        "direction": format!("{:?}", item.direction),
        "grid_size": [item.grid_size.x, item.grid_size.y, item.grid_size.z],
        "layout": item.layout,
        "editable": item.editable,
    })
}

fn selection(selected: &EditSelectionView) -> Value {
    json!({
        "item": item(&selected.item),
        "from_inventory": selected.from_inventory,
        "is_new": selected.is_new,
        "can_store": selected.can_clean_up,
        "put_status": match selected.put_status {
            PutStatus::Ok => "ok".to_owned(),
            PutStatus::Refused(refusal) => format!("{refusal:?}"),
        },
    })
}

fn catalog(row: &EditCatalogView) -> Value {
    json!({
        "index": row.index,
        "package": row.package,
        "fixture_id": row.fixture_id,
        "texture_id": row.texture_id,
        "grid_size": [row.grid_size.x, row.grid_size.y, row.grid_size.z],
        "placeable": row.placeable,
    })
}

/// The command object as the editor's command, the reader `moly.layout.command`
/// and the director's own layout step share.
pub fn command(params: &Map<String, Value>) -> Result<EditCommand, String> {
    let name = params
        .get("command")
        .and_then(Value::as_str)
        .ok_or("moly.layout.command: params need command")?;
    let text = |key: &str| -> Result<String, String> {
        params
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("moly.layout.command: {name} needs {key} (a string)"))
    };
    let small = |value: Option<&Value>, what: &str| -> Result<i8, String> {
        value
            .and_then(Value::as_i64)
            .and_then(|value| i8::try_from(value).ok())
            .ok_or_else(|| format!("moly.layout.command: {name} needs {what} as integers from -128 to 127"))
    };
    Ok(match name {
        "enter" => EditCommand::Enter,
        "select_content" => {
            let kind = params.get("kind").and_then(Value::as_i64)
                .ok_or("select_content needs source kind 0, 1, 2, 3, 4 or 19")?;
            EditCommand::SelectContent(crate::fixture_edit::EditContentType::from_tab_kind(kind)
                .ok_or("select_content kind is not a source content kind")?)
        }
        "select_environment" => {
            let phenomena_id = match params.get("phenomena_id") {
                Some(Value::Null) => None,
                Some(value) => Some(value.as_i64().and_then(|id| i32::try_from(id).ok())
                    .filter(|id| *id > 0).ok_or("phenomena_id must be positive or null for default")?),
                None => return Err("select_environment needs phenomena_id (null means default)".into()),
            };
            EditCommand::SelectEnvironment { phenomena_id }
        }
        "select_catalog" => EditCommand::SelectCatalog {
            index: params
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
                .ok_or("moly.layout.command: select_catalog needs index (a catalog row's index)")?,
        },
        "select_placed" => EditCommand::SelectPlaced { uid: text("uid")? },
        "select_inventory" => EditCommand::SelectInventory { uid: text("uid")? },
        "move_to" => {
            let center = params
                .get("center")
                .and_then(Value::as_array)
                .filter(|center| center.len() == 3)
                .ok_or("moly.layout.command: move_to needs center as [x, y, z]")?;
            EditCommand::MoveTo {
                center: GridPosition::new(
                    small(center.first(), "center")?,
                    small(center.get(1), "center")?,
                    small(center.get(2), "center")?,
                ),
            }
        }
        "nudge" => EditCommand::Nudge {
            x: small(params.get("x"), "x and z")?,
            z: small(params.get("z"), "x and z")?,
        },
        "rotate" => EditCommand::Rotate,
        "decide" => EditCommand::Decide,
        "cancel" => EditCommand::Cancel,
        "store" => EditCommand::ReturnToInventory,
        "save" => EditCommand::Save,
        "save_and_exit" => EditCommand::SaveAndExit,
        "request_exit" => EditCommand::RequestExit,
        "keep_editing" => EditCommand::KeepEditing,
        "discard_and_exit" => EditCommand::DiscardAndExit,
        "rotate_camera" => EditCommand::RotateCamera,
        "change_look" => EditCommand::ChangeLookCamera,
        "request_clean_up" => EditCommand::RequestCleanUp,
        "clean_up_all" => EditCommand::CleanUpAll,
        "cancel_clean_up" => EditCommand::CancelCleanUp,
        other => {
            return Err(format!(
                "moly.layout.command: unknown command {other:?}; one of {}",
                COMMANDS.join(", ")
            ))
        }
    })
}
fn piece(value: Option<&Value>, what: &str) -> Result<moly_layout::Piece, String> {
    let raw = value
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("moly.layout.apply: {what} needs a piece number above 0"))?;
    Ok(moly_layout::Piece(raw))
}

fn rotation(value: Option<&Value>, what: &str) -> Result<moly_layout::Rot, String> {
    let raw = value
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| format!("moly.layout.apply: {what} needs a rotation in degrees"))?;
    moly_layout::Rot::from_degrees(raw)
        .ok_or_else(|| format!("moly.layout.apply: {what} rotation must be 0, 90, 180 or 270"))
}

fn layer(value: Option<&Value>) -> Result<Option<moly_layout::Layer>, String> {
    match value.and_then(Value::as_str) {
        None | Some("") => Ok(None),
        Some("floor") => Ok(Some(moly_layout::Layer::Floor)),
        Some("rug") => Ok(Some(moly_layout::Layer::Rug)),
        Some("wall") => Ok(Some(moly_layout::Layer::Wall)),
        Some("top") => Ok(Some(moly_layout::Layer::Top)),
        Some(other) => Err(format!(
            "moly.layout.apply: layer {other:?} is one of floor, rug, wall, top"
        )),
    }
}

fn anchor(value: &Value) -> Result<moly_layout::Anchor, String> {
    let object = value
        .as_object()
        .ok_or("moly.layout.apply: at is an anchor object")?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("moly.layout.apply: at needs kind")?;
    let int = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_i64)
            .and_then(|value| i8::try_from(value).ok())
            .ok_or_else(|| format!("moly.layout.apply: anchor {kind} needs integer {key}"))
    };
    Ok(match kind {
        "cell" => moly_layout::Anchor::Cell {
            x: int("x")?,
            z: int("z")?,
            layer: layer(object.get("layer"))?,
        },
        "near" => moly_layout::Anchor::Near {
            of: piece(object.get("of"), "near")?,
            side: match object.get("side").and_then(Value::as_str) {
                Some("front") => moly_layout::Side::Front,
                Some("back") => moly_layout::Side::Back,
                Some("left") => moly_layout::Side::Left,
                Some("right") => moly_layout::Side::Right,
                _ => return Err("moly.layout.apply: near needs side front, back, left or right".into()),
            },
            gap: object
                .get("gap")
                .and_then(Value::as_u64)
                .and_then(|value| u8::try_from(value).ok())
                .unwrap_or(0),
        },
        "on_top_of" => moly_layout::Anchor::OnTopOf {
            of: piece(object.get("of"), "on_top_of")?,
        },
        "against" => moly_layout::Anchor::Against {
            wall: match object.get("wall").and_then(Value::as_str) {
                Some("north") => moly_layout::Wall::North,
                Some("east") => moly_layout::Wall::East,
                Some("south") => moly_layout::Wall::South,
                Some("west") => moly_layout::Wall::West,
                Some("nearest") => moly_layout::Wall::Nearest,
                _ => return Err("moly.layout.apply: against needs wall north, east, south, west or nearest".into()),
            },
        },
        "around" => moly_layout::Anchor::Around {
            of: piece(object.get("of"), "around")?,
            index: object
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|value| u8::try_from(value).ok())
                .ok_or("moly.layout.apply: around needs index 0..255")?,
        },
        other => return Err(format!("moly.layout.apply: unknown anchor kind {other:?}")),
    })
}

/// Parses the `moly.layout.apply` parameter object into an [`Apply`].
pub fn apply_request(params: &Map<String, Value>) -> Result<moly_layout::Apply, String> {
    let ops = params
        .get("ops")
        .and_then(Value::as_array)
        .ok_or("moly.layout.apply: params need ops")?
        .iter()
        .map(|value| {
            let object = value
                .as_object()
                .ok_or("moly.layout.apply: every op is an object")?;
            let name = object
                .get("op")
                .and_then(Value::as_str)
                .ok_or("moly.layout.apply: every op needs op")?;
            let texture = |key: &str| -> Result<u32, String> {
                object
                    .get(key)
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| format!("moly.layout.apply: {name} needs {key} as a texture id"))
            };
            Ok(match name {
                "place" => moly_layout::LayoutOp::Place {
                    fixture: object
                        .get("fixture")
                        .and_then(Value::as_i64)
                        .and_then(|value| i32::try_from(value).ok())
                        .ok_or("moly.layout.apply: place needs fixture")?,
                    texture: object.get("texture").map(|_| texture("texture")).transpose()?,
                    at: anchor(
                        object
                            .get("at")
                            .ok_or("moly.layout.apply: place needs at")?,
                    )?,
                    rotation: rotation(object.get("rotation"), "place")?,
                },
                "move" => moly_layout::LayoutOp::Move {
                    piece: piece(object.get("piece"), "move")?,
                    at: anchor(
                        object
                            .get("at")
                            .ok_or("moly.layout.apply: move needs at")?,
                    )?,
                    rotation: object
                        .get("rotation")
                        .map(|_| rotation(object.get("rotation"), "move"))
                        .transpose()?,
                },
                "rotate" => moly_layout::LayoutOp::Rotate {
                    piece: piece(object.get("piece"), "rotate")?,
                    rotation: rotation(object.get("rotation"), "rotate")?,
                },
                "remove" => moly_layout::LayoutOp::Remove {
                    piece: piece(object.get("piece"), "remove")?,
                },
                "recolor" => moly_layout::LayoutOp::Recolor {
                    piece: piece(object.get("piece"), "recolor")?,
                    texture: texture("texture")?,
                },
                other => return Err(format!("moly.layout.apply: unknown op {other:?}")),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(moly_layout::Apply {
        ops,
        dry_run: params.get("dry_run").and_then(Value::as_bool).unwrap_or(false),
        base: params
            .get("base")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
        check_paths: params.get("check_paths").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn result(value: &moly_layout::OpResult) -> Value {
    json!({
        "piece": value.piece.0,
        "minimum": [value.minimum.x, value.minimum.y, value.minimum.z],
        "maximum": [value.maximum.x, value.maximum.y, value.maximum.z],
        "center_y": value.center_y,
        "rotation": value.rotation.degrees(),
    })
}

fn code(value: moly_layout::RefusalCode) -> &'static str {
    match value {
        moly_layout::RefusalCode::OutOfBounds => "out_of_bounds",
        moly_layout::RefusalCode::Overlap => "overlap",
        moly_layout::RefusalCode::WrongLayer => "wrong_layer",
        moly_layout::RefusalCode::NotOwned => "not_owned",
        moly_layout::RefusalCode::UnavailableZone => "unavailable_zone",
        moly_layout::RefusalCode::StackNotAllowed => "stack_not_allowed",
        moly_layout::RefusalCode::BlocksPath => "blocks_path",
        moly_layout::RefusalCode::UnknownPiece => "unknown_piece",
        moly_layout::RefusalCode::StaleRevision => "stale_revision",
        moly_layout::RefusalCode::Locked => "locked",
    }
}

fn refusal(value: &moly_layout::Refusal) -> Value {
    json!({
        "code": code(value.code),
        "piece": value.piece.map(|piece| piece.0),
        "nearest": value.nearest.map(|cell| [cell.x, cell.z]),
        "holder": value.holder,
    })
}

/// The `moly.layout.apply` answer.
pub fn apply_outcome(outcome: &moly_layout::ApplyOutcome) -> Value {
    match outcome {
        Ok(applied) => json!({
            "status": "applied",
            "results": applied.results.iter().map(result).collect::<Vec<_>>(),
            "revision": applied.revision,
        }),
        Err(rejected) => json!({
            "status": "rejected",
            "index": rejected.index,
            "refusal": refusal(&rejected.refusal),
            "results_before": rejected.results_before.iter().map(result).collect::<Vec<_>>(),
        }),
    }
}
