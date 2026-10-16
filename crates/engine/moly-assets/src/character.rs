//! 角色清单与名册的 schema：manifest.json 与 characters.json。
//!
//! 字段名与提取产物一一对应；serde 默认忽略未知字段，产物持续加字段不打扰
//! 这里的读取。分母统计只认 manifest 的 `units` 行数，不 glob 目录——产物
//! 根下常年躺着历次提取的备份目录，glob 会把分母翻倍。

use serde::Deserialize;
use std::collections::BTreeMap;

/// manifest.json：角色包清单。
#[derive(Debug, Deserialize)]
pub struct CharacterManifest {
    pub version: u32,
    pub units: Vec<CharacterEntry>,
}

/// 一行清单：代号与包文件名。
#[derive(Debug, Deserialize)]
pub struct CharacterEntry {
    /// 组合代号，字符串形式的整数（如 "101"）。
    pub unit: String,
    /// 角色主包，自含贴图。
    pub glb: String,
    /// 附属骨架档案（面部图集、挂点等），本crate只透传文件名。
    pub rig: String,
}

impl CharacterManifest {
    /// 解析只发生在装载边界这一次。
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

/// characters.json：角色名册，键是代号减 100（与清单的 101..155 一一对应）。
#[derive(Debug, Deserialize)]
pub struct CharacterRegistry {
    pub characters: BTreeMap<String, CharacterProfile>,
}

/// 名册一行：本crate只消费位移段名与校验用代号，其余字段留给后续消费方。
#[derive(Debug, Deserialize)]
pub struct CharacterProfile {
    #[serde(rename = "unitId")]
    pub unit_id: u32,
    pub name: String,
    pub locomotion: Locomotion,
}

/// 位移段：族名，共享动作库在族名上加 `_S/_L/_E/_O` 段后缀。
#[derive(Debug, Deserialize)]
pub struct Locomotion {
    #[serde(rename = "idleMotion")]
    pub idle_motion: String,
    #[serde(rename = "walkMotion")]
    pub walk_motion: String,
}

impl CharacterRegistry {
    /// 解析只发生在装载边界这一次。
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}
