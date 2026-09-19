//! ParticleSystemRenderer stream packing for the current exported layouts.
//! Dedicated semantics do not consume TEXCOORD channels. UV and UV2 occupy
//! two each, followed by the full Custom1/Custom2 vectors. An empty compiled
//! vertexChannels list is not an empty vertex interface.
use bevy::{mesh::VertexBufferLayout, prelude::*, render::render_resource::*};
use moly_assets::source_shader::{ProgramAbi, Result, SourceShaderError};
fn require(value: bool, message: impl Into<String>) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(SourceShaderError(message.into()))
    }
}
use bevy::mesh::{Indices, VertexAttributeValues};

#[derive(Clone, Debug)]
pub struct ParticleStreams {
    custom: bool,
}
impl ParticleStreams {
    pub fn parse(renderer: &serde_json::Value) -> Result<Self> {
        let codes: Vec<u32> = serde_json::from_value(renderer["vertexStreams"]["codes"].clone())?;
        require(
            matches!(
                codes.as_slice(),
                [0, 1, 3, 4] | [0, 1, 3, 4, 5] | [0, 1, 3, 4, 5, 34, 38]
            ),
            format!("unconsumed particle vertex streams {codes:?}"),
        )?;
        require(
            renderer["useCustomVertexStreams"] == true || codes == [0, 1, 3, 4],
            "disabled custom streams disagree with the default layout",
        )?;
        Ok(Self {
            custom: codes.len() == 7,
        })
    }

    pub fn layout(&self, abi: &ProgramAbi) -> Result<VertexBufferLayout> {
        let mut offset = 0;
        let mut attributes = Vec::new();
        for field in abi.interfaces.vertex.iter().filter(|f| f.direction == "in") {
            let format = match (
                field.name.as_str(),
                field.ty.as_str(),
                field.adapter.as_deref(),
            ) {
                ("in_POSITION0", "vec4", Some("vec3-position-w-one")) => VertexFormat::Float32x3,
                ("in_NORMAL0", "vec3", None) => VertexFormat::Float32x3,
                ("in_COLOR0", "vec4", None) => VertexFormat::Float32x4,
                ("in_TEXCOORD0", "vec2", None) => VertexFormat::Float32x2,
                ("in_TEXCOORD1" | "in_TEXCOORD2", "vec4", None) => {
                    VertexFormat::Float32x4
                }
                _ => {
                    return Err(SourceShaderError(format!(
                        "particle stream has no writer for {} {}",
                        field.name, field.ty
                    )));
                }
            };
            attributes.push(VertexAttribute {
                format,
                offset,
                shader_location: field.location,
            });
            offset += format.size();
        }
        Ok(VertexBufferLayout {
            array_stride: offset,
            step_mode: VertexStepMode::Vertex,
            attributes,
        })
    }

    pub fn pack(&self, abi: &ProgramAbi, mesh: &Mesh) -> Result<(Vec<u8>, Vec<u8>, u32)> {
        let layout = self.layout(abi)?;
        let count = mesh.count_vertices();
        let mut bytes = Vec::with_capacity(count * layout.array_stride as usize);
        for vertex in 0..count {
            for field in abi.interfaces.vertex.iter().filter(|f| f.direction == "in") {
                // Native AddDefaultStreamsToChannelInfo maps absent UV channels
                // to offset 0 of its zero UNorm8x4 default stream. GLES uses
                // the same four zeroes when replacing that zero-stride stream
                // with a generic vertex attribute; w is also zero.
                if !self.custom && matches!(field.name.as_str(), "in_TEXCOORD1" | "in_TEXCOORD2") {
                    bytes.extend_from_slice(&[0; 16]);
                    continue;
                }
                let attribute = match field.name.as_str() {
                    "in_POSITION0" => Mesh::ATTRIBUTE_POSITION,
                    "in_NORMAL0" => Mesh::ATTRIBUTE_NORMAL,
                    "in_COLOR0" => Mesh::ATTRIBUTE_COLOR,
                    "in_TEXCOORD0" => Mesh::ATTRIBUTE_UV_0,
                    "in_TEXCOORD1" => crate::billboard::ATTRIBUTE_CUSTOM1,
                    "in_TEXCOORD2" => crate::billboard::ATTRIBUTE_CUSTOM2,
                    _ => unreachable!("validated particle interface"),
                };
                let values: &[f32] = match mesh.attribute(attribute) {
                    Some(VertexAttributeValues::Float32x2(v)) => {
                        v.get(vertex).map(|v| v.as_slice())
                    }
                    Some(VertexAttributeValues::Float32x3(v)) => {
                        v.get(vertex).map(|v| v.as_slice())
                    }
                    Some(VertexAttributeValues::Float32x4(v)) => {
                        v.get(vertex).map(|v| v.as_slice())
                    }
                    _ => None,
                }
                .ok_or_else(|| {
                    SourceShaderError(format!(
                        "particle geometry lacks {} at vertex {vertex}",
                        field.name
                    ))
                })?;
                for (component, value) in values.iter().enumerate() {
                    // Geometry is shared with Bevy consumers in reflected world
                    // space. Original source programs and material vectors use
                    // source world space; undo that reflection at this boundary.
                    let value = if component == 0 && matches!(field.name.as_str(), "in_POSITION0" | "in_NORMAL0") {
                        -*value
                    } else { *value };
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        let indices: Vec<u32> = match mesh.indices() {
            Some(Indices::U32(v)) => v.clone(),
            Some(Indices::U16(v)) => v.iter().map(|v| u32::from(*v)).collect(),
            None => return Err(SourceShaderError("particle geometry lacks indices".into())),
        };
        require(
            indices.iter().all(|i| (*i as usize) < count),
            "particle index exceeds vertex count",
        )?;
        let length = u32::try_from(indices.len()).map_err(|e| SourceShaderError(e.to_string()))?;
        Ok((
            bytes,
            indices.iter().flat_map(|i| i.to_le_bytes()).collect(),
            length,
        ))
    }
}
