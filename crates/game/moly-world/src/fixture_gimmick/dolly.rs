//! `CinemachineDollyCart` on a `CinemachinePath`: while the cart behaviour is
//! enabled, each update advances its position along the path and sets the
//! cart's world position and rotation from it.
//!
//! The path is a chain of cubic Bezier segments between waypoints, evaluated
//! in the path object's local space (Unity's basis) and carried to the node
//! basis at the end. Distance and normalized positions go through the path's
//! lazily built distance table, sampled in world space.

use std::sync::Arc;

use bevy::{math::Affine3A, prelude::*};
use serde_json::Value;

/// `CinemachineUtility` epsilon.
const EPSILON: f32 = 0.0001;

#[derive(Clone, Copy)]
struct Waypoint {
    position: Vec3,
    tangent: Vec3,
    roll: f32,
}

pub(super) struct Path {
    looped: bool,
    resolution: u32,
    waypoints: Vec<Waypoint>,
}

fn number(value: &Value, what: &str) -> Result<f32, String> {
    value.as_f64().map(|value| value as f32).filter(|value| value.is_finite())
        .ok_or_else(|| format!("{what} is not a finite number"))
}

fn vector(value: &Value, what: &str) -> Result<Vec3, String> {
    Ok(Vec3::new(number(&value["x"], what)?, number(&value["y"], what)?, number(&value["z"], what)?))
}

fn flag(value: &Value, what: &str) -> Result<bool, String> {
    match value.as_u64() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(format!("{what} is not a serialized bool")),
    }
}

fn bezier(t: f32, p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3) -> Vec3 {
    let t = t.clamp(0.0, 1.0);
    let d = 1.0 - t;
    p0 * (d * d * d) + p1 * (3.0 * d * d * t) + p2 * (3.0 * d * t * t) + p3 * (t * t * t)
}

fn bezier_tangent(t: f32, p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3) -> Vec3 {
    let t = t.clamp(0.0, 1.0);
    (p0 * -3.0 + p1 * 9.0 - p2 * 9.0 + p3 * 3.0) * t * t
        + (p0 * 6.0 - p1 * 12.0 + p2 * 6.0) * t
        - p0 * 3.0 + p1 * 3.0
}

/// Unity's `Quaternion.LookRotation(forward)` with the world up axis.
fn look_rotation(forward: Vec3) -> Result<Quat, String> {
    let z = forward.normalize();
    let x = Vec3::Y.cross(z);
    if x.length_squared() < 1.0e-12 { return Err("path tangent is parallel to the up axis".into()); }
    let x = x.normalize();
    Ok(Quat::from_mat3(&Mat3::from_cols(x, z.cross(x), z)))
}

impl Path {
    pub(super) fn parse(fields: &Value) -> Result<Self, String> {
        let resolution = fields["m_Resolution"].as_u64().filter(|value| (1..=100).contains(value))
            .ok_or("path resolution is outside 1..=100")? as u32;
        let mut waypoints = Vec::new();
        for waypoint in fields["m_Waypoints"].as_array().ok_or("path has no waypoint array")? {
            waypoints.push(Waypoint {
                position: vector(&waypoint["position"], "waypoint position")?,
                tangent: vector(&waypoint["tangent"], "waypoint tangent")?,
                roll: number(&waypoint["roll"], "waypoint roll")?,
            });
        }
        if waypoints.len() < 2 { return Err("a path needs at least two waypoints".into()); }
        Ok(Self { looped: flag(&fields["m_Looped"], "path loop flag")?, resolution, waypoints })
    }

    fn max_pos(&self) -> f32 {
        let segments = self.waypoints.len() - 1;
        (if self.looped { self.waypoints.len() } else { segments }) as f32
    }

    fn standardize_pos(&self, pos: f32) -> f32 {
        let max = self.max_pos();
        if self.looped && max > 0.0 {
            let pos = pos % max;
            return if pos < 0.0 { pos + max } else { pos };
        }
        if pos < 0.0 { 0.0 } else if pos <= max { pos } else { max }
    }

    /// The waypoint pair around a position, with the position that pairs with
    /// the first index. A position within epsilon of a waypoint snaps to it.
    fn bounding(&self, pos: f32) -> (usize, usize, f32) {
        let count = self.waypoints.len();
        let pos = self.standardize_pos(pos);
        let nearest = pos.round_ties_even() as usize;
        if (pos - nearest as f32).abs() < EPSILON {
            let index = if nearest == count { 0 } else { nearest };
            return (index, index, pos);
        }
        let mut a = pos.floor() as usize;
        let mut pos = pos;
        if a >= count {
            pos -= self.max_pos();
            a = 0;
        }
        let b = pos.ceil() as usize;
        (a, if b >= count { 0 } else { b }, pos)
    }

    fn local_position(&self, pos: f32) -> Vec3 {
        let (a, b, pos) = self.bounding(pos);
        let (wa, wb) = (self.waypoints[a], self.waypoints[b]);
        if a == b { return wa.position; }
        bezier(pos - a as f32, wa.position, wa.position + wa.tangent, wb.position - wb.tangent, wb.position)
    }

    fn local_tangent(&self, pos: f32) -> Vec3 {
        let (a, b, pos) = self.bounding(pos);
        let (wa, wb) = (self.waypoints[a], self.waypoints[b]);
        if a == b { return wa.tangent; }
        bezier_tangent(pos - a as f32, wa.position, wa.position + wa.tangent, wb.position - wb.tangent, wb.position)
    }

    fn roll(&self, a: usize, b: usize, pos: f32) -> f32 {
        let mut roll = self.waypoints[a].roll;
        if a != b {
            let mut next = self.waypoints[b].roll;
            if b == 0 {
                roll %= 360.0;
                next %= 360.0;
            }
            roll += (pos - a as f32).max(0.0) * (next - roll);
        }
        roll
    }

    fn local_orientation(&self, pos: f32) -> Result<Quat, String> {
        let forward = self.local_tangent(pos);
        if forward.length_squared() < EPSILON * EPSILON { return Ok(Quat::IDENTITY); }
        let (a, b, pos) = self.bounding(pos);
        let half = self.roll(a, b, pos) * 0.5 * 0.017453292;
        let (sin, cos) = half.sin_cos();
        Ok(look_rotation(forward)? * Quat::from_xyzw(0.0, 0.0, sin, cos))
    }

    /// The path's position in the node basis for a path-native position.
    fn world_position(&self, world: &Affine3A, pos: f32) -> Vec3 {
        world.transform_point3(moly_assets::coordinates::source_position(self.local_position(pos)))
    }

    fn build_cache(&self, world: &Affine3A) -> Cache {
        let step = 1.0 / self.resolution as f32;
        let keys = (self.max_pos() / step).round_ties_even() as usize + 1;
        let mut pos_to_distance = vec![0.0f32; keys];
        let mut length = 0.0f32;
        let mut pos = 0.0f32;
        let mut previous = self.world_position(world, pos);
        for entry in pos_to_distance.iter_mut().skip(1) {
            pos += step;
            let current = self.world_position(world, pos);
            length += previous.distance(current);
            previous = current;
            *entry = length;
        }
        let distance_step = length / (keys - 1) as f32;
        let mut distance_to_pos = vec![0.0f32; keys];
        let mut distance = 0.0f32;
        let mut index = 1;
        for entry in distance_to_pos.iter_mut().skip(1) {
            distance += distance_step;
            let mut reached = pos_to_distance[index];
            while reached < distance && index < keys - 1 {
                index += 1;
                reached = pos_to_distance[index];
            }
            let before = pos_to_distance[index - 1];
            *entry = ((distance - before) / (reached - before) + index as f32 - 1.0) * step;
        }
        Cache { length, distance_step, distance_to_pos }
    }
}

struct Cache {
    length: f32,
    distance_step: f32,
    distance_to_pos: Vec<f32>,
}

impl Cache {
    fn standardize_distance(&self, path: &Path, distance: f32) -> f32 {
        if self.length < 1.0e-5 { return 0.0; }
        let mut distance = distance;
        if path.looped {
            distance %= self.length;
            if distance < 0.0 { distance += self.length; }
        }
        if distance < 0.0 { 0.0 } else if distance <= self.length { distance } else { self.length }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Units {
    Path,
    Distance,
    Normalized,
}

#[derive(Clone)]
pub(super) struct Cart {
    enabled: bool,
    speed: f32,
    position: f32,
    units: Units,
}

impl Cart {
    pub(super) fn parse(fields: &Value) -> Result<Self, String> {
        let units = match fields["m_PositionUnits"].as_u64() {
            Some(0) => Units::Path,
            Some(1) => Units::Distance,
            Some(2) => Units::Normalized,
            _ => return Err("unknown dolly position units".into()),
        };
        if fields["m_UpdateMethod"].as_u64() != Some(0) {
            return Err("only per-frame Update dolly carts have a consumer".into());
        }
        Ok(Self {
            enabled: flag(&fields["m_Enabled"], "cart enable flag")?,
            speed: number(&fields["m_Speed"], "cart speed")?,
            position: number(&fields["m_Position"], "cart position")?,
            units,
        })
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }
}

pub(super) struct State {
    cart: Cart,
    path: Arc<Path>,
    position: f32,
    cache: Option<Cache>,
}

impl State {
    pub(super) fn new(cart: &Cart, path: Arc<Path>) -> Self {
        Self { cart: cart.clone(), path, position: cart.position, cache: None }
    }

    /// One Update: advance by the cart speed and return the cart's world
    /// position and rotation in the node basis.
    pub(super) fn step(&mut self, delta: f32, path_world: &Affine3A) -> Result<(Vec3, Quat), String> {
        let path = &*self.path;
        let cache = self.cache.get_or_insert_with(|| path.build_cache(path_world));
        let travelled = self.position + self.cart.speed * delta;
        let standard = match self.cart.units {
            Units::Path => path.standardize_pos(travelled),
            Units::Distance => cache.standardize_distance(path, travelled),
            Units::Normalized => {
                if cache.length < EPSILON { 0.0 } else {
                    cache.standardize_distance(path, cache.length * travelled) / cache.length
                }
            }
        };
        self.position = standard;
        let native = match self.cart.units {
            Units::Path => standard,
            _ if cache.length < EPSILON => 0.0,
            units => {
                let mut distance = standard;
                if units == Units::Normalized { distance *= cache.length; }
                let distance = cache.standardize_distance(path, distance) / cache.distance_step;
                let index = distance as usize;
                if index >= cache.distance_to_pos.len() - 1 {
                    path.max_pos()
                } else {
                    let before = cache.distance_to_pos[index];
                    let fraction = (distance - index as f32).max(0.0);
                    before + fraction * (cache.distance_to_pos[index + 1] - before)
                }
            }
        };
        let position = path.world_position(path_world, native);
        let (_, path_rotation, _) = path_world.to_scale_rotation_translation();
        let rotation = path_rotation * moly_assets::coordinates::source_rotation(path.local_orientation(native)?);
        Ok((position, rotation))
    }
}
