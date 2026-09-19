//! Opt-in real-device render tests. These are analytic source-branch/depth tests,
//! not screenshots of Moly treated as original-game goldens.
use bevy::prelude::*;
use bevy::camera::{RenderTarget, visibility::NoFrustumCulling};
use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::tonemapping::Tonemapping;
use crate::weather_depth::WeatherDepthSnapshot;
use bevy::render::{gpu_readback::{Readback, ReadbackComplete}, render_resource::*};
use std::{collections::HashMap, sync::{Arc,Mutex}, time::{Duration,Instant}};
use crate::uber_particle::{UberParticleMaterial, UberT1Params, ParticleEmission, CullArm, BlendArm};
use crate::fixture_emission::{FixtureEmissionPlugin, WeatherEffectProbes};

const SIZE: u32 = 256;
#[derive(Resource,Clone,Default)]
struct Captures(Arc<Mutex<HashMap<&'static str,Vec<u8>>>>);
#[derive(Component)]
struct CaptureName(&'static str);
#[derive(Component)]
struct ProbeCamera;

fn observe(event:On<ReadbackComplete>, names:Query<&CaptureName>, captures:Res<Captures>) {
    let name=names.get(event.entity).unwrap().0;
    captures.0.lock().unwrap().insert(name,event.data.clone());
}

fn setup(mut commands:Commands, mut meshes:ResMut<Assets<Mesh>>,
    mut images:ResMut<Assets<Image>>, mut standards:ResMut<Assets<StandardMaterial>>,
    mut particles:ResMut<Assets<UberParticleMaterial>>) {
    let extent=Extent3d {width:SIZE,height:SIZE,depth_or_array_layers:1};
    let mut scene=Image::new_target_texture(SIZE,SIZE,TextureFormat::Rgba8UnormSrgb,None);
    scene.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let scene=images.add(scene);
    let probe=|images:&mut Assets<Image>| {
        let mut image=Image::new_target_texture(SIZE,SIZE,crate::fixture_emission::EMISSION_FORMAT,None);
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC | TextureUsages::COPY_DST;
        images.add(image)
    };
    let early=probe(&mut images); let late=probe(&mut images);
    for (name,handle) in [("scene",scene.clone()),("effect-early",early.clone()),("effect-late",late.clone())] {
        commands.spawn((Readback::texture(handle),CaptureName(name))).observe(observe);
    }
    commands.insert_resource(WeatherEffectProbes {after_opaques:Some(early),after_transparents:Some(late)});
    commands.spawn((Camera3d { depth_texture_usages: (TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING).into(), ..default() }, Camera {clear_color:ClearColorConfig::Custom(Color::BLACK),..default()},
        RenderTarget::Image(scene.into()), ProbeCamera, WeatherDepthSnapshot, Msaa::Off, crate::weather_depth::WeatherCameraRole::Base,
        Tonemapping::None, bevy::render::view::NoIndirectDrawing,
        Projection::Perspective(PerspectiveProjection {fov:std::f32::consts::FRAC_PI_2,near:0.1,far:100.0,..default()}),
        Transform::IDENTITY));
    // A black opaque wall at eye-depth 3. The first three particles are at depth
    // 2, so source soft intensity 2 implies precisely (3-2)/2 = 0.5.
    commands.spawn((Mesh3d(meshes.add(Rectangle::new(20.0,20.0))),
        MeshMaterial3d(standards.add(StandardMaterial {base_color:Color::BLACK,unlit:true,..default()})),
        Transform::from_xyz(0.0,0.0,-3.0)));
    let tex=images.add(Image::new_fill(Extent3d {width:1,height:1,depth_or_array_layers:1},
        TextureDimension::D2,&[255u8;4],TextureFormat::Rgba8UnormSrgb,RenderAssetUsages::all()));
    let basis=crate::billboard::CameraBasis {position:Vec3::ZERO,forward:Vec3::NEG_Z,right:Vec3::X,
        up:Vec3::Y,fov_y:std::f32::consts::FRAC_PI_2,aspect:1.0};
    // Columns: soft+emission, hard+HDR emission (source ARGB32 clamps to 1),
    // soft+NO emission area, occluded. A half-float effect target fails this test.
    for (i,(x,z,soft,area)) in [(-1.5,-2.0,true,true),(-0.5,-2.0,false,true),
        (0.5,-2.0,true,false),(3.0,-4.0,true,true)].into_iter().enumerate() {
        let params=UberT1Params {base_st:Vec4::new(1.0,1.0,0.0,0.0),tint_colour:Vec4::ONE,
            scalars:Vec4::ZERO,luminance:Vec4::ZERO,coords:Vec4::new(0.0,2.0,f32::from(soft),0.0)};
        let mut mesh=crate::billboard::empty_mesh();
        crate::billboard::write_quads(&mut mesh,&[crate::billboard::Quad {centre:Vec3::new(x,0.0,z),
            size:Vec2::splat(0.6),rotation:0.0,colour:[1.0;4],custom1:[0.0;4],custom2:[0.0;4]}],
            crate::billboard::Alignment::View,basis,
            crate::billboard::SizeClamp {max_screen_fraction:1.0,min_size:0.0},[0.0;3]);
        let material=particles.add(UberParticleMaterial::new(params,tex.clone(),false,false,CullArm::Off,BlendArm::AlphaBlend));
        commands.spawn((Name::new(format!("analytic-case-{i}")),Mesh3d(meshes.add(mesh)),MeshMaterial3d(material),
            Transform::IDENTITY,NoFrustumCulling,crate::shadowmap::NoShadowCast,
            ParticleEmission {source_state:Some(moly_assets::material_passes::SourceRenderState {cull:0,depth_test:4,depth_write:false,color_mask:14,src_color:5,dst_color:10,src_alpha:5,dst_alpha:10,color_op:0,alpha_op:0}),render_queue:3000,params,colour:Vec4::ONE,intensity:if i==1 {2.0} else {1.0},colour_type:0.0,area,tint_area:false,
                plain_colour:false,cull:CullArm::Off,blend:BlendArm::AlphaBlend}));
    }
    let _=extent;
}

fn center_values(bytes:&[u8],normalized:bool)->Vec<f32> {
    [32usize,96,160,224].iter().map(|x| {
        let n=(128*SIZE as usize+x)*4;
        let value=bytes[n] as f32;
        if normalized {value/255.0} else {value}
    }).collect()
}

#[test]
#[ignore="requires a real GPU; set MOLY_WEATHER_GPU_OUT for raw target captures"]
fn actual_opaque_depth_and_effect_pass_readbacks() {
    let mut app=App::new();
    app.add_plugins(DefaultPlugins
        .set(WindowPlugin {primary_window:None,exit_condition:bevy::window::ExitCondition::DontExit,..default()})
        .set(bevy::render::RenderPlugin {synchronous_pipeline_compilation:true,..default()})
        .disable::<bevy::winit::WinitPlugin>()
        .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>())
        .add_plugins((crate::env::SiteEnvPlugin,FixtureEmissionPlugin))
        .init_resource::<Captures>()
        .add_systems(Startup,(crate::uber_particle::load,setup));
    crate::uber_particle::install(&mut app);
    app.finish(); app.cleanup();
    let mut reports=Vec::new();
    for (case,samples,role,has_snapshot) in [
        ("base-single",1,crate::weather_depth::WeatherCameraRole::Base,true),
        ("base-msaa-refused",4,crate::weather_depth::WeatherCameraRole::Base,true),
        ("overlay-refused",1,crate::weather_depth::WeatherCameraRole::Overlay,true),
        ("base-no-snapshot",1,crate::weather_depth::WeatherCameraRole::Base,false),
        ("base-single-restored",1,crate::weather_depth::WeatherCameraRole::Base,true),
    ] {
        if case != "base-single" {
            let world=app.world_mut();
            let mut query=world.query_filtered::<&mut Msaa,With<ProbeCamera>>();
            *query.single_mut(world).unwrap()=if samples==1 {Msaa::Off} else {Msaa::Sample4};
            let mut query=world.query_filtered::<&mut crate::weather_depth::WeatherCameraRole,With<ProbeCamera>>();
            *query.single_mut(world).unwrap()=role;
            let mut query=world.query_filtered::<Entity,With<ProbeCamera>>();
            let camera=query.single(world).unwrap();
            if has_snapshot {world.entity_mut(camera).insert(WeatherDepthSnapshot);} else {world.entity_mut(camera).remove::<WeatherDepthSnapshot>();}
            world.resource::<Captures>().0.lock().unwrap().clear();
        }
        let deadline=Instant::now()+Duration::from_secs(90);
        let mut frame=0;
        loop {
            app.update(); frame+=1;
            let captures=app.world().resource::<Captures>().0.lock().unwrap().clone();
            let complete=captures.get("scene").is_some_and(|b| b.len()==(SIZE*SIZE*4) as usize)
                && ["effect-early","effect-late"].iter().all(|k| captures.get(k).is_some_and(|b|b.len()==(SIZE*SIZE*4) as usize));
            if complete && frame>30 {
                let early=center_values(&captures["effect-early"],true);
                let late=center_values(&captures["effect-late"],true);
                let scene=center_values(&captures["scene"],false);
                let permitted=crate::weather_depth::effect_attachment_compatible(Some(role),samples);
                let soft_depth=permitted && has_snapshot;
                let soft_effect=if soft_depth {0.5} else {0.0};
                let soft_forward=if soft_depth {188.0} else {0.0};
                let correct=if permitted { (early[0]-soft_effect).abs()<0.015 && (early[1]-1.0).abs()<0.015 }
                    else {early.iter().all(|v| *v==0.0)};
                if correct {
                    assert!(captures["effect-early"].chunks_exact(4).all(|pixel| pixel[3] == 0), "source clear alpha must remain zero under RGB-only writes");
                    assert!(early[2].abs()<0.001 && early[3].abs()<0.001,"source no-area and opaque occlusion: {early:?}");
                    assert_eq!(early,late,"later empty transparent replay must preserve the effect buffer");
                    // Main target currently blends in linear hardware space. This
                    // checks the depth route, NOT equivalence to gamma-direct Unity.
                    assert!((scene[0]-soft_forward).abs()<3.0 && scene[1]>252.0 && (scene[2]-soft_forward).abs()<3.0 && scene[3]<2.0,"forward soft / hard / no-area / occluded: {scene:?}");
                    if let Some(out)=std::env::var_os("MOLY_WEATHER_GPU_OUT") {
                        let out=std::path::PathBuf::from(out);std::fs::create_dir_all(&out).unwrap();
                        for (name,bytes) in &captures {std::fs::write(out.join(format!("{case}-{name}.raw")),bytes).unwrap();}
                    }
                    println!("GPU case={case} samples={samples}: early={early:?}, late={late:?}, scene={scene:?}");
                    reports.push(serde_json::json!({"case":case,"samples":samples,"effectPermitted":permitted,"softDepthAvailable":soft_depth,"early":early,"late":late,"scene":scene,"frames":frame}));
                    break;
                }
            }
            if Instant::now()>deadline {
                panic!("GPU source-branch probe failed to converge; captures={:?}",captures.iter().map(|(n,b)|(*n,b.len(),center_values(b,*n!="scene"))).collect::<Vec<_>>());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    // Metamorphic compositor regression: a no-clear, empty overlay must not
    // replace already-rendered source-branch pixels. This is not a source golden.
    let before = app.world().resource::<Captures>().0.lock().unwrap()["scene"].clone();
    {
        let world = app.world_mut();
        let mut cameras = world.query_filtered::<&RenderTarget, With<ProbeCamera>>();
        let target = cameras.single(world).unwrap().clone();
        world.spawn((Camera2d, Camera { order: 1, clear_color: ClearColorConfig::None, ..default() },
            target, crate::camera::MYSEKAI_CAMERA_MSAA,
            bevy::camera::visibility::RenderLayers::none()));
        world.resource::<Captures>().0.lock().unwrap().clear();
    }
    for _ in 0..40 { app.update(); std::thread::sleep(Duration::from_millis(10)); }
    let after = app.world().resource::<Captures>().0.lock().unwrap()["scene"].clone();
    assert_eq!(before, after, "empty overlay changed the completed field image");
    reports.push(serde_json::json!({"case":"empty-overlay-preserves-main", "samples":1,
        "byteIdentical":true,"bytes":after.len()}));
    println!("GPU case=empty-overlay-preserves-main samples=1 byte-identical={}", after.len());
    {
        let world=app.world_mut();
        let mut query=world.query_filtered::<Entity,With<Readback>>();
        let observers=query.iter(world).collect::<Vec<_>>();
        for entity in observers { world.entity_mut(entity).remove::<Readback>(); }
    }
    for _ in 0..8 { app.update(); std::thread::sleep(Duration::from_millis(10)); }
    if let Some(out)=std::env::var_os("MOLY_WEATHER_GPU_OUT") {
        std::fs::write(std::path::PathBuf::from(out).join("analytic-readbacks.json"),serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
    }
}
