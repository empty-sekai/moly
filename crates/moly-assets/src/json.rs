//! 清单类 JSON 提取产物的 Bevy Asset 翻译：整文件读成一个字符串资产。
//!
//! 这类文件不匹配任何内置装载器的形状；自定义装载器让它们走与 glb 同一条
//! AssetServer 通路——native 与 web 同路，装载延迟与失败归同一套判据，不另
//! 开旁路。schema 的解释归各域模块，这里只管把字节变成字符串。

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
}
