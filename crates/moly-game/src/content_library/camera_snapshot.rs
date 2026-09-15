//! Restore all camera state after free exploration in a temporary scene.
//! Restoring Transform alone is overwritten by the next follow-avatar frame.
use crate::camera::{
    CameraTween, FieldCameraModel, FieldCameraState, FpsViewMemory, NormalCameraMemory,
};
use bevy::prelude::*;
#[derive(Clone)]
pub(super) struct CameraSnapshot {
    model: Option<FieldCameraModel>,
    state: Option<FieldCameraState>,
    fps: Option<FpsViewMemory>,
    normal: Option<NormalCameraMemory>,
    tween: Option<CameraTween>,
    projections: Vec<(Entity, Projection)>,
    visibility: Vec<(Entity, Visibility)>,
}
impl CameraSnapshot {
    pub(super) fn capture(world: &mut World) -> Self {
        Self {
            model: world.get_resource::<FieldCameraModel>().cloned(),
            state: world.get_resource::<FieldCameraState>().copied(),
            fps: world.get_resource::<FpsViewMemory>().copied(),
            normal: world.get_resource::<NormalCameraMemory>().cloned(),
            tween: world.get_resource::<CameraTween>().copied(),
            projections: world
                .query_filtered::<(Entity, &Projection), With<Camera3d>>()
                .iter(world)
                .map(|(entity, projection)| (entity, projection.clone()))
                .collect(),
            visibility: world
                .query_filtered::<(Entity, &Visibility), With<crate::character::AvatarRoot>>()
                .iter(world)
                .map(|(entity, visibility)| (entity, *visibility))
                .collect(),
        }
    }
    /// The temporary courtyard starts in the normal third-person view even
    /// when the saved original scene was first person. The snapshot retains
    /// the original lens, FPS memory and avatar visibility for return.
    pub(super) fn prepare_temporary(world: &mut World) {
        world.insert_resource(FieldCameraState(crate::camera::CameraStateType::Normal));
        world.remove_resource::<CameraTween>();
        world.remove_resource::<crate::menu_shell::CameraResetTween>();
        world.remove_resource::<NormalCameraMemory>();
        let fov = world
            .get_resource::<crate::camera::CameraSetting>()
            .map(|setting| setting.fov.to_radians());
        if let Some(fov) = fov {
            for mut projection in world
                .query_filtered::<&mut Projection, With<Camera3d>>()
                .iter_mut(world)
            {
                if let Projection::Perspective(projection) = &mut *projection {
                    projection.fov = fov;
                }
            }
        }
        for mut visibility in world
            .query_filtered::<&mut Visibility, With<crate::character::AvatarRoot>>()
            .iter_mut(world)
        {
            *visibility = Visibility::Visible;
        }
    }
    pub(super) fn restore(&self, world: &mut World) {
        replace(world, self.model.clone());
        replace(world, self.state);
        replace(world, self.fps);
        replace(world, self.normal.clone());
        replace(world, self.tween);
        // A reset begun inside the preview must not overwrite restored values.
        world.remove_resource::<crate::menu_shell::CameraResetTween>();
        for (entity, projection) in &self.projections {
            if let Ok(mut entity) = world.get_entity_mut(*entity) {
                entity.insert(projection.clone());
            }
        }
        for (entity, visibility) in &self.visibility {
            if let Ok(mut entity) = world.get_entity_mut(*entity) {
                entity.insert(*visibility);
            }
        }
    }
}
fn replace<T: Resource>(world: &mut World, value: Option<T>) {
    if let Some(value) = value {
        world.insert_resource(value);
    } else {
        world.remove_resource::<T>();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn return_restores_first_person_state_projection_and_player_visibility() {
        let mut world = World::new();
        world.insert_resource(FieldCameraState(crate::camera::CameraStateType::Normal));
        world.insert_resource(FpsViewMemory {
            yaw: 12.,
            pitch: 3.,
        });
        let player = world
            .spawn((crate::character::AvatarRoot, Visibility::Visible))
            .id();
        let camera = world
            .spawn((
                Camera3d::default(),
                Projection::Perspective(PerspectiveProjection::default()),
            ))
            .id();
        let snapshot = CameraSnapshot::capture(&mut world);
        world.insert_resource(FieldCameraState(crate::camera::CameraStateType::Fps));
        world.insert_resource(FpsViewMemory {
            yaw: 99.,
            pitch: 40.,
        });
        world.entity_mut(player).insert(Visibility::Hidden);
        world
            .entity_mut(camera)
            .insert(Projection::Perspective(PerspectiveProjection {
                fov: 0.2,
                ..default()
            }));
        snapshot.restore(&mut world);
        assert_eq!(
            world.resource::<FieldCameraState>().0,
            crate::camera::CameraStateType::Normal
        );
        assert_eq!(world.resource::<FpsViewMemory>().yaw, 12.);
        assert_eq!(
            *world.get::<Visibility>(player).unwrap(),
            Visibility::Visible
        );
        let Projection::Perspective(projection) = world.get::<Projection>(camera).unwrap() else {
            panic!()
        };
        assert_eq!(projection.fov, PerspectiveProjection::default().fov);
    }
    #[test]
    fn temporary_scene_normalizes_fps_without_losing_saved_first_person_return() {
        let mut world = World::new();
        world.insert_resource(FieldCameraState(crate::camera::CameraStateType::Fps));
        let player = world
            .spawn((crate::character::AvatarRoot, Visibility::Hidden))
            .id();
        let snapshot = CameraSnapshot::capture(&mut world);
        CameraSnapshot::prepare_temporary(&mut world);
        assert_eq!(
            world.resource::<FieldCameraState>().0,
            crate::camera::CameraStateType::Normal
        );
        assert_eq!(
            *world.get::<Visibility>(player).unwrap(),
            Visibility::Visible
        );
        snapshot.restore(&mut world);
        assert_eq!(
            world.resource::<FieldCameraState>().0,
            crate::camera::CameraStateType::Fps
        );
        assert_eq!(
            *world.get::<Visibility>(player).unwrap(),
            Visibility::Hidden
        );
    }
}
