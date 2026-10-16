//! 家具域的律：从真源逐句转录的纯计算，无引擎依赖。
//!
//! 覆盖八件：outline 全局量下发（`outline`）、摆放位置公式（`position`）、
//! timeline 高度分类（`height`）、挂点配对（`attach`）、占用格旋转
//! （`areas`）、着色器族谱与 ShaderAttribute 选择（`family`）、
//! 主贴图 mip 全链级数（`mip`）、质感分支菲涅尔加色（`fresnel`）。
//! 每件带一份移植时的逐值比对，期望值手算自源式子，在各文件的
//! `#[cfg(test)]` 里。

pub mod areas;
pub mod attach;
pub mod family;
pub mod fence;
pub mod fresnel;
pub mod height;
pub mod mip;
pub mod outline;
pub mod position;
pub mod road;

/// 朝向。源枚举把 Front 排在 0，旋转与换算都按这个取值分支；
/// 越界值在源头是字节透传（switch default），这里收窄成闭集。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Front = 0,
    Left = 1,
    Back = 2,
    Right = 3,
}

impl Direction {
    /// 源把朝向存成一个字节；非法值没有对应分支，按闭集拒绝。
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Direction::Front,
            1 => Direction::Left,
            2 => Direction::Back,
            3 => Direction::Right,
            _ => return None,
        })
    }
}

/// 源网格坐标：三根有符号字节轴。源结构里另有一位 IsInvalid 哨兵，
/// 只用于标记「无效格」、不参与任何运算，本模块的律都在有效格上，
/// 所以不携带它。
///
/// 加减是逐字节的带符号回绕算术：源的两端算子都按「符号扩展取出
/// 分量 → 整数加减 → 写回字节」实现，跨字节不借位，所以这里用
/// wrapping 而不是让极端输入在 debug 下炸掉。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GridPosition {
    pub x: i8,
    pub y: i8,
    pub z: i8,
}

impl GridPosition {
    pub const ZERO: GridPosition = GridPosition { x: 0, y: 0, z: 0 };

    pub fn new(x: i8, y: i8, z: i8) -> Self {
        GridPosition { x, y, z }
    }
}

impl std::ops::Add for GridPosition {
    type Output = GridPosition;
    fn add(self, rhs: GridPosition) -> GridPosition {
        GridPosition::new(
            self.x.wrapping_add(rhs.x),
            self.y.wrapping_add(rhs.y),
            self.z.wrapping_add(rhs.z),
        )
    }
}

impl std::ops::Sub for GridPosition {
    type Output = GridPosition;
    fn sub(self, rhs: GridPosition) -> GridPosition {
        GridPosition::new(
            self.x.wrapping_sub(rhs.x),
            self.y.wrapping_sub(rhs.y),
            self.z.wrapping_sub(rhs.z),
        )
    }
}

/// 源的整型三维（格数一类的量）。字段名沿用源序 X/Y/Z。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Vector3Int {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Vector3Int {
    pub const ONE: Vector3Int = Vector3Int { x: 1, y: 1, z: 1 };

    pub fn new(x: i32, y: i32, z: i32) -> Self {
        Vector3Int { x, y, z }
    }
}

/// 源侧反复出现的两个整型小式，这里集中一份：
/// `floor_half` 是源对 int 的算术右移一位（负数向负无穷取整）。
pub(crate) fn floor_half(n: i32) -> i32 {
    n >> 1
}

/// 负数夹 0。源写成 `(n & ~(n >> 31))` 一类的位式，语义就是 max(n, 0)。
pub(crate) fn max0(n: i32) -> i32 {
    if n < 0 {
        0
    } else {
        n
    }
}
