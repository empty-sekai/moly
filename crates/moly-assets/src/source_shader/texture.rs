//! Source texture payloads, independent of a preview PNG or sampler default.
use super::{require, sha256, ContentReference, ObjectIdentity, Result, SourceShaderError};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct SourceSampler {
    #[serde(rename = "m_FilterMode")]
    pub filter: i32,
    #[serde(rename = "m_WrapU")]
    pub wrap_u: i32,
    #[serde(rename = "m_WrapV")]
    pub wrap_v: i32,
    #[serde(rename = "m_WrapW")]
    pub wrap_w: i32,
    #[serde(rename = "m_Aniso")]
    pub anisotropy: i32,
    #[serde(rename = "m_MipBias")]
    pub mip_bias: f32,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexturePlane {
    pub layer: u32,
    pub mip: u32,
    pub width: u32,
    pub height: u32,
    pub offset: u64,
    pub bytes: u64,
    pub sha256: String,
    pub encoded_offset: u64,
    pub encoded_bytes: u64,
}
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub enum SourceTextureKind {
    Texture2D,
    Texture2DArray,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceTexture {
    pub schema_version: u32,
    pub source: ObjectIdentity,
    pub kind: SourceTextureKind,
    pub width: u32,
    pub height: u32,
    pub layers: u32,
    pub mip_count: u32,
    pub source_color_space: Option<i32>,
    pub source_texture_format: Option<i32>,
    pub source_graphics_format: Option<i32>,
    pub source_texture_settings: Option<SourceSampler>,
    pub texel_origin: String,
    pub data_order: String,
    pub decoded_format: String,
    pub status: String,
    pub error: Option<String>,
    pub encoded: Option<ContentReference>,
    pub pixels: Option<ContentReference>,
    #[serde(default)]
    pub planes: Vec<TexturePlane>,
}
impl SourceTexture {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let value: Self = serde_json::from_slice(bytes)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        require(
            self.schema_version == 1,
            "unsupported source texture schema",
        )?;
        self.source.validate()?;
        require(
            self.status == "decoded",
            self.error
                .clone()
                .unwrap_or_else(|| "source texture is unavailable".into()),
        )?;
        require(
            self.width > 0
                && self.height > 0
                && self.layers > 0
                && self.mip_count > 0
                && self.mip_count <= 32 - self.width.max(self.height).leading_zeros(),
            "invalid source texture extent or mip count",
        )?;
        require(
            self.kind == SourceTextureKind::Texture2DArray || self.layers == 1,
            "2D source texture has multiple layers",
        )?;
        require(
            self.texel_origin == "unity-lower-left"
                && self.data_order == "layer-major-mips"
                && self.decoded_format == "rgba8unorm",
            "unknown source texture byte layout",
        )?;
        let pixels = self
            .pixels
            .as_ref()
            .ok_or_else(|| SourceShaderError("decoded texture lacks a pixel receipt".into()))?;
        let encoded = self
            .encoded
            .as_ref()
            .ok_or_else(|| SourceShaderError("decoded texture lacks its encoded source".into()))?;
        pixels.validate()?;
        encoded.validate()?;
        require(
            self.planes.len() as u64 == u64::from(self.layers) * u64::from(self.mip_count),
            "source texture has missing or extra mip planes",
        )?;
        let (mut decoded_end, mut encoded_end) = (0u64, 0u64);
        for (i, plane) in self.planes.iter().enumerate() {
            let layer = i as u64 / u64::from(self.mip_count);
            let mip = i as u32 % self.mip_count;
            let width = (self.width >> mip).max(1);
            let height = (self.height >> mip).max(1);
            let bytes = u64::from(width)
                .checked_mul(u64::from(height))
                .and_then(|n| n.checked_mul(4))
                .ok_or_else(|| SourceShaderError("source texture plane size overflow".into()))?;
            require(
                u64::from(plane.layer) == layer
                    && plane.mip == mip
                    && plane.width == width
                    && plane.height == height
                    && plane.bytes == bytes
                    && plane.offset == decoded_end
                    && plane.encoded_offset == encoded_end
                    && plane.encoded_bytes > 0,
                "source mip dimensions, byte range or layer ordering disagree",
            )?;
            require(
                plane.sha256.len() == 64,
                "source mip lacks a content identity",
            )?;
            decoded_end = decoded_end
                .checked_add(bytes)
                .ok_or_else(|| SourceShaderError("decoded texture size overflow".into()))?;
            encoded_end = encoded_end
                .checked_add(plane.encoded_bytes)
                .ok_or_else(|| SourceShaderError("encoded texture size overflow".into()))?;
        }
        require(
            decoded_end == pixels.bytes && encoded_end == encoded.bytes,
            "source texture byte account does not close",
        )
    }
    pub fn verify_pixels(&self, bytes: &[u8]) -> Result<()> {
        self.validate()?;
        self.pixels
            .as_ref()
            .expect("validated pixels")
            .verify(bytes)?;
        for plane in &self.planes {
            let end = plane.offset + plane.bytes;
            let range = bytes
                .get(plane.offset as usize..end as usize)
                .ok_or_else(|| SourceShaderError("source mip exceeds decoded payload".into()))?;
            require(
                sha256(range) == plane.sha256,
                "source texture mip SHA-256 mismatch",
            )?;
        }
        Ok(())
    }
}
