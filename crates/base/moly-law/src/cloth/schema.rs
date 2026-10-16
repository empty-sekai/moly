//! rig.json `cloth` 节 → 律侧入参结构的翻译。
//!
//! 数据面以 `sd_*.rig.json` 为准（31 份普查）：
//! * `cloth.components[*]`：`vertices{bones,parent,root,selection,depth}`
//!   + `clothParams`（ClothParams 序列化原样）+ `constraints`
//!   （本批只有 `structDistanceDataList` / `rootDistanceDataList`
//!   两族有数据，条目形 `{vertexIndex,targetVertexIndex,length}`）；
//! * `cloth.colliders[*]`：`{kind, bone, node, pathId, center,
//!   radius | axis,length,startRadius,endRadius}`；
//! * `selectionEnum`：invalid=0 / move=1 / fixed=2 / extend=3。
//!   31 份 rig 全量普查只有 fixed(482)/move(990)——`extend` 与
//!   `invalid` 在解算器里响亮拒绝（不具名其行为，数据来了再说）。
//!
//! 翻译只做形状搬运与响亮报错，不造默认值；`clothParams` 的
//! 消费子集逐字段对应反编译 `ClothParams` 的序列化槽（按声明里的
//! 字段偏移对齐，不按名字对），读不到的参数 = 数据损伤 = `Err`。

use super::bezier::BezierParam;
use super::collider::ColliderDef;

// ---------------------------------------------------------------- 最小 JSON
//
// rig.json 的定向翻译只认对象/数组/字符串/数/布尔/null 五形。
// 本 crate 内 weather::json 已有一份读取器，但其模块声明私有、
// weather/mod.rs 在本单可写范围之外——按「不改别人车道」的规矩
// 在此放第二份最小形。本模块只消费它，性能不敏感。

/// 一个已解析的 JSON 值。
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            JsonValue::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

/// 解析 UTF-8 JSON 文本。失败信息带字节偏移，响亮。
pub fn json_parse(bytes: &[u8]) -> Result<JsonValue, String> {
    let mut p = Parser { b: bytes, i: 0 };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    if p.i != p.b.len() {
        return Err(format!("json: 尾部有多余内容 @{}", p.i));
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("json: 期望 `{}` @{}", c as char, self.i))
        }
    }

    fn value(&mut self) -> Result<JsonValue, String> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(JsonValue::Str(self.string()?)),
            Some(b't') => self.lit("true", JsonValue::Bool(true)),
            Some(b'f') => self.lit("false", JsonValue::Bool(false)),
            Some(b'n') => self.lit("null", JsonValue::Null),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
            _ => Err(format!("json: 意外字符 @{}", self.i)),
        }
    }

    fn lit(&mut self, word: &str, v: JsonValue) -> Result<JsonValue, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(format!("json: 坏字面量 @{}", self.i))
        }
    }

    fn object(&mut self) -> Result<JsonValue, String> {
        self.eat(b'{')?;
        let mut out = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(JsonValue::Object(out));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.eat(b':')?;
            self.skip_ws();
            let v = self.value()?;
            out.push((key, v));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(JsonValue::Object(out));
                }
                _ => return Err(format!("json: 期待 `,` 或 `}}` @{}", self.i)),
            }
        }
    }

    fn array(&mut self) -> Result<JsonValue, String> {
        self.eat(b'[')?;
        let mut out = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(JsonValue::Array(out));
        }
        loop {
            self.skip_ws();
            out.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(JsonValue::Array(out));
                }
                _ => return Err(format!("json: 期待 `,` 或 `]` @{}", self.i)),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.eat(b'"')?;
        let mut out = String::new();
        let mut seg_start = self.i;
        loop {
            match self.peek() {
                None => return Err("json: 字符串未闭合".to_string()),
                Some(b'"') => {
                    let seg = std::str::from_utf8(&self.b[seg_start..self.i])
                        .map_err(|_| "json: 坏 UTF-8")?;
                    out.push_str(seg);
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    let seg = std::str::from_utf8(&self.b[seg_start..self.i])
                        .map_err(|_| "json: 坏 UTF-8")?;
                    out.push_str(seg);
                    self.i += 1;
                    let esc = self.b.get(self.i).copied().ok_or("json: 截断的转义")?;
                    self.i += 1;
                    let decoded = match esc {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hex = self.b.get(self.i..self.i + 4).ok_or("json: 截断的 \\u")?;
                            self.i += 4;
                            let cp = u32::from_str_radix(
                                std::str::from_utf8(hex).map_err(|_| "json: 坏 \\u")?,
                                16,
                            )
                            .map_err(|_| "json: 坏 \\u")?;
                            char::from_u32(cp).ok_or("json: 坏码点")?
                        }
                        _ => return Err(format!("json: 未知转义 @{}", self.i)),
                    };
                    out.push(decoded);
                    seg_start = self.i;
                }
                Some(_) => self.i += 1,
            }
        }
    }

    fn number(&mut self) -> Result<JsonValue, String> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == b'.' || c == b'e' || c == b'E' || c == b'+' || c == b'-' {
                self.i += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| "json: 坏数字")?;
        text.parse::<f64>()
            .map(JsonValue::Num)
            .map_err(|_| format!("json: 坏数字 `{text}`"))
    }
}

// ---------------------------------------------------------------- 数据面

/// `selectionEnum` 语义表原样。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// 1：随物理。
    Move,
    /// 2：钉在动画位置。
    Fixed,
}

/// 一条布料链的拓扑与绑定数据（`cloth.components[*]` 的骨架面）。
#[derive(Debug, Clone, PartialEq)]
pub struct ChainDef {
    /// 组件名（如 `C_hair_N_offset`），诊断用。
    pub name: String,
    /// 提取器按骨名模式的分类（hair/other/…）；**分类输出不是
    /// 真源语义**——"other" 链实为饰品（见 REPORT 未决①），只作
    /// 记录不作行为开关。
    pub class: String,
    /// 骨名（链内序 = 顶点序）。
    pub bones: Vec<String>,
    /// 父顶点下标；链根为 -1。
    pub parent: Vec<i32>,
    /// 所属根顶点下标；根自身为 -1。
    pub root: Vec<i32>,
    /// 选择集。
    pub selection: Vec<Selection>,
    /// 深度 ∈ [0,1]（曲线求值横轴）。
    pub depth: Vec<f32>,
    /// 结构距离条目（directed，rig 双向原样：0→1 与 1→0 各一条）。
    pub struct_distance: Vec<(u32, u32, f32)>,
    /// 根距条目（顶点 → 根）。
    pub root_distance: Vec<(u32, u32, f32)>,
}

/// `clothParams` 的消费子集（字段名 = rig 键名）。
#[derive(Debug, Clone, PartialEq)]
pub struct ClothParamsDef {
    /// 曲线分派算法（sd 数据恒 1；决定限角/回拉取 `-2` 槽）。
    pub algorithm: i64,
    pub radius: BezierParam,
    pub mass: BezierParam,
    pub mass_influence: f32,
    pub use_gravity: bool,
    pub gravity: BezierParam,
    pub gravity_direction: [f32; 3],
    pub use_drag: bool,
    pub drag: BezierParam,
    pub use_max_velocity: bool,
    pub max_velocity: BezierParam,
    /// 米/秒。
    pub max_move_speed: f32,
    /// 度/秒。
    pub max_rotation_speed: f32,
    pub world_move_influence: BezierParam,
    pub world_rotation_influence: BezierParam,
    pub use_collision: bool,
    pub use_clamp_distance_ratio: bool,
    pub clamp_distance_min_ratio: f32,
    pub clamp_distance_max_ratio: f32,
    pub clamp_distance_velocity_influence: f32,
    pub use_clamp_rotation: bool,
    /// `GetClampRotationAngle(algorithm)`：algorithm=1 取本槽（度）。
    pub clamp_rotation_angle: BezierParam,
    /// algorithm≠1 取本槽。
    pub clamp_rotation_angle_alt: BezierParam,
    pub use_restore_rotation: bool,
    /// `GetRestoreRotationPower(algorithm)`：algorithm=1 取本槽。
    pub restore_rotation: BezierParam,
    /// algorithm≠1 取本槽。
    pub restore_rotation_alt: BezierParam,
    pub struct_distance_stiffness: BezierParam,
    pub use_reset_teleport: bool,
    /// 米。
    pub teleport_distance: f32,
    /// 度。
    pub teleport_rotation: f32,
    /// 风参数三件（未决②：本批数据 1.0/0.7/0.6，站点无风源=休眠值）。
    pub wind_influence: f32,
    pub wind_random_scale: f32,
    pub wind_synchronization: f32,
}

/// 整个 `cloth` 节的律侧形。碰撞体的骨/pathId 绑定归上屏接线层
/// （每帧把 [`ColliderDef`] 搬进 `collider::WorldCollider`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ClothRig {
    pub chains: Vec<ChainDef>,
    pub colliders: Vec<ColliderDef>,
}

// ---------------------------------------------------------------- 翻译

fn err_at(what: &str, key: &str) -> String {
    format!("cloth schema: {what}（键 `{key}`）")
}

fn f32_at(v: &JsonValue, key: &str) -> Result<f32, String> {
    v.get(key)
        .and_then(|x| x.as_f64())
        .map(|x| x as f32)
        .ok_or_else(|| err_at("缺 f32", key))
}

/// rig 的布尔两形并存：`useEndValue: 1`（整数）与 `enabled: true`
/// （布尔），实测如此，都收。
fn bool_at(v: &JsonValue, key: &str) -> Result<bool, String> {
    let x = v.get(key).ok_or_else(|| err_at("缺 bool", key))?;
    match x {
        JsonValue::Bool(b) => Ok(*b),
        JsonValue::Num(n) => Ok(*n != 0.0),
        _ => Err(err_at("bool 形状不对", key)),
    }
}

fn bezier_at(v: &JsonValue, key: &str) -> Result<BezierParam, String> {
    let b = v.get(key).ok_or_else(|| err_at("缺 BezierParam", key))?;
    Ok(BezierParam {
        start_value: f32_at(b, "startValue")?,
        end_value: f32_at(b, "endValue")?,
        use_end_value: bool_at(b, "useEndValue")?,
        curve_value: f32_at(b, "curveValue")?,
        use_curve_value: bool_at(b, "useCurveValue")?,
    })
}

/// 三维浮点向量：数组形（碰撞体 center）与对象形（gravityDirection
/// 的 `{x,y,z}`）都收——rig 两种形状并存，实测如此。
fn vec3_at(v: &JsonValue, key: &str) -> Result<[f32; 3], String> {
    let x = v.get(key).ok_or_else(|| err_at("缺 [f32;3]", key))?;
    match x {
        JsonValue::Array(a) if a.len() == 3 => {
            let mut out = [0.0f32; 3];
            for (i, e) in a.iter().enumerate() {
                out[i] = e.as_f64().ok_or_else(|| err_at("非数", key))? as f32;
            }
            Ok(out)
        }
        JsonValue::Object(_) => {
            let mut out = [0.0f32; 3];
            for (i, axis) in ["x", "y", "z"].iter().enumerate() {
                out[i] = f32_at(x, axis)?;
            }
            Ok(out)
        }
        _ => Err(err_at("形状不是 3 数组也不是 {x,y,z}", key)),
    }
}

fn i64_at(v: &JsonValue, key: &str) -> Result<i64, String> {
    v.get(key)
        .and_then(|x| x.as_f64())
        .map(|x| x as i64)
        .ok_or_else(|| err_at("缺整数", key))
}

/// 约束条目表：`[{vertexIndex,targetVertexIndex,length}]`。
fn edge_list(v: &JsonValue, key: &str) -> Result<Vec<(u32, u32, f32)>, String> {
    let empty = [];
    let list = v.get(key).and_then(|x| x.as_array()).unwrap_or(&empty);
    let mut out = Vec::with_capacity(list.len());
    for e in list {
        let a = i64_at(e, "vertexIndex")?;
        let b = i64_at(e, "targetVertexIndex")?;
        let len = f32_at(e, "length")?;
        if a < 0 || b < 0 {
            return Err(err_at("负顶点下标", key));
        }
        out.push((a as u32, b as u32, len));
    }
    Ok(out)
}

fn parse_selection(v: &JsonValue, chain: &str) -> Result<Selection, String> {
    let n = v
        .as_f64()
        .ok_or_else(|| err_at("selection 元素非整数", chain))?;
    match n as i64 {
        1 => Ok(Selection::Move),
        2 => Ok(Selection::Fixed),
        other => Err(format!(
            "cloth schema: 链 `{chain}` 出现 selection={other}：本批 31 份 rig \
             普查只有 move/fixed，extend/invalid 的解算行为在反编译里未转录，不猜"
        )),
    }
}

fn ints(v: &JsonValue, key: &str, n: usize) -> Option<Vec<i32>> {
    let a = v.get(key)?.as_array()?;
    if a.len() != n {
        return None;
    }
    a.iter().map(|x| x.as_f64().map(|f| f as i32)).collect()
}

fn parse_chain(comp: &JsonValue) -> Result<ChainDef, String> {
    let name = comp
        .get("component")
        .and_then(|x| x.as_str())
        .unwrap_or("?")
        .to_string();
    let verts = comp
        .get("vertices")
        .ok_or_else(|| err_at("缺 vertices", &name))?;
    let bones = verts
        .get("bones")
        .and_then(|x| x.as_array())
        .ok_or_else(|| err_at("缺 bones", &name))?
        .iter()
        .map(|b| {
            b.as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| err_at("骨名非字符串", &name))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let n = bones.len();
    if n == 0 {
        return Err(format!("cloth schema: 链 `{name}` 零顶点"));
    }

    let parent = ints(verts, "parent", n).ok_or_else(|| err_at("缺 parent", &name))?;
    // root：rig 有就用；缺则沿 parent 走到链根重建（demo 同名语义）。
    let root = match ints(verts, "root", n) {
        Some(r) => r,
        None => (0..n)
            .map(|i| {
                let mut v = i as i32;
                let mut guard = 0;
                while parent[v as usize] >= 0 && guard <= n {
                    v = parent[v as usize];
                    guard += 1;
                }
                if v == i as i32 {
                    -1
                } else {
                    v
                }
            })
            .collect(),
    };
    let depth = verts
        .get("depth")
        .and_then(|x| x.as_array())
        .ok_or_else(|| err_at("缺 depth", &name))?
        .iter()
        .map(|x| x.as_f64().map(|f| f as f32))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| err_at("depth 非数", &name))?;
    let selection = verts
        .get("selection")
        .and_then(|x| x.as_array())
        .ok_or_else(|| err_at("缺 selection", &name))?
        .iter()
        .map(|s| parse_selection(s, &name))
        .collect::<Result<Vec<_>, _>>()?;
    if depth.len() != n || selection.len() != n || root.len() != n {
        return Err(format!("cloth schema: 链 `{name}` 顶点列长度不一致"));
    }

    let (struct_distance, root_distance) = match comp.get("constraints") {
        Some(c) => (
            edge_list(c, "structDistanceDataList")?,
            edge_list(c, "rootDistanceDataList")?,
        ),
        // constraints 整节缺席 = 没有任何约束表；由 solver 用 parent 边
        // + 绑定长度重建结构距离（见 build_chain，demo 同形）。
        None => (Vec::new(), Vec::new()),
    };

    Ok(ChainDef {
        name,
        class: comp
            .get("class")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        bones,
        parent,
        root,
        selection,
        depth,
        struct_distance,
        root_distance,
    })
}

fn parse_collider(v: &JsonValue) -> Result<ColliderDef, String> {
    let kind = v
        .get("kind")
        .and_then(|x| x.as_str())
        .ok_or_else(|| err_at("缺 kind", "collider"))?
        .to_string();
    let center = vec3_at(v, "center")?;
    match kind.as_str() {
        "sphere" => Ok(ColliderDef::Sphere {
            center,
            radius: f32_at(v, "radius")?,
        }),
        "capsule" => Ok(ColliderDef::Capsule {
            center,
            axis: match i64_at(v, "axis")? {
                0 => 0u8,
                1 => 1,
                2 => 2,
                other => return Err(format!("cloth schema: 胶囊轴 {other} 越界")),
            },
            length: f32_at(v, "length")?,
            start_radius: f32_at(v, "startRadius")?,
            end_radius: f32_at(v, "endRadius")?,
        }),
        "plane" => Ok(ColliderDef::Plane {
            center,
            normal: vec3_at(v, "normalDirection")?,
        }),
        other => Err(format!("cloth schema: 未知碰撞体种类 `{other}`")),
    }
}

fn parse_params(v: &JsonValue) -> Result<ClothParamsDef, String> {
    Ok(ClothParamsDef {
        algorithm: i64_at(v, "algorithm")?,
        radius: bezier_at(v, "radius")?,
        mass: bezier_at(v, "mass")?,
        mass_influence: f32_at(v, "massInfluence")?,
        use_gravity: bool_at(v, "useGravity")?,
        gravity: bezier_at(v, "gravity")?,
        gravity_direction: vec3_at(v, "gravityDirection")?,
        use_drag: bool_at(v, "useDrag")?,
        drag: bezier_at(v, "drag")?,
        use_max_velocity: bool_at(v, "useMaxVelocity")?,
        max_velocity: bezier_at(v, "maxVelocity")?,
        max_move_speed: f32_at(v, "maxMoveSpeed")?,
        max_rotation_speed: f32_at(v, "maxRotationSpeed")?,
        world_move_influence: bezier_at(v, "worldMoveInfluence")?,
        world_rotation_influence: bezier_at(v, "worldRotationInfluence")?,
        use_collision: bool_at(v, "useCollision")?,
        use_clamp_distance_ratio: bool_at(v, "useClampDistanceRatio")?,
        clamp_distance_min_ratio: f32_at(v, "clampDistanceMinRatio")?,
        clamp_distance_max_ratio: f32_at(v, "clampDistanceMaxRatio")?,
        clamp_distance_velocity_influence: f32_at(v, "clampDistanceVelocityInfluence")?,
        use_clamp_rotation: bool_at(v, "useClampRotation")?,
        clamp_rotation_angle: bezier_at(v, "clampRotationAngle")?,
        clamp_rotation_angle_alt: bezier_at(v, "clampRotationAngle2")?,
        use_restore_rotation: bool_at(v, "useRestoreRotation")?,
        restore_rotation: bezier_at(v, "restoreRotation")?,
        restore_rotation_alt: bezier_at(v, "restoreRotation2")?,
        struct_distance_stiffness: bezier_at(v, "structDistanceStiffness")?,
        use_reset_teleport: bool_at(v, "useResetTeleport")?,
        teleport_distance: f32_at(v, "teleportDistance")?,
        teleport_rotation: f32_at(v, "teleportRotation")?,
        wind_influence: f32_at(v, "windInfluence")?,
        wind_random_scale: f32_at(v, "windRandomScale")?,
        wind_synchronization: f32_at(v, "windSynchronization")?,
    })
}

/// 从 rig 根值（或直接 `cloth` 节值）翻译。缺节响亮报错；
/// 链数据空不是错（一个角色可以没有二级物理）。
pub fn cloth_from_value(v: &JsonValue) -> Result<ClothRig, String> {
    let cloth = v.get("cloth").unwrap_or(v);
    let empty = [];
    let chains_json = cloth
        .get("components")
        .and_then(|x| x.as_array())
        .ok_or_else(|| err_at("缺 components", "cloth"))?;
    let mut chains = Vec::with_capacity(chains_json.len());
    for c in chains_json {
        chains.push(parse_chain(c)?);
    }
    let colliders_json = cloth
        .get("colliders")
        .and_then(|x| x.as_array())
        .unwrap_or(&empty);
    let mut colliders = Vec::with_capacity(colliders_json.len());
    for c in colliders_json {
        colliders.push(parse_collider(c)?);
    }
    Ok(ClothRig { chains, colliders })
}

/// 从 `cloth.components[i]` 翻译 clothParams。
pub fn params_from_value(comp: &JsonValue) -> Result<ClothParamsDef, String> {
    let p = comp
        .get("clothParams")
        .ok_or_else(|| err_at("缺 clothParams", "component"))?;
    parse_params(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成 rig：两顶点链（fixed→move）+ 球碰撞体。
    /// 手写 JSON 字符串，形状照 sd_101 实测。
    const RIG: &str = r#"{
        "cloth": {
            "colliders": [
                {"pathId": 7, "kind": "sphere", "bone": "Hips", "center": [0.01, 0.0, 0.01], "radius": 0.075}
            ],
            "components": [
                {
                    "component": "C_hair_N_offset",
                    "class": "hair",
                    "clothParams": {
                        "algorithm": 1,
                        "radius": {"startValue": 0.015, "endValue": 0.023, "useEndValue": 1, "curveValue": 0.0, "useCurveValue": 0},
                        "mass": {"startValue": 5.0, "endValue": 1.0, "useEndValue": 1, "curveValue": 0.0, "useCurveValue": 0},
                        "massInfluence": 0.3,
                        "useGravity": 1,
                        "gravity": {"startValue": 0.0, "endValue": 0.0, "useEndValue": 0, "curveValue": 0.0, "useCurveValue": 0},
                        "gravityDirection": {"x": 0.0, "y": 1.0, "z": 0.0},
                        "useDrag": 1,
                        "drag": {"startValue": 0.03, "endValue": 0.02, "useEndValue": 1, "curveValue": 0.0, "useCurveValue": 0},
                        "useMaxVelocity": 1,
                        "maxVelocity": {"startValue": 3.0, "endValue": 3.0, "useEndValue": 0, "curveValue": 0.0, "useCurveValue": 0},
                        "maxMoveSpeed": 2.0,
                        "maxRotationSpeed": 720.0,
                        "worldMoveInfluence": {"startValue": 1.0, "endValue": 1.0, "useEndValue": 0, "curveValue": 0.0, "useCurveValue": 0},
                        "worldRotationInfluence": {"startValue": 1.0, "endValue": 1.0, "useEndValue": 0, "curveValue": 0.0, "useCurveValue": 0},
                        "useCollision": 1,
                        "useClampDistanceRatio": 1,
                        "clampDistanceMinRatio": 0.7,
                        "clampDistanceMaxRatio": 1.1,
                        "clampDistanceVelocityInfluence": 0.2,
                        "useClampRotation": 1,
                        "clampRotationAngle": {"startValue": 3.0, "endValue": 20.0, "useEndValue": 1, "curveValue": 0.0, "useCurveValue": 0},
                        "clampRotationAngle2": {"startValue": 3.0, "endValue": 20.0, "useEndValue": 1, "curveValue": 0.0, "useCurveValue": 0},
                        "useRestoreRotation": 1,
                        "restoreRotation": {"startValue": 0.11, "endValue": 0.02, "useEndValue": 1, "curveValue": 0.0, "useCurveValue": 0},
                        "restoreRotation2": {"startValue": 0.10, "endValue": 0.02, "useEndValue": 1, "curveValue": 0.0, "useCurveValue": 0},
                        "structDistanceStiffness": {"startValue": 1.0, "endValue": 1.0, "useEndValue": 0, "curveValue": 0.0, "useCurveValue": 0},
                        "useResetTeleport": 0,
                        "teleportDistance": 0.2,
                        "teleportRotation": 45.0,
                        "windInfluence": 1.0,
                        "windRandomScale": 0.7,
                        "windSynchronization": 0.6
                    },
                    "vertices": {
                        "bones": ["C_hair_N_offset", "EX_C_hair_N_01"],
                        "parent": [-1, 0],
                        "root": [-1, 0],
                        "selection": [2, 1],
                        "depth": [0.0, 1.0]
                    },
                    "constraints": {
                        "structDistanceDataList": [
                            {"vertexIndex": 0, "targetVertexIndex": 1, "length": 0.5},
                            {"vertexIndex": 1, "targetVertexIndex": 0, "length": 0.5}
                        ],
                        "rootDistanceDataList": [
                            {"vertexIndex": 1, "targetVertexIndex": 0, "length": 0.5}
                        ]
                    }
                }
            ]
        }
    }"#;

    #[test]
    fn parse_synthetic_rig() {
        let v = json_parse(RIG.as_bytes()).expect("json");
        let rig = cloth_from_value(&v).expect("rig");
        assert_eq!(rig.chains.len(), 1);
        assert_eq!(rig.colliders.len(), 1);
        let c = &rig.chains[0];
        assert_eq!(c.name, "C_hair_N_offset");
        assert_eq!(c.bones, vec!["C_hair_N_offset", "EX_C_hair_N_01"]);
        assert_eq!(c.parent, vec![-1, 0]);
        assert_eq!(c.root, vec![-1, 0]);
        assert_eq!(c.selection, vec![Selection::Fixed, Selection::Move]);
        assert_eq!(c.depth, vec![0.0, 1.0]);
        // 双向结构条目原样保留（去重是 solver 的事）
        assert_eq!(c.struct_distance, vec![(0, 1, 0.5), (1, 0, 0.5)]);
        assert_eq!(c.root_distance, vec![(1, 0, 0.5)]);
        // 碰撞体：球
        assert_eq!(
            rig.colliders[0],
            ColliderDef::Sphere {
                center: [0.01, 0.0, 0.01],
                radius: 0.075
            }
        );
    }

    #[test]
    fn parse_params_algorithm_and_wind() {
        let v = json_parse(RIG.as_bytes()).expect("json");
        let comp = v
            .get("cloth")
            .unwrap()
            .get("components")
            .unwrap()
            .as_array()
            .unwrap();
        let p = params_from_value(&comp[0]).expect("params");
        assert_eq!(p.algorithm, 1);
        // 未决②：风三件原样进结构，不解释不造值
        assert_eq!(
            (p.wind_influence, p.wind_random_scale, p.wind_synchronization),
            (1.0, 0.7, 0.6)
        );
        // 重力：useGravity=1 但曲线恒 0（数据面全零）
        assert!(p.use_gravity);
        assert_eq!(p.gravity.evaluate(0.5), 0.0);
        assert_eq!(p.gravity_direction, [0.0, 1.0, 0.0]);
        // sd_101 原样：teleport 阈 0.2m/45° 但 useResetTeleport=0
        assert!(!p.use_reset_teleport);
        assert_eq!(p.teleport_distance, 0.2);
        assert_eq!(p.teleport_rotation, 45.0);
    }

    /// selection=3（extend）响亮拒绝——本批数据不存在，行为不猜。
    #[test]
    fn extend_selection_is_loud_error() {
        let v = json_parse(
            r#"{"cloth":{"components":[{"component":"x","vertices":{
                "bones":["a","b"],"parent":[-1,0],"root":[-1,0],
                "selection":[2,3],"depth":[0.0,1.0]}}]}}"#.as_bytes(),
        )
        .expect("json");
        let msg = cloth_from_value(&v).unwrap_err();
        assert!(msg.contains("selection=3"), "{msg}");
    }

    /// root 列缺席时按 parent 链重建（demo 同形）。
    #[test]
    fn root_rebuild_from_parent() {
        let v = json_parse(
            r#"{"cloth":{"components":[{"component":"x","vertices":{
                "bones":["a","b","c"],"parent":[-1,0,1],"selection":[2,1,1],
                "depth":[0.0,0.5,1.0]}}]}}"#.as_bytes(),
        )
        .expect("json");
        let rig = cloth_from_value(&v).expect("rig");
        assert_eq!(rig.chains[0].root, vec![-1, 0, 0]);
    }

    /// 缺参数 = 数据损伤 = Err（不造默认值）。
    #[test]
    fn missing_param_is_error() {
        let v = json_parse(
            r#"{"cloth":{"components":[{"component":"x","vertices":{
                "bones":["a"],"parent":[-1],"root":[-1],"selection":[2],"depth":[0.0]}}]}}"#
                .as_bytes(),
        )
        .expect("json");
        let comp = v
            .get("cloth")
            .unwrap()
            .get("components")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(params_from_value(&comp[0]).is_err());
    }

    /// JSON 读取器：字符串转义与非 ASCII（骨名带日文假名的场合也
    /// 不能坏 UTF-8）。
    #[test]
    fn json_strings_and_escapes() {
        let v = json_parse(br#"{"a": "x\nyA\u3042", "b": [1.5, -2, true, null, false]}"#)
            .expect("parse");
        let a = v.get("a").unwrap().as_str().unwrap();
        assert_eq!(a, "x\nyA\u{3042}");
        let b = v.get("b").unwrap().as_array().unwrap();
        assert_eq!(b[0].as_f64(), Some(1.5));
        assert_eq!(b[1].as_f64(), Some(-2.0));
        assert_eq!(b[2].as_bool(), Some(true));
    }
}
