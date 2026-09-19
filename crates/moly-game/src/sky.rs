//! 天空壳上屏：站点包清单里那个自带网格的 shell 包，配默认现象的渐变条。
//!
//! 天空不是天空盒：网格与材质（渐变窗口 `_GradientMinY`/`_GradientMaxY`、
//! 附加色 `_AdditiveColor`、两个渐变槽 `_RampTex1`/`_RampTex2`）都在天空
//! 壳包里，渐变条内容却来自现象侧——材质记录里两个槽恒为 null，运行时由
//! 当前现象填。天空壳按**属性**找不按名字找：`kind:"shell"` 的包里同时有
//! `EnvironmentSkyView` 脚本、有网格、有几何产物的那个才是；材质记录认
//! 「窗口两端成正经区间＋两个渐变槽在场」。网格或窗口缺一就不画，渐变条
//! 不是 32×1 也不画——缺数据要可见，不造默认值顶替。
//!
//! # 默认现象取 001_sunny，为什么
//!
//! 现象切换是天气系统的活，本单只挂一个默认：001_sunny 是 id 1 的白天
//! 基准档（刷新时段 5–17 时），光照取景用的太阳角就是它的 config 值，
//! 用户验收过的环境层画面描述也是它——同基线，视觉联调时变量最少。
//! D7 落地前不切换；shader 与律都已带双渐变槽与淡化进度，到时只翻参数。
//!
//! # 上屏形状：自定义 Material，不是 StandardMaterial
//!
//! 天空的着色律（UV 翻转采样渐变条＋附加色往返）与 PBR 无一处同形，
//! 套 StandardMaterial 只能靠贴图技巧硬凑。自写一份单 pass 片元，
//! WGSL 逐式镜像 `moly_law::weather::sky` 的律。shader 以字符串内嵌进
//! `Assets<Shader>`（仓里没有供默认资产源的 assets/ 目录；直接插入会走
//! `AssetEvent::Added` 进管线缓存，导入解析照常）。
//!
//! 挂载：glb 默认 scene 里其余节点（Timeline/PostProcess/EffectRoot 等）
//! 都是天气系统的货，本单不展开；只取天空网格自身成一个实体，每帧钉在
//! 相机水平位置（真源把天空钉在玩家位置：天空永远不被走出去）。

use bevy::asset::{AssetPath, LoadState, RecursiveDependencyLoadState};
use bevy::asset::uuid::Uuid;
use bevy::gltf::{Gltf, GltfMesh};
use bevy::image::ImageLoaderSettings;
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use moly_assets::json::JsonAsset;
use moly_law::weather::sky::{self, GradientWindow};
use std::marker::PhantomData;

/// 天空着色程序的稳定句柄：Startup 直接插入 `Assets<Shader>`。
const SKY_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x5d4a_1c39_9b2e_4f60_a7c1_2e8b_6f05_d317),
    PhantomData,
);

/// 默认现象（理由见模块注释）；渐变条文件名从现象清单的 ramp 字段读。
const DEFAULT_PHENOMENON: &str = "001_sunny";

/// 天空实体标记。
#[derive(Component)]
pub struct SkyDome;

/// 天空着色材质：窗口两端、附加色、附加强度与淡化进度、两张渐变条。
/// 律的权威算式在 `moly_law::weather::sky`；这里的字段只是它的喂入。
#[derive(Asset, TypePath, Debug, Clone, AsBindGroup)]
pub(crate) struct SkyGradient {
    /// (minY, maxY, padding, padding); full 16-byte binding for WebGL2.
    #[uniform(0)]
    window: Vec4,
    /// 材质 `_AdditiveColor`，时间轴驱动前的底值。
    #[uniform(1)]
    additive_color: Vec4,
    /// (intensity, fade progress, padding, padding).
    #[uniform(2)]
    params: Vec4,
    #[texture(4)]
    #[sampler(5)]
    ramp1: Handle<Image>,
    #[texture(6)]
    #[sampler(7)]
    ramp2: Handle<Image>,
}

impl Material for SkyGradient {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(SKY_SHADER.clone())
    }
}

impl SkyGradient {
    pub(crate) fn set_timeline_additive(&mut self, color: [f32; 4], intensity: f32) {
        self.additive_color = Vec4::from_array(color);
        self.params.x = intensity;
    }

    /// 天气系统切换现象时翻渐变条与淡化进度（L25 双 ramp mix 的喂入点）：
    /// 淡化期两槽各持一侧、进度 0→1；稳态两槽同持当前档、进度 0（mix 退化
    /// 为直通）。附加强度（params.x）不在这里动——它是时间轴域的量，底值
    /// 恒 0。
    pub(crate) fn set_ramps(
        &mut self,
        ramp1: Handle<Image>,
        ramp2: Handle<Image>,
        progress: f32,
    ) {
        self.ramp1 = ramp1;
        self.ramp2 = ramp2;
        self.params.y = progress.clamp(0.0, 1.0);
    }
}

/// 站点包清单的装载请求；解析成功后即撤。
#[derive(Resource)]
pub(crate) struct ShellListings(Handle<JsonAsset>);

/// 天空壳包定位结果：壳包目录＋几何产物名（侧车的兜底），等侧车装载。
#[derive(Resource)]
pub(crate) struct Sidecar {
    handle: Handle<JsonAsset>,
    directory: String,
    geometry_fallback: String,
}

/// 天空侧的完整计划：材质记录判读完成，等 glTF 与渐变条到齐。
#[derive(Resource)]
pub(crate) struct SkyPlan {
    glb: Handle<Gltf>,
    material_name: String,
    window: GradientWindow,
    additive: [f32; 4],
    /// 附加色是记录里读到的还是默认零（日志里如实区分）。
    additive_from_record: bool,
}

/// 现象清单的装载请求；解析成功后即撤。
#[derive(Resource)]
pub(crate) struct PhenomenaIndex(Handle<JsonAsset>);

/// 默认现象的渐变条请求；到齐后随计划一起消费。
#[derive(Resource)]
pub(crate) struct RampSlot {
    handle: Handle<Image>,
    file: String,
}

/// 一次性闩：天空只铺一次。
#[derive(Resource)]
pub(crate) struct Spawned;

/// Material 管线注册：`Assets<SkyGradient>` 与专用渲染管线集合由它落进
/// App；不挂它，任何引用该材质资产的 system 都会在参数校验上响亮失败。
/// 在 `app()` 里 DefaultPlugins 之后调用一次。
pub(crate) fn install(app: &mut App) {
    app.add_plugins(MaterialPlugin::<SkyGradient>::default());
}

/// Startup：请求装载三路资产，内嵌天空着色程序。
///
/// 路径沿用提取产物布局，在本仓内联成 `moly://` 资产路径——路径助手是
/// moly-assets 的逐域清单，本单不为此碰那个 crate（理由已报留言区）。
pub(crate) fn load(
    mut commands: Commands,
    mut shaders: ResMut<Assets<Shader>>,
    server: Res<AssetServer>,
) {
    // 返回的句柄就是钉死的 SKY_SHADER，丢弃以免 must_use 告警。
    let _ = shaders.insert(
        SKY_SHADER.id(),
        Shader::from_wgsl(
            include_str!("shaders/sky_gradient.wgsl"),
            "moly_game/src/shaders/sky_gradient.wgsl".to_owned(),
        ),
    );
    commands.insert_resource(ShellListings(server.load::<JsonAsset>(AssetPath::from(
        "moly://site/packages.json".to_owned(),
    ))));
    commands.insert_resource(PhenomenaIndex(server.load::<JsonAsset>(AssetPath::from(
        "moly://phenomena/index.json".to_owned(),
    ))));
}

/// Update：按属性从包清单里认出天空壳包，请求它的侧车文档。
///
/// 装载失败响亮 panic（资产边界的唯一拒绝点）；「没有符合条件的包」不是
/// 装载失败，是数据里没有天空——warn 一声后停画，不造替身。
pub(crate) fn parse_packages(
    mut commands: Commands,
    server: Res<AssetServer>,
    listings: Res<Assets<JsonAsset>>,
    handle: Option<Res<ShellListings>>,
) {
    let Some(handle) = handle else {
        return;
    };
    if let LoadState::Failed(err) = server.load_state(&handle.0) {
        panic!("站点包清单装载失败：{err:?}");
    }
    let Some(listings) = listings.get(&handle.0) else {
        return;
    };
    let value: serde_json::Value = serde_json::from_str(&listings.0)
        .unwrap_or_else(|err| panic!("站点包清单不是合法 JSON：{err}"));
    let packages = value
        .get("packages")
        .and_then(|v| v.as_object())
        .unwrap_or_else(|| panic!("站点包清单缺 packages 对象"));
    let hit = packages.values().find(|pkg| {
        pkg.get("kind").and_then(|v| v.as_str()) == Some("shell")
            && pkg
                .pointer("/inventory/scripts/EnvironmentSkyView")
                .is_some()
            && pkg
                .pointer("/inventory/types/Mesh")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                > 0.0
            && pkg
                .pointer("/artifacts/geometry")
                .and_then(|v| v.as_str())
                .is_some()
    });
    let Some(pkg) = hit else {
        warn!("站点包清单里没有自带网格的天空壳包：不画天空（fail-closed）");
        commands.remove_resource::<ShellListings>();
        return;
    };
    let directory = pkg
        .get("directory")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("天空壳包缺 directory"))
        .to_owned();
    let geometry = pkg
        .pointer("/artifacts/geometry")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("天空壳包缺 artifacts.geometry"))
        .to_owned();
    // 侧车与几何同名不同扩展名；几何产物名在侧车里可能有更准的一份。
    let sidecar = format!("moly://site/{directory}/{}.json", strip_ext(&geometry));
    let handle = server.load::<JsonAsset>(AssetPath::from(sidecar));
    commands.insert_resource(Sidecar {
        handle,
        directory,
        geometry_fallback: geometry,
    });
    commands.remove_resource::<ShellListings>();
}

/// Update：从侧车文档里按属性认出天空材质，请求 glTF。
pub(crate) fn parse_sidecar(
    mut commands: Commands,
    server: Res<AssetServer>,
    docs: Res<Assets<JsonAsset>>,
    sidecar: Option<Res<Sidecar>>,
) {
    let Some(sidecar) = sidecar else {
        return;
    };
    if let LoadState::Failed(err) = server.load_state(&sidecar.handle) {
        panic!("天空壳侧车文档装载失败：{err:?}");
    }
    let Some(doc) = docs.get(&sidecar.handle) else {
        return;
    };
    let value: serde_json::Value = serde_json::from_str(&doc.0)
        .unwrap_or_else(|err| panic!("天空壳侧车文档不是合法 JSON：{err}"));
    let materials = value
        .get("materials")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("天空壳侧车缺 materials 数组"));
    // 材质认属性不认名字：窗口两端成正经区间（律判）＋两个渐变槽在场。
    let record = materials.iter().find(|m| {
        m.pointer("/textures/_RampTex1").is_some()
            && m.pointer("/textures/_RampTex2").is_some()
            && GradientWindow::new(
                m.pointer("/floats/_GradientMinY")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(f64::NAN) as f32,
                m.pointer("/floats/_GradientMaxY")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(f64::NAN) as f32,
            )
            .is_some()
    });
    let Some(record) = record else {
        warn!("天空壳包里没有带齐渐变属性的材质：不画天空（fail-closed）");
        commands.remove_resource::<Sidecar>();
        return;
    };
    let window = GradientWindow::new(
        record
            .pointer("/floats/_GradientMinY")
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| panic!("天空材质缺 _GradientMinY")) as f32,
        record
            .pointer("/floats/_GradientMaxY")
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| panic!("天空材质缺 _GradientMaxY")) as f32,
    )
    .expect("上面按窗口过滤过，这里必是正经区间");
    let additive_from_record = record.pointer("/colors/_AdditiveColor").is_some();
    let additive = record
        .pointer("/colors/_AdditiveColor")
        .and_then(|v| v.as_array())
        .map(|items| {
            let channel = |i: usize| items.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            [channel(0), channel(1), channel(2), channel(3)]
        })
        .unwrap_or([0.0; 4]);
    let material_name = record
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("天空材质记录缺 name"))
        .to_owned();
    // 几何产物名以侧车的 geometry.file 为准，包清单的 artifacts.geometry 兜底。
    let file = value
        .pointer("/geometry/file")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| sidecar.geometry_fallback.clone());
    let glb = server.load::<Gltf>(AssetPath::from(format!(
        "moly://site/{}/{}",
        sidecar.directory, file
    )));
    commands.insert_resource(SkyPlan {
        glb,
        material_name,
        window,
        additive,
        additive_from_record,
    });
    commands.remove_resource::<Sidecar>();
}

/// Update：从现象清单取默认现象的渐变条文件，请求贴图。
///
/// 贴图按非 sRGB 读（同存储域原值），让 shader 里的 gamma 往返与律逐式
/// 同形；sRGB 装载会把采样值先转线性，律就得改写一遍才能对上。
pub(crate) fn parse_phenomena(
    mut commands: Commands,
    server: Res<AssetServer>,
    index: Res<Assets<JsonAsset>>,
    handle: Option<Res<PhenomenaIndex>>,
) {
    let Some(handle) = handle else {
        return;
    };
    if let LoadState::Failed(err) = server.load_state(&handle.0) {
        panic!("现象清单装载失败：{err:?}");
    }
    let Some(index) = index.get(&handle.0) else {
        return;
    };
    let value: serde_json::Value = serde_json::from_str(&index.0)
        .unwrap_or_else(|err| panic!("现象清单不是合法 JSON：{err}"));
    value
        .get("phenomena")
        .and_then(|v| v.as_object())
        .unwrap_or_else(|| panic!("现象清单缺 phenomena 对象"));
    let ramp_file = value
        .pointer(&format!("/phenomena/{DEFAULT_PHENOMENON}/ramp/file"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    commands.remove_resource::<PhenomenaIndex>();
    let Some(file) = ramp_file else {
        warn!("现象清单里没有 {DEFAULT_PHENOMENON} 的 ramp 文件：不画天空（fail-closed）");
        return;
    };
    let handle = server.load_with_settings::<Image, _>(
        AssetPath::from(format!("moly://phenomena/{file}")),
        |settings: &mut ImageLoaderSettings| settings.is_srgb = false,
    );
    commands.insert_resource(RampSlot { handle, file });
}

/// Update：glTF 与渐变条都到齐后，铺一次天空实体。
///
/// 认网格也认材质句柄不认名字：glTF 材质与侧车记录同名同源，按句柄匹
/// primitive。网格缺 UV0、渐变条不是 32×1，同属「画不成」——warn 后不画。
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    mut materials: ResMut<Assets<SkyGradient>>,
    plan: Option<Res<SkyPlan>>,
    ramp: Option<Res<RampSlot>>,
    spawned: Option<Res<Spawned>>,
) {
    let (Some(plan), Some(ramp)) = (plan, ramp) else {
        return;
    };
    if spawned.is_some() {
        return;
    }
    if let LoadState::Failed(err) = server.load_state(&ramp.handle) {
        panic!("天空渐变条装载失败：{err:?}");
    }
    if let LoadState::Failed(err) = server.load_state(&plan.glb) {
        panic!("天空壳 glTF 装载失败：{err:?}");
    }
    if let RecursiveDependencyLoadState::Failed(err) =
        server.recursive_dependency_load_state(&plan.glb)
    {
        panic!("天空壳 glTF 的依赖装载失败：{err:?}");
    }
    let loaded = server.is_loaded_with_dependencies(&plan.glb)
        && server.load_state(&ramp.handle).is_loaded();
    if !loaded {
        return;
    }
    let Some(image) = images.get(&ramp.handle) else {
        return;
    };
    if image.width() != sky::RAMP_TEXELS as u32 || image.height() != 1 {
        warn!(
            "渐变条 {} 是 {}×{}，律只认 {}×1：不画天空（fail-closed）",
            ramp.file,
            image.width(),
            image.height(),
            sky::RAMP_TEXELS
        );
        commands.remove_resource::<RampSlot>();
        commands.remove_resource::<SkyPlan>();
        return;
    }
    let Some(gltf) = gltfs.get(&plan.glb) else {
        return;
    };
    let Some(sky_material) = gltf.named_materials.get(plan.material_name.as_str()) else {
        warn!(
            "天空壳 glTF 里没有材质 {}：不画天空（fail-closed）",
            plan.material_name
        );
        commands.remove_resource::<SkyPlan>();
        commands.remove_resource::<RampSlot>();
        return;
    };
    let mut picks: Vec<(String, Handle<Mesh>)> = Vec::new();
    for (name, handle) in &gltf.named_meshes {
        let Some(gltf_mesh) = gltf_meshes.get(handle) else {
            continue;
        };
        for primitive in &gltf_mesh.primitives {
            if primitive.material.as_ref() == Some(sky_material) {
                picks.push((name.to_string(), primitive.mesh.clone()));
            }
        }
    }
    let Some((mesh_name, mesh)) = picks.first() else {
        warn!("天空壳 glTF 里没有用天空材质绘制的网格：不画天空（fail-closed）");
        commands.remove_resource::<SkyPlan>();
        commands.remove_resource::<RampSlot>();
        return;
    };
    let (mesh_name, mesh) = (mesh_name.clone(), mesh.clone());
    let Some(mesh_asset) = meshes.get(&mesh) else {
        return;
    };
    if mesh_asset.attribute(Mesh::ATTRIBUTE_UV_0).is_none() {
        warn!("天空网格 {mesh_name} 缺 UV0：不画天空（fail-closed）");
        commands.remove_resource::<SkyPlan>();
        commands.remove_resource::<RampSlot>();
        return;
    }
    // 渐变条两个槽先同绑默认现象、淡化进度恒 0：不切换，mix 退化为直通。
    // 附加强度底值 0：没有时间轴驱动时附加项整项不出力。
    let material = materials.add(SkyGradient {
        window: Vec4::new(plan.window.min_y, plan.window.max_y, 0.0, 0.0),
        additive_color: Vec4::from(plan.additive),
        params: Vec4::ZERO,
        ramp1: ramp.handle.clone(),
        ramp2: ramp.handle.clone(),
    });
    commands.spawn((
        SkyDome,
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::IDENTITY,
    ));
    commands.insert_resource(Spawned);
    commands.remove_resource::<SkyPlan>();
    commands.remove_resource::<RampSlot>();
    // 一次性参数行：天空真的在画、按什么参数画，全部可从这行与资产复算。
    info!(
        "天空壳上屏：网格 {mesh_name}（材质匹配 {} 处，取 1），窗口 minY={} maxY={}，附加色 {:?}（{}），强度底值 0，ramp1=ramp2={}（默认现象 {DEFAULT_PHENOMENON}，进度 0）",
        picks.len(),
        plan.window.min_y,
        plan.window.max_y,
        plan.additive,
        if plan.additive_from_record { "材质记录" } else { "记录缺席按零" },
        ramp.file,
    );
}

/// PostUpdate：天空钉在相机水平位置（y 归 0），天空永远不被走出去。
pub(crate) fn follow_camera(
    mut domes: Query<&mut Transform, With<SkyDome>>,
    cameras: Query<&Transform, (With<Camera3d>, Without<SkyDome>)>,
    mut announced: Local<bool>,
) {
    let count = domes.iter().len();
    let Ok(camera) = cameras.single() else {
        return;
    };
    for mut transform in &mut domes {
        transform.translation.x = camera.translation.x;
        transform.translation.z = camera.translation.z;
        transform.translation.y = 0.0;
    }
    if !*announced && count > 0 {
        *announced = true;
        info!(
            "天空每帧钉在相机水平位置：天空实体 {count}，相机水平位 ({:.2},{:.2})",
            camera.translation.x, camera.translation.z
        );
    }
}

/// 去掉扩展名（侧车与几何同名不同扩展名的产物约定）。
fn strip_ext(file: &str) -> &str {
    match file.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => file,
    }
}
