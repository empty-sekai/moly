//! `GetArrivePosition(nextSiteModel)`: where the cannon flight lands.
//!
//! - A harvest site: its `SitePosition` (the master offset), which is the
//!   destination's own origin.
//! - The delivery site: `DeliverySiteController.GetArrivePosition()`, its
//!   `SitePosition` plus the `_playerJumpOffsetPosition` serialized on its
//!   `DeliverySiteView`, added as a plain vector (the view's transform is not
//!   applied), read from the destination's scene document.
//! - The home site: `FixtureManager.GetHouseView().insideDoorActionPoint`,
//!   the house fixture's `loc_inside` locator in world space.
//!
//! The house point is resolved from the destination's restored layout and
//! the house package's `HouseView` data (the locator transform and its parent
//! chain up to the prefab root, whose own pose is replaced by the placement).
//! The house is `GetHomeFixture()`: the first placed fixture, in layout
//! order, whose master is a home system fixture; a `HouseView` record on any
//! other fixture is never read. A home layout without a home fixture uses the
//! named mock: the home site's origin, with one WARN.

use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use serde_json::Value;
use std::collections::HashMap;

const INDEX: &str = "moly://fixture-gimmick/browser-index.json";

pub(crate) enum Arrival {
    /// The destination's origin (harvest sites).
    Origin,
    /// Reading the delivery site's jump offset.
    Delivery(DeliveryArrival),
    /// Resolving the home house locator.
    Home(HomeArrival),
}

pub(crate) struct DeliveryArrival {
    /// The destination's scene document (the preload holds it).
    scene: Handle<JsonAsset>,
    resolved: Option<Result<Vec3, String>>,
}

pub(crate) struct HomeArrival {
    /// (package, placement pose) of every placed row, in layout order.
    placed: Vec<(String, Transform)>,
    /// The house: the first placed home fixture.
    house: Option<(String, Transform)>,
    index: Handle<JsonAsset>,
    doc: Option<(String, Transform, Handle<JsonAsset>)>,
    resolved: Option<Result<Vec3, String>>,
}

impl Arrival {
    pub(crate) fn for_destination(
        server: &AssetServer,
        category: &str,
        layout: Option<Result<crate::fixture::FixturePlacements, String>>,
        scene: Handle<JsonAsset>,
    ) -> Self {
        if category == "delivery" {
            return Self::Delivery(DeliveryArrival {
                scene,
                resolved: None,
            });
        }
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
            house: None,
            index: server.load(INDEX),
            doc: None,
            resolved: None,
        })
    }

    /// Drive the loads; true once the point is known (or its mock chosen).
    pub(crate) fn poll(&mut self, world: &World) -> bool {
        let home = match self {
            Self::Origin => return true,
            Self::Delivery(delivery) => return delivery.poll(world),
            Self::Home(home) => home,
        };
        if home.resolved.is_some() {
            return true;
        }
        let server = world.resource::<AssetServer>();
        let jsons = world.resource::<Assets<JsonAsset>>();
        if home.doc.is_none() {
            if home.placed.is_empty() {
                home.resolved = Some(Err("the home layout has no placed furniture".into()));
                return true;
            }
            // `GetHouseView` reads the `HouseView` of `GetHomeFixture()`: the
            // first placed fixture whose master is a home system fixture. A
            // `HouseView` record on any other fixture is never read.
            if home.house.is_none() {
                let Some(homes) = world.get_resource::<crate::entry::house::HomeFixtures>() else {
                    return false;
                };
                let mut rows = Vec::new();
                for (package, pose) in &home.placed {
                    match homes.is_home(package) {
                        Ok(true) => rows.push((package.clone(), *pose)),
                        Ok(false) => {}
                        Err(reason) => {
                            home.resolved = Some(Err(format!("home fixture tables: {reason}")));
                            return true;
                        }
                    }
                }
                let Some(first) = rows.first().cloned() else {
                    home.resolved =
                        Some(Err("no placed fixture of the home layout is a home fixture".into()));
                    return true;
                };
                if rows.len() > 1 {
                    info!(
                        "[site-move] {} home fixtures placed; the first in layout order is the house ({})",
                        rows.len(),
                        first.0
                    );
                }
                home.house = Some(first);
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
            let (package, pose) = home.house.clone().expect("set above");
            let Some(path) = paths.get(package.as_str()) else {
                home.resolved = Some(Err(format!("{package}: the house is not in {INDEX}")));
                return true;
            };
            home.doc = Some((package, pose, server.load(format!("moly://{path}"))));
        }
        let (package, pose, handle) = home.doc.as_ref().expect("set above");
        if server.load_state(handle).is_failed() {
            home.resolved = Some(Err(format!("{package}: the house document failed to load")));
            return true;
        }
        let Some(doc) = jsons.get(handle) else {
            return false;
        };
        let doc: Value = match serde_json::from_str(&doc.0) {
            Ok(doc) => doc,
            Err(error) => {
                home.resolved = Some(Err(format!("{package}: the house document is unreadable: {error}")));
                return true;
            }
        };
        let Some(local) = house_door_point(&doc["package"]["houseViews"]) else {
            home.resolved = Some(Err(format!(
                "{package}: the home fixture carries no HouseView record"
            )));
            return true;
        };
        let world_point = pose.transform_point(local);
        info!("[site-move] home arrival at house {package} insideDoorActionPoint {world_point:.3}");
        home.resolved = Some(Ok(world_point));
        true
    }

    /// The arrival point in the destination's local frame.
    pub(crate) fn point(&self) -> Vec3 {
        match self {
            Self::Origin => Vec3::ZERO,
            Self::Delivery(delivery) => match &delivery.resolved {
                Some(Ok(point)) => *point,
                Some(Err(reason)) => {
                    warn!(
                        "[site-move] delivery arrival uses the named mock (site origin): {reason}"
                    );
                    Vec3::ZERO
                }
                None => unreachable!("arrival point read before it was resolved"),
            },
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

impl DeliveryArrival {
    fn poll(&mut self, world: &World) -> bool {
        if self.resolved.is_some() {
            return true;
        }
        if world.resource::<AssetServer>().load_state(&self.scene).is_failed() {
            self.resolved = Some(Err("the delivery site's scene document failed to load".into()));
            return true;
        }
        let Some(doc) = world.resource::<Assets<JsonAsset>>().get(&self.scene) else {
            return false;
        };
        let resolved = jump_offset(&doc.0);
        if let Ok(point) = resolved {
            info!("[site-move] delivery arrival: SitePosition + _playerJumpOffsetPosition = site origin + {point:.3}");
        }
        self.resolved = Some(resolved);
        true
    }
}

/// `DeliverySiteView._playerJumpOffsetPosition` of a delivery scene document,
/// in the runtime frame (the source x axis reflected).
fn jump_offset(text: &str) -> Result<Vec3, String> {
    let doc: Value = serde_json::from_str(text)
        .map_err(|error| format!("the delivery site's scene document is unreadable: {error}"))?;
    jump_offset_of(delivery_site_view(&doc)?)
}

/// The one `DeliverySiteView` instance of a delivery scene document (the
/// delivery controller reads one).
pub(crate) fn delivery_site_view(doc: &Value) -> Result<&Value, String> {
    let instances = doc["components"]["DeliverySiteView"]["instances"]
        .as_array()
        .ok_or("the delivery site's scene has no DeliverySiteView")?;
    let [view] = instances.as_slice() else {
        return Err(format!(
            "the delivery site's scene has {} DeliverySiteView instances; its controller reads one",
            instances.len()
        ));
    };
    Ok(view)
}

/// `_playerJumpOffsetPosition` of a `DeliverySiteView` instance, reflected.
pub(crate) fn jump_offset_of(view: &Value) -> Result<Vec3, String> {
    let offset = &view["fields"]["_playerJumpOffsetPosition"];
    let axis = |name: &str| {
        offset[name].as_f64().map(|value| value as f32).ok_or_else(|| {
            format!("DeliverySiteView._playerJumpOffsetPosition has no {name}")
        })
    };
    Ok(crate::particle_geometry::reflect(Vec3::new(
        axis("x")?,
        axis("y")?,
        axis("z")?,
    )))
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
