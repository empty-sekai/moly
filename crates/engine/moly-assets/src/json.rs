//! Bevy Asset translation of manifest-style JSON extraction output: the whole
//! file is read into one string asset.
//!
//! Such files match the shape of no built-in loader; a custom loader routes
//! them through the same AssetServer path as glb files — native and web share
//! the path, load latency and failures go through the same checks, and no
//! side channel is opened. Interpreting the schema belongs to each domain
//! module; this module only turns bytes into a string.
//!
//! Master tables are JSON assets too; [`master`] loads them from the regions'
//! master hosts and names each one it cannot give its consumer, and
//! `master_merge` holds the rows of a table across the regions.

pub mod master;
mod master_merge;

use bevy::app::App;
use bevy::asset::io::Reader;
use bevy::asset::{Asset, AssetApp, AssetLoader, LoadContext};
use bevy::reflect::TypePath;

/// 原始 JSON 文本资产。
#[derive(Asset, TypePath, Debug)]
pub struct JsonAsset(pub String);

/// 装载失败：读盘或非 UTF-8 文本。
#[derive(Debug)]
pub struct JsonAssetError(pub String);

impl std::fmt::Display for JsonAssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for JsonAssetError {}

/// 把 JSON 字节读成 UTF-8 字符串；解析归消费方。
#[derive(TypePath)]
pub struct JsonAssetLoader;

impl AssetLoader for JsonAssetLoader {
    type Asset = JsonAsset;

    type Settings = ();

    type Error = JsonAssetError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| JsonAssetError(e.to_string()))?;
        String::from_utf8(bytes)
            .map(JsonAsset)
            .map_err(|e| JsonAssetError(e.to_string()))
    }

    fn extensions(&self) -> &[&str] {
        &["json"]
    }
}

/// 注册 JSON 资产类型与装载器。必须在 DefaultPlugins 之后调用：
/// AssetServer 由 AssetPlugin 创建，装载器注册落在 AssetServer 上。
pub fn register(app: &mut App) {
    app.init_asset::<JsonAsset>()
        .register_asset_loader(JsonAssetLoader);
    master::register(app);
}
