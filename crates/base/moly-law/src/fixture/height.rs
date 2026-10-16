//! 家具 timeline 的高度分类律。
//!
//! 源路径：`FixtureTimelineFactory` 在为 NPC 生成家具 timeline 时，读
//! 视图根节点的 localPosition.y，把它分进三个离散档，再按摆放类型
//! 决定是否在资产名上追加高度后缀。
//!
//! 钉在源方法体上的要点：
//! - 三档是**位级精确等值**：源用「最小正 denormal ≤ |差|」的形态
//!   比较，等价于 y 与 0/0.22/0.35 三个 f32 常数逐位相等——0.2200001
//!   这种「肉眼一样」的值照样进错误臂；
//! - 档位不在 {0, 0.22, 0.35} 里 → 记错误日志并**中止**（fail-closed），
//!   不是取最近档；
//! - 后缀只在 PutType == put_target 时加：ground 档加 `-ground`、
//!   low 档加 `-low`、table 档不加；其它摆放类型名原样通过；
//! - 换名规则：后缀名非空且资产名包含原名时，把资产名里**所有**
//!   原名出现替换成后缀名；不包含则资产名原样返回。

/// 源 MysekaiFixturePutType 的取值（后缀规则只认 put_target）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutType {
    None,
    PutBase,
    PutTarget,
    PutEither,
}

impl PutType {
    /// 源枚举的整值（none=-1 起步，put_target 是 1）。
    pub fn from_i32(v: i32) -> Option<Self> {
        Some(match v {
            -1 => PutType::None,
            0 => PutType::PutBase,
            1 => PutType::PutTarget,
            2 => PutType::PutEither,
            _ => return None,
        })
    }
}

/// 高度三档。数值是源里的三个 f32 常数，逐位照抄。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HeightClass {
    Ground,
    LowTable,
    Table,
}

/// 最小正 denormal：源比较链里的「非零」下界。与它比较等价于
/// 位级非零，所以整条链就是精确等值。用位构造而不是字面量，
/// 避免「1.4013e-45 四舍五入到 0」这一类解析歧义。
const SMALLEST_DENORMAL: f32 = f32::from_bits(1);

const LOW_TABLE_Y: f32 = 0.22;
const TABLE_Y: f32 = 0.35;

/// 高度分类：y 必须逐位等于三档之一，否则响亮报错。
pub fn classify_height(y: f32) -> Result<HeightClass, String> {
    if SMALLEST_DENORMAL <= y.abs() {
        if SMALLEST_DENORMAL <= (y - LOW_TABLE_Y).abs() {
            if SMALLEST_DENORMAL <= (y - TABLE_Y).abs() {
                Err(format!(
                    "fixture timeline height {y} is none of 0 / 0.22 / 0.35"
                ))
            } else {
                Ok(HeightClass::Table)
            }
        } else {
            Ok(HeightClass::LowTable)
        }
    } else {
        Ok(HeightClass::Ground)
    }
}

/// 高度档的后缀名：put_target 加后缀（ground/low/table → -ground/-low/无），
/// 其它摆放类型不加。返回 None 表示「名不加后缀」。
fn suffixed_name(fixture_name: &str, class: HeightClass, put_type: PutType) -> String {
    if put_type != PutType::PutTarget {
        return fixture_name.to_string();
    }
    match class {
        HeightClass::Table => fixture_name.to_string(),
        HeightClass::LowTable => format!("{fixture_name}-low"),
        HeightClass::Ground => format!("{fixture_name}-ground"),
    }
}

/// 完整的换名律：高度分类 → 后缀 → 资产名替换。
/// 高度非法时整条链中止（fail-closed），不给「最近档」的降级。
pub fn timeline_asset_name(
    asset_name: &str,
    fixture_name: &str,
    y: f32,
    put_type: PutType,
) -> Result<String, String> {
    let class = classify_height(y)?;
    let suffixed = suffixed_name(fixture_name, class, put_type);
    if !suffixed.is_empty() && asset_name.contains(fixture_name) {
        Ok(asset_name.replace(fixture_name, &suffixed))
    } else {
        Ok(asset_name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_heights_classify_exactly() {
        assert_eq!(classify_height(0.0).unwrap(), HeightClass::Ground);
        assert_eq!(classify_height(0.22).unwrap(), HeightClass::LowTable);
        assert_eq!(classify_height(0.35).unwrap(), HeightClass::Table);
    }

    #[test]
    fn near_misses_are_rejected_not_snapped() {
        // 三个肉眼近似值全部进错误臂：源的判定是位级等值。
        // 若实现被改成「最近档」或带容差的比较，这一臂红。
        for y in [0.22000001, 0.3499, 0.1, 1.0, -0.22] {
            assert!(classify_height(y).is_err(), "y = {y}");
        }
        // 判定顺序也钉住：0.22 必须先于 0.35 命中（同差量级时
        // 低档优先），0 只被 Ground 捕获。
        assert_eq!(classify_height(0.22f32).unwrap(), HeightClass::LowTable);
    }

    #[test]
    fn put_target_appends_height_suffix() {
        // ground 档 + put_target：原名 → -ground。
        assert_eq!(
            timeline_asset_name("mdl_chair_a", "chair", 0.0, PutType::PutTarget).unwrap(),
            "mdl_chair-ground_a"
        );
        // low 档：-low。
        assert_eq!(
            timeline_asset_name("mdl_chair_a", "chair", 0.22, PutType::PutTarget).unwrap(),
            "mdl_chair-low_a"
        );
        // table 档：不加后缀，资产名原样。
        assert_eq!(
            timeline_asset_name("mdl_chair_a", "chair", 0.35, PutType::PutTarget).unwrap(),
            "mdl_chair_a"
        );
    }

    #[test]
    fn other_put_types_never_append() {
        // 同一高度、非 put_target：名原样通过（后缀门只认 put_target）。
        for put in [PutType::None, PutType::PutBase, PutType::PutEither] {
            assert_eq!(
                timeline_asset_name("mdl_chair_a", "chair", 0.0, put).unwrap(),
                "mdl_chair_a"
            );
        }
    }

    #[test]
    fn replacement_hits_every_occurrence_and_respects_guard() {
        // 资产名里原名出现两次：全部替换（源的 Replace 是全量替换）。
        assert_eq!(
            timeline_asset_name("a_chair_b_chair", "chair", 0.22, PutType::PutTarget).unwrap(),
            "a_chair-low_b_chair-low"
        );
        // 资产名不含原名：即便有后缀也不替换。
        assert_eq!(
            timeline_asset_name("mdl_table_x", "chair", 0.0, PutType::PutTarget).unwrap(),
            "mdl_table_x"
        );
    }

    #[test]
    fn illegal_height_aborts_the_whole_chain() {
        // 高度非法 → 整条换名律拒绝，不给降级名。
        assert!(timeline_asset_name("mdl_chair_a", "chair", 0.5, PutType::PutTarget).is_err());
    }
}
