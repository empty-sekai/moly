//! What a falling body meets: the ground under it and the furniture placed
//! around it.
//!
//! The scene is made when a character falls, placed where it falls:
//!
//! - the ground is a solid box under the character whose top is the height the
//!   caller found the floor at (the site's navigation surface under the
//!   character, or the character's own feet where there is none), so a giant
//!   standing on the floor falls to the same floor;
//! - each placed piece of furniture near the character gets a box from the
//!   bounds of the meshes that make up its model, where they stand.
//!
//! Every length here is read per metre of the falling character's own height:
//! the reach around it, the width of the floor and its thickness. The scene is
//! removed when the character gets up.
//!
//! A part collides with the scene and with the parts of the other characters
//! that are falling, but not with the other parts of its own body: every part
//! names the character it belongs to ([`PartOf`]) and the pair filter
//! ([`OwnBody`]) drops a pair of parts of the same character, and the
//! collision layers keep the falling parts off the world's own colliders (the
//! bodies of the characters that stand are not in the scene at all).

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::mesh::skinning::SkinnedMesh;

/// The character a part belongs to.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartOf(pub Entity);

/// The pair filter of the physics: two parts of the same character never
/// collide, so parts laid out overlapping (an arm against the chest, the two
/// thighs) do not push each other apart.
#[derive(bevy::ecs::system::SystemParam)]
pub struct OwnBody<'w, 's> {
    parts: Query<'w, 's, &'static PartOf>,
}

impl CollisionHooks for OwnBody<'_, '_> {
    fn filter_pairs(&self, collider1: Entity, collider2: Entity, _commands: &mut Commands) -> bool {
        match self.parts.get_many([collider1, collider2]) {
            Ok([first, second]) => first != second,
            Err(_) => true,
        }
    }
}

/// The layer the parts of every falling body belong to.
pub const RAGDOLL_PARTS: LayerMask = LayerMask(1 << 20);

/// The layer the ground and the furniture of a falling character stand on.
pub const RAGDOLL_SCENE: LayerMask = LayerMask(1 << 21);

/// How far from the character the furniture is taken in, as a share of the
/// character's height: near enough to fall over.
const FURNITURE_REACH: f32 = 3.0;
/// How wide the floor under a character is, as a share of its height.
const FLOOR_WIDE: f32 = 40.0;
/// How thick the floor under a character is, as a share of its height: solid,
/// so that a fast thin limb cannot pass through it.
const FLOOR_THICK: f32 = 5.0;
/// How wide another falling character's box is, as a share of its height.
const OTHER_WIDE: f32 = 0.8;

/// The collision layers of a falling body's parts: they meet the scene and
/// every other falling part, whatever the body.
pub fn part_layers() -> CollisionLayers {
    CollisionLayers::new(RAGDOLL_PARTS, RAGDOLL_SCENE | RAGDOLL_PARTS)
}

/// The collision layers of the scene a body falls in: it meets the falling
/// parts and nothing else.
pub fn scene_layers() -> CollisionLayers {
    CollisionLayers::new(RAGDOLL_SCENE, RAGDOLL_PARTS)
}

/// The static shapes one falling character needs.
pub struct Scene {
    pub bodies: Vec<SceneBody>,
}

/// One static shape of a falling character's scene.
pub struct SceneBody {
    pub transform: Transform,
    pub half: Vec3,
}

impl SceneBody {
    fn collider(&self) -> Collider {
        Collider::cuboid(self.half.x * 2.0, self.half.y * 2.0, self.half.z * 2.0)
    }
}

/// The floor under a character of `height` whose feet were at `feet`: a box
/// whose top is `floor`.
pub fn floor(floor: f32, feet: Vec3, height: f32) -> SceneBody {
    let thick = height * FLOOR_THICK;
    SceneBody {
        transform: Transform::from_translation(Vec3::new(feet.x, floor - thick, feet.z)),
        half: Vec3::new(height * FLOOR_WIDE * 0.5, thick, height * FLOOR_WIDE * 0.5),
    }
}

/// Builds a character's scene: the ground under it, the furniture within its
/// reach, and the other bodies that are falling near it.
pub fn build(world: &mut World, center: Vec3, feet: Vec3, height: f32, floor_at: f32, own: Entity) -> Scene {
    let mut bodies = vec![floor(floor_at, feet, height)];
    let reach = height * FURNITURE_REACH;

    let roots: Vec<Entity> = world.query_filtered::<Entity, With<moly_features::fixture::FixtureRoot>>().iter(world).collect();
    let mut collected = Vec::new();
    {
        // The queries are made on the world, then walk it by a read of it
        // while the roots are walked.
        let mut children = world.query::<&Children>();
        let mut meshes = world.query_filtered::<(&GlobalTransform, &Mesh3d), (With<Mesh3d>, Without<SkinnedMesh>)>();
        let world: &World = world;
        for root in roots {
            let Some(place) = world.get::<GlobalTransform>(root).copied() else {
                continue;
            };
            if place.translation().distance(center) > reach {
                continue;
            }
            let mut stack = vec![root];
            while let Some(entity) = stack.pop() {
                if let Ok(kids) = children.get(world, entity) {
                    stack.extend(kids.iter());
                }
                if let Ok((place, mesh)) = meshes.get(world, entity) {
                    collected.push((*place, mesh.0.id()));
                }
            }
        }
    }
    {
        let Some(mesh_assets) = world.get_resource::<Assets<Mesh>>() else {
            return Scene { bodies };
        };
        for (place, id) in collected {
            let Some(bounds) = mesh_assets.get(id).and_then(|mesh| mesh.final_aabb) else {
                continue;
            };
            let middle = place.transform_point(bounds.center().into());
            let half = Vec3::from(bounds.half_size()) * place.scale().abs();
            if half.length_squared() < 1e-6 {
                continue;
            }
            bodies.push(SceneBody { transform: Transform::from_translation(middle), half });
        }
    }

    // The other falling characters: a box the size of their own body.
    let others: Vec<(Vec3, f32)> = {
        let mut falling = world.query_filtered::<Entity, With<crate::drop::Drop>>();
        falling
            .iter(world)
            .filter(|entity| *entity != own)
            .filter_map(|other| {
                let drop = world.get::<crate::drop::Drop>(other)?;
                let at = drop.root_part().frame(world).map_or(drop.root_part().position, |(position, _)| position);
                Some((at, drop.height))
            })
            .collect()
    };
    for (at, other_height) in others {
        if at.distance(center) > reach + other_height {
            continue;
        }
        let half = Vec3::new(other_height * OTHER_WIDE * 0.5, other_height * 0.5, other_height * OTHER_WIDE * 0.5);
        bodies.push(SceneBody { transform: Transform::from_translation(at + Vec3::new(0.0, other_height * 0.25, 0.0)), half });
    }
    Scene { bodies }
}

/// Places a scene's bodies in the world as static bodies on the scene layer.
pub fn spawn(world: &mut World, scene: Scene) -> Vec<Entity> {
    let mut bodies = Vec::with_capacity(scene.bodies.len());
    for body in scene.bodies {
        let entity = world
            .spawn((
                RigidBody::Static,
                body.collider(),
                body.transform,
                GlobalTransform::from(body.transform),
                Position::from(body.transform.translation),
                Rotation::from(body.transform.rotation),
                scene_layers(),
            ))
            .id();
        bodies.push(entity);
    }
    bodies
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_top_is_the_ground_the_character_stands_on() {
        let slab = floor(1.5, Vec3::new(3.0, 1.5, -2.0), 20.0);
        assert!((slab.transform.translation.y + slab.half.y - 1.5).abs() < 1e-5);
        // A giant's floor is as wide and as thick as the giant is tall.
        assert!(slab.half.x > 100.0 && slab.half.y > 10.0);
    }

    /// A small ball dropped on a character's floor in a world of its own: the
    /// app the ragdoll installs, stepped at sixty frames a second.
    fn ball_on_floor(radius: f32, height: f32, frames: u32) -> Vec3 {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f32(1.0 / 60.0)));
        crate::add_physics(&mut app);
        app.finish();
        app.cleanup();
        app.update();
        let world = app.world_mut();
        spawn(world, Scene { bodies: vec![floor(0.0, Vec3::ZERO, height)] });
        let start = Transform::from_xyz(0.0, height * 0.5, 0.0);
        let ball = world
            .spawn((RigidBody::Dynamic, Collider::sphere(radius), start, GlobalTransform::from(start), Position::from(start.translation), part_layers(), PartOf(Entity::PLACEHOLDER), ActiveCollisionHooks::FILTER_PAIRS))
            .id();
        for _ in 0..frames {
            app.update();
        }
        app.world().get::<Position>(ball).map_or(Vec3::NAN, |position| position.0)
    }

    #[test]
    fn a_part_comes_to_rest_on_the_floor() {
        for (radius, height) in [(0.2, 1.7), (0.01, 0.72)] {
            let rest = ball_on_floor(radius, height, 240);
            assert!((rest.y - radius).abs() < radius * 0.2, "a ball of radius {radius} rests at {rest}");
        }
    }

    #[test]
    fn parts_meet_the_scene_and_each_other_whatever_the_body() {
        let parts = part_layers();
        let scene = scene_layers();
        // Two bodies' parts collide with each other.
        assert!(parts.interacts_with(parts));
        // The parts collide with the ground and the furniture.
        assert!(parts.interacts_with(scene));
        // The world's own colliders are on the default layer and stay clear.
        assert!(!parts.interacts_with(CollisionLayers::DEFAULT));
    }
}
