//! Photo mode: capture, native presentation, frame selection and local export.
//! The screen follows ScreenManager and uses its serialized component bindings.
//! Capture renders the field camera alone at its current pixel dimensions, then
//! crops the source frame rectangle. Saving exports a local image; no album
//! server, film purchase or online success is simulated by this module.
use bevy::{
    asset::RenderAssetUsages,
    camera::{visibility::RenderLayers, RenderTarget},
    gltf::{Gltf, GltfNode},
    math::Vec3A,
    prelude::*,
    render::{render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
        render_asset::RenderAssets, texture::GpuImage, Render, RenderApp, RenderSystems,
        view::screenshot::{Screenshot, ScreenshotCaptured}},
    window::PrimaryWindow,
};
use moly_assets::ui_layout::{UiComponent, UiPrefab};
use crate::{gesture::{GestureEvent, GestureState}, ui_layers::{MenuScreenType, ScreenManager},
    ui_layout::{UiLayouts, UiPrefabView}};

mod select;
pub use select::PhotoCharset;

const SCREEN: MenuScreenType = MenuScreenType::MysekaiPhotoShot;
const KEY: &str = "PhotoShot";
const PHOTO_ROOT: &str = "ComponentRoot/Root/PhotoShot";
const SELECT_ROOT: &str = "ComponentRoot/Root/SelectFrame";
const SAVE_ROOT: &str = "ComponentRoot/PhotoSave";
const CALL_EFFECTS: [(&str,&str);2] = [
    ("photo-shot/call-button/call-button.json","fx_mysekai_photo_shot_call_character_pt"),
    ("photo-shot/call-wave/call-wave.json","fx_mysekai_photo_shot_call_character_wave"),
];

#[derive(Message)]
pub struct OpenPhoto;
#[derive(Message, Clone, Copy)]
enum PhotoAction { Open, Back, Call, Capture(UVec2), Skip, Save, Frame(i32) }
#[derive(Component)]
struct PhotoView;
#[derive(Component)]
struct PhotoHeader;
#[derive(Resource, Default)]
struct PhotoState {
    entered: bool, capture: Option<CaptureJob>, call_effect: Option<CallEffect>, last_error: Option<String>,
    pending: Option<Image>, image: Option<Handle<Image>>, selection: Option<select::Selection>,
    previous_frame: i32, discard: Option<crate::ui_layers::DialogId>,
}
struct CallEffect { hosts:Vec<Entity>, elapsed:f32, fade:Option<f32> }
#[derive(Resource)]
struct PhotoEffects { documents:[Handle<moly_assets::json::JsonAsset>;2], textures:Vec<Handle<Image>>, prepared:bool }
struct CaptureJob { camera: Entity, previous: RenderTarget, target: Handle<Image>, elapsed: f32,
    requested: UVec2, dispatched: bool, restored: bool }
#[derive(Resource, Clone, Default)]
struct CaptureReadiness(std::sync::Arc<std::sync::Mutex<Option<(AssetId<Image>, bool)>>>);
#[derive(Resource)]
struct PhotoBindings {
    frame: String, shutter: String, call: String,
    cover: String, call_cover: String,
    textures: Vec<(String, &'static str)>,
    unavailable: Vec<String>,
    disabled_covers: Vec<String>,
    disabled_alpha: Vec<(String,f32)>,
    call_graphics:Vec<i64>,
    call_wave_root:String,
    back: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all="camelCase")]
struct RendererBounds { node:usize, root_bone:Option<usize>, name:String,
    center:[f32;3], extents:[f32;3], update_when_offscreen:bool }
#[derive(serde::Deserialize)]
#[serde(rename_all="camelCase")]
struct BoundsDocument { renderer_bounds:Vec<RendererBounds> }

fn component<'a>(doc: &'a UiPrefab, class: &str) -> Result<&'a UiComponent, String> {
    let mut found = doc.nodes.iter().flat_map(|node| &node.components).filter(|c| c.class == class);
    let value = found.next().ok_or_else(|| format!("photo layout lacks {class}"))?;
    if found.next().is_some() { return Err(format!("photo layout duplicates {class}")); }
    Ok(value)
}
fn reference(doc: &UiPrefab, fields: &serde_json::Value, field: &str) -> Result<String, String> {
    let ptr = fields[field].as_array().filter(|ptr| ptr.len() == 2)
        .ok_or_else(|| format!("photo layout lacks {field} binding"))?;
    let id = ptr[1].as_i64().filter(|id| *id != 0).ok_or("invalid photo component reference")?;
    if ptr[0].as_i64() != Some(0) || !doc.nodes.iter().any(|node| node.game_object_id == id
        || node.transform_id == id || node.components.iter().any(|c| c.path_id == id)) {
        return Err(format!("unresolved photo component {field}"));
    }
    Ok(format!("@{id}"))
}
impl PhotoBindings {
    fn read(doc: &UiPrefab) -> Result<Self, String> {
        let view = component(doc, "Sekai.Mysekai.MysekaiPhotoShotView")?;
        let fields = &view.fields;
        let r = |name| reference(doc, fields, name);
        let mut textures = Vec::new();
        for (field, texture) in [
            ("_overlayImage", "overlay_photoshot_screen"), ("_frameImage", "frame_PhotoShot"),
            ("_photoShotBaseImage", "btn_photoshot_gr_h180"), ("_photoShotCoverBaseImage", "btn_photoshot_gr_h180"),
            ("_photoShotIconImage", "icon_camera_wh_h80"), ("_photoShotIconCoverImage", "icon_camera_wh_h80"),
            ("_callCharacterButtonIcon", "icon_callout_wh"), ("_callCharacterButtonCoverIcon", "icon_callout_wh"),
            ("_photoEffectIconImage", "icon_cameraEffect_h56"), ("_photoAlbumIcon", "icon_photoalbum_wh_h42"),
            ("_dofTextImage", "txt_photoshot_blur"), ("_remainCountTextImage", "txt_photoshot_filmCount"),
        ] { textures.push((r(field)?, texture)); }
        for (field, texture) in [("_lineCornerImageList", "line_cornerguide_photoshot"), ("_iconBaseImageList", "btn_circle120_lgr"), ("_tripodIconList", "icon_tripod_wh_h44")] {
            for ptr in fields[field].as_array().ok_or_else(|| format!("photo layout lacks {field}"))? {
                let fake = serde_json::json!({"item":ptr});
                textures.push((reference(doc, &fake, "item")?, texture));
            }
        }
        // These controls depend on the album service or separate editing views.
        // Keep their authored layout with disabled appearance until those
        // separate views are implemented. Camera, call, capture and Back run.
        let unavailable = ["_openPhotoAlbumButton", "_openAvatarMotionSelectorButton",
            "_showsSelectPhotoEffectDialogButton", "_switchEditModeToggle", "_dofAdjustSlider"]
            .into_iter().map(r).collect::<Result<Vec<_>, _>>()?;
        let mut disabled_covers=Vec::new();let mut disabled_alpha=Vec::new();
        for path in &unavailable {
            let node=&doc.nodes[doc.find(path).map_err(|e|e.to_string())?];
            let control=node.components.iter().find(|c|c.fields.get("m_DisabledColor").is_some())
                .ok_or("photo disabled control lacks its selectable appearance")?;
            if control.fields["coverImage"].as_array().is_some_and(|ptr|ptr.get(1).and_then(|id|id.as_i64()).is_some_and(|id|id!=0)) {
                disabled_covers.push(reference(doc,&control.fields,"coverImage")?);
            } else {
                let alpha=control.fields["m_DisabledColor"][3].as_f64().ok_or("photo disabled color is missing")? as f32;
                disabled_alpha.push((path.clone(),alpha));
            }
        }
        let call_graphics=fields["_callCharacterButtonGraphics"].as_array().ok_or("photo call graphics are missing")?
            .iter().map(|ptr| reference(doc,&serde_json::json!({"graphic":ptr}),"graphic")?
                .trim_start_matches('@').parse::<i64>().map_err(|e|e.to_string())).collect::<Result<Vec<_>,_>>()?;
        Ok(Self { frame:r("_frameImage")?, shutter:r("_photoShotButton")?, call:r("_callCharacterButton")?,
            cover:r("_photoShotCoverButton")?, call_cover:r("_callCharacterButtonCover")?, textures, unavailable,
            disabled_covers,disabled_alpha,call_graphics,call_wave_root:r("_callCharacterWaveParticleRoot")?,back:String::new() })
    }
}

/// This source may predate the photo screen. The menu advertises only a
/// source whose serialized bindings and dynamic texture inventory are present.
pub fn source_available(layouts: &UiLayouts) -> bool {
    layouts.document(KEY).and_then(|doc|PhotoBindings::read(doc).ok())
        .is_some_and(|bindings|bindings.textures.iter().all(|(_,name)|layouts.has_runtime_texture(name)))
}

pub fn install(app: &mut App) {
    select::install(app);
    let readiness=CaptureReadiness::default();
    app.insert_resource(readiness.clone());
    if let Some(render)=app.get_sub_app_mut(RenderApp) {
        render.insert_resource(readiness).add_systems(Render, mark_capture_ready.after(RenderSystems::PrepareAssets));
    }
    app.init_resource::<PhotoState>().add_message::<OpenPhoto>().add_message::<PhotoAction>()
        .add_systems(Update, spawn_when_ready)
        .add_systems(Update,prepare_effects.after(spawn_when_ready).before(click))
        .add_systems(Update, click.after(crate::menu_dialog::click).before(crate::pick::pick)
            .before(crate::ui_layers::advance).run_if(crate::game_settings::scene_input_enabled))
        .add_systems(Update, dispatch.after(click).before(crate::ui_layers::advance))
        .add_systems(Update, lifecycle.after(crate::ui_layers::advance).before(place))
        .add_systems(Update, place.after(crate::menu_shell::place).before(crate::ui_layout::render))
        .add_systems(Update,fit_call_effect.after(place).before(crate::ui_layout::render));
}

fn mark_capture_ready(readiness: Res<CaptureReadiness>, images: Res<RenderAssets<GpuImage>>) {
    if let Ok(mut slot)=readiness.0.lock() {
        if let Some((id,ready))=slot.as_mut() { *ready=images.get(*id).is_some(); }
    }
}

fn spawn_when_ready(mut commands: Commands, layouts: Res<UiLayouts>, server: Res<AssetServer>,
    bindings: Option<Res<PhotoBindings>>, mut state: ResMut<PhotoState>) {
    if bindings.is_some() || !layouts.ready(KEY, &server) || !layouts.ready("EditorHeader", &server) || !layouts.ready("PhotoFrameCell", &server) || !source_available(&layouts) { return; }
    let result = PhotoBindings::read(layouts.document(KEY).expect("ready photo document"));
    match result {
        Ok(mut bindings) => {
            let doc = layouts.document("EditorHeader").expect("ready shared header");
            let mut buttons = doc.nodes.iter().flat_map(|node| &node.components).filter(|c| c.class.ends_with(".CustomButton")
                && c.fields["m_OnClick"]["m_Calls"].as_array().is_some_and(|calls| calls.iter().any(|call| call["m_MethodName"].as_str()==Some("OnClickBackUIScreen"))));
            let Some(button) = buttons.next() else { error!("[photo] shared header BackUI callback missing"); return; };
            assert!(buttons.next().is_none(), "shared header BackUI callback is ambiguous");
            bindings.back = format!("@{}",button.path_id);
            let path = &doc.nodes[doc.find(&bindings.back).expect("bound header back")].path;
            let mut header = UiPrefabView::new("EditorHeader",crate::sitemap::SITEMAP_LAYER);
            for node in &doc.nodes {
                if node.path != *path && !path.starts_with(&format!("{}/",node.path)) && !node.path.starts_with(&format!("{path}/")) {
                    header.set_visible(&format!("@{}",node.transform_id),false);
                }
            }
            header.set_behaviour_enabled(button.path_id,true);
            let root=layouts.root().expect("ready UI root").trim_end_matches('/');
            commands.insert_resource(PhotoEffects {documents:CALL_EFFECTS.map(|(file,_)|server.load(format!("{root}/{file}"))),textures:Vec::new(),prepared:false});
            commands.insert_resource(bindings);
            commands.spawn((PhotoView, Visibility::Hidden, Transform::default(),
                RenderLayers::layer(crate::sitemap::SITEMAP_LAYER), UiPrefabView::new(KEY, crate::sitemap::SITEMAP_LAYER)));
            commands.spawn((PhotoHeader, Visibility::Hidden, Transform::from_xyz(0.,0.,20.),
                RenderLayers::layer(crate::sitemap::SITEMAP_LAYER), header));
        }
        Err(error) => { if state.last_error.as_deref() != Some(&error) { error!("[photo] {error}"); state.last_error = Some(error); } }
    }
}

fn prepare_effects(effects:Option<ResMut<PhotoEffects>>,docs:Res<Assets<moly_assets::json::JsonAsset>>,server:Res<AssetServer>,layouts:Res<UiLayouts>) {
    let Some(mut effects)=effects else {return};
    if effects.prepared {return;}
    let Some(root)=layouts.root() else {return};
    let mut textures=Vec::new();
    for (index,(file,_)) in CALL_EFFECTS.iter().enumerate() {
        let Some(raw)=docs.get(&effects.documents[index]) else {return};
        let Ok(doc)=serde_json::from_str::<serde_json::Value>(&raw.0) else {return};
        let Some(rows)=doc["particles"].as_array() else {return};
        for row in rows {
            let Some(map)=row["renderer"]["material"]["textures"].as_object() else {return};
            for value in map.values() {
                let Some(path)=value.as_str() else {return};
                textures.push(moly_assets::residency::load_image(&server,format!("{}{}/{path}",root,file.rsplit_once('/').expect("effect directory").0)));
            }
        }
    }
    effects.textures=textures;effects.prepared=true;
}

fn click(mut open: MessageReader<OpenPhoto>, mut gestures: MessageReader<GestureEvent>,
    mut actions: MessageWriter<PhotoAction>, windows: Query<&Window, With<PrimaryWindow>>,
    canvas: Option<Res<crate::canvas::RootCanvas>>, layouts: Res<UiLayouts>, screens: Res<ScreenManager>,
    views: Query<&UiPrefabView, With<PhotoView>>, headers: Query<&UiPrefabView, With<PhotoHeader>>, bindings: Option<Res<PhotoBindings>>,
    mut consumed: ResMut<crate::action_button::ActionTapConsumed>, state: Res<PhotoState>, mut sounds: ResMut<crate::audio::SeRequests>) {
    for _ in open.read() { actions.write(PhotoAction::Open); }
    let taps: Vec<_> = gestures.read().filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position).collect();
    if screens.current_screen() != Some(SCREEN) || screens.is_ui_layer_working() || state.capture.is_some() || state.pending.is_some() || state.discard.is_some() { return; }
    let (Ok(window), Some(canvas), Ok(view), Some(bindings)) = (windows.single(), canvas.as_deref(), views.single(), bindings.as_deref()) else { return; };
    let size = canvas.size(window);
    for tap in taps {
        if consumed.0 { break; }
        let position = Vec2::new(tap.x - window.width() / 2., window.height() / 2. - tap.y) / canvas.scale(window);
        let hit = |path:&str| view.rect(&layouts, path, size).is_some_and(|rect| rect.active && rect.contains(position));
        if let Some(selection)=state.selection.as_ref() {
            if select::tap(selection, &layouts, view, position, size, &mut actions, &mut sounds) {
                consumed.0=true; continue;
            }
            if selection.playing { consumed.0=true;continue; }
        }
        if headers.single().ok().and_then(|header| header.rect(&layouts,&bindings.back,size)).is_some_and(|rect| rect.active && rect.contains(position)) {
            consumed.0=true;layouts.queue_button_se(&mut sounds,"EditorHeader",&bindings.back);actions.write(PhotoAction::Back);
        }
        else if state.selection.is_none() && hit(&bindings.call) {
            consumed.0 = true;
            if state.call_effect.as_ref().is_none_or(|effect|effect.fade.is_some()) {
                layouts.queue_button_se(&mut sounds,KEY,&bindings.call); actions.write(PhotoAction::Call);
            }
        }
        else if state.selection.is_none() && hit(&bindings.shutter) {
            consumed.0 = true;
            layouts.queue_button_se(&mut sounds,KEY,&bindings.shutter);
            if let Some(rect) = view.rect(&layouts, &bindings.frame, size) {
                let pixels = rect.size * canvas.scale(window) * window.scale_factor();
                actions.write(PhotoAction::Capture(pixels.as_uvec2()));
            }
        }
    }
}

fn dispatch(world: &mut World, mut reader: Local<bevy::ecs::message::MessageCursor<PhotoAction>>) {
    let actions: Vec<_> = reader.read(world.resource::<Messages<PhotoAction>>()).copied().collect();
    for action in actions {
        let result = match action {
            PhotoAction::Open => {
                if world.resource::<PhotoState>().entered { continue; }
                if !world.contains_resource::<PhotoBindings>() { Err("photo screen is still loading".to_owned()) }
                else { crate::npc_objective::events::set_photo_mode(world, true).map(|()| {
                    world.resource_mut::<crate::game_state::GameStateManager>().change_state(
                        crate::game_state::GameStateType::PhotoShot,"PhotoShotGameState.OnEnter");
                    crate::camera::change_state(world, crate::camera::CameraStateType::PhotoShot, "photo");
                    world.resource_mut::<PhotoState>().entered = true;
                    world.resource_mut::<ScreenManager>().push_ui_screen(SCREEN, None, "PhotoShotGameState.OnEnter");
                }) }
            }
            PhotoAction::Call => {
                visible_units(world).and_then(|units| {
                    for unit in units { crate::npc_objective::events::call_photo_character(world,unit)?; }
                    start_call_effect(world)
                })
            }
            PhotoAction::Back => select::back(world),
            PhotoAction::Skip => select::skip(world),
            PhotoAction::Save => select::save(world),
            PhotoAction::Frame(id) => select::choose(world,id),
            PhotoAction::Capture(size) => capture(world, size),
        };
        if let Err(error) = result { error!("[photo] {error}"); world.resource_mut::<PhotoState>().last_error = Some(error); }
    }
}

fn lifecycle(world: &mut World) {
    let delta = world.resource::<Time>().delta_secs();
    advance_call_effect(world,delta);
    select::advance(world,delta);
    let mut state = world.resource_mut::<PhotoState>();
    // The preceding frame has extracted the capture target. Return the field
    // camera to the window while the screenshot's GPU readback completes.
    let restore = state.capture.as_mut().and_then(|capture| {
        if !capture.dispatched || capture.restored { return None; }
        capture.restored = true;
        Some((capture.camera,capture.previous.clone()))
    });
    let timed_out = state.capture.as_mut().is_some_and(|capture| { capture.elapsed += delta; capture.elapsed > 15. });
    let capture = timed_out.then(|| state.capture.take()).flatten();
    if let Some((entity,target))=restore {
        if let Ok(mut camera)=world.get_entity_mut(entity) { camera.insert(target); }
    }
    if let Some(capture) = capture {
        if let Ok(mut camera) = world.get_entity_mut(capture.camera) { camera.insert(capture.previous); }
        if let Ok(mut ready)=world.resource::<CaptureReadiness>().0.lock() { *ready=None; }
        let error = "photo capture did not return an image".to_owned();
        error!("[photo] {error}"); world.resource_mut::<PhotoState>().last_error = Some(error);
    }
    let ready = world.resource::<CaptureReadiness>().0.lock().ok().and_then(|slot|*slot);
    let dispatch = world.resource::<PhotoState>().capture.as_ref().is_some_and(|job|
        !job.dispatched && ready == Some((job.target.id(),true)));
    if dispatch { dispatch_capture(world); }
    let exited = world.resource::<PhotoState>().entered && !world.resource::<ScreenManager>().is_active(SCREEN)
        && !world.resource::<ScreenManager>().is_ui_layer_working();
    if exited {
        stop_call_effect(world);
        select::dispose(world);
        if let Err(error) = crate::npc_objective::events::set_photo_mode(world, false) { error!("[photo] {error}"); }
        let restored=world.get_resource::<crate::game_state::HeldGameState>().map(|held|held.0)
            .unwrap_or(crate::game_state::GameStateType::Normal);
        world.resource_mut::<crate::game_state::GameStateManager>().change_state(restored,"PhotoShotGameState.OnExit");
        crate::camera::change_state(world, crate::camera::CameraStateType::Normal, "photo");
        world.resource_mut::<PhotoState>().entered = false;
    }
}

fn place(windows: Query<&Window, With<PrimaryWindow>>, canvas: Option<Res<crate::canvas::RootCanvas>>, layouts:Res<UiLayouts>,
    screens: Res<ScreenManager>, bindings: Option<Res<PhotoBindings>>, state: Res<PhotoState>,
    mut views: Query<(&mut Visibility, &mut Transform, &mut UiPrefabView), (With<PhotoView>,Without<PhotoHeader>)>,
    mut headers: Query<(&mut Visibility, &mut Transform, &mut UiPrefabView), (With<PhotoHeader>,Without<PhotoView>)>) {
    let (Ok(window), Some(canvas), Some(bindings), Ok((mut visible, mut transform, mut view))) =
        (windows.single(), canvas.as_deref(), bindings.as_deref(), views.single_mut()) else { return; };
    let shown = screens.is_active(SCREEN);
    for (mut visible,mut transform,mut header) in &mut headers {
        *visible=if shown {Visibility::Inherited}else{Visibility::Hidden};transform.scale=Vec3::splat(canvas.scale(window));
        header.set_alpha("",state.selection.as_ref().map_or(1.,|selection|selection.header_fade/0.1));
        if state.selection.as_ref().is_some_and(|selection| selection.playing) { *visible=Visibility::Hidden; }
    }
    *visible = if shown { Visibility::Inherited } else { Visibility::Hidden };
    if !shown { return; }
    let visual = screens.screen_visual(SCREEN);
    transform.scale = visual.scale * canvas.scale(window);
    view.set_alpha("", visual.alpha.unwrap_or(1.));
    let selecting=state.selection.is_some();
    view.set_visible(PHOTO_ROOT, !selecting); view.set_visible(SELECT_ROOT, selecting); view.set_visible(SAVE_ROOT, false);
    view.set_visible(&bindings.cover, state.capture.is_some());
    view.set_visible(&bindings.call_cover, state.capture.is_some());
    view.set_visible("ComponentRoot/Root/PhotoShot/SafeArea/RemainCount", false);
    view.set_visible("ComponentRoot/Root/PhotoShot/VirtualLiveAvatarMotionSelector", false);
    for (path, texture) in &bindings.textures { view.set_texture(path, texture); }
    for path in &bindings.unavailable { if let Ok(id) = path.trim_start_matches('@').parse::<i64>() { view.set_behaviour_enabled(id, false); } }
    for path in &bindings.disabled_covers { view.set_visible(path,true); }
    for (path,alpha) in &bindings.disabled_alpha { view.set_alpha(path,*alpha); }
    if let (Some(tap),Some(white))=(layouts.palette_color(44),layouts.palette_color(2)) {
        let amount=state.call_effect.as_ref().map_or(1.,|effect|effect.fade.map_or(0.,|elapsed| {
            let t=(elapsed/0.2).clamp(0.,1.);1.-(1.-t)*(1.-t)
        }));
        let color=std::array::from_fn(|i|tap[i]+(white[i]-tap[i])*amount);
        for id in &bindings.call_graphics { view.set_graphic_color(*id,color); }
    }
}

fn stop_call_effect(world:&mut World) {
    let previous=world.resource_mut::<PhotoState>().call_effect.take();
    if let Some(effect)=previous {
        for host in effect.hosts { if let Ok(entity)=world.get_entity_mut(host) { entity.despawn(); } }
    }
}

fn start_call_effect(world:&mut World)->Result<(),String> {
    stop_call_effect(world);
    let layouts=world.resource::<UiLayouts>();
    if layouts.palette_color(44).is_none() || layouts.palette_color(2).is_none() || layouts.tween_defaults().is_none() {
        return Err("photo call palette or tween settings are unavailable".into());
    }
    let directory_root=layouts.root().ok_or("photo UI root unavailable")?.trim_start_matches("moly://").trim_end_matches('/').to_owned();
    let effects=world.resource::<PhotoEffects>();
    if !effects.prepared || effects.textures.iter().any(|image|!world.resource::<AssetServer>().is_loaded_with_dependencies(image)) {
        return Err("photo call effect textures are loading".into());
    }
    let handles=effects.documents.clone();
    let docs=world.resource::<Assets<moly_assets::json::JsonAsset>>();
    let mut plans=Vec::new();
    for (index,(file,root)) in CALL_EFFECTS.iter().enumerate() {
        let raw=docs.get(&handles[index]).ok_or("photo call effects are loading")?;
        let doc:serde_json::Value=serde_json::from_str(&raw.0).map_err(|e|e.to_string())?;
        let roots=doc["roots"].as_array().ok_or("photo call effect roots are missing")?;
        if roots.len()!=1 || roots[0]["name"].as_str()!=Some(*root) {return Err("photo call effect root mismatch".into());}
        let nodes=doc["components"]["UIParticle"]["instances"].as_array().ok_or("photo call UIParticle is missing")?;
        if nodes.len()!=1 {return Err("photo call UIParticle is ambiguous".into());}
        let node=nodes[0]["node"].as_str().ok_or("photo call UIParticle path is missing")?.to_owned();
        plans.push((handles[index].clone(),format!("{directory_root}/{}",file.rsplit_once('/').expect("effect directory").0),(*root).to_owned(),node));
    }
    let window=world.query_filtered::<&Window,With<PrimaryWindow>>().single(world).map_err(|_|"photo window is unavailable")?;
    let size=world.resource::<crate::canvas::RootCanvas>().size(window);
    let (canvas,view)=world.query_filtered::<(Entity,&UiPrefabView),With<PhotoView>>().single(world).map_err(|_|"photo view is unavailable")?;
    let bindings=world.resource::<PhotoBindings>();
    let doc=world.resource::<UiLayouts>().document(KEY).ok_or("photo layout unavailable")?;
    let call_index=doc.find(&bindings.call).map_err(|e|e.to_string())?;
    let call_path=&doc.nodes[call_index].path;
    let call_end=doc.nodes.iter().enumerate().filter(|(_,node)|node.path==*call_path || node.path.starts_with(&format!("{call_path}/")))
        .map(|(index,_)|index).max().unwrap_or(call_index);
    let wave_index=doc.find(&bindings.call_wave_root).map_err(|e|e.to_string())?;
    let rect=view.rect(world.resource::<UiLayouts>(),&bindings.call,size).ok_or("photo call button is not laid out")?;
    let position=rect.world.transform_point3(Vec3::ZERO);
    let depths=[20.+call_end as f32*0.02+0.019,20.+wave_index as f32*0.02+0.019];
    let mut hosts=Vec::new();
    for (index,(document,directory,root,node)) in plans.into_iter().enumerate() {
        let host=world.spawn((Transform::from_xyz(position.x,position.y,depths[index]),Visibility::Inherited,crate::ui_particle::UiParticlePlay,
            crate::ui_particle::UiParticleHost {document,directory,root,node,canvas,layer:crate::sitemap::SITEMAP_LAYER,alpha:1.})).id();
        world.entity_mut(canvas).add_child(host);hosts.push(host);
    }
    world.resource_mut::<PhotoState>().call_effect=Some(CallEffect {hosts,elapsed:0.,fade:None});
    Ok(())
}

fn advance_call_effect(world:&mut World,delta:f32) {
    let Some(effect)=world.resource::<PhotoState>().call_effect.as_ref() else {return};
    let status=effect.hosts.first().map(|host|crate::ui_particle::playback(world,*host));
    let refused=effect.hosts.iter().any(|host|crate::ui_particle::playback(world,*host)==crate::ui_particle::UiParticlePlayback::Refused);
    let expired=effect.fade.is_none() && effect.elapsed>15.;
    let mut remove=Vec::new();
    let mut done=false;
    if let Some(effect)=world.resource_mut::<PhotoState>().call_effect.as_mut() {
        effect.elapsed+=delta;
        if let Some(fade)=effect.fade.as_mut() { *fade+=delta;done=*fade>=0.2; }
        else if expired || refused || status==Some(crate::ui_particle::UiParticlePlayback::Complete) {
            remove=std::mem::take(&mut effect.hosts);effect.fade=Some(0.);
        }
    }
    for host in remove { if let Ok(entity)=world.get_entity_mut(host) {entity.despawn();} }
    if expired || refused {
        error!("[photo] call particle playback failed");
        world.resource_mut::<PhotoState>().last_error=Some("photo call effect is unavailable".into());
    }
    if done {world.resource_mut::<PhotoState>().call_effect=None;}
}

fn fit_call_effect(world:&mut World) {
    let Some(effect)=world.resource::<PhotoState>().call_effect.as_ref() else {return};
    if effect.hosts.is_empty() {return;}
    let hosts=effect.hosts.clone();
    let Some(bindings)=world.get_resource::<PhotoBindings>() else {return};
    let call=bindings.call.clone();
    let Ok(window)=world.query_filtered::<&Window,With<PrimaryWindow>>().single(world) else {return};
    let size=world.resource::<crate::canvas::RootCanvas>().size(window);
    let Ok(view)=world.query_filtered::<&UiPrefabView,With<PhotoView>>().single(world) else {return};
    let Some(rect)=view.rect(world.resource::<UiLayouts>(),&call,size) else {return};
    let position=rect.world.transform_point3(Vec3::ZERO);
    let alpha=world.resource::<ScreenManager>().screen_visual(SCREEN).alpha.unwrap_or(1.);
    for host in hosts {
        if let Some(mut transform)=world.get_mut::<Transform>(host) {transform.translation.x=position.x;transform.translation.y=position.y;}
        if let Some(mut particle)=world.get_mut::<crate::ui_particle::UiParticleHost>(host) {particle.alpha=alpha;}
    }
}

fn visible_units(world: &mut World) -> Result<Vec<u32>, String> {
    let camera = world.query_filtered::<(&Camera, &GlobalTransform), With<Camera3d>>().iter(world)
        .find(|(camera,_)| camera.is_active).map(|(camera,pose)| (camera.clone(), *pose));
    let Some((camera, pose)) = camera else { return Err("photo camera is unavailable".into()); };
    let pixel_size = world.query_filtered::<&Window,With<PrimaryWindow>>().iter(world).next()
        .map(|window|UVec2::new(window.physical_width(),window.physical_height()))
        .filter(|size|size.min_element()>0).ok_or("photo screen size is unavailable")?;
    let range = Vec2::new(600., 700.) / pixel_size.as_vec2();
    let candidates: Vec<_> = world.query_filtered::<(Entity, &crate::npc::CharacterUnitId, &GlobalTransform),
        (With<crate::npc::NpcActions>, Without<crate::npc::residency::Away>)>().iter(world)
        .filter(|(_,_,position)| position.translation().distance(pose.translation()) < 12.)
        .map(|(entity,unit,_)| (entity,unit.0)).collect();
    let mut selected=Vec::new();
    for (root,unit) in candidates {
        let pack=world.get::<crate::markers::CharacterPack>(root).ok_or("photo character source is loading")?;
        let document=world.resource::<Assets<moly_assets::json::JsonAsset>>().get(&pack.rig).ok_or("photo character bounds are loading")?;
        let bounds:BoundsDocument=serde_json::from_str(&document.0).map_err(|e|format!("photo renderer bounds: {e}"))?;
        let gltf=world.resource::<Assets<Gltf>>().get(&pack.gltf).ok_or("photo character hierarchy is loading")?;
        let nodes=world.resource::<Assets<GltfNode>>();
        let mut visible=false;
        for bounds in bounds.renderer_bounds {
            let renderer=gltf.nodes.get(bounds.node).and_then(|node|nodes.get(node)).ok_or("photo renderer node is missing")?;
            if renderer.name != bounds.name { return Err("photo renderer identity mismatch".into()); }
            if bounds.update_when_offscreen { return Err("photo source requests dynamic skinned bounds".into()); }
            if bounds.center.iter().chain(&bounds.extents).any(|value|!value.is_finite()) || bounds.extents.iter().any(|value|*value<0.) {
                return Err("invalid photo renderer bounds".into());
            }
            let node=bounds.root_bone.unwrap_or(bounds.node);
            let entity=source_node(world,root,gltf,nodes,node)?;
            let transform=world.get::<GlobalTransform>(entity).ok_or("photo bounds transform is missing")?;
            let center = transform.transform_point(Vec3::from(bounds.center));
            let affine = transform.affine();
            let half = affine.matrix3.abs() * Vec3A::from_array(bounds.extents);
            for x in [-1.,1.] { for y in [-1.,1.] { for z in [-1.,1.] {
                let point = center + Vec3::new(x,y,z) * Vec3::from(half);
                // Unity's WorldToViewportPoint test keeps points beyond the
                // far plane: only a positive camera-space depth is required.
                if pose.affine().inverse().transform_point3(point).z >= 0. { continue; }
                let Some(ndc) = camera.world_to_ndc(&pose, point) else { continue; };
                let uv = (ndc.truncate()+Vec2::ONE)*0.5;
                if uv.cmpge(Vec2::splat(0.5)-range*0.5).all() && uv.cmple(Vec2::splat(0.5)+range*0.5).all() { visible=true; }
            } } }
        }
        if visible { selected.push(unit); }
    }
    Ok(selected)
}

/// The sidecar's node indices address the same glTF hierarchy. Resolve its
/// complete parent/name path, so duplicate leaf bone names cannot cross rigs.
fn source_node(world:&World,root:Entity,gltf:&Gltf,nodes:&Assets<GltfNode>,index:usize)->Result<Entity,String> {
    let mut path=Vec::new();let mut current=index;
    loop {
        let node=gltf.nodes.get(current).and_then(|node|nodes.get(node)).ok_or("photo node index is invalid")?;
        path.push(node.name.as_str());
        let parent=gltf.nodes.iter().filter_map(|node|nodes.get(node)).find(|parent|
            parent.children.iter().any(|child|nodes.get(child).is_some_and(|child|child.index==current)));
        let Some(parent)=parent else {break};current=parent.index;
    }
    path.reverse();
    let mut stack=vec![(root,0usize)];let mut matched=None;
    while let Some((entity,depth))=stack.pop() {
        let name=world.get::<Name>(entity);
        let next=if entity==root || name.is_none() {depth} else if name.is_some_and(|name|name.as_str()==path[depth]) {depth+1} else {continue};
        if next==path.len() { if matched.replace(entity).is_some() {return Err("photo source node path is ambiguous".into());}continue; }
        if let Some(children)=world.get::<Children>(entity) {stack.extend(children.iter().map(|child|(child,next)));}
    }
    matched.ok_or_else(||"photo source node is not instantiated".into())
}

fn capture(world: &mut World, requested: UVec2) -> Result<(), String> {
    if world.resource::<PhotoState>().capture.is_some() || world.resource::<PhotoState>().selection.is_some() { return Ok(()); }
    let (camera, size, previous) = world.query_filtered::<(Entity, &Camera, &RenderTarget), With<Camera3d>>()
        .iter(world).find(|(_,camera,_)| camera.is_active)
        .and_then(|(entity,camera,target)| camera.physical_viewport_size().map(|size| (entity,size,target.clone())))
        .ok_or("photo field camera is unavailable")?;
    if requested.min_element() == 0 || size.min_element() == 0 { return Err("photo frame has no pixels".into()); }
    let mut image = Image::new_fill(Extent3d { width:size.x,height:size.y,depth_or_array_layers:1 },
        TextureDimension::D2, &[0,0,0,255], TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
    let target = world.resource_mut::<Assets<Image>>().add(image);
    *world.resource::<CaptureReadiness>().0.lock().map_err(|_|"photo render readiness unavailable")?=Some((target.id(),false));
    world.resource_mut::<crate::camera::PhotoGestureHeld>().0=true;
    world.resource_mut::<PhotoState>().capture = Some(CaptureJob { camera, previous, target, elapsed:0., requested, dispatched:false, restored:false });
    Ok(())
}

fn dispatch_capture(world: &mut World) {
    let (camera,target,requested) = {
        let mut state=world.resource_mut::<PhotoState>();
        let job=state.capture.as_mut().expect("ready capture job");job.dispatched=true;
        (job.camera,job.target.clone(),job.requested)
    };
    world.entity_mut(camera).insert(RenderTarget::Image(target.clone().into()));
    let capture_target = target.id();
    world.spawn(Screenshot::image(target)).observe(move |captured: On<ScreenshotCaptured>, mut commands: Commands, mut state: ResMut<PhotoState>, readiness: Res<CaptureReadiness>| {
        if !state.capture.as_ref().is_some_and(|job| job.target.id() == capture_target) { return; }
        let job=state.capture.take().expect("matching capture job");
        if !job.restored { commands.entity(camera).insert(job.previous); }
        if let Ok(mut ready)=readiness.0.lock() { *ready=None; }
        let result = captured.image.clone().try_into_dynamic().map_err(|e|e.to_string()).map(|image| {
            let (left,bottom,width,height)=capture_rect(image.width(),image.height(),requested.x,requested.y);
            let crop=image.crop_imm(left,image.height()-bottom-height,width,height).to_rgba8();
            let mut output=image::RgbaImage::from_pixel(requested.x,requested.y,image::Rgba([0,0,0,255]));
            // ReadPixels writes at the lower-left of the requested texture.
            image::imageops::replace(&mut output,&crop,0,(requested.y-height) as i64);
            Image::from_dynamic(image::DynamicImage::ImageRgba8(output),true,RenderAssetUsages::default())
        });
        match result {
            Ok(image)=>{ state.pending=Some(image); }
            Err(error)=>{error!("[photo] {error}");state.last_error=Some(error);}
        }
    });
}

/// The capture camera retains its full render size; the source reads a
/// centered rectangle into a separate texture of the frame's requested size.
fn capture_rect(camera_width:u32,camera_height:u32,width:u32,height:u32)->(u32,u32,u32,u32) {
    let dx=camera_width as i64-width as i64;let dy=camera_height as i64-height as i64;
    let half_x=dx/2;let half_y=dy/2;
    let x=if dx>=-1 {half_x as u32}else{0};let y=if dy>=-1 {half_y as u32}else{0};
    let w=if dx>=-1 {width}else{width-half_x.unsigned_abs() as u32};
    let h=if dy>=-1 {height}else{height-half_y.unsigned_abs() as u32};
    (x,y,w.min(camera_width.saturating_sub(x)),h.min(camera_height.saturating_sub(y)))
}
