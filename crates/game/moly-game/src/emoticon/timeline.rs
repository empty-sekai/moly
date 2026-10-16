//! Source Timeline intervals use the existing emoticon renderer. A lease is
//! the renderer's monotonically increasing instance key, so late cleanup can
//! never hide a newer Rest/talk/Timeline emoticon on the same character.
use super::*;
use bevy::ecs::system::{SystemParam, SystemState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EmoteLease(u64);
#[derive(SystemParam)]
struct Presentation<'w, 's> {
    commands: Commands<'w, 's>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<EmoticonMaterial>>,
    emotes: ResMut<'w, Emotes>,
    archive: Res<'w, EmoticonArchive>,
    globals: Query<'w, 's, &'static GlobalTransform>,
    children: Query<'w, 's, &'static Children>,
    names: Query<'w, 's, &'static Name>,
    cameras: Query<
        'w,
        's,
        (
            &'static GlobalTransform,
            &'static Projection,
            &'static Camera,
        ),
        With<Camera3d>,
    >,
}

pub(crate) fn show(
    world: &mut World,
    actor: Entity,
    name: &str,
    use_root: bool,
) -> Result<EmoteLease, String> {
    let mut state = SystemState::<Presentation>::new(world);
    let lease = {
        let mut p = state.get_mut(world);
        if !p.archive.has(name) {
            return Err(format!("source emoticon {name} is unavailable"));
        }
        show_emote(
            &mut p.commands,
            &mut p.meshes,
            &mut p.materials,
            &p.globals,
            &p.children,
            &p.names,
            &p.cameras,
            &mut p.emotes,
            &p.archive.archive,
            name,
            actor,
            0.,
        );
        let instance = p
            .emotes
            .instances
            .iter_mut()
            .find(|instance| instance.npc == actor)
            .ok_or_else(|| "source emoticon did not create a rendered instance".to_owned())?;
        // NPCAvatarView.SetEmoticonTransform uses the logical root when true,
        // HipsRoot when false. This changes particle keepPosition orientation;
        // the authored Face/Spine/Hips parent anchor remains unchanged.
        instance.timeline_rotation_reference = if use_root {
            Some(actor)
        } else {
            instance.mounts.hips
        };
        EmoteLease(instance.key)
    };
    state.apply(world);
    Ok(lease)
}

pub(crate) fn hide(world: &mut World, lease: EmoteLease, immediate: bool) {
    if !world.contains_resource::<Emotes>() || !world.contains_resource::<EmoticonArchive>() {
        return;
    }
    let mut state = SystemState::<Presentation>::new(world);
    {
        let mut p = state.get_mut(world);
        if let Some(index) = p
            .emotes
            .instances
            .iter()
            .position(|instance| instance.key == lease.0)
        {
            if immediate {
                clear_instance(
                    &mut p.commands,
                    &mut p.meshes,
                    &mut p.materials,
                    &mut p.emotes,
                    index,
                );
            } else {
                let clips = p.archive.archive.items[p.emotes.instances[index].item]
                    .clips
                    .as_ref();
                request_hide(&mut p.emotes.instances[index], clips);
            }
        }
    }
    state.apply(world);
}
