//! Generated binding ABI, validated independently of the shader's display name.
//! Padding is deterministic; a missing input is an error, never a zero uniform.
use super::{require, require_with, Result, SourceShaderError};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum ShaderStage {
    Vertex,
    Fragment,
}
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum TextureDimension {
    #[serde(rename = "2d")]
    D2,
    #[serde(rename = "2d-array")]
    D2Array,
}
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScalarType {
    Float,
    Vec2,
    Vec3,
    Vec4,
    Int,
    Ivec2,
    Ivec3,
    Ivec4,
    Uint,
    Uvec2,
    Uvec3,
    Uvec4,
    Bool,
}
impl ScalarType {
    pub fn components(self) -> usize {
        match self {
            Self::Vec2 | Self::Ivec2 | Self::Uvec2 => 2,
            Self::Vec3 | Self::Ivec3 | Self::Uvec3 => 3,
            Self::Vec4 | Self::Ivec4 | Self::Uvec4 => 4,
            _ => 1,
        }
    }
    fn reflected_kind(self) -> &'static str {
        match self {
            Self::Float | Self::Vec2 | Self::Vec3 | Self::Vec4 => "Float",
            // The lowering stores signed ints as uint and reinterprets the
            // bits at the source expression, preserving the original int.
            _ => "Uint",
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
pub struct UniformField {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: ScalarType,
    pub array: Option<u32>,
    pub offset: u32,
    pub bytes: u32,
    pub member: String,
    pub stages: Vec<ShaderStage>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureBinding {
    pub name: String,
    pub dimension: TextureDimension,
    pub texture_binding: u32,
    pub sampler_binding: u32,
    pub stages: Vec<ShaderStage>,
    pub array: Option<u32>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Interface {
    pub name: String,
    pub direction: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub location: u32,
    pub interpolation: String,
    pub adapter: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Interfaces {
    pub vertex: Vec<Interface>,
    pub fragment: Vec<Interface>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramAbi {
    pub version: u32,
    pub bind_group: u32,
    pub uniform_binding: u32,
    pub uniform_bytes: u32,
    pub uniforms: Vec<UniformField>,
    pub textures: Vec<TextureBinding>,
    pub interfaces: Interfaces,
    pub source_clip_space: String,
    pub texture_data_origin: String,
    pub array_layer_rule: String,
}

/// Inputs supplied by a material/global/vertex owner. Scalar types and arities
/// are not inferred from the source's uniform name.
#[derive(Debug, Clone, PartialEq)]
pub enum UniformValue {
    Float(Vec<f32>),
    Signed(Vec<i32>),
    Unsigned(Vec<u32>),
    Bool(bool),
}
impl ProgramAbi {
    pub fn validate(&self) -> Result<()> {
        require(
            self.version == 1 && self.bind_group == 0 && self.uniform_binding == 0,
            "unsupported source binding ABI",
        )?;
        require(
            self.source_clip_space == "gl-minus-one-to-one"
                && self.texture_data_origin == "unity-lower-left"
                && self.array_layer_rule == "clamp(floor(layer+0.5),0,layers-1)",
            "unknown source coordinate or array-layer convention",
        )?;
        let mut cursor = 0u32;
        let mut names = BTreeSet::new();
        let mut members = BTreeSet::new();
        for field in &self.uniforms {
            require(
                !field.name.is_empty()
                    && names.insert(field.name.as_str())
                    && !field.member.is_empty()
                    && members.insert(field.member.as_str()),
                "duplicate or empty uniform identity",
            )?;
            require(
                !field.stages.is_empty()
                    && field.stages.iter().collect::<BTreeSet<_>>().len() == field.stages.len(),
                "invalid uniform stage visibility",
            )?;
            let count = field.array.unwrap_or(1);
            let bytes = count
                .checked_mul(16)
                .ok_or_else(|| SourceShaderError("uniform array byte count overflow".into()))?;
            require_with(
                count > 0 && field.bytes == bytes && field.offset == cursor,
                || {
                    format!(
                        "{}: overlapping or inconsistent uniform byte layout",
                        field.name
                    )
                },
            )?;
            cursor = cursor
                .checked_add(bytes)
                .ok_or_else(|| SourceShaderError("uniform span overflow".into()))?;
        }
        require(
            cursor == self.uniform_bytes && cursor % 16 == 0,
            "uniform block span disagrees with its fields",
        )?;
        let mut bindings = BTreeSet::from([self.uniform_binding]);
        for texture in &self.textures {
            require(
                !texture.name.is_empty()
                    && names.insert(texture.name.as_str())
                    && texture.array.is_none(),
                "duplicate texture identity or unimplemented texture binding array",
            )?;
            require(
                bindings.insert(texture.texture_binding)
                    && bindings.insert(texture.sampler_binding),
                "shader resource bindings overlap",
            )?;
            require(
                !texture.stages.is_empty()
                    && texture.stages.iter().collect::<BTreeSet<_>>().len() == texture.stages.len(),
                "invalid texture stage visibility",
            )?;
        }
        for interfaces in [&self.interfaces.vertex, &self.interfaces.fragment] {
            let mut locations = BTreeSet::new();
            for field in interfaces {
                require(
                    matches!(field.direction.as_str(), "in" | "out")
                        && locations.insert((field.direction.as_str(), field.location)),
                    "duplicate shader interface location",
                )?;
            }
        }
        for input in self
            .interfaces
            .fragment
            .iter()
            .filter(|i| i.direction == "in")
        {
            let output = self
                .interfaces
                .vertex
                .iter()
                .find(|o| o.direction == "out" && o.location == input.location)
                .ok_or_else(|| {
                    SourceShaderError(format!(
                        "fragment varying {} has no source vertex output",
                        input.name
                    ))
                })?;
            require(
                output.ty == input.ty && output.interpolation == input.interpolation,
                "vertex/fragment varying ABI mismatch",
            )?;
        }
        Ok(())
    }

    pub fn validate_reflection(&self, bindings: &[Value]) -> Result<()> {
        let mut seen = BTreeSet::new();
        let mut uniform_found = self.uniform_bytes == 0;
        for value in bindings {
            let group = value.get("group").and_then(Value::as_u64);
            let binding = value
                .get("binding")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| SourceShaderError("reflected binding address is absent".into()))?;
            require(
                group == Some(u64::from(self.bind_group)) && seen.insert(binding),
                "reflected bind group differs from source ABI",
            )?;
            if binding == self.uniform_binding {
                require(
                    value["kind"] == "struct"
                        && value["addressSpace"] == "Uniform"
                        && value["bytes"].as_u64() == Some(self.uniform_bytes as u64),
                    "reflected uniform span differs from source ABI",
                )?;
                let fields = value["fields"]
                    .as_array()
                    .ok_or_else(|| SourceShaderError("reflected uniform fields absent".into()))?;
                require(
                    fields.len() == self.uniforms.len(),
                    "reflected uniform member count differs",
                )?;
                for (source, reflected) in self.uniforms.iter().zip(fields) {
                    require(
                        reflected["offset"].as_u64() == Some(source.offset as u64),
                        "reflected uniform member offset differs",
                    )?;
                    let ty = &reflected["type"];
                    let element = if let Some(count) = source.array {
                        require(
                            ty["kind"] == "array" && ty["stride"] == 16 && ty["count"] == count,
                            "reflected uniform array layout differs",
                        )?;
                        &ty["element"]
                    } else {
                        ty
                    };
                    require(
                        element["kind"] == "vector"
                            && element["components"] == 4
                            && element["width"] == 4
                            && element["scalar"] == source.ty.reflected_kind(),
                        "reflected uniform scalar representation differs",
                    )?;
                }
                uniform_found = true;
            } else if let Some(texture) =
                self.textures.iter().find(|t| t.texture_binding == binding)
            {
                require(
                    value["kind"] == "texture"
                        && value["dimension"] == "D2"
                        && value["array"].as_bool()
                            == Some(texture.dimension == TextureDimension::D2Array)
                        && value["class"] == "Sampled { kind: Float, multi: false }",
                    "reflected texture dimension or sample type differs",
                )?;
            } else if self.textures.iter().any(|t| t.sampler_binding == binding) {
                require(
                    value["kind"] == "sampler" && value["comparison"] == false,
                    "reflected sampler type differs",
                )?;
            } else {
                return Err(SourceShaderError(
                    "generated shader declares an unowned resource binding".into(),
                ));
            }
        }
        require(
            uniform_found,
            "generated shader lacks its source uniform block",
        )
    }

    /// The validated uniform block layout, for packing repeatedly without
    /// validating again. This is the only way to obtain a [`UniformLayout`].
    pub fn uniform_layout(&self) -> Result<UniformLayout> {
        self.validate()?;
        Ok(UniformLayout {
            bytes: self.uniform_bytes,
            slots: self
                .uniforms
                .iter()
                .map(|field| UniformSlot {
                    ty: field.ty,
                    array: field.array,
                    offset: field.offset,
                })
                .collect(),
        })
    }

    /// Pack one complete update. Resolving all fields first prevents an error
    /// from committing a partly updated material or leaking old global values.
    pub fn pack_uniforms(
        &self,
        mut resolve: impl FnMut(&UniformField) -> Result<UniformValue>,
    ) -> Result<Vec<u8>> {
        let layout = self.uniform_layout()?;
        let mut bytes = vec![0u8; layout.bytes()];
        for (index, field) in self.uniforms.iter().enumerate() {
            let value = resolve(field)?;
            layout
                .write(index, &value, &mut bytes)
                .map_err(|fault| fault.error(field))?;
        }
        Ok(bytes)
    }
}

/// Where one uniform field's words go in the packed block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UniformSlot {
    pub ty: ScalarType,
    pub array: Option<u32>,
    pub offset: u32,
}
impl UniformSlot {
    fn count(self) -> usize {
        self.array.unwrap_or(1) as usize
    }
}

/// Why a value does not fit its uniform slot. The message, which names the
/// field, is built only when the fault is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UniformFault {
    Nonfinite,
    TypeMismatch,
    ComponentCount,
}
impl UniformFault {
    pub fn error(self, field: &UniformField) -> SourceShaderError {
        SourceShaderError(match self {
            Self::Nonfinite => format!("{}: nonfinite source uniform", field.name),
            Self::TypeMismatch => format!("{}: source uniform type mismatch", field.name),
            Self::ComponentCount => {
                format!("{}: source uniform component count mismatch", field.name)
            }
        })
    }
}

/// A uniform block layout proven by [`ProgramAbi::validate`]: one slot per
/// field, in declaration order, every slot inside the block. Writing a value
/// checks only the value against its slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniformLayout {
    bytes: u32,
    slots: Vec<UniformSlot>,
}
impl UniformLayout {
    /// The block's byte count.
    pub fn bytes(&self) -> usize {
        self.bytes as usize
    }
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
    pub fn slot(&self, index: usize) -> UniformSlot {
        self.slots[index]
    }

    /// Write field `index` into `block`, the whole uniform block. Each element
    /// starts a 16-byte row; padding is left as it is. Nothing is written when
    /// the value does not fit.
    pub fn write(
        &self,
        index: usize,
        value: &UniformValue,
        block: &mut [u8],
    ) -> std::result::Result<(), UniformFault> {
        let slot = self.slots[index];
        match (slot.ty, value) {
            (
                ScalarType::Float | ScalarType::Vec2 | ScalarType::Vec3 | ScalarType::Vec4,
                UniformValue::Float(values),
            ) => {
                if !values.iter().all(|v| v.is_finite()) {
                    return Err(UniformFault::Nonfinite);
                }
                put(slot, values.iter().map(|v| v.to_bits()), values.len(), block)
            }
            (
                ScalarType::Int | ScalarType::Ivec2 | ScalarType::Ivec3 | ScalarType::Ivec4,
                UniformValue::Signed(values),
            ) => put(slot, values.iter().map(|v| *v as u32), values.len(), block),
            (
                ScalarType::Uint | ScalarType::Uvec2 | ScalarType::Uvec3 | ScalarType::Uvec4,
                UniformValue::Unsigned(values),
            ) => put(slot, values.iter().copied(), values.len(), block),
            (ScalarType::Bool, UniformValue::Bool(value)) if slot.array.is_none() => {
                put(slot, std::iter::once(u32::from(*value)), 1, block)
            }
            _ => Err(UniformFault::TypeMismatch),
        }
    }

    /// [`Self::write`] for a float value held in a slice.
    pub fn write_floats(
        &self,
        index: usize,
        values: &[f32],
        block: &mut [u8],
    ) -> std::result::Result<(), UniformFault> {
        let slot = self.slots[index];
        if !matches!(
            slot.ty,
            ScalarType::Float | ScalarType::Vec2 | ScalarType::Vec3 | ScalarType::Vec4
        ) {
            return Err(UniformFault::TypeMismatch);
        }
        if !values.iter().all(|v| v.is_finite()) {
            return Err(UniformFault::Nonfinite);
        }
        put(slot, values.iter().map(|v| v.to_bits()), values.len(), block)
    }
}

fn put(
    slot: UniformSlot,
    words: impl Iterator<Item = u32>,
    length: usize,
    block: &mut [u8],
) -> std::result::Result<(), UniformFault> {
    let components = slot.ty.components();
    if length != slot.count() * components {
        return Err(UniformFault::ComponentCount);
    }
    for (index, word) in words.enumerate() {
        let offset = slot.offset as usize + (index / components) * 16 + (index % components) * 4;
        block[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod layout_parity {
    //! Packing through the validated layout is compared with the packing
    //! definition written out field by field: the same bytes for every
    //! accepted input and the same first error for every rejected one.
    use super::*;

    /// The definition: validate, then resolve and write each field in order.
    fn reference(
        abi: &ProgramAbi,
        mut resolve: impl FnMut(&UniformField) -> Result<UniformValue>,
    ) -> Result<Vec<u8>> {
        abi.validate()?;
        let mut bytes = vec![0u8; abi.uniform_bytes as usize];
        for field in &abi.uniforms {
            let count = field.array.unwrap_or(1) as usize;
            let components = field.ty.components();
            let value = resolve(field)?;
            let words: Vec<u32> = match (field.ty, value) {
                (
                    ScalarType::Float | ScalarType::Vec2 | ScalarType::Vec3 | ScalarType::Vec4,
                    UniformValue::Float(values),
                ) => {
                    if !values.iter().all(|v| v.is_finite()) {
                        return Err(SourceShaderError(format!(
                            "{}: nonfinite source uniform",
                            field.name
                        )));
                    }
                    values.into_iter().map(f32::to_bits).collect()
                }
                (
                    ScalarType::Int | ScalarType::Ivec2 | ScalarType::Ivec3 | ScalarType::Ivec4,
                    UniformValue::Signed(values),
                ) => values.into_iter().map(|v| v as u32).collect(),
                (
                    ScalarType::Uint | ScalarType::Uvec2 | ScalarType::Uvec3 | ScalarType::Uvec4,
                    UniformValue::Unsigned(values),
                ) => values,
                (ScalarType::Bool, UniformValue::Bool(value)) if field.array.is_none() => {
                    vec![u32::from(value)]
                }
                _ => {
                    return Err(SourceShaderError(format!(
                        "{}: source uniform type mismatch",
                        field.name
                    )))
                }
            };
            if words.len() != count * components {
                return Err(SourceShaderError(format!(
                    "{}: source uniform component count mismatch",
                    field.name
                )));
            }
            for (index, word) in words.iter().enumerate() {
                let offset =
                    field.offset as usize + (index / components) * 16 + (index % components) * 4;
                bytes[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
            }
        }
        Ok(bytes)
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
            match self.below(8) {
                0 => 0.0,
                1 => -0.0,
                2 => f32::MIN_POSITIVE / 2.0,
                _ => loop {
                    let value = f32::from_bits(self.next() as u32);
                    if value.is_finite() {
                        break value;
                    }
                },
            }
        }
    }

    const TYPES: [ScalarType; 13] = [
        ScalarType::Float,
        ScalarType::Vec2,
        ScalarType::Vec3,
        ScalarType::Vec4,
        ScalarType::Int,
        ScalarType::Ivec2,
        ScalarType::Ivec3,
        ScalarType::Ivec4,
        ScalarType::Uint,
        ScalarType::Uvec2,
        ScalarType::Uvec3,
        ScalarType::Uvec4,
        ScalarType::Bool,
    ];

    fn abi(rng: &mut Rng) -> ProgramAbi {
        let mut uniforms = Vec::new();
        let mut offset = 0;
        for index in 0..1 + rng.below(8) {
            let array = (rng.below(3) == 0).then(|| 1 + rng.below(4) as u32);
            uniforms.push(UniformField {
                name: format!("u{index}"),
                ty: TYPES[rng.below(TYPES.len())],
                array,
                offset,
                bytes: array.unwrap_or(1) * 16,
                member: format!("m{index}"),
                stages: vec![ShaderStage::Vertex],
            });
            offset += array.unwrap_or(1) * 16;
        }
        ProgramAbi {
            version: 1,
            bind_group: 0,
            uniform_binding: 0,
            uniform_bytes: offset,
            uniforms,
            textures: Vec::new(),
            interfaces: Interfaces {
                vertex: Vec::new(),
                fragment: Vec::new(),
            },
            source_clip_space: "gl-minus-one-to-one".into(),
            texture_data_origin: "unity-lower-left".into(),
            array_layer_rule: "clamp(floor(layer+0.5),0,layers-1)".into(),
        }
    }

    /// Mostly a fitting value; sometimes a wrong count, kind or a nonfinite one.
    fn value(rng: &mut Rng, field: &UniformField) -> UniformValue {
        let mut length = field.array.unwrap_or(1) as usize * field.ty.components();
        if rng.below(12) == 0 {
            length = if length > 1 && rng.below(2) == 0 {
                length - 1
            } else {
                length + 1
            };
        }
        let kind = if rng.below(12) == 0 {
            rng.below(4)
        } else {
            match field.ty {
                ScalarType::Float | ScalarType::Vec2 | ScalarType::Vec3 | ScalarType::Vec4 => 0,
                ScalarType::Int | ScalarType::Ivec2 | ScalarType::Ivec3 | ScalarType::Ivec4 => 1,
                ScalarType::Uint | ScalarType::Uvec2 | ScalarType::Uvec3 | ScalarType::Uvec4 => 2,
                ScalarType::Bool => 3,
            }
        };
        match kind {
            0 => {
                let mut values: Vec<f32> = (0..length).map(|_| rng.float()).collect();
                if rng.below(16) == 0 {
                    let at = rng.below(values.len());
                    values[at] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY][rng.below(3)];
                }
                UniformValue::Float(values)
            }
            1 => UniformValue::Signed((0..length).map(|_| rng.next() as i32).collect()),
            2 => UniformValue::Unsigned((0..length).map(|_| rng.next() as u32).collect()),
            _ => UniformValue::Bool(rng.below(2) == 0),
        }
    }

    fn case(rng: &mut Rng) -> (ProgramAbi, Vec<UniformValue>, Option<usize>) {
        let abi = abi(rng);
        let values = abi.uniforms.iter().map(|field| value(rng, field)).collect();
        let unowned = (rng.below(10) == 0).then(|| rng.below(abi.uniforms.len()));
        (abi, values, unowned)
    }

    fn resolver(
        values: &[UniformValue],
        unowned: Option<usize>,
    ) -> impl FnMut(&UniformField) -> Result<UniformValue> + '_ {
        move |field| {
            let index: usize = field.name[1..].parse().unwrap();
            if Some(index) == unowned {
                return Err(SourceShaderError(format!(
                    "unowned source uniform {}",
                    field.name
                )));
            }
            Ok(values[index].clone())
        }
    }

    #[test]
    fn layout_packing_matches_the_field_definition() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let (mut packed, mut rejected) = (0, 0);
        for index in 0..20_000 {
            let (abi, values, unowned) = case(&mut rng);
            let expected = reference(&abi, resolver(&values, unowned));
            let actual = abi.pack_uniforms(resolver(&values, unowned));
            assert_eq!(actual, expected, "case {index}");
            if expected.is_ok() {
                packed += 1;
            } else {
                rejected += 1;
            }
        }
        // Both arms carry signal: accepted blocks are compared byte for byte,
        // rejected ones by their first error.
        assert!(
            packed > 5_000 && rejected > 2_000,
            "{packed} packed, {rejected} rejected"
        );
    }

    /// The comparison above sees a value written to a wrong slot: exchanging
    /// two equally sized slots of the layout makes it disagree with the
    /// definition.
    #[test]
    fn an_exchanged_slot_is_seen_by_the_comparison() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        let (mut compared, mut differing) = (0, 0);
        for _ in 0..4_000 {
            let (abi, values, _) = case(&mut rng);
            let Ok(expected) = reference(&abi, resolver(&values, None)) else {
                continue;
            };
            let mut layout = abi.uniform_layout().unwrap();
            let Some(other) =
                (1..abi.uniforms.len()).find(|&i| abi.uniforms[i].bytes == abi.uniforms[0].bytes)
            else {
                continue;
            };
            let (a, b) = (layout.slots[0].offset, layout.slots[other].offset);
            layout.slots[0].offset = b;
            layout.slots[other].offset = a;
            let mut bytes = vec![0; layout.bytes()];
            for (index, value) in values.iter().enumerate() {
                layout.write(index, value, &mut bytes).unwrap();
            }
            compared += 1;
            if bytes != expected {
                differing += 1;
            }
        }
        assert!(compared > 100, "{compared} exchanged layouts");
        // Equal blocks stay possible when both fields hold the same words.
        assert!(
            differing * 100 >= compared * 95,
            "{differing} of {compared} exchanged layouts differ"
        );
    }
}
