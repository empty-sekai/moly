//! ParticleSystemRenderer stream packing for the current exported layouts.
//! Dedicated semantics do not consume TEXCOORD channels. UV and UV2 occupy
//! two each, followed by the full Custom1/Custom2 vectors. An empty compiled
//! vertexChannels list is not an empty vertex interface.
use bevy::{
    mesh::{MeshVertexAttribute, VertexBufferLayout},
    prelude::*,
    render::render_resource::*,
};
use moly_assets::source_shader::{ProgramAbi, Result, SourceShaderError};
fn require(value: bool, message: impl Into<String>) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(SourceShaderError(message.into()))
    }
}
use bevy::mesh::{Indices, VertexAttributeValues};

#[derive(Clone, Debug, PartialEq)]
pub struct ParticleStreams {
    custom: bool,
}
impl ParticleStreams {
    pub fn parse(renderer: &serde_json::Value) -> Result<Self> {
        let codes: Vec<u32> = serde_json::from_value(renderer["vertexStreams"]["codes"].clone())?;
        if !matches!(
            codes.as_slice(),
            [0, 1, 3, 4] | [0, 1, 3, 4, 5] | [0, 1, 3, 4, 5, 34, 38]
        ) {
            return Err(SourceShaderError(format!(
                "unconsumed particle vertex streams {codes:?}"
            )));
        }
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
                // The position stream is three floats; a float3 input reads them unpadded.
                ("in_POSITION0", "vec3", Some("vec3-position")) => VertexFormat::Float32x3,
                ("in_NORMAL0", "vec3", None) => VertexFormat::Float32x3,
                ("in_COLOR0", "vec4", None) => VertexFormat::Float32x4,
                ("in_TEXCOORD0", "vec2", None) => VertexFormat::Float32x2,
                // The default layout's UV stream is two floats; a float4 input
                // gets the vertex fetch's fill for the absent components, 0 and 1.
                ("in_TEXCOORD0", "vec4", None) if !self.custom => VertexFormat::Float32x4,
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

    /// Pack `geometry` for `abi` into `vertices` and `indices`, replacing
    /// their contents (their allocations are reused), and return the index
    /// count. After an error the two buffers hold no meaningful bytes.
    pub fn pack(
        &self,
        abi: &ProgramAbi,
        geometry: &ParticleGeometry,
        vertices: &mut Vec<u8>,
        indices: &mut Vec<u8>,
    ) -> Result<u32> {
        // The layout's refusals (an input with no writer) come first.
        self.layout(abi)?;
        let count = geometry.vertex_count;
        // Resolve each input's attribute column once; the per-vertex loop
        // below reads the same values the attribute lookups would.
        let columns: Vec<Column> = abi
            .interfaces
            .vertex
            .iter()
            .filter(|f| f.direction == "in")
            .map(|field| {
                // Native AddDefaultStreamsToChannelInfo maps absent UV channels
                // to offset 0 of its zero UNorm8x4 default stream. GLES uses
                // the same four zeroes when replacing that zero-stride stream
                // with a generic vertex attribute; w is also zero.
                if !self.custom && matches!(field.name.as_str(), "in_TEXCOORD1" | "in_TEXCOORD2") {
                    return Column::Zero;
                }
                let channel = Channel::of_input(&field.name).expect("validated particle interface");
                let column = &geometry.columns[channel as usize];
                Column::Values {
                    name: &field.name,
                    data: &column.data,
                    width: column.width,
                    // Geometry is shared with Bevy consumers in reflected world
                    // space. Original source programs and material vectors use
                    // source world space; undo that reflection at this boundary.
                    reflect: matches!(field.name.as_str(), "in_POSITION0" | "in_NORMAL0"),
                    fill_zw: field.name == "in_TEXCOORD0" && field.ty == "vec4",
                }
            })
            .collect();
        // The first refusal in vertex order, then input order: an input's
        // first vertex without values, or a UV fill over a stream that is
        // not two-wide, refused at the first vertex that has values.
        let refusal = columns
            .iter()
            .filter_map(|column| match column {
                Column::Zero => None,
                Column::Values { name, data, width, fill_zw, .. } => {
                    let present = if *width == 0 { 0 } else { data.len() / width };
                    if count > 0 && present > 0 && *fill_zw && *width != 2 {
                        Some((0, SourceShaderError(format!(
                            "particle geometry {name} has {width} components, not the UV stream's two"
                        ))))
                    } else {
                        (present < count).then(|| (present, SourceShaderError(format!(
                            "particle geometry lacks {name} at vertex {present}"
                        ))))
                    }
                }
            })
            .min_by_key(|(vertex, _)| *vertex);
        if let Some((_, error)) = refusal {
            return Err(error);
        }
        // Each vertex is its inputs in order: a zero stream is 16 zero bytes,
        // an attribute its components (x negated when reflected), and a UV
        // fill z = 0 and w = 1 after the two components.
        let stride: usize = columns
            .iter()
            .map(|column| match column {
                Column::Zero => 16,
                Column::Values { width, fill_zw, .. } => 4 * width + if *fill_zw { 8 } else { 0 },
            })
            .sum();
        let bytes = vertices;
        bytes.clear();
        bytes.resize(count * stride, 0);
        if stride > 0 {
            for (vertex, out) in bytes.chunks_exact_mut(stride).enumerate() {
                let mut at = 0;
                for column in &columns {
                    let Column::Values { data, width, reflect, fill_zw, .. } = column else {
                        at += 16;
                        continue;
                    };
                    let values = &data[vertex * width..(vertex + 1) * width];
                    for (component, value) in values.iter().enumerate() {
                        let value = if component == 0 && *reflect { -*value } else { *value };
                        out[at..at + 4].copy_from_slice(&value.to_le_bytes());
                        at += 4;
                    }
                    if *fill_zw {
                        out[at + 4..at + 8].copy_from_slice(&1.0f32.to_le_bytes());
                        at += 8;
                    }
                }
            }
        }
        let length = match &geometry.indices {
            GeometryIndices::U32(v) => {
                index_bytes(v.iter().copied(), count, indices)?;
                v.len()
            }
            GeometryIndices::U16(v) => {
                index_bytes(v.iter().map(|v| u32::from(*v)), count, indices)?;
                v.len()
            }
            GeometryIndices::None => {
                return Err(SourceShaderError("particle geometry lacks indices".into()))
            }
        };
        u32::try_from(length).map_err(|e| SourceShaderError(e.to_string()))
    }
}

/// The mesh attributes a particle program can read as vertex input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Channel {
    Position,
    Normal,
    Color,
    Uv0,
    Custom1,
    Custom2,
}
impl Channel {
    const ALL: [Self; 6] = [
        Self::Position,
        Self::Normal,
        Self::Color,
        Self::Uv0,
        Self::Custom1,
        Self::Custom2,
    ];
    fn of_input(name: &str) -> Option<Self> {
        Some(match name {
            "in_POSITION0" => Self::Position,
            "in_NORMAL0" => Self::Normal,
            "in_COLOR0" => Self::Color,
            "in_TEXCOORD0" => Self::Uv0,
            "in_TEXCOORD1" => Self::Custom1,
            "in_TEXCOORD2" => Self::Custom2,
            _ => return None,
        })
    }
    fn attribute(self) -> MeshVertexAttribute {
        match self {
            Self::Position => Mesh::ATTRIBUTE_POSITION,
            Self::Normal => Mesh::ATTRIBUTE_NORMAL,
            Self::Color => Mesh::ATTRIBUTE_COLOR,
            Self::Uv0 => Mesh::ATTRIBUTE_UV_0,
            Self::Custom1 => crate::billboard::ATTRIBUTE_CUSTOM1,
            Self::Custom2 => crate::billboard::ATTRIBUTE_CUSTOM2,
        }
    }
}

/// What packing reads from a particle mesh: the float columns of the
/// attributes a program can take as input, the indices and the mesh's vertex
/// count. It is kept across frames and updated in place, so an unchanged mesh
/// costs one comparison and no allocation. [`Self::generation`] changes
/// exactly when the copied content changes, compared bit for bit (a -0.0
/// written over 0.0 is a change, and so is any NaN payload);
/// [`Self::index_generation`] only when the indices or the vertex count do.
#[derive(Default)]
pub struct ParticleGeometry {
    columns: [GeometryColumn; 6],
    indices: GeometryIndices,
    vertex_count: usize,
    generation: u64,
    index_generation: u64,
}
impl ParticleGeometry {
    /// Nonzero once copied. Equal generations from one counter mean equal content.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Nonzero once copied. Equal index generations from one counter mean
    /// equal indices and vertex count, the only inputs of the packed indices.
    pub fn index_generation(&self) -> u64 {
        self.index_generation
    }

    /// Copy `mesh`. When anything differs from the held copy, `next` is
    /// advanced and becomes this geometry's generation. Returns whether it did.
    pub fn update(&mut self, mesh: &Mesh, next: &mut u64) -> bool {
        let mut changed = self.generation == 0;
        for (column, channel) in self.columns.iter_mut().zip(Channel::ALL) {
            // Any other attribute representation reads as absent: packing
            // then reports the missing input.
            let (data, width): (&[f32], usize) = match mesh.attribute(channel.attribute()) {
                Some(VertexAttributeValues::Float32x2(v)) => (v.as_flattened(), 2),
                Some(VertexAttributeValues::Float32x3(v)) => (v.as_flattened(), 3),
                Some(VertexAttributeValues::Float32x4(v)) => (v.as_flattened(), 4),
                _ => (&[], 0),
            };
            changed |= column.copy(data, width);
        }
        let count = mesh.count_vertices();
        let indexed = self.indices.copy(mesh.indices()) | (self.vertex_count != count);
        changed |= indexed;
        self.vertex_count = count;
        if changed {
            *next += 1;
            self.generation = *next;
            if indexed || self.index_generation == 0 {
                self.index_generation = *next;
            }
        }
        changed
    }

    /// The midpoint of the three-float positions' bounds; zero without them.
    pub fn center(&self) -> Vec3 {
        let positions = &self.columns[Channel::Position as usize];
        if positions.width != 3 || positions.data.is_empty() {
            return Vec3::ZERO;
        }
        let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for p in positions.data.chunks_exact(3) {
            min = min.min(Vec3::new(p[0], p[1], p[2]));
            max = max.max(Vec3::new(p[0], p[1], p[2]));
        }
        (min + max) * 0.5
    }
}

/// One attribute's flattened floats; width 0 and no data when absent.
#[derive(Default)]
struct GeometryColumn {
    width: usize,
    data: Vec<f32>,
}
impl GeometryColumn {
    fn copy(&mut self, data: &[f32], width: usize) -> bool {
        if self.width != width || self.data.len() != data.len() {
            self.width = width;
            self.data.clear();
            self.data.extend_from_slice(data);
            return true;
        }
        let mut difference = 0u32;
        for (held, value) in self.data.iter_mut().zip(data) {
            difference |= held.to_bits() ^ value.to_bits();
            *held = *value;
        }
        difference != 0
    }
}

#[derive(Default)]
enum GeometryIndices {
    #[default]
    None,
    U16(Vec<u16>),
    U32(Vec<u32>),
}
impl GeometryIndices {
    fn copy(&mut self, indices: Option<&Indices>) -> bool {
        fn copy_values<T: Copy + PartialEq>(held: &mut Vec<T>, values: &[T]) -> bool {
            if held.len() != values.len() {
                held.clear();
                held.extend_from_slice(values);
                return true;
            }
            let mut changed = false;
            for (held, value) in held.iter_mut().zip(values) {
                changed |= *held != *value;
                *held = *value;
            }
            changed
        }
        match (&mut *self, indices) {
            (Self::None, None) => false,
            (Self::U16(held), Some(Indices::U16(values))) => copy_values(held, values),
            (Self::U32(held), Some(Indices::U32(values))) => copy_values(held, values),
            (_, indices) => {
                *self = match indices {
                    None => Self::None,
                    Some(Indices::U16(values)) => Self::U16(values.clone()),
                    Some(Indices::U32(values)) => Self::U32(values.clone()),
                };
                true
            }
        }
    }
}

/// One vertex input's source column.
enum Column<'a> {
    /// The zero default stream of an absent UV channel.
    Zero,
    Values {
        name: &'a str,
        /// Flattened attribute values; empty with width 0 when the attribute
        /// is absent or not a float vector.
        data: &'a [f32],
        width: usize,
        reflect: bool,
        /// A float4 input over the two-float UV stream: z = 0 and w = 1 follow.
        fill_zw: bool,
    },
}

fn index_bytes(
    indices: impl ExactSizeIterator<Item = u32> + Clone,
    count: usize,
    bytes: &mut Vec<u8>,
) -> Result<()> {
    require(
        indices.clone().all(|i| (i as usize) < count),
        "particle index exceeds vertex count",
    )?;
    bytes.clear();
    bytes.resize(indices.len() * 4, 0);
    for (out, index) in bytes.chunks_exact_mut(4).zip(indices) {
        out.copy_from_slice(&index.to_le_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Packing from the kept geometry copy is compared with packing written
    //! out as reads of the mesh itself, over meshes that change between
    //! frames while one copy and one pair of output buffers are reused.
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;
    use serde_json::json;

    /// The definition: every vertex input read from the mesh, vertex by vertex.
    fn reference(
        streams: &ParticleStreams,
        abi: &ProgramAbi,
        mesh: &Mesh,
    ) -> Result<(Vec<u8>, Vec<u8>, u32)> {
        streams.layout(abi)?;
        let count = mesh.count_vertices();
        let mut bytes = Vec::new();
        for vertex in 0..count {
            for field in abi.interfaces.vertex.iter().filter(|f| f.direction == "in") {
                let name = field.name.as_str();
                if !streams.custom && matches!(name, "in_TEXCOORD1" | "in_TEXCOORD2") {
                    bytes.extend_from_slice(&[0; 16]);
                    continue;
                }
                let attribute = match name {
                    "in_POSITION0" => Mesh::ATTRIBUTE_POSITION,
                    "in_NORMAL0" => Mesh::ATTRIBUTE_NORMAL,
                    "in_COLOR0" => Mesh::ATTRIBUTE_COLOR,
                    "in_TEXCOORD0" => Mesh::ATTRIBUTE_UV_0,
                    "in_TEXCOORD1" => crate::billboard::ATTRIBUTE_CUSTOM1,
                    "in_TEXCOORD2" => crate::billboard::ATTRIBUTE_CUSTOM2,
                    _ => unreachable!(),
                };
                let (data, width): (&[f32], usize) = match mesh.attribute(attribute) {
                    Some(VertexAttributeValues::Float32x2(v)) => (v.as_flattened(), 2),
                    Some(VertexAttributeValues::Float32x3(v)) => (v.as_flattened(), 3),
                    Some(VertexAttributeValues::Float32x4(v)) => (v.as_flattened(), 4),
                    _ => (&[], 0),
                };
                let values = (width > 0)
                    .then(|| data.get(vertex * width..(vertex + 1) * width))
                    .flatten()
                    .ok_or_else(|| {
                        SourceShaderError(format!(
                            "particle geometry lacks {name} at vertex {vertex}"
                        ))
                    })?;
                let reflect = matches!(name, "in_POSITION0" | "in_NORMAL0");
                for (component, value) in values.iter().enumerate() {
                    let value = if component == 0 && reflect { -*value } else { *value };
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                if name == "in_TEXCOORD0" && field.ty == "vec4" {
                    if width != 2 {
                        return Err(SourceShaderError(format!(
                            "particle geometry {name} has {width} components, not the UV stream's two"
                        )));
                    }
                    bytes.extend_from_slice(&0.0f32.to_le_bytes());
                    bytes.extend_from_slice(&1.0f32.to_le_bytes());
                }
            }
        }
        let indices: Vec<u32> = match mesh.indices() {
            Some(Indices::U32(v)) => v.clone(),
            Some(Indices::U16(v)) => v.iter().map(|i| u32::from(*i)).collect(),
            None => return Err(SourceShaderError("particle geometry lacks indices".into())),
        };
        if indices.iter().any(|i| *i as usize >= count) {
            return Err(SourceShaderError("particle index exceeds vertex count".into()));
        }
        let index_bytes = indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        Ok((bytes, index_bytes, indices.len() as u32))
    }

    /// The old extraction's sort position.
    fn reference_center(mesh: &Mesh) -> Vec3 {
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(positions)) if !positions.is_empty() => {
                let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
                for p in positions {
                    min = min.min(Vec3::from_array(*p));
                    max = max.max(Vec3::from_array(*p));
                }
                (min + max) * 0.5
            }
            _ => Vec3::ZERO,
        }
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn float(&mut self) -> f32 {
            match self.below(6) {
                0 => 0.0,
                1 => -0.0,
                _ => (self.next() % 2001) as f32 / 100.0 - 10.0,
            }
        }
    }

    fn abi(rng: &mut Rng) -> ProgramAbi {
        let mut inputs = vec![if rng.below(2) == 0 {
            json!({"name":"in_POSITION0","type":"vec4","adapter":"vec3-position-w-one"})
        } else {
            json!({"name":"in_POSITION0","type":"vec3","adapter":"vec3-position"})
        }];
        if rng.below(2) == 0 {
            inputs.push(json!({"name":"in_NORMAL0","type":"vec3","adapter":null}));
        }
        inputs.push(json!({"name":"in_COLOR0","type":"vec4","adapter":null}));
        let uv = ["vec2", "vec4"][rng.below(2)];
        inputs.push(json!({"name":"in_TEXCOORD0","type":uv,"adapter":null}));
        if rng.below(2) == 0 {
            inputs.push(json!({"name":"in_TEXCOORD1","type":"vec4","adapter":null}));
            inputs.push(json!({"name":"in_TEXCOORD2","type":"vec4","adapter":null}));
        }
        let vertex: Vec<_> = inputs
            .into_iter()
            .enumerate()
            .map(|(location, mut input)| {
                input["direction"] = json!("in");
                input["location"] = json!(location);
                input["interpolation"] = json!("smooth");
                input
            })
            .collect();
        serde_json::from_value(json!({
            "version":1,"bindGroup":0,"uniformBinding":0,"uniformBytes":0,
            "uniforms":[],"textures":[],"interfaces":{"vertex":vertex,"fragment":[]},
            "sourceClipSpace":"gl-minus-one-to-one","textureDataOrigin":"unity-lower-left",
            "arrayLayerRule":"clamp(floor(layer+0.5),0,layers-1)"
        }))
        .unwrap()
    }

    fn streams(rng: &mut Rng) -> ParticleStreams {
        ParticleStreams::parse(&if rng.below(2) == 0 {
            json!({"vertexStreams":{"codes":[0,1,3,4]},"useCustomVertexStreams":false})
        } else {
            json!({"vertexStreams":{"codes":[0,1,3,4,5,34,38]},"useCustomVertexStreams":true})
        })
        .unwrap()
    }

    fn column<const N: usize>(rng: &mut Rng, count: usize) -> Vec<[f32; N]> {
        (0..count).map(|_| std::array::from_fn(|_| rng.float())).collect()
    }

    /// A particle mesh: usually every attribute at one length, sometimes one
    /// absent or short, and indices that sometimes run past the vertices.
    fn mesh(rng: &mut Rng) -> Mesh {
        let count = rng.below(7);
        let mut length = || {
            if rng.below(10) == 0 {
                count.saturating_sub(1)
            } else {
                count
            }
        };
        let lengths: [usize; 6] = std::array::from_fn(|_| length());
        let present: [bool; 6] = std::array::from_fn(|_| rng.below(12) != 0);
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        if present[0] {
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, column::<3>(rng, lengths[0]));
        }
        if present[1] {
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, column::<3>(rng, lengths[1]));
        }
        if present[2] {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, column::<4>(rng, lengths[2]));
        }
        if present[3] {
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, column::<2>(rng, lengths[3]));
        }
        if present[4] {
            mesh.insert_attribute(crate::billboard::ATTRIBUTE_CUSTOM1, column::<4>(rng, lengths[4]));
        }
        if present[5] {
            mesh.insert_attribute(crate::billboard::ATTRIBUTE_CUSTOM2, column::<4>(rng, lengths[5]));
        }
        let indices: Vec<u32> = (0..rng.below(4) * 3)
            .map(|_| {
                // Index `count` runs past the vertices (always so when there are none).
                let bound = count + usize::from(rng.below(15) == 0);
                if bound == 0 { 0 } else { rng.below(bound) as u32 }
            })
            .collect();
        match rng.below(8) {
            0 => {}
            1..=3 => mesh.insert_indices(Indices::U16(indices.iter().map(|i| *i as u16).collect())),
            _ => mesh.insert_indices(Indices::U32(indices)),
        }
        mesh
    }

    /// One small in-place edit, as a simulation step makes to a kept mesh.
    fn edit(rng: &mut Rng, mesh: &mut Mesh) {
        if let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute_mut(Mesh::ATTRIBUTE_COLOR) {
            if !colors.is_empty() {
                let at = rng.below(colors.len());
                colors[at][rng.below(4)] = rng.float();
            }
        }
    }

    #[test]
    fn kept_geometry_packs_as_the_mesh_reads() {
        let mut rng = Rng(0x5851_f42d_4c95_7f2d);
        let mut geometry = ParticleGeometry::default();
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        let mut next = 0;
        let (mut packed, mut rejected, mut unchanged) = (0, 0, 0);
        let mut mesh = mesh(&mut rng);
        for index in 0..20_000 {
            match rng.below(3) {
                0 => mesh = self::mesh(&mut rng),
                1 => edit(&mut rng, &mut mesh),
                _ => {}
            }
            let (streams, abi) = (streams(&mut rng), abi(&mut rng));
            let before = geometry.generation();
            let changed = geometry.update(&mesh, &mut next);
            assert_eq!(changed, geometry.generation() != before, "case {index}");
            unchanged += usize::from(!changed);
            assert_eq!(geometry.center().to_array().map(f32::to_bits),
                reference_center(&mesh).to_array().map(f32::to_bits), "case {index}");
            let expected = reference(&streams, &abi, &mesh);
            let actual = streams
                .pack(&abi, &geometry, &mut vertices, &mut indices)
                .map(|count| (vertices.clone(), indices.clone(), count));
            assert_eq!(actual, expected, "case {index}");
            if expected.is_ok() {
                packed += 1;
            } else {
                rejected += 1;
            }
        }
        assert!(packed > 5_000 && rejected > 1_000 && unchanged > 3_000,
            "{packed} packed, {rejected} rejected, {unchanged} unchanged copies");
    }

    /// The generation moves with every bit of copied content, and the
    /// comparison above sees a copy that missed an edit.
    #[test]
    fn every_copied_bit_moves_the_generation_and_a_stale_copy_is_seen() {
        let mut rng = Rng(0x2c1b_3c6d_9e37_79b9);
        let (streams, abi) = (
            ParticleStreams::parse(&json!({"vertexStreams":{"codes":[0,1,3,4]},"useCustomVertexStreams":false})).unwrap(),
            abi(&mut rng),
        );
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32, 1.0, 2.0]; 3])
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 0.0, 1.0]; 3])
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; 3])
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.5f32; 2]; 3])
            .with_inserted_indices(Indices::U32(vec![0, 1, 2]));
        let mut geometry = ParticleGeometry::default();
        let mut next = 0;
        assert!(geometry.update(&mesh, &mut next));
        assert!(!geometry.update(&mesh, &mut next));
        let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION) else {
            unreachable!()
        };
        positions[1][0] = -0.0;
        let stale = streams.pack(&abi, &geometry, &mut Vec::new(), &mut Vec::new()).map(|_| ());
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        streams.pack(&abi, &geometry, &mut vertices, &mut indices).unwrap();
        let expected = reference(&streams, &abi, &mesh).unwrap();
        assert!(stale.is_ok() && vertices != expected.0, "a copy that missed -0.0 over 0.0 differs");
        assert!(geometry.update(&mesh, &mut next), "-0.0 over 0.0 is a change");
        streams.pack(&abi, &geometry, &mut vertices, &mut indices).unwrap();
        assert_eq!((vertices, indices), (expected.0, expected.1));
        mesh.insert_indices(Indices::U16(vec![0, 1, 2]));
        assert!(geometry.update(&mesh, &mut next), "the index width is part of the copy");
    }
}
