//! 材质值的中性形状：Unity 属性名到值的表。
//!
//! 着色律按名字读这里的值；哪个 key 有意义由律决定，本模块不做筛选。

use std::collections::BTreeMap;

/// 按名取一个材质的浮点属性。
pub trait FloatLookup {
    fn get(&self, key: &str) -> Option<f32>;
}

/// 一个材质的取值表，key 是 Unity 属性名的原拼写，值不换算、不补默认。
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialSlot {
    pub name: String,
    /// Unity shader 名，如 `"Mysekai/Site/FieldObject"`。
    pub shader: String,
    pub keywords: Vec<String>,
    /// `(纹理属性名, 图像索引)`，每个非空纹理槽一条，顺序原样保留。
    pub textures: Vec<(String, usize)>,
    /// 颜色属性，RGBA、shader 原生浮点空间，不做 sRGB 换算。
    pub colors: BTreeMap<String, [f32; 4]>,
    /// 每个纹理属性的 `(scale.x, scale.y, offset.x, offset.y)`；
    /// 图像指针为空的槽也可能带变换，所以独立于 [`Self::textures`] 存。
    pub texture_scale_offsets: BTreeMap<String, [f32; 4]>,
    /// 全部浮点条目。`BTreeMap` 让 key 顺序跨运行可复现。
    pub floats: BTreeMap<String, f32>,
}

impl FloatLookup for MaterialSlot {
    fn get(&self, key: &str) -> Option<f32> {
        self.floats.get(key).copied()
    }
}

/// 按名找一个纹理槽的图像索引，不假设槽的顺序。
pub fn texture_slot(material: &MaterialSlot, name: &str) -> Option<usize> {
    material
        .textures
        .iter()
        .find(|(slot, _)| slot == name)
        .map(|(_, index)| *index)
}
