//! `GetArrivePosition(nextSiteModel)`: where the cannon flight lands.
//!
//! - A harvest site: its `SitePosition` (the master offset), which is the
//!   destination's own origin.
//! - The delivery site: `DeliverySiteController.GetArrivePosition()`, which
//!   returns its `SitePosition` as well.
//! - The home site: `FixtureManager.GetHouseView().insideDoorActionPoint`,
//!   the house fixture's `loc_inside` locator in world space.
//!
//! The house point is resolved from the destination's restored layout and
//! the house package's `HouseView` data (the locator transform and its parent
//! chain up to the prefab root, whose own pose is replaced by the placement).
//! A home layout without a house (the offline starter has none) uses the
//! named mock: the home site's origin, with one WARN.

use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use serde_json::Value;
use std::collections::HashMap;

const INDEX: &str = "moly://fixture-gimmick/browser-index.json";

pub(crate) enum Arrival {
    /// The destination's origin (harvest and delivery sites).
    Origin,
    /// Resolving the home house locator.
    Home(HomeArrival),
}

pub(crate) struct HomeArrival {
    /// (package, placement pose) of every placed row.
    placed: Vec<(String, Transform)>,
    index: Handle<JsonAsset>,
    docs: Option<Vec<(String, Transform, Handle<JsonAsset>)>>,
    resolved: Option<Result<Vec3, String>>,
}

impl Arrival {
    pub(crate) fn for_destination(
        server: &AssetServer,
        category: &str,
        layout: Option<Result<crate::fixture::FixturePlacements, String>>,
    ) -> Self {
        if category != "housing_home" {
            return Self::Origin;
        }
        let placed = match layout {
            Some(Ok(layout)) => layout
                .placed_rows()
                .into_iter()
                .map(|(package, position, yaw)| {
                    (
                        package.to_owned(),
                        crate::fixture::source_transform(position, yaw),
                    )
                })
                .collect(),
            Some(Err(error)) => {
                warn!("[site-move] home layout unavailable for the arrival point: {error}");
                Vec::new()
            }
            None => Vec::new(),
        };
        Self::Home(HomeArrival {
            placed,
            index: server.load(INDEX),
            docs: None,
            resolved: None,
        })
    }

    /// Drive the loads; true once the point is known (or its mock chosen).
    pub(crate) fn poll(&mut self, world: &World) -> bool {
        let Self::Home(home) = self else {
            return true;
        };
        if home.resolved.is_some() {
            return true;
        }
        let server = world.resource::<AssetServer>();
        let jsons = world.resource::<Assets<JsonAsset>>();
        if home.docs.is_none() {
            if home.placed.is_empty() {
                home.resolved = Some(Err("the home layout has no placed furniture".into()));
                return true;
            }
            if server.load_state(&home.index).is_failed() {
                home.resolved = Some(Err(format!("{INDEX} failed to load")));
                return true;
            }
            let Some(index) = jsons.get(&home.index) else {
                return false;
            };
            let index: Value = match serde_json::from_str(&index.0) {
                Ok(index) => index,
                Err(error) => {
                    home.resolved = Some(Err(format!("{INDEX} unreadable: {error}")));
                    return true;
                }
            };
            let paths: HashMap<&str, &str> = index["packages"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| Some((row["name"].as_str()?, row["path"].as_str()?)))
                        .collect()
                })
                .unwrap_or_default();
            home.docs = Some(
                home.placed
                    .iter()
                    .filter_map(|(package, pose)| {
                        let path = paths.get(package.as_str())?;
                        Some((
                            package.clone(),
                            *pose,
                            server.load(format!("moly://{path}")),
                        ))
                    })
                    .collect(),
            );
        }
        let docs = home.docs.as_ref().expect("set above");
        let mut pending = false;
        for (package, pose, handle) in docs {
            if server.load_state(handle).is_failed() {
                continue;
            }
            let Some(doc) = jsons.get(handle) else {
                pending = true;
                continue;
            };
            let Ok(doc) = serde_json::from_str::<Value>(&doc.0) else {
                continue;
            };
            if let Some(local) = house_door_point(&doc["package"]["houseViews"]) {
                let world_point = pose.transform_point(local);
                info!("[site-move] home arrival at house {package} insideDoorActionPoint {world_point:.3}");
                home.resolved = Some(Ok(world_point));
                return true;
            }
        }
        if pending {
            return false;
        }
        home.resolved = Some(Err(
            "no placed furniture of the home layout has a HouseView".into(),
        ));
        true
    }

    /// The arrival point in the destination's local frame.
    pub(crate) fn point(&self) -> Vec3 {
        match self {
            Self::Origin => Vec3::ZERO,
            Self::Home(home) => match &home.resolved {
                Some(Ok(point)) => *point,
                Some(Err(reason)) => {
                    warn!(
                        "[site-move] home arrival uses the named mock (home site origin): {reason}"
                    );
                    Vec3::ZERO
                }
                None => unreachable!("arrival point read before it was resolved"),
            },
        }
    }
}

/// The first enabled `HouseView`'s `insideDoorActionPoint` in the house
/// prefab's frame (runtime axes), composed from the locator's local
/// transform up to, not including, the prefab root.
fn house_door_point(house_views: &Value) -> Option<Vec3> {
    let views = house_views["views"].as_array()?;
    let transforms = house_views["transforms"].as_array()?;
    let key = |id: &Value| -> Option<(String, String)> {
        Some((
            id["file"].as_str()?.to_owned(),
            id["pathId"].as_str()?.to_owned(),
        ))
    };
    let by_id: HashMap<(String, String), &Value> = transforms
        .iter()
        .filter_map(|row| Some((key(&row["asset"])?, row)))
        .collect();
    let view = views.iter().find(|view| {
        view["enabled"].as_i64() != Some(0) && !view["insideDoorActionPoint"].is_null()
    })?;
    let mut current = by_id.get(&key(&view["insideDoorActionPoint"])?).copied()?;
    let mut chain = Transform::IDENTITY;
    // Walk up while the node has a parent: the parentless node is the prefab
    // root, whose pose the placement supplies.
    while !current["parent"].is_null() {
        chain = local_transform(current)? * chain;
        current = by_id.get(&key(&current["parent"])?).copied()?;
    }
    Some(chain.translation)
}

/// One serialized local TRS, reflected on x into the runtime frame.
fn local_transform(row: &Value) -> Option<Transform> {
    let v = |value: &Value, axis: &str| value[axis].as_f64().map(|f| f as f32);
    let p = &row["localPosition"];
    let r = &row["localRotation"];
    let s = &row["localScale"];
    Some(Transform {
        translation: Vec3::new(-v(p, "x")?, v(p, "y")?, v(p, "z")?),
        rotation: Quat::from_xyzw(v(r, "x")?, -v(r, "y")?, -v(r, "z")?, v(r, "w")?),
        scale: Vec3::new(v(s, "x")?, v(s, "y")?, v(s, "z")?),
    })
}
