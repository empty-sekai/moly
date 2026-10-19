//! `moly.observe`: the world as structured data, by section. Each section
//! reads what the game itself reads for that domain (the craft lists through
//! the craft screen's own checks, the bag through the client's copy of the
//! user's tables), so an agent sees what the screens would show.

use bevy::prelude::*;
use serde_json::{json, Map, Value};

use crate::harvest::action::{HarvestPlayerModel, HarvestTargeting};
use crate::harvest::catalog::HarvestCatalog;
use crate::harvest::HarvestObject;
use crate::userdata::client::inventory::{ClientMysekaiInventory, PossessionMasters};

pub(super) const SECTIONS: &[&str] =
    &["site", "gatherables", "bag", "tools", "recipes", "fixtures", "characters", "weather", "time"];

/// The sections named in `what`, every section when it is empty.
pub(super) fn observe(world: &mut World, what: &[String]) -> Result<Value, String> {
    if let Some(unknown) = what.iter().find(|name| !SECTIONS.contains(&name.as_str())) {
        return Err(format!("moly.observe: unknown section {unknown:?}; the sections are {}", SECTIONS.join(", ")));
    }
    let wanted = |name: &str| what.is_empty() || what.iter().any(|wanted| wanted == name);
    let mut out = Map::new();
    out.insert("frame".into(), json!(world.get_resource::<bevy::diagnostic::FrameCount>().map(|frame| frame.0)));
    if wanted("site") {
        out.insert("site".into(), site(world));
    }
    if wanted("gatherables") {
        out.insert("gatherables".into(), gatherables(world));
    }
    if wanted("bag") {
        out.insert("bag".into(), bag(world));
    }
    if wanted("tools") {
        out.insert("tools".into(), tools(world));
    }
    if wanted("recipes") {
        out.insert("recipes".into(), crate::home_action::craft::recipes(world));
    }
    if wanted("fixtures") {
        out.insert("fixtures".into(), fixtures(world));
    }
    if wanted("characters") {
        out.insert("characters".into(), super::state::characters(world));
    }
    if wanted("weather") {
        out.insert("weather".into(), super::state::weather(world));
    }
    if wanted("time") {
        out.insert(
            "time".into(),
            json!({ "game": super::state::time(world), "server_clock_ms": crate::userdata::clock::server_now_ms() }),
        );
    }
    Ok(Value::Object(out))
}

/// The site and its layout grid: the floor's size in tiles (rooms) and the
/// tiles each placed fixture covers, as its footprint's two corners.
fn site(world: &World) -> Value {
    let mut out = match super::state::site(world) {
        Value::Object(site) => site,
        _ => return Value::Null,
    };
    if let Some(placements) = world.get_resource::<crate::fixture::FixturePlacements>() {
        let floor = placements.floor_grid().map(|floor| {
            json!({ "layout_id": floor.layout_id, "width": floor.width, "height": floor.height, "depth": floor.depth })
        });
        let occupied: Vec<Value> = placements
            .editor_rows()
            .iter()
            .filter_map(|row| {
                let (min, max) = row.footprint().ok()?;
                Some(json!({ "uid": row.uid, "min": [min.x, min.y, min.z], "max": [max.x, max.y, max.z] }))
            })
            .collect();
        out.insert("layout_site_id".into(), json!(placements.site_id()));
        out.insert("floor".into(), json!(floor));
        out.insert("occupied".into(), json!(occupied));
    }
    Value::Object(out)
}

/// The harvest objects of the loaded site: each with its world position,
/// remaining hit points, whether it is harvested, the tool type it needs,
/// and whether it is the player's current target.
fn gatherables(world: &mut World) -> Value {
    let target = world.get_resource::<HarvestTargeting>().and_then(|targeting| targeting.target);
    let mut objects = world.query::<(Entity, &HarvestObject, &GlobalTransform)>();
    let mut rows: Vec<(u64, Value)> = objects
        .iter(world)
        .map(|(entity, object, transform)| {
            // The tool-type table covers the fixture types 0 to 9.
            let tool = (0..=9)
                .contains(&object.fixture_type)
                .then(|| crate::harvest::law::tool_type_for(object.fixture_type))
                .flatten()
                .map(|tool| format!("{tool:?}"));
            let at = transform.translation();
            (
                object.uid,
                json!({
                    "uid": object.uid,
                    "entity": entity.to_bits(),
                    "fixture_id": object.fixture_id,
                    "kind": crate::harvest::kind_word(object.fixture_type),
                    "position": [at.x, at.y, at.z],
                    "tile": [object.position_x, object.position_z],
                    "hp": object.hp,
                    "harvested": object.status == crate::harvest::STATUS_HARVESTED,
                    "rare": object.is_rare,
                    "radius": object.radius,
                    "tool": tool,
                    "targeted": target == Some(entity),
                }),
            )
        })
        .collect();
    rows.sort_by_key(|(uid, _)| *uid);
    let mut drops = world.query::<(&crate::harvest::HarvestDropItem, &GlobalTransform)>();
    let drops: Vec<Value> = drops
        .iter(world)
        .map(|(drop, transform)| {
            let at = transform.translation();
            json!({
                "uid": drop.uid,
                "resource_type": drop.resource_type,
                "resource_id": drop.resource_id,
                "rarity": drop.rarity,
                "landed": drop.radius > 0.0,
                "position": [at.x, at.y, at.z],
            })
        })
        .collect();
    json!({ "objects": rows.into_iter().map(|(_, row)| row).collect::<Vec<_>>(), "drops": drops })
}

/// The materials and items held, the total held, and the holding limit of
/// the user's possession level.
fn bag(world: &World) -> Value {
    let Some(inventory) = world.get_resource::<ClientMysekaiInventory>() else {
        return json!({ "error": "no client copy of the user's tables" });
    };
    let limit = world
        .get_resource::<PossessionMasters>()
        .and_then(|masters| inventory.material_max_count(masters));
    json!({
        "materials": inventory.materials().iter().map(|row| json!({ "material_id": row.mysekai_material_id, "quantity": row.quantity })).collect::<Vec<_>>(),
        "items": inventory.items().iter().map(|row| json!({ "item_id": row.mysekai_item_id, "quantity": row.quantity })).collect::<Vec<_>>(),
        "held": inventory.total_material_possession(),
        "limit": limit,
    })
}

/// The gathering tools and the stamina.
fn tools(world: &World) -> Value {
    let Some(model) = world.get_resource::<HarvestPlayerModel>() else {
        return json!({ "error": "the harvest model is not set up (no harvest site has been entered)" });
    };
    let catalog = world.get_resource::<HarvestCatalog>();
    let tools: Vec<Value> = model
        .tools
        .iter()
        .map(|tool| {
            let def = catalog.and_then(|catalog| catalog.tool(tool.tool_id));
            json!({
                "tool_id": tool.tool_id,
                "type": def.map(|def| format!("{:?}", def.tool_type)),
                "level": def.map(|def| def.level),
                "quantity": tool.quantity,
                "durability": tool.durability,
                "max_durability": def.map(|def| def.max_durability),
            })
        })
        .collect();
    json!({
        "stamina": { "normal": model.stamina.normal, "enhance": model.stamina.enhance, "boost": model.stamina.boost, "sum": model.stamina.sum() },
        "tools": tools,
        "selected": model.selected,
    })
}

/// The fixtures owned and the ones placed on the loaded site; a placed
/// system fixture names its type (`CraftTool` is a workbench).
fn fixtures(world: &World) -> Value {
    let owned: Vec<Value> = world
        .get_resource::<ClientMysekaiInventory>()
        .map(|inventory| {
            inventory
                .fixtures()
                .iter()
                .map(|row| json!({ "fixture_id": row.mysekai_fixture_id, "texture_id": row.texture_id, "quantity": row.quantity }))
                .collect()
        })
        .unwrap_or_default();
    let systems = world.get_resource::<crate::system_fixture::SystemFixtures>();
    let placed: Vec<Value> = world
        .get_resource::<crate::fixture::FixturePlacements>()
        .map(|placements| {
            placements
                .editor_rows()
                .iter()
                .map(|row| {
                    json!({
                        "uid": row.uid,
                        "fixture_id": row.fixture_id,
                        "system": systems
                            .and_then(|systems| systems.of_master(row.fixture_id))
                            .map(|system| format!("{system:?}")),
                        "texture_id": row.texture_id,
                        "center": [row.center.x, row.center.y, row.center.z],
                        "grid_size": [row.grid_size.x, row.grid_size.y, row.grid_size.z],
                        "direction": format!("{:?}", row.direction),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({ "owned": owned, "placed": placed })
}
