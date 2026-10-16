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
pub(crate) fn command(params: &Map<String, Value>) -> Result<EditCommand, String> {
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
