//! 家具着色器族谱与 ShaderAttribute 选择律。
//!
//! 两件事：
//! - **族键闭集**：家具侧出现过的 shader 名全集（数据面现算口径，
//!   999 个家具包的全部材质）。族键 = shader 名精确匹配；全集之外
//!   的名字按未知拒绝，其中跨进来的通用 URP 材质按名字具名拒绝；
//! - **二维选择表**：`MysekaiMaterialBuilder.GetBasicFixtureShaderAttribute`
//!   读材质上的 `_FixtureShaderUsage` 与 `_FixtureObjectBlendMode` 两个
//!   整值，按 (usage, blend) 二维选 ShaderAttribute。越界（值不在
//!   两枚举里）源抛 ArgumentOutOfRangeException，这里折成 Err。
//!
//! 注意源对「材质上缺属性」并不走错误臂：取值失败默认 0（= Indoor /
//! Opaque）。这是材质读取侧的语义，归装配层；本律收的是取值之后
//! 的选择表，闭集输入、越界即拒。

/// 家具侧出现过的 shader 名全集（族键）。数量是数据面现算的材质数，
/// 作为闭集的锚：多一个名字、少一个名字，这一族都会跟着变。
pub const FAMILY_SHADER_NAMES: [&str; 12] = [
    "Mysekai/Fixture/Basic",          // 1001
    "Mysekai/Fixture/ShadowMesh",     // 604
    "Mysekai/Fixture/Road",           // 161
    "Mysekai/Fixture/Rug",            // 15
    "Mysekai/Object",                 // 22
    "Mysekai/Site/Tree",              // 9
    "Mysekai/Fixture/Fence",          // 7
    "Mysekai/Effect/UberUnlit",       // 6
    "Mysekai/Fixture/Canvas",         // 6
    "Particles/Standard Unlit",       // 4
    "Mysekai/Fixture/TransparentBlock", // 2
    "Universal Render Pipeline/Lit",  // 25
];

/// 按名字具名拒绝的通用材质：不在家具族里，但真实出现在家具包的
/// 材质面上（迁移自站点侧材质契约的同款裁决）。
pub const NAMED_REJECTION: &str = "Universal Render Pipeline/Lit";

/// 族键判定：闭集内的名字放行，具名拒绝的那位给出指名报错，
/// 其余未知名字给出泛化报错。两种报错必须分开——具名拒绝是
/// 「知道你是什么、也知道不该用」，未知是「不认识」。
pub fn check_family_shader(shader_name: &str) -> Result<(), String> {
    if shader_name == NAMED_REJECTION {
        Err(format!(
            "shader {shader_name:?} is a general-purpose URP shader \
             and is rejected by name for fixtures"
        ))
    } else if FAMILY_SHADER_NAMES.contains(&shader_name) {
        Ok(())
    } else {
        Err(format!(
            "shader {shader_name:?} is not in the fixture family set"
        ))
    }
}

/// 源 FixtureShaderUsage 的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureShaderUsage {
    Indoor = 0,
    WindowOutside = 1,
}

/// 源 FixtureObjectBlendMode 的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureObjectBlendMode {
    Opaque = 0,
    AlphaBlended = 1,
}

/// 选择表的四个落点，名字与源 ShaderAttribute 枚举一一对应。
pub const FIXTURE_OBJECT_OPAQUE: u32 = 5;
pub const FIXTURE_OBJECT_ALPHA_BLENDED: u32 = 6;
pub const WINDOW_OUTSIDE_FIXTURE_OPAQUE: u32 = 26;
pub const WINDOW_OUTSIDE_FIXTURE_ALPHA_BLENDED: u32 = 27;

/// `(FixtureShaderUsage, FixtureObjectBlendMode) → ShaderAttribute`
/// 的二维选择表。输入收原始整值（材质上的数）而不是先收窄的枚举：
/// 越界值必须在这里响亮地拒掉，闭集收窄会把「越界即拒」这一臂
/// 在编译期吞掉。
pub fn basic_fixture_shader_attribute(usage: i32, blend: i32) -> Result<u32, String> {
    match (usage, blend) {
        (0, 0) => Ok(FIXTURE_OBJECT_OPAQUE),
        (0, 1) => Ok(FIXTURE_OBJECT_ALPHA_BLENDED),
        (1, 0) => Ok(WINDOW_OUTSIDE_FIXTURE_OPAQUE),
        (1, 1) => Ok(WINDOW_OUTSIDE_FIXTURE_ALPHA_BLENDED),
        _ => Err(format!(
            "fixture shader usage {usage} / blend mode {blend} is out of range"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_set_is_closed_at_twelve_names() {
        // 闭集大小钉在 12：名字增删会让这一臂红。
        assert_eq!(FAMILY_SHADER_NAMES.len(), 12);
        // 主力三族在集内。
        for name in [
            "Mysekai/Fixture/Basic",
            "Mysekai/Fixture/ShadowMesh",
            "Mysekai/Fixture/Road",
        ] {
            assert_eq!(check_family_shader(name), Ok(()), "{name}");
        }
    }

    #[test]
    fn known_names_pass_and_unknown_names_are_rejected() {
        // 集内全部放行。
        for name in FAMILY_SHADER_NAMES {
            if name != NAMED_REJECTION {
                assert_eq!(check_family_shader(name), Ok(()));
            }
        }
        // 集外未知名字：泛化拒绝，报错不含具名拒绝的措辞。
        let err = check_family_shader("Mysekai/Fixture/Unknown").unwrap_err();
        assert!(err.contains("not in the fixture family set"));
        // 大小写/前缀近似不算命中：族键是精确匹配。
        assert!(check_family_shader("mysekai/fixture/basic").is_err());
        assert!(check_family_shader("Mysekai/Fixture/Basic2").is_err());
    }

    #[test]
    fn urp_lit_is_rejected_by_name() {
        // 具名拒绝：报错指名道姓，与未知名字的报错是两种话。
        let err = check_family_shader(NAMED_REJECTION).unwrap_err();
        assert!(err.contains(NAMED_REJECTION));
        assert!(!err.contains("not in the fixture family set"));
    }

    #[test]
    fn shader_attribute_table_four_cells() {
        // 二维表四个格子逐值：与源选择表逐项相等。
        assert_eq!(basic_fixture_shader_attribute(0, 0).unwrap(), 5);
        assert_eq!(basic_fixture_shader_attribute(0, 1).unwrap(), 6);
        assert_eq!(basic_fixture_shader_attribute(1, 0).unwrap(), 26);
        assert_eq!(basic_fixture_shader_attribute(1, 1).unwrap(), 27);
    }

    #[test]
    fn shader_attribute_table_rejects_out_of_range() {
        // 越界即拒：usage 或 blend 任一越界，整表拒绝。
        // 若输入被先收窄成枚举，这一臂在编译期就没了——所以
        // 函数签名收原始整值。
        for (usage, blend) in [
            (2, 0),
            (-1, 0),
            (0, 2),
            (0, -1),
            (7, 7),
            (1, 3),
        ] {
            assert!(
                basic_fixture_shader_attribute(usage, blend).is_err(),
                "usage {usage} blend {blend}"
            );
        }
        // 值语义不漂移：Indoor=0/WindowOutside=1、Opaque=0/AlphaBlended=1
        // 是源枚举的整值，材质上存的就是这两个数。
        assert_eq!(FixtureShaderUsage::Indoor as i32, 0);
        assert_eq!(FixtureShaderUsage::WindowOutside as i32, 1);
        assert_eq!(FixtureObjectBlendMode::Opaque as i32, 0);
        assert_eq!(FixtureObjectBlendMode::AlphaBlended as i32, 1);
    }
}
