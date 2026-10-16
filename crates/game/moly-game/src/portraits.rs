//! Explicit native-only SD photo export using the production character pipeline.
//! No mesh, material, rig, facial texture, or animation is substituted. Framing
//! is a documented product portrait preset, not a claim about a source camera.
use bevy::{
    app::AppExit,
    asset::AssetServer,
    camera::{RenderTarget, visibility::RenderLayers},
    prelude::*,
    render::{render_resource::{TextureFormat, TextureUsages}, view::screenshot::{Screenshot, ScreenshotCaptured}},
};
use moly_assets::json::JsonAsset;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::{Arc, Mutex}, time::Instant};
use crate::{character::{CharacterPack, CharacterShell, MotionDriver}, character_material::ToonMaterials, npc::CharacterUnitId};

const SIZE: u32 = 512;
const PHOTO_LAYER: usize = 30;
const WARM_FRAMES: u32 = 32;

#[derive(Resource)]
struct PortraitJob {
    directory: PathBuf,
    only: Option<u32>,
    manifest: Option<Handle<JsonAsset>>,
    metadata: Option<Handle<JsonAsset>>,
    rows: Vec<(u32, String, String)>,
    source: Option<Value>,
    index: usize,
    actor: Option<Entity>,
    temporary: Vec<Entity>,
    target: Option<Handle<Image>>,
    warm: u32,
    frozen: bool,
    capturing: bool,
    received: Arc<Mutex<Option<Result<Value, String>>>>,
    records: Vec<Value>,
    started: Instant,
    last_report: u32,
}

pub fn configure(app: &mut App, directory: PathBuf, only: Option<u32>) {
    app.insert_resource(PortraitJob { directory, only, manifest: None, metadata: None, rows: vec![], source: None,
        index: 0, actor: None, temporary: vec![], target: None, warm: 0, frozen: false,
        capturing: false, received: Default::default(), records: vec![], started: Instant::now(), last_report: 0 });
    app.add_systems(Last, advance);
    app.add_systems(PostUpdate, frame
        .after(crate::talk_camera::advance)
        .before(crate::character_material::write_frame_state));
}

fn advance(world: &mut World) {
    let Some(mut job) = world.remove_resource::<PortraitJob>() else { return; };
    let result = advance_inner(world, &mut job);
    if let Err(error) = result {
        error!("[sd-portraits] {error}");
        let _ = std::fs::write(job.directory.join("failure.json"), json!({"error":error,"completed":job.records,"index":job.index}).to_string());
        world.write_message(AppExit::error());
    }
    world.insert_resource(job);
}

fn advance_inner(world: &mut World, job: &mut PortraitJob) -> Result<(), String> {
    if job.manifest.is_none() {
        job.manifest = Some(world.resource::<AssetServer>().load(moly_assets::character_manifest()));
        job.metadata = Some(world.resource::<AssetServer>().load("moly://mysekai-fixtures.json"));
        info!("[sd-portraits] loading source character manifest and metadata");
    }
    if job.source.is_none() {
        let metadata = world.resource::<Assets<JsonAsset>>().get(job.metadata.as_ref().unwrap());
        let Some(metadata) = metadata else { return Ok(()); };
        let metadata: Value = serde_json::from_str(&metadata.0).map_err(|e|e.to_string())?;
        let region = metadata["region"].as_str().ok_or("Source metadata has no region")?;
        let version = metadata["gameVersion"].as_str().ok_or("Source metadata has no version")?;
        if !matches!(region, "cn" | "jp") || version.is_empty() { return Err("Invalid source identity".into()); }
        let source = world.resource::<Assets<JsonAsset>>().get(job.manifest.as_ref().unwrap());
        let Some(source) = source else { return Ok(()); };
        let manifest = moly_assets::character::CharacterManifest::parse(&source.0).map_err(|e| e.to_string())?;
        for row in manifest.units {
            let code = row.unit.parse::<u32>().map_err(|e| e.to_string())?;
            let unit = code.checked_sub(100).ok_or("Source SD code is not unit + 100")?;
            if job.only.is_none_or(|only| only == unit) { job.rows.push((unit, row.glb, row.rig)); }
        }
        if job.rows.is_empty() { return Err("No source SD units match the requested export".into()); }
        job.rows.sort_by_key(|row| row.0);
        job.source = Some(json!({"region":region,"version":version}));
        info!("[sd-portraits] {} exact SD units from {} {}", job.rows.len(), region, version);
        let mut image = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba8UnormSrgb, None);
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        job.target = Some(world.resource_mut::<Assets<Image>>().add(image));
    }
    let delivered = job.received.lock().map_err(|_| "Screenshot result mutex poisoned")?.take();
    if let Some(delivered) = delivered {
        let record = delivered?;
        info!("[sd-portraits] saved {}/{} {}", job.index + 1, job.rows.len(), record["file"]);
        job.records.push(record);
        // An existing scene actor (notably the player) is not a temporary NPC.
        // Remove its capture visibility before photographing the next entity.
        if let Some(actor) = job.actor {
            let mut stack = vec![actor];
            while let Some(entity) = stack.pop() {
                if let Some(children) = world.get::<Children>(entity) { stack.extend(children.iter()); }
                if let Ok(mut entity) = world.get_entity_mut(entity) {
                    entity.remove::<RenderLayers>();
                    entity.remove::<bevy::camera::visibility::NoFrustumCulling>();
                }
            }
        }
        crate::npc::remove_temporary_units(world, &job.temporary);
        job.temporary.clear();
        job.actor = None;
        job.index += 1;
        job.warm = 0;
        job.frozen = false;
        job.capturing = false;
    }
    if job.index >= job.rows.len() {
        let manifest = json!({"schemaVersion":1,"generator":"moly-production-sd-renderer",
            "region":job.source.as_ref().unwrap()["region"],"version":job.source.as_ref().unwrap()["version"],
            "size":[SIZE,SIZE],"alpha":"RGBA straight; real offscreen render, no chroma key",
            "preset":{"id":"sd-head-v1","fovDegrees":30,"verticalSpanInBodyHeights":1.00,"aimAboveHeadInBodyHeights":0.17,"pose":"source idle at 0.5 seconds","tonemapping":"production None","environmentScreenEffects":false},
            "elapsedSeconds":job.started.elapsed().as_secs_f64(),"expected":job.rows.len(),"portraits":job.records});
        std::fs::write(job.directory.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).map_err(|e| e.to_string())?;
        world.write_message(AppExit::Success);
        return Ok(());
    }
    if job.capturing { return Ok(()); }
    if job.last_report % 300 == 0 {
        let _ = std::fs::write(job.directory.join("runtime-state.json"), crate::library_snapshot());
    }
    let (unit, glb, rig) = job.rows[job.index].clone();
    if job.actor.is_none() {
        // The controlled avatar has its own animation driver. A photo always
        // uses the same genuine NPC blueprint, even when that unit is already
        // the current avatar. Temporarily hide only the admission identity and
        // restore it immediately; no original entity or model is altered.
        let occupied: Vec<Entity> = world.query::<(Entity, &CharacterUnitId)>().iter(world)
            .filter(|(_, id)|id.0 == unit).map(|(entity,_)|entity).collect();
        for entity in &occupied { world.entity_mut(*entity).remove::<CharacterUnitId>(); }
        let admission = crate::npc::spawn_temporary_units(world, &[unit]);
        for entity in occupied { world.entity_mut(entity).insert(CharacterUnitId(unit)); }
        let spawned = match admission {
            Ok(entities) => entities,
            Err(reason) => { job.last_report += 1; if job.last_report % 300 == 1 { info!("[sd-portraits] waiting: {reason}"); } return Ok(()); }
        };
        job.temporary = spawned;
        job.actor = job.temporary.first().copied();
        let actor = job.actor.ok_or("Source NPC factory did not provide requested unit")?;
        world.entity_mut(actor).insert(crate::talk::TalkHold);
        info!("[sd-portraits] preparing source unit {unit}: {glb}");
        return Ok(());
    }
    let actor = job.actor.unwrap();
    if world.get::<ToonMaterials>(actor).is_none() || world.get::<CharacterShell>(actor).is_none() { return Ok(()); }
    let Some(driver) = world.get::<MotionDriver>(actor) else { return Ok(()); };
    let (player, idle) = (driver.player, driver.idle);
    if !job.frozen {
        let Some(mut animations) = world.get_mut::<AnimationPlayer>(player) else { return Ok(()); };
        animations.stop_all();
        animations.play(idle).repeat().seek_to(0.5).pause();
        world.get_mut::<MotionDriver>(actor).unwrap().alone_holds = true;
        job.frozen = true;
        return Ok(());
    }
    // Apply the render layer to the fully assembled real hierarchy, including
    // skinned mesh children. World and UI geometry retain their ordinary layer.
    let mut stack = vec![actor];
    while let Some(entity) = stack.pop() {
        if let Some(children) = world.get::<Children>(entity) { stack.extend(children.iter()); }
        world.entity_mut(entity).insert((RenderLayers::layer(PHOTO_LAYER),
            bevy::camera::visibility::NoFrustumCulling));
    }
    job.warm += 1;
    if job.warm < WARM_FRAMES { return Ok(()); }
    let filename = format!("unit-{unit}.png");
    let output = job.directory.join(&filename);
    let received = job.received.clone();
    let target = job.target.clone().unwrap();
    let source_pack = world.get::<CharacterPack>(actor).ok_or("Missing genuine source pack")?;
    if source_pack.file != glb || source_pack.rig_file != rig { return Err("Source character pack identity mismatch".into()); }
    let transforms = |world: &World, entity| world.get::<GlobalTransform>(entity).map(|t|json!({
        "position":t.translation().to_array(),"scale":t.scale().to_array(),"rotation":t.rotation().to_array()}));
    let head = world.get::<ToonMaterials>(actor).map(|value|value.head_entity());
    let actor_pose = transforms(world, actor);
    let head_pose = head.and_then(|entity|transforms(world,entity));
    let shell_height = world.get::<CharacterShell>(actor).map(|value|value.height);
    let cameras: Vec<_> = world.query::<(&Camera,&Projection,&GlobalTransform,Option<&RenderLayers>)>()
        .iter(world).map(|(camera,projection,transform,layers)|json!({
            "active":camera.is_active,"projection":format!("{projection:?}"),"position":transform.translation().to_array(),
            "rotation":transform.rotation().to_array(),"layers":format!("{layers:?}") })).collect();
    let meshes: Vec<_> = world.query::<(Entity,&Mesh3d,&GlobalTransform,Option<&RenderLayers>,Option<&Visibility>,Option<&InheritedVisibility>,Option<&ViewVisibility>)>()
        .iter(world).filter(|(_,_,_,layers,_,_,_)|layers.is_some_and(|layers|layers.intersects(&RenderLayers::layer(PHOTO_LAYER))))
        .map(|(entity,_,transform,_,visible,inherited,view)|json!({"entity":format!("{entity:?}"),
            "position":transform.translation().to_array(),"visible":format!("{visible:?}"),"inherited":format!("{inherited:?}"),"view":format!("{view:?}")})).collect();
    let _ = std::fs::write(job.directory.join(format!("unit-{unit}.capture.json")),
        json!({"actor":actor_pose,"head":head_pose,"height":shell_height,"cameras":cameras,"meshes":meshes}).to_string());
    world.spawn(Screenshot::image(target)).observe(move |capture: On<ScreenshotCaptured>| {
        let result = (|| {
            // Bevy's save_to_disk deliberately discards alpha; use its actual
            // captured Image but preserve RGBA all the way through the encoder.
            let image = capture.image.clone().try_into_dynamic().map_err(|e| e.to_string())?.to_rgba8();
            let (mut transparent, mut opaque) = (0usize, 0usize);
            for pixel in image.pixels() { if pixel[3] == 0 { transparent += 1; } if pixel[3] == 255 { opaque += 1; } }
            if transparent == 0 || opaque == 0 { let _ = image.save(output.with_extension("invalid.png")); return Err(format!("Invalid portrait alpha for unit {unit}: transparent={transparent}, opaque={opaque}")); }
            image.save(&output).map_err(|e| e.to_string())?;
            Ok(json!({"unit":unit,"model":glb,"rig":rig,"file":filename,"transparentPixels":transparent,"opaquePixels":opaque,"width":image.width(),"height":image.height()}))
        })();
        if let Ok(mut slot) = received.lock() { *slot = Some(result); }
    });
    job.capturing = true;
    Ok(())
}

fn frame(
    job: Res<PortraitJob>,
    actors: Query<(&GlobalTransform, &CharacterShell, &ToonMaterials), Without<Camera3d>>,
    transforms: Query<&GlobalTransform, Without<Camera3d>>,
    mut cameras: Query<(Entity, &mut Camera, &mut Projection, &mut Transform, &mut GlobalTransform), With<Camera3d>>,
    mut commands: Commands,
) {
    let (Some(actor), Some(target)) = (job.actor, job.target.as_ref()) else { return; };
    let Ok((actor_transform, shell, materials)) = actors.get(actor) else { return; };
    let Ok(head) = transforms.get(materials.head_entity()) else { return; };
    let Ok((entity, mut camera, mut projection, mut transform, mut global)) = cameras.single_mut() else { return; };
    let height = shell.height;
    let aim = head.translation() + Vec3::Y * (height * 0.17);
    let forward = actor_transform.rotation() * Vec3::Z;
    let fov = 30_f32.to_radians();
    let distance = height * 1.00 / (2.0 * (fov * 0.5).tan());
    let pose = Transform::from_translation(aim + forward * distance).looking_at(aim, Vec3::Y);
    *transform = pose;
    *global = GlobalTransform::from(pose);
    *projection = Projection::Perspective(PerspectiveProjection { fov, aspect_ratio: 1., near: 0.01, far: 1000., ..default() });
    camera.clear_color = ClearColorConfig::Custom(Color::NONE);
    commands.entity(entity).insert((RenderTarget::Image(target.clone().into()), RenderLayers::layer(PHOTO_LAYER), crate::weather::TransparentCapture));
}
