//! BulkHarvestToolTimelineView's attached hammer and field-camera binding.
use super::*;
use crate::camera::{self, CameraStateType, FieldCameraModel, FieldCameraState};

pub(super) struct Entry {
    tool: Box<StepEntry>,
    document: String,
    pub(super) camera_node: String,
}

impl Entry {
    pub(super) fn parse(value: Option<&Value>) -> Result<Option<Self>, String> {
        let Some(value) = value.filter(|value| !value.is_null()) else {
            return Ok(None);
        };
        let leaf = |row: &Value, key: &str| -> Result<String, String> {
            row[key]
                .as_str()
                .filter(|value| !value.is_empty() && !value.contains(['/', '\\', ':']))
                .map(str::to_owned)
                .ok_or_else(|| format!("bulk {key} is not a plain name"))
        };
        let tool = &value["tool"];
        if value["camera"]["rotationBasis"].as_str() != Some("unity-transform-reflect-x")
            || value["camera"]["cameraForwardCorrection"].as_str() != Some("runtime-y-pi")
        {
            return Err("bulk camera proxy basis is missing or unsupported".into());
        }
        Ok(Some(Self {
            tool: Box::new(StepEntry {
                package: leaf(tool, "package")?,
                prefab: String::new(),
                glb: format!("{STEP_ITEM_ROOT}{}", leaf(tool, "glb")?),
                clips: Arc::new(tool["clips"].clone()),
                view_director: None,
                bulk: None,
            }),
            document: format!("{STEP_ITEM_ROOT}{}", leaf(tool, "document")?),
            camera_node: leaf(&value["camera"], "node")?,
        }))
    }
}

#[derive(Default)]
pub(super) struct Object {
    gltf: Option<Handle<Gltf>>,
    document: Option<Handle<JsonAsset>>,
    tool: Option<Entity>,
    pub(super) camera_proxy: Option<Entity>,
}

impl Object {
    pub(super) fn material_model(&self) -> Option<(Entity, Handle<JsonAsset>)> {
        Some((self.tool?, self.document.clone()?))
    }

    pub(super) fn retire(&mut self, world: &mut World) {
        if let Some(tool) = self.tool.take() {
            if let Ok(entity) = world.get_entity_mut(tool) {
                entity.despawn();
            }
        }
    }
}

fn one_named(world: &World, root: Entity, wanted: &str) -> Result<Entity, Pending> {
    let mut stack = vec![root];
    let mut found = Vec::new();
    while let Some(entity) = stack.pop() {
        if world
            .get::<Name>(entity)
            .is_some_and(|name| name.as_str() == wanted)
        {
            found.push(entity);
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    match found.as_slice() {
        [entity] => Ok(*entity),
        _ => Err(Pending::Refused(format!(
            "bulk binding {wanted:?} has {} matching nodes",
            found.len()
        ))),
    }
}

pub(super) fn prepare_object(
    world: &mut World,
    item: &mut StepItem,
    view: Entity,
) -> Result<(), Pending> {
    let Some(entry) = item.entry.as_ref().and_then(|entry| entry.bulk.as_ref()) else {
        return Ok(());
    };
    let server = world.resource::<AssetServer>().clone();
    let gltf = item
        .bulk
        .gltf
        .get_or_insert_with(|| server.load(format!("moly://{}", entry.tool.glb)))
        .clone();
    item.bulk
        .document
        .get_or_insert_with(|| server.load(format!("moly://{}", entry.document)));
    if let LoadState::Failed(error) = server.load_state(&gltf) {
        return Err(Pending::Refused(format!("bulk hammer model: {error}")));
    }
    if item.bulk.tool.is_none() {
        let Some(asset) = world.resource::<Assets<Gltf>>().get(&gltf) else {
            return Err(Pending::Wait);
        };
        let scene = asset
            .default_scene
            .clone()
            .ok_or_else(|| Pending::Refused("bulk hammer has no scene".into()))?;
        let (actor, root) = world
            .query_filtered::<(Entity, &AvatarDriver), With<PlayerControlled>>()
            .single(world)
            .map(|(actor, driver)| (actor, driver.visual_root))
            .map_err(|_| Pending::Wait)?;
        let mount = one_named(world, root, "Penlight_R")?;
        item.bulk.tool = Some(
            world
                .spawn((
                    WorldAssetRoot(scene),
                    Transform::IDENTITY,
                    Visibility::Inherited,
                    ChildOf(mount),
                    timeline::StepAttachment { view, actor },
                ))
                .id(),
        );
    }
    let tool = item.bulk.tool.expect("spawned above");
    let Some(instance) = world.get::<WorldInstance>(tool) else {
        return Err(Pending::Wait);
    };
    if !world
        .resource::<WorldInstanceSpawner>()
        .instance_is_ready(**instance)
    {
        return Err(Pending::Wait);
    }
    install_graphs(world, tool);
    let ObjectState::Spawned(object) = item.object else {
        return Err(Pending::Wait);
    };
    let proxy = one_named(world, object, &entry.camera_node)?;
    let actor = world
        .get::<timeline::StepAttachment>(tool)
        .expect("attached above")
        .actor;
    world
        .entity_mut(proxy)
        .insert(timeline::StepAttachment { view, actor });
    install_graphs(world, proxy);
    item.bulk.camera_proxy = Some(proxy);
    Ok(())
}

pub(super) fn bind(
    world: &mut World,
    request: &mut StartTimeline,
    item: &StepItem,
) -> Result<(), Pending> {
    let Some(entry) = item.entry.as_ref().and_then(|entry| entry.bulk.as_ref()) else {
        return Ok(());
    };
    let tool = item.bulk.tool.ok_or(Pending::Wait)?;
    let gltf = item.bulk.gltf.as_ref().ok_or(Pending::Wait)?;
    // Reuse the identity/animation-target checks with only Fixture1 selected.
    let mut tool_request = request.clone();
    let mut definition = (*request.definition).clone();
    definition
        .tracks
        .retain(|track| track.class == "AnimationTrack" && track.name == "Fixture1");
    tool_request.definition = Arc::new(definition);
    tool_request.fixture = tool;
    tool_request.bindings = TimelineBindings::default();
    prepare_prop_bindings(world, &mut tool_request, &entry.tool, gltf)?;
    for track in &tool_request.definition.tracks {
        request
            .bindings
            .step_objects
            .insert(track.identity.clone(), tool);
    }
    request
        .bindings
        .animations
        .extend(tool_request.bindings.animations);
    let proxy = item.bulk.camera_proxy.ok_or(Pending::Wait)?;
    let mut camera_request = request.clone();
    let mut definition = (*request.definition).clone();
    definition.tracks.retain(|track| {
        track.class == "AnimationTrack" && track.name.to_ascii_lowercase().contains("camera")
    });
    camera_request.definition = Arc::new(definition);
    camera_request.fixture = proxy;
    camera_request.bindings = TimelineBindings::default();
    let original = item.entry.as_ref().expect("loaded entry");
    let camera_entry = StepEntry {
        package: original.package.clone(),
        prefab: original.prefab.clone(),
        glb: original.glb.clone(),
        clips: original.clips.clone(),
        view_director: None,
        bulk: None,
    };
    prepare_prop_bindings(
        world,
        &mut camera_request,
        &camera_entry,
        item.gltf.as_ref().ok_or(Pending::Wait)?,
    )?;
    for track in &camera_request.definition.tracks {
        request
            .bindings
            .step_objects
            .insert(track.identity.clone(), proxy);
    }
    request
        .bindings
        .animations
        .extend(camera_request.bindings.animations);
    let blur = one_named(world, tool, "fx_hit_HammerBlur")?;
    for clip in request
        .definition
        .tracks
        .iter()
        .filter(|track| track.name == "HammerBlurEffectControl")
        .flat_map(|track| &track.clips)
    {
        request.bindings.step_effects.insert(
            clip.key.clone(),
            timeline::StepEffectBinding {
                owner: tool,
                object: blur,
                package: entry.tool.package.clone(),
            },
        );
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum CameraPhase {
    Follow,
    Hold,
    Return(Option<f64>),
    Inherit,
}

pub(super) struct Camera {
    proxy: Entity,
    enter: Transform,
    last: Transform,
    entered_at: f64,
    fixed_yaw: f32,
    phase: CameraPhase,
}

impl Camera {
    pub(super) fn enter(world: &mut World, proxy: Option<Entity>) -> Result<Option<Self>, String> {
        let Some(proxy) = proxy else {
            return Ok(None);
        };
        let enter = *world
            .query_filtered::<&Transform, With<Camera3d>>()
            .single(world)
            .map_err(|_| "bulk needs one field camera")?;
        let model = world
            .get_resource::<FieldCameraModel>()
            .ok_or("bulk field camera model is absent")?;
        let delta = enter.translation - model.look_at - model.offset;
        let fixed_yaw = delta.x.atan2(delta.z);
        let previous = world.resource::<FieldCameraState>().0;
        camera::exit_state(world, previous, "bulk-camera");
        world.resource_mut::<FieldCameraState>().0 = CameraStateType::BulkHarvest;
        Ok(Some(Self {
            proxy,
            enter,
            last: enter,
            fixed_yaw,
            entered_at: world.resource::<Time<Real>>().elapsed_secs_f64(),
            phase: CameraPhase::Follow,
        }))
    }

    pub(super) fn hold(&mut self) {
        if matches!(self.phase, CameraPhase::Follow) {
            self.phase = CameraPhase::Hold;
        }
    }

    pub(super) fn return_to_normal(&mut self) {
        if !matches!(self.phase, CameraPhase::Return(_) | CameraPhase::Inherit) {
            self.phase = CameraPhase::Return(None);
        }
    }

    pub(super) fn inherit(&mut self) {
        self.phase = CameraPhase::Inherit;
    }
}

fn smooth(value: f32) -> f32 {
    let t = value.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn blend(from: Transform, to: Transform, weight: f32) -> Transform {
    Transform {
        translation: from.translation.lerp(to.translation, weight),
        rotation: from.rotation.slerp(to.rotation, weight),
        scale: Vec3::ONE,
    }
}

fn fixed_yaw_pose(proxy: Transform, anchor: Vec3, yaw: f32) -> Transform {
    let offset = proxy.translation - anchor;
    let rotation = Quat::from_rotation_y(yaw - offset.x.atan2(offset.z));
    Transform {
        translation: anchor + rotation * offset,
        rotation: rotation * proxy.rotation * Quat::from_rotation_y(std::f32::consts::PI),
        scale: Vec3::ONE,
    }
}

/// After transform propagation and the normal camera writer; before billboard readers.
pub fn advance_camera(world: &mut World) {
    let Some(mut service) = world.remove_resource::<PlayerStepItem>() else {
        return;
    };
    if let Some(mut bulk) = service.bulk_camera.take() {
        if world.resource::<FieldCameraState>().0 == CameraStateType::BulkHarvest {
            let now = world.resource::<Time<Real>>().elapsed_secs_f64();
            let model = world.resource::<FieldCameraModel>();
            let anchor = model.look_at + model.offset;
            let mut complete = false;
            let pose = match bulk.phase {
                CameraPhase::Follow => {
                    if let Some(proxy) = world.get::<GlobalTransform>(bulk.proxy) {
                        // The proxy retains reflected Unity Transform curves. The
                        // field camera looks along -Z, so change its local basis once.
                        let target =
                            fixed_yaw_pose(proxy.compute_transform(), anchor, bulk.fixed_yaw);
                        blend(
                            bulk.enter,
                            target,
                            smooth(((now - bulk.entered_at) / 0.5) as f32),
                        )
                    } else {
                        bulk.hold();
                        bulk.last
                    }
                }
                CameraPhase::Hold => bulk.last,
                CameraPhase::Return(start) => {
                    let start = start.unwrap_or(now);
                    bulk.phase = CameraPhase::Return(Some(start));
                    let t = ((now - start) / 0.3) as f32;
                    let target = Transform::from_translation(
                        anchor + camera::view_dir(model.pitch, model.yaw) * model.distance,
                    )
                    .looking_at(anchor, Vec3::Y);
                    complete = t >= 1.0;
                    blend(bulk.last, target, smooth(t))
                }
                CameraPhase::Inherit => {
                    let offset = bulk.last.translation - anchor;
                    let distance = offset.length();
                    let mut model = world.resource_mut::<FieldCameraModel>();
                    if distance >= 0.01 {
                        model.pitch = (offset.y / distance).clamp(-1.0, 1.0).asin().to_degrees();
                        model.yaw = offset.x.atan2(-offset.z).to_degrees();
                        model.distance = distance.clamp(model.min_distance, model.max_distance);
                    }
                    complete = true;
                    bulk.last
                }
            };
            if let Ok(mut camera) = world
                .query_filtered::<&mut Transform, With<Camera3d>>()
                .single_mut(world)
            {
                *camera = pose;
            }
            if !matches!(bulk.phase, CameraPhase::Return(_) | CameraPhase::Inherit) {
                bulk.last = pose;
            }
            if complete {
                camera::enter_state(
                    world,
                    CameraStateType::Normal,
                    CameraStateType::BulkHarvest,
                    "bulk-camera",
                );
            } else {
                service.bulk_camera = Some(bulk);
            }
        }
    }
    world.insert_resource(service);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_blend_uses_smoothstep_and_retains_endpoints() {
        let from = Transform::from_xyz(1., 2., 3.);
        let to = Transform::from_xyz(5., 4., 9.);
        assert_eq!(blend(from, to, smooth(0.)).translation, from.translation);
        assert_eq!(blend(from, to, smooth(1.)).translation, to.translation);
        assert_eq!(smooth(0.5), 0.5);
        assert_eq!(smooth(-1.), 0.);
    }

    #[test]
    fn camera_proxy_rotation_keeps_distance_and_corrects_forward_once() {
        let anchor = Vec3::new(4., 1., -2.);
        let proxy = Transform::from_translation(anchor + Vec3::new(0., 2., -6.));
        let result = fixed_yaw_pose(proxy, anchor, 0.6);
        let offset = result.translation - anchor;
        assert!((offset.length() - Vec3::new(0., 2., -6.).length()).abs() < 1e-5);
        assert!((offset.x.atan2(offset.z) - 0.6).abs() < 1e-5);
        let correction = Quat::from_rotation_y(0.6 - std::f32::consts::PI);
        let source_forward = correction * proxy.rotation * Vec3::Z;
        assert!((result.rotation * Vec3::NEG_Z - source_forward).length() < 1e-5);
    }
}
