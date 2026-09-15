//! Actor GLBs use the shared glTF X reflection; fixture geometry and locators
//! retain authored Unity coordinates. Bridge those coordinate frames only for
//! a fixture-owned performance, including mesh winding. No fixture-specific
//! angle or actor animation data is changed.
use super::*;
use bevy::mesh::{Indices, PrimitiveTopology};

#[derive(Component)]
struct ActorSpaceOwner(TimelineToken);
pub(super) struct ActorSpaceLease {
    model: Entity,
    transform: Transform,
    meshes: Vec<(Entity, Handle<Mesh>, Handle<Mesh>)>,
}
pub(super) fn acquire(
    world: &mut World,
    actor: Entity,
    token: TimelineToken,
) -> Result<ActorSpaceLease, TimelineFailure> {
    let models: Vec<_> = world
        .query_filtered::<Entity, With<crate::character::CharacterModel>>()
        .iter(world)
        .filter(|entity| {
            world
                .get::<ChildOf>(*entity)
                .is_some_and(|parent| parent.0 == actor)
        })
        .collect();
    let [model] = models.as_slice() else {
        return Err(invalid(
            "actor coordinate bridge requires one actual character model",
        ));
    };
    let model = *model;
    if world.get::<ActorSpaceOwner>(model).is_some() {
        return Err(invalid("actor coordinate bridge already owned"));
    }
    let transform = *world
        .get::<Transform>(model)
        .ok_or_else(|| invalid("character model transform is unavailable"))?;
    // CharacterModel is the unanimated scene wrapper. Imported source root and
    // all bone channels remain beneath it, so no sampler overwrites this bridge.
    if transform.translation != Vec3::ZERO || transform.rotation != Quat::IDENTITY {
        return Err(invalid(
            "actor coordinate wrapper has an unowned transform offset",
        ));
    }
    let rows: Vec<_> = world
        .query::<(Entity, &Mesh3d)>()
        .iter(world)
        .filter(|(entity, _)| descendant_of(world, *entity, model))
        .map(|(entity, mesh)| (entity, mesh.0.clone()))
        .collect();
    let mut prepared = Vec::new();
    for (entity, handle) in rows {
        let mesh = world
            .resource::<Assets<Mesh>>()
            .get(&handle)
            .ok_or_else(|| TimelineFailure::loading("actor mesh still loading"))?;
        // Vertices, normals, tangents and inverse bindposes stay in the
        // imported frame: joint world matrices acquire Sx through their parent,
        // while inverseBindpose already contains Sx * IBM * Sx. Reflection is
        // therefore applied once, not once per channel. This toon shader does
        // not consume tangent-space normals; imported tangent data is retained.
        let mut reflected = mesh.clone();
        reverse_winding(&mut reflected)?;
        prepared.push((entity, handle, reflected));
    }
    let mut meshes = Vec::new();
    for (entity, original, reflected) in prepared {
        let reflected = world.resource_mut::<Assets<Mesh>>().add(reflected);
        world.entity_mut(entity).insert(Mesh3d(reflected.clone()));
        meshes.push((entity, original, reflected));
    }
    let reflected = Transform {
        scale: transform.scale * Vec3::new(-1., 1., 1.),
        ..transform
    };
    world
        .entity_mut(model)
        .insert((reflected, ActorSpaceOwner(token)));
    Ok(ActorSpaceLease {
        model,
        transform,
        meshes,
    })
}
pub(super) fn restore(world: &mut World, token: TimelineToken, lease: ActorSpaceLease) {
    if !world
        .get::<ActorSpaceOwner>(lease.model)
        .is_some_and(|owner| owner.0 == token)
    {
        return;
    }
    for (entity, original, reflected) in lease.meshes {
        if world
            .get::<Mesh3d>(entity)
            .is_some_and(|mesh| mesh.0 == reflected)
        {
            world.entity_mut(entity).insert(Mesh3d(original));
        }
    }
    world
        .entity_mut(lease.model)
        .insert(lease.transform)
        .remove::<ActorSpaceOwner>();
}
fn reverse_winding(mesh: &mut Mesh) -> Result<(), TimelineFailure> {
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return Err(invalid("actor coordinate bridge needs triangle geometry"));
    }
    if mesh.indices().is_none() {
        mesh.insert_indices(Indices::U32((0..mesh.count_vertices() as u32).collect()));
    }
    match mesh.indices_mut().unwrap() {
        Indices::U16(indices) => {
            for triangle in indices.chunks_exact_mut(3) {
                triangle.swap(1, 2);
            }
        }
        Indices::U32(indices) => {
            for triangle in indices.chunks_exact_mut(3) {
                triangle.swap(1, 2);
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_space_bridge_cancels_import_reflection_under_any_locator_yaw() {
        let reflection = Mat4::from_scale(Vec3::new(-1., 1., 1.));
        let source_pose = Mat4::from_rotation_translation(
            Quat::from_rotation_y(0.9),
            Vec3::new(0.04, 0.35, 0.55),
        );
        let imported = reflection * source_pose * reflection;
        let source_vertex = Vec3::new(0.1, 0.3, -0.2);
        let imported_vertex = reflection.transform_point3(source_vertex);
        for yaw in [-1.57, 0., 0.61, 3.14] {
            let locator = Mat4::from_rotation_translation(
                Quat::from_rotation_y(yaw),
                Vec3::new(0.5, 0., 0.08),
            );
            let wanted = (locator * source_pose).transform_point3(source_vertex);
            let actual = (locator * reflection * imported).transform_point3(imported_vertex);
            assert!(wanted.distance(actual) < 0.00001);
        }
    }
    #[test]
    fn winding_bridge_preserves_geometry_and_reverses_only_orientation() {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2, 2, 3, 0]));
        reverse_winding(&mut mesh).unwrap();
        assert_eq!(
            mesh.indices().unwrap().iter().collect::<Vec<_>>(),
            vec![0, 2, 1, 2, 0, 3]
        );
    }
    #[test]
    fn bridge_restores_exact_handles_and_does_not_overwrite_a_new_generation() {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        let actor = world.spawn_empty().id();
        let model = world
            .spawn((
                crate::character::CharacterModel,
                Transform::default(),
                ChildOf(actor),
            ))
            .id();
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
        let original = world.resource_mut::<Assets<Mesh>>().add(mesh);
        let drawn = world.spawn((Mesh3d(original.clone()), ChildOf(model))).id();
        let token = TimelineToken {
            serial: 1,
            owner: FixtureActivityOwner {
                actor,
                generation: 1,
            },
        };
        let lease = acquire(&mut world, actor, token).unwrap();
        assert_eq!(
            world.get::<Transform>(model).unwrap().scale,
            Vec3::new(-1., 1., 1.)
        );
        assert_ne!(world.get::<Mesh3d>(drawn).unwrap().0, original);
        restore(&mut world, token, lease);
        assert_eq!(world.get::<Transform>(model).unwrap().scale, Vec3::ONE);
        assert_eq!(world.get::<Mesh3d>(drawn).unwrap().0, original);
        assert!(world.get::<ActorSpaceOwner>(model).is_none());
        let lease = acquire(&mut world, actor, token).unwrap();
        let replacement = TimelineToken {
            serial: 2,
            owner: FixtureActivityOwner {
                actor,
                generation: 2,
            },
        };
        world.entity_mut(model).insert(ActorSpaceOwner(replacement));
        restore(&mut world, token, lease);
        assert_eq!(world.get::<ActorSpaceOwner>(model).unwrap().0, replacement);
        assert_eq!(
            world.get::<Transform>(model).unwrap().scale,
            Vec3::new(-1., 1., 1.)
        );
    }
}
