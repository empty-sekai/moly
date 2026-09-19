//! AssetServer loading for the formal source shader and texture artefacts.
//! The same dependency reads work with native, browser and packed asset sources.
use super::{
    require, texture::SourceTexture, ContentReference, ProgramReceipt, Result,
    SourceShaderCatalogue, SourceShaderError,
};
use bevy::{
    asset::{io::Reader, AssetApp, AssetLoader, AssetPath, LoadContext},
    prelude::*,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Asset, TypePath, Debug, Clone)]
pub struct SourceProgramAsset {
    pub receipt: Arc<ProgramReceipt>,
    /// The source program with its independently validated output-only clip
    /// adapter. Unadapted GLES clip positions are never exposed as this handle.
    #[dependency]
    pub vertex: Handle<Shader>,
    #[dependency]
    pub fragment: Handle<Shader>,
}
#[derive(Asset, TypePath, Debug, Clone)]
pub struct SourceTextureAsset {
    pub source: Arc<SourceTexture>,
    /// Unflipped layer-major pixels, including every source mip. A renderer
    /// chooses a qualified colour-space view and sampler; loading does not.
    pub pixels: Arc<[u8]>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceLoadSettings {
    /// Optional extraction root for a producer using non-default directory
    /// prefixes. Otherwise the formal shaders/shader-textures path is required.
    pub root: Option<String>,
}

fn extraction_root(
    path: &AssetPath<'static>,
    settings: &SourceLoadSettings,
    folders: &[&str],
) -> Result<AssetPath<'static>> {
    if let Some(root) = &settings.root {
        let root = AssetPath::try_parse(root)
            .map_err(|e| SourceShaderError(e.to_string()))?
            .into_owned();
        require(
            root.label().is_none() && root.source() == path.source(),
            "source content root changes the asset namespace",
        )?;
        return Ok(root);
    }
    let mut root = path
        .parent()
        .ok_or_else(|| SourceShaderError("source artefact has no parent directory".into()))?;
    for expected in folders {
        require(
            root.path().file_name().and_then(|s| s.to_str()) == Some(*expected),
            "non-standard source artefact path requires an explicit extraction root",
        )?;
        root = root
            .parent()
            .ok_or_else(|| SourceShaderError("source extraction root is absent".into()))?;
    }
    Ok(root.into_owned())
}

async fn read_document(reader: &mut dyn Reader, path: &AssetPath<'static>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| SourceShaderError(e.to_string()))?;
    let filename = path
        .path()
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| SourceShaderError("source document filename is invalid".into()))?;
    let expected = filename.split('.').next().unwrap_or("");
    require(
        expected.len() == 64 && super::sha256(&bytes) == expected,
        "source document bytes differ from their content-addressed filename",
    )?;
    Ok(bytes)
}

async fn dependency(
    context: &mut LoadContext<'_>,
    root: &AssetPath<'static>,
    reference: &ContentReference,
) -> Result<Vec<u8>> {
    reference.validate()?;
    let path = root
        .resolve(&reference.file)
        .map_err(|e| SourceShaderError(e.to_string()))?;
    let bytes = context
        .read_asset_bytes(path)
        .await
        .map_err(|e| SourceShaderError(e.to_string()))?;
    reference.verify(&bytes)?;
    Ok(bytes)
}

#[derive(TypePath)]
pub struct SourceCatalogueLoader;
impl AssetLoader for SourceCatalogueLoader {
    type Asset = SourceShaderCatalogue;
    type Settings = SourceLoadSettings;
    type Error = SourceShaderError;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &Self::Settings,
        context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset> {
        let bytes = read_document(reader, context.path()).await?;
        let catalogue = SourceShaderCatalogue::parse(&bytes)?;
        let root = extraction_root(context.path(), settings, &["shaders"])?;
        // Verify the full source document too. A valid compact catalogue is not
        // permission to substitute a different binary program/source identity.
        let original: serde_json::Value =
            serde_json::from_slice(&dependency(context, &root, &catalogue.source_document).await?)?;
        let identity: super::ObjectIdentity = serde_json::from_value(original["source"].clone())?;
        require(
            identity == catalogue.source,
            "source catalogue and shader document have different owners",
        )?;
        Ok(catalogue)
    }
    fn extensions(&self) -> &[&str] {
        &["variants.json"]
    }
}

#[derive(TypePath)]
pub struct SourceProgramLoader;
impl AssetLoader for SourceProgramLoader {
    type Asset = SourceProgramAsset;
    type Settings = SourceLoadSettings;
    type Error = SourceShaderError;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &Self::Settings,
        context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset> {
        let bytes = read_document(reader, context.path()).await?;
        let receipt = ProgramReceipt::parse(&bytes)?;
        let root = extraction_root(context.path(), settings, &["compiled", "shaders"])?;
        // Code identity is verified independently of the translated output.
        dependency(context, &root, &receipt.source.program.code).await?;
        dependency(context, &root, &receipt.stages.vertex.shader).await?;
        let adapter = receipt.stages.vertex.renderer.as_ref().ok_or_else(|| {
            SourceShaderError("source vertex shader lacks a qualified renderer clip adapter".into())
        })?;
        let vertex_bytes = dependency(context, &root, &adapter.shader).await?;
        let fragment = dependency(context, &root, &receipt.stages.fragment.shader).await?;
        let vertex = context.add_labeled_asset(
            "Vertex".into(),
            Shader::from_wgsl(
                String::from_utf8(vertex_bytes).map_err(|e| SourceShaderError(e.to_string()))?,
                adapter.shader.file.clone(),
            ),
        );
        let fragment = context.add_labeled_asset(
            "Fragment".into(),
            Shader::from_wgsl(
                String::from_utf8(fragment).map_err(|e| SourceShaderError(e.to_string()))?,
                receipt.stages.fragment.shader.file.clone(),
            ),
        );
        Ok(SourceProgramAsset {
            receipt: Arc::new(receipt),
            vertex,
            fragment,
        })
    }
    fn extensions(&self) -> &[&str] {
        &["program.json"]
    }
}

#[derive(TypePath)]
pub struct SourceTextureLoader;
impl AssetLoader for SourceTextureLoader {
    type Asset = SourceTextureAsset;
    type Settings = SourceLoadSettings;
    type Error = SourceShaderError;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &Self::Settings,
        context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset> {
        let bytes = read_document(reader, context.path()).await?;
        let source = SourceTexture::parse(&bytes)?;
        let root = extraction_root(context.path(), settings, &["shader-textures"])?;
        let pixels = dependency(
            context,
            &root,
            source.pixels.as_ref().expect("validated pixels"),
        )
        .await?;
        source.verify_pixels(&pixels)?;
        Ok(SourceTextureAsset {
            source: Arc::new(source),
            pixels: Arc::from(pixels),
        })
    }
    fn extensions(&self) -> &[&str] {
        &["texture.json"]
    }
}

/// Register after AssetPlugin/ShaderPlugin. Metadata loaders take precedence
/// over the generic JSON loader only for their own qualified file extensions.
pub fn register(app: &mut App) {
    app.init_asset::<SourceShaderCatalogue>()
        .init_asset::<SourceProgramAsset>()
        .init_asset::<SourceTextureAsset>()
        .register_asset_loader(SourceCatalogueLoader)
        .register_asset_loader(SourceProgramLoader)
        .register_asset_loader(SourceTextureLoader);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dependency_root_preserves_pack_namespace_and_rejects_unqualified_layout() {
        let path = AssetPath::from("pack://phenomena/shaders/compiled/p.program.json").into_owned();
        let root = extraction_root(
            &path,
            &SourceLoadSettings::default(),
            &["compiled", "shaders"],
        )
        .unwrap();
        assert_eq!(root.to_string(), "pack://phenomena");
        assert_eq!(
            root.resolve("shader-textures/t.rgba8").unwrap().path(),
            &std::path::Path::new("phenomena")
                .join("shader-textures")
                .join("t.rgba8")
        );
        assert!(extraction_root(
            &path,
            &SourceLoadSettings {
                root: Some("other://phenomena".into())
            },
            &[]
        )
        .is_err());
        assert!(extraction_root(&path, &SourceLoadSettings::default(), &["wrong"]).is_err());
    }
}
