//! `.json` 资产的装载层：站点场景包的 sidecar 材质表，与现象 postprocess
//! 解析出的雾全局量。两类内容共用 `.json` 扩展名，装载器按路径分派。
//!
//! sidecar 逐字段解析：缺席容许的字段回空，在场但形状不对的整包拒绝，
//! 不静默取默认值。

use std::collections::BTreeMap;

use crate::material_passes::SourceMaterialPasses;
use bevy::asset::io::Reader;
use bevy::asset::{Asset, AssetLoader, LoadContext};
use bevy::reflect::TypePath;
use moly_law::material::MaterialSlot;
use moly_law::weather::{FogGlobals, PostProcessProfile};
use serde_json::Value;

/// 一个粒子系统记录：发射节点 + 律侧参数块 + 渲染器侧记录。
///
/// 仿真参数**不在此层解析**——`system` 整块原样存进 [`ParticleSystem::system`]，
/// 由消费侧对**实际要跑**的那条单独调粒子律。不能在此层全量解析：律对
/// 带权曲线模式（`weightedMode: 2`，只见于 Mesh 绘制模式的系统）具名拒绝，
/// 在装载层全量解析会让一条永远不被放行的记录拖垮整个包。消费侧的门
/// （绘制模式 / 对齐档）先把那批挡掉，进到律解析的只剩律认得的形状。
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleSystem {
    /// 发射节点路径，相对承载它的那个 prefab 根、不含根自身——与
    /// `inactiveNodes` 同一套坐标系。
    pub node: String,
    /// `system` 块原样（律档案里 `particles[]` 单条的形状）。缺席时为
    /// None——缺席容许（不模拟的系统不解析），在场但不是对象整包拒绝。
    pub system: Option<Value>,
    /// 渲染器记录；`renderer` 缺席时为 None。
    pub renderer: Option<ParticleRenderer>,
}

/// 粒子渲染器记录：呈现层要的那几格。
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleRenderer {
    /// 关着的渲染器不产生 draw。
    pub enabled: bool,
    /// `ParticleSystemRenderMode` 的名字（Billboard / Stretch /
    /// HorizontalBillboard / VerticalBillboard / Mesh / None）。
    pub render_mode: String,
    /// `ParticleSystemRenderSpace`：**View = 0 · World = 1 · Local = 2 ·
    /// Facing = 3 · Velocity = 4**（引擎枚举体与它的脚本绑定两处互证）。
    pub alignment: i64,
    /// 视口占比下限/上限（渲染器的 min/maxParticleSize）。
    pub min_particle_size: f32,
    pub max_particle_size: f32,
    /// 四边形轴心偏移，单位是粒子尺寸的倍数。
    pub pivot: [f32; 3],
    /// 自定义顶点流开关与流名表：`custom1XYZW` / `custom2XYZW` 落在
    /// 逐粒子选择器读的那两个属性槽上。
    pub use_custom_vertex_streams: bool,
    pub vertex_stream_names: Vec<String>,
    /// 内联材质记录；`material` 为 null 时为 None。
    pub material: Option<MaterialSlot>,
    /// Source tag + transparent-queue decision, independent of shader family.
    pub effect_pass: crate::material_passes::EffectPassEligibility,
    pub effect_render_state: Option<crate::material_passes::SourceRenderState>,
    pub render_queue: Option<i32>,
    /// Mesh 绘制模式要实例化的网格。
    ///
    /// **只有 `renderMode == "Mesh"` 的记录带这个键**，Billboard 记录没有
    /// ——所以缺席按空表，不是错。现象语料现算：90 条 Mesh 模式渲染器里
    /// 82 条恰一个网格、8 条为空表。
    pub meshes: Vec<ParticleMesh>,
}

/// 一个网格引用：包内文件 + 包内节点名。
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleMesh {
    /// 相对该现象目录的 glb 路径，例如 `models/mysekai_sekai_pt-b79d7ecb.glb`。
    pub file: String,
    /// glb 里承载这份网格的节点名。
    pub node: String,
}

/// One source material row: shading properties and pass availability share
/// the same identity/index rather than living in parallel vectors.
#[derive(Debug, Clone, PartialEq)]
pub struct SiteMaterialSource {
    pub slot: MaterialSlot,
    pub passes: Option<SourceMaterialPasses>,
}

impl std::ops::Deref for SiteMaterialSource {
    type Target = MaterialSlot;
    fn deref(&self) -> &Self::Target {
        &self.slot
    }
}

/// 一个站点场景包的 sidecar 材质表。
#[derive(Debug, Clone, PartialEq)]
pub struct SiteSidecar {
    pub materials: Vec<SiteMaterialSource>,
    /// 顶层 `textures[]`，URI 原样；`MaterialSlot::textures` 里的下标指它。
    pub texture_uris: Vec<String>,
    /// 每张导出图的著色空间声明（True = sRGB 色彩、False = 线性数据，
    /// 来自源纹理的 m_ColorSpace；PNG 本身不带色彩空间元数据，这是唯一
    /// 存活处）。键 = `texture_uris` 里的 URI。缺席（旧产物）按全 sRGB
    /// ——装载侧的既有默认口径。
    pub texture_colour_space: std::collections::HashMap<String, bool>,
    /// 槽值是 URI 字符串、但不在顶层 `textures[]` 里的 `(材质名, 属性名, URI)`。
    /// 槽不进 `MaterialSlot::textures`；记在这里，消费侧可以具名核对而不是
    /// 把它当成缺件。
    pub unmatched: Vec<(String, String, String)>,
    /// 顶层 `particles[]`：这个包里的粒子系统。缺席回空表（大多数场景包
    /// 没有粒子）；在场而形状不对整包拒绝。
    pub particles: Vec<ParticleSystem>,
}

/// 把 fixture 粒子档案接进同一套材质与渲染器解析（与 `ParticleSystem` 同构的
/// 另一份 sidecar：`emitters[]` 里每条带 `renderer/material`）。
pub fn parse_fixture_particles(bytes: &[u8]) -> Result<SiteSidecar, MolyJsonError> {
    let mut root: Value = serde_json::from_slice(bytes)?;
    let mut particles = root
        .get_mut("emitters")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| shape_err("fixture particle archive: emitters missing"))?
        .clone();
    let mut textures = Vec::<String>::new();
    for particle in &mut particles {
        let Some(material) = particle
            .pointer_mut("/renderer/material")
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        if !material.contains_key("keywords") {
            return Err(shape_err(
                "fixture particle material keywords absent from archive",
            ));
        }
        if let Some(slots) = material.get_mut("textures").and_then(Value::as_object_mut) {
            for slot in slots.values_mut() {
                if slot.is_null() {
                    continue;
                }
                let uri = slot
                    .get("file")
                    .and_then(Value::as_str)
                    .ok_or_else(|| shape_err("fixture particle texture unresolved"))?
                    .to_owned();
                if !textures.contains(&uri) {
                    textures.push(uri.clone());
                }
                *slot = Value::String(uri);
            }
        }
    }
    let sidecar =
        serde_json::json!({"materials": [], "textures": textures, "particles": particles});
    parse_site_sidecar(sidecar.to_string().as_bytes())
}

/// `.json` 的装载产物；按路径后缀分派。
#[derive(Debug, TypePath, Asset)]
pub enum MolyJson {
    SiteSidecar(SiteSidecar),
    PostProcessFog(FogGlobals),
}

/// 把 `postprocess.json` 结尾的路径认成现象档案，其余 `.json` 认成
/// 站点 sidecar。装载器按扩展名注册，一个扩展名只能有一个装载器，
/// 分派只能落在装载器内部。
#[derive(Default, TypePath)]
pub struct MolyJsonLoader;

#[derive(Debug)]
pub enum MolyJsonError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Shape(String),
    Law(String),
}

impl std::fmt::Display for MolyJsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MolyJsonError::Io(err) => write!(f, "io: {err}"),
            MolyJsonError::Json(err) => write!(f, "json: {err}"),
            MolyJsonError::Shape(message) => write!(f, "shape: {message}"),
            MolyJsonError::Law(message) => write!(f, "law: {message}"),
        }
    }
}

impl std::error::Error for MolyJsonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MolyJsonError::Io(err) => Some(err),
            MolyJsonError::Json(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for MolyJsonError {
    fn from(err: std::io::Error) -> Self {
        MolyJsonError::Io(err)
    }
}

impl From<serde_json::Error> for MolyJsonError {
    fn from(err: serde_json::Error) -> Self {
        MolyJsonError::Json(err)
    }
}

impl AssetLoader for MolyJsonLoader {
    type Asset = MolyJson;
    type Settings = ();
    type Error = MolyJsonError;

    fn extensions(&self) -> &[&str] {
        &["json"]
    }

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        if load_context.path().path().ends_with("postprocess.json") {
            let profile = PostProcessProfile::from_bytes(&bytes).map_err(MolyJsonError::Law)?;
            let post = profile.resolve().map_err(MolyJsonError::Law)?;
            // 站点渲染侧没有关雾开关：档案自己的 enabled 门已进采纳结果，
            // 关着时 alpha 为 0、高度雾项精确化简为无贡献。
            Ok(MolyJson::PostProcessFog(post.fog.params.globals(false)))
        } else {
            Ok(MolyJson::SiteSidecar(parse_site_sidecar(&bytes)?))
        }
    }
}

/// 顶层形状：根对象、`materials[]` 数组、`textures[]` 字符串数组；
/// 任一不对整包拒绝。
pub fn parse_site_sidecar(bytes: &[u8]) -> Result<SiteSidecar, MolyJsonError> {
    let root: serde_json::Value = serde_json::from_slice(bytes)?;
    let root = root
        .as_object()
        .ok_or_else(|| shape_err("sidecar: root is not an object"))?;
    let materials = root
        .get("materials")
        .and_then(|value| value.as_array())
        .ok_or_else(|| shape_err("sidecar: materials is not an array"))?;
    let textures = root
        .get("textures")
        .and_then(|value| value.as_array())
        .ok_or_else(|| shape_err("sidecar: textures is not an array"))?;
    let texture_uris = textures
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| shape_err("sidecar: textures[] entry is not a string"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let texture_colour_space = root
        .get("textureColourSpace")
        .and_then(|value| value.as_object())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(uri, flag)| flag.as_bool().map(|flag| (uri.clone(), flag)))
                .collect::<std::collections::HashMap<_, _>>()
        })
        .unwrap_or_default();
    let mut parsed = Vec::with_capacity(materials.len());
    let mut unmatched = Vec::new();
    for (index, item) in materials.iter().enumerate() {
        parsed.push(SiteMaterialSource {
            slot: parse_material_slot(item, &texture_uris, &mut unmatched, index)?,
            passes: SourceMaterialPasses::from_extras(item),
        });
    }
    let particles = parse_particles(root.get("particles"), &texture_uris, &mut unmatched)?;
    Ok(SiteSidecar {
        materials: parsed,
        texture_uris,
        texture_colour_space,
        unmatched,
        particles,
    })
}

/// 顶层 `particles[]`。缺席回空表；在场必须是数组，逐条必须带 `node`。
///
/// `system` 块原样搬运、不在本层解析（原因见 [`ParticleSystem::system`]）。
/// 渲染器那半是呈现层的输入（绘制模式、对齐档、视口占比、自定义顶点流
/// 声明）加一份**与 `materials[]` 条目同形**的内联材质记录，因此材质走
/// 同一个 [`parse_material_slot`]、贴图槽走同一个顶层 `textures[]` 下标表。
fn parse_particles(
    value: Option<&serde_json::Value>,
    texture_uris: &[String],
    unmatched: &mut Vec<(String, String, String)>,
) -> Result<Vec<ParticleSystem>, MolyJsonError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or_else(|| shape_err("sidecar: particles is not an array"))?;
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let ctx = format!("particles[{index}]");
        let object = item
            .as_object()
            .ok_or_else(|| shape_err(format!("{ctx}: not an object")))?;
        let node = object
            .get("node")
            .and_then(|value| value.as_str())
            .ok_or_else(|| shape_err(format!("{ctx}.node is not a string")))?
            .to_string();
        let system = match object.get("system") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) if value.is_object() => Some(value.clone()),
            Some(_) => return Err(shape_err(format!("{ctx}.system is not an object"))),
        };
        let renderer = match object.get("renderer") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(parse_particle_renderer(
                value,
                texture_uris,
                unmatched,
                &ctx,
            )?),
        };
        out.push(ParticleSystem {
            node,
            system,
            renderer,
        });
    }
    Ok(out)
}

/// 一条 `renderer` 记录。呈现层要的每一格都必需在场——缺格拒绝，不取
/// 默认值：绘制模式与对齐档决定四边形的几何，静默取默认会画出一个看起来
/// 合理但错向的东西。
fn parse_particle_renderer(
    value: &serde_json::Value,
    texture_uris: &[String],
    unmatched: &mut Vec<(String, String, String)>,
    ctx: &str,
) -> Result<ParticleRenderer, MolyJsonError> {
    let object = value
        .as_object()
        .ok_or_else(|| shape_err(format!("{ctx}.renderer is not an object")))?;
    let need_f32 = |key: &str| -> Result<f32, MolyJsonError> {
        object
            .get(key)
            .and_then(|value| value.as_f64())
            .map(|value| value as f32)
            .ok_or_else(|| shape_err(format!("{ctx}.renderer.{key} is not a number")))
    };
    let enabled = object
        .get("enabled")
        .and_then(|value| value.as_bool())
        .ok_or_else(|| shape_err(format!("{ctx}.renderer.enabled is not a bool")))?;
    let render_mode = object
        .get("renderMode")
        .and_then(|value| value.as_str())
        .ok_or_else(|| shape_err(format!("{ctx}.renderer.renderMode is not a string")))?
        .to_string();
    let alignment = object
        .get("alignment")
        .and_then(|value| value.as_i64())
        .ok_or_else(|| shape_err(format!("{ctx}.renderer.alignment is not an integer")))?;
    let pivot = object
        .get("pivot")
        .and_then(|value| value.as_array())
        .ok_or_else(|| shape_err(format!("{ctx}.renderer.pivot is not an array")))?;
    if pivot.len() != 3 {
        return Err(shape_err(format!(
            "{ctx}.renderer.pivot: not exactly 3 components"
        )));
    }
    let mut pivot_out = [0.0f32; 3];
    for (component, item) in pivot.iter().enumerate() {
        pivot_out[component] = item.as_f64().ok_or_else(|| {
            shape_err(format!("{ctx}.renderer.pivot[{component}] is not a number"))
        })? as f32;
    }
    let use_custom_vertex_streams = object
        .get("useCustomVertexStreams")
        .and_then(|value| value.as_bool())
        .ok_or_else(|| {
            shape_err(format!(
                "{ctx}.renderer.useCustomVertexStreams is not a bool"
            ))
        })?;
    let vertex_stream_names = match object
        .get("vertexStreams")
        .and_then(|value| value.as_object())
        .and_then(|streams| streams.get("names"))
    {
        None => Vec::new(),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str().map(str::to_string).ok_or_else(|| {
                    shape_err(format!(
                        "{ctx}.renderer.vertexStreams.names entry is not a string"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(shape_err(format!(
                "{ctx}.renderer.vertexStreams.names is not an array"
            )))
        }
    };
    let material = match object.get("material") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(parse_material_slot(
            value,
            texture_uris,
            unmatched,
            // 内联材质没有 materials[] 下标；上下文借渲染器那条的下标。
            0,
        )?),
    };
    // Mesh 绘制模式的网格引用。缺席按空表（Billboard 记录本来就没有这个
    // 键）；在场就必须是对象数组、每项两个字符串，否则整包拒绝。
    let meshes = match object.get("meshes") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|item| {
                let entry = item
                    .as_object()
                    .ok_or_else(|| shape_err(format!("{ctx}.renderer.meshes entry is not an object")))?;
                let field = |key: &str| -> Result<String, MolyJsonError> {
                    entry
                        .get(key)
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                        .ok_or_else(|| {
                            shape_err(format!("{ctx}.renderer.meshes entry {key} is not a string"))
                        })
                };
                Ok(ParticleMesh {
                    file: field("file")?,
                    node: field("node")?,
                })
            })
            .collect::<Result<Vec<_>, MolyJsonError>>()?,
        Some(_) => {
            return Err(shape_err(format!(
                "{ctx}.renderer.meshes is not an array"
            )))
        }
    };
    Ok(ParticleRenderer {
        enabled,
        render_mode,
        alignment,
        min_particle_size: need_f32("minParticleSize")?,
        max_particle_size: need_f32("maxParticleSize")?,
        pivot: pivot_out,
        use_custom_vertex_streams,
        vertex_stream_names,
        effect_render_state: object.get("material").and_then(SourceMaterialPasses::from_extras).and_then(|p| p.effect_state()),
        render_queue: object.get("material").and_then(|m| m.get("renderQueue")).and_then(Value::as_i64).and_then(|v| i32::try_from(v).ok()),
        effect_pass: crate::material_passes::EffectPassEligibility::from_material(
            object.get("material").unwrap_or(&Value::Null)),
        material,
        meshes,
    })
}

fn shape_err(message: impl Into<String>) -> MolyJsonError {
    MolyJsonError::Shape(message.into())
}

/// 逐字段填 [`MaterialSlot`]。缺席容许：shader 回空串、keywords/textures/
/// floats/colors/textureScaleOffset 回空表；`renderQueue` 与
/// `textureArrays` 不读。
fn parse_material_slot(
    value: &serde_json::Value,
    texture_uris: &[String],
    unmatched: &mut Vec<(String, String, String)>,
    index: usize,
) -> Result<MaterialSlot, MolyJsonError> {
    let ctx = format!("materials[{index}]");
    let object = value
        .as_object()
        .ok_or_else(|| shape_err(format!("{ctx}: not an object")))?;
    let name = object
        .get("name")
        .and_then(|value| value.as_str())
        .ok_or_else(|| shape_err(format!("{ctx}.name is not a string")))?
        .to_string();
    // 站点材质用对象的 name，家具粒子档案直接用字符串；两种形状都保留
    // 原始族名。缺席或形状不对仍回空串，由消费侧具名拒绝，不猜族。
    let shader = match object.get("shader") {
        Some(serde_json::Value::String(name)) => name.as_str(),
        Some(serde_json::Value::Object(shader)) => shader
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        _ => "",
    }
    .to_string();
    let keywords = match object.get("keywords") {
        None => Vec::new(),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| shape_err(format!("{ctx}.keywords entry is not a string")))
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(shape_err(format!("{ctx}.keywords is not an array"))),
    };
    let textures =
        parse_texture_slots(object.get("textures"), &name, texture_uris, unmatched, &ctx)?;
    let floats = parse_float_map(object.get("floats"), &ctx)?;
    let colors = parse_color_map(object.get("colors"), &ctx)?;
    let texture_scale_offsets = parse_st_map(object.get("textureScaleOffset"), &ctx)?;
    Ok(MaterialSlot {
        name,
        shader,
        keywords,
        textures,
        colors,
        texture_scale_offsets,
        floats,
    })
}

/// Fixture GLB metadata uses texture indices and validKeywords rather than
/// sidecar URIs. Normalize that representation once at the asset boundary.
pub fn parse_fixture_material(name: &str, extras: &Value) -> Result<MaterialSlot, MolyJsonError> {
    let mut value = extras.clone();
    let object = value
        .as_object_mut()
        .ok_or_else(|| shape_err("fixture material extras must be an object"))?;
    object.insert("name".into(), Value::String(name.into()));
    let keywords = object
        .get("validKeywords")
        .cloned()
        .ok_or_else(|| shape_err(format!("fixture material {name}: validKeywords missing")))?;
    object.insert("keywords".into(), keywords);
    // The table carries identities only; no URI is invented or fetched here.
    let mut indices = Vec::new();
    if let Some(textures) = object.get_mut("textures").and_then(Value::as_object_mut) {
        for (property, value) in textures {
            if value.is_null() {
                continue;
            }
            let index = value
                .as_u64()
                .and_then(|v| usize::try_from(v).ok())
                .ok_or_else(|| {
                    shape_err(format!(
                        "fixture material {name}: invalid {property} texture index"
                    ))
                })?;
            indices.push((property.clone(), index));
            *value = Value::Null;
        }
    }
    let mut slot = parse_material_slot(&value, &[], &mut Vec::new(), 0)?;
    slot.textures = indices;
    Ok(slot)
}

/// 槽值：`null` 跳过；URI 字符串在顶层 `textures[]` 里精确等值匹配取下标，
/// 匹配不上记进 `unmatched` 并跳过该槽；其它形状拒绝。
fn parse_texture_slots(
    value: Option<&serde_json::Value>,
    material_name: &str,
    texture_uris: &[String],
    unmatched: &mut Vec<(String, String, String)>,
    ctx: &str,
) -> Result<Vec<(String, usize)>, MolyJsonError> {
    let mut slots = Vec::new();
    let Some(value) = value else {
        return Ok(slots);
    };
    let map = value
        .as_object()
        .ok_or_else(|| shape_err(format!("{ctx}.textures is not an object")))?;
    for (prop, slot) in map {
        match slot {
            serde_json::Value::Null => {}
            serde_json::Value::String(uri) => match texture_uris.iter().position(|u| u == uri) {
                Some(index) => slots.push((prop.clone(), index)),
                None => unmatched.push((material_name.to_string(), prop.clone(), uri.clone())),
            },
            _ => {
                return Err(shape_err(format!(
                    "{ctx}.textures.{prop} is neither null nor a string"
                )))
            }
        }
    }
    Ok(slots)
}

fn parse_float_map(
    value: Option<&serde_json::Value>,
    ctx: &str,
) -> Result<BTreeMap<String, f32>, MolyJsonError> {
    let mut out = BTreeMap::new();
    let Some(value) = value else {
        return Ok(out);
    };
    let map = value
        .as_object()
        .ok_or_else(|| shape_err(format!("{ctx}.floats is not an object")))?;
    for (key, entry) in map {
        let Some(number) = entry.as_f64() else {
            return Err(shape_err(format!("{ctx}.floats.{key} is not a number")));
        };
        out.insert(key.clone(), number as f32);
    }
    Ok(out)
}

fn parse_color_map(
    value: Option<&serde_json::Value>,
    ctx: &str,
) -> Result<BTreeMap<String, [f32; 4]>, MolyJsonError> {
    let mut out = BTreeMap::new();
    let Some(value) = value else {
        return Ok(out);
    };
    let map = value
        .as_object()
        .ok_or_else(|| shape_err(format!("{ctx}.colors is not an object")))?;
    for (key, entry) in map {
        let rgba = parse_rgba(entry, format!("{ctx}.colors.{key}"))?;
        out.insert(key.clone(), rgba);
    }
    Ok(out)
}

/// RGBA 恰四分量。分量可以是数，也可以是 IEEE 哨兵串
/// `"Infinity"` / `"-Infinity"` / `"NaN"`，按非有限 f32 原样保留：
/// 清洗成 0 会改变消费侧的分支行为。
fn parse_rgba(value: &serde_json::Value, ctx: String) -> Result<[f32; 4], MolyJsonError> {
    let items = value
        .as_array()
        .ok_or_else(|| shape_err(format!("{ctx}: not an array")))?;
    if items.len() != 4 {
        return Err(shape_err(format!("{ctx}: not exactly 4 components")));
    }
    let mut out = [0.0; 4];
    for (component, item) in items.iter().enumerate() {
        out[component] = match item {
            serde_json::Value::Number(number) => number.as_f64().unwrap_or_default() as f32,
            serde_json::Value::String(text) => match text.as_str() {
                "Infinity" => f32::INFINITY,
                "-Infinity" => f32::NEG_INFINITY,
                "NaN" => f32::NAN,
                _ => {
                    return Err(shape_err(format!(
                        "{ctx}[{component}]: not a number or IEEE sentinel"
                    )))
                }
            },
            _ => {
                return Err(shape_err(format!(
                    "{ctx}[{component}]: not a number or IEEE sentinel"
                )))
            }
        };
    }
    Ok(out)
}

/// 每个纹理属性的 `(scaleX, scaleY, offsetX, offsetY)`，恰四分量、
/// 全有限数；独立于 `textures` 存——槽带变换而图像为空是常态。
fn parse_st_map(
    value: Option<&serde_json::Value>,
    ctx: &str,
) -> Result<BTreeMap<String, [f32; 4]>, MolyJsonError> {
    let mut out = BTreeMap::new();
    let Some(value) = value else {
        return Ok(out);
    };
    let map = value
        .as_object()
        .ok_or_else(|| shape_err(format!("{ctx}.textureScaleOffset is not an object")))?;
    for (key, entry) in map {
        let items = entry
            .as_array()
            .ok_or_else(|| shape_err(format!("{ctx}.textureScaleOffset.{key}: not an array")))?;
        if items.len() != 4 {
            return Err(shape_err(format!(
                "{ctx}.textureScaleOffset.{key}: not exactly 4 components"
            )));
        }
        let mut st = [0.0; 4];
        for (component, item) in items.iter().enumerate() {
            let Some(number) = item.as_f64() else {
                return Err(shape_err(format!(
                    "{ctx}.textureScaleOffset.{key}[{component}]: not a number"
                )));
            };
            if !number.is_finite() {
                return Err(shape_err(format!(
                    "{ctx}.textureScaleOffset.{key}[{component}]: not finite"
                )));
            }
            st[component] = number as f32;
        }
        out.insert(key.clone(), st);
    }
    Ok(out)
}
