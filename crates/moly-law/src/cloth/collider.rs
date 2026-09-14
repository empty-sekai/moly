//! 碰撞基元：球 / 胶囊 / 无限平面的世界空间形与推出。
//!
//! 语义表来自 rig `cloth.capsuleSemantics` / `planeSemantics`（与
//! 反编译 ColliderCollisionConstraint 的 Sphere/Capsule/Plane
//! Detection 对照一致）：
//! * 胶囊 `length` 是**半长**，端点 = `center ± axis·length`；
//!   `startRadius` 属 **-axis 端**（t=0），`endRadius` 属 +axis 端
//!   （t=1），面半径 = `lerp(start, end, t)`；
//! * 平面法线 = 骨本地 `normalDirection`（变换 up 轴）经世界旋转，
//!   过 `boneWorld·center`，粒子推到法线一侧；
//! * 缩放按骨世界矩阵三轴绝对值平均（demo colliderToWorld 形）。
//!
//! 反编译差异（不转录，见 mod.rs 表）：CollisionJob 对全部
//! 粒子×碰撞体做「最深命中」挑选并在 0.03m 深度窗内做摩擦簿记；
//! 本律按 demo 形顺序遍历逐个推出（用户验收形），摩擦数据
//! （0.05/0.03）量级不改变这个选择。

use super::math;

/// 局部空间碰撞体定义（rig `cloth.colliders[*]`）。
#[derive(Debug, Clone, PartialEq)]
pub enum ColliderDef {
    Sphere {
        center: [f32; 3],
        radius: f32,
    },
    Capsule {
        center: [f32; 3],
        /// 0=X 1=Y 2=Z（局部轴下标）。
        axis: u8,
        /// 半长。
        length: f32,
        /// -axis 端半径。
        start_radius: f32,
        /// +axis 端半径。
        end_radius: f32,
    },
    Plane {
        center: [f32; 3],
        /// 局部单位法线（变换 up 轴）。
        normal: [f32; 3],
    },
}

/// 世界空间碰撞基元（调用方每帧用骨世界矩阵把 [`ColliderDef`]
/// 搬进来；缩放已在搬入时按三轴平均乘进半径）。
#[derive(Debug, Clone, PartialEq)]
pub enum WorldCollider {
    Sphere {
        center: [f32; 3],
        radius: f32,
    },
    Capsule {
        p0: [f32; 3],
        p1: [f32; 3],
        r0: f32,
        r1: f32,
    },
    Plane {
        origin: [f32; 3],
        /// 世界单位法线。
        normal: [f32; 3],
    },
}

impl WorldCollider {
    /// 局部定义 → 世界基元。`m` 是骨世界矩阵的
    /// `(position, rotation, scale)`；缩放取三轴绝对值平均
    /// （demo colliderToWorld 的 `s`）。
    pub fn from_def(def: &ColliderDef, pos: [f32; 3], rot: [f32; 4], scale: [f32; 3]) -> Self {
        let s = (scale[0].abs() + scale[1].abs() + scale[2].abs()) / 3.0;
        let world_center = {
            let local = match def {
                ColliderDef::Sphere { center, .. }
                | ColliderDef::Capsule { center, .. }
                | ColliderDef::Plane { center, .. } => *center,
            };
            // 骨世界矩阵 M = T·R·S：偏移先吃缩放再旋转
            // （demo colliderToWorld 的 applyMatrix4 同义）。
            let scaled = [
                local[0] * scale[0],
                local[1] * scale[1],
                local[2] * scale[2],
            ];
            add(pos, math::quat_rotate(rot, scaled))
        };
        match def {
            ColliderDef::Sphere { radius, .. } => WorldCollider::Sphere {
                center: world_center,
                radius: radius * s,
            },
            ColliderDef::Capsule {
                axis,
                length,
                start_radius,
                end_radius,
                ..
            } => {
                let mut dir = [0.0; 3];
                dir[*axis as usize] = 1.0;
                let dir = math::quat_rotate(rot, dir);
                WorldCollider::Capsule {
                    p0: sub(world_center, mul(dir, *length)),
                    p1: add(world_center, mul(dir, *length)),
                    r0: start_radius * s,
                    r1: end_radius * s,
                }
            }
            ColliderDef::Plane { normal, .. } => WorldCollider::Plane {
                origin: world_center,
                normal: math::normalize(math::quat_rotate(rot, *normal)),
            },
        }
    }

    /// 有符号净空：`粒子位置` 到表面的距离减粒子半径；负 = 穿插。
    /// （胶囊返回 `distance - (面半径 + r)`。）
    pub fn clearance(&self, p: [f32; 3], r: f32) -> f32 {
        match self {
            WorldCollider::Sphere { center, radius } => {
                math::length(sub(p, *center)) - (radius + r)
            }
            WorldCollider::Capsule { p0, p1, r0, r1 } => {
                let (cp, face) = closest_on_capsule(p, *p0, *p1, *r0, *r1);
                math::length(sub(p, cp)) - (face + r)
            }
            WorldCollider::Plane { origin, normal } => math::dot(sub(p, *origin), *normal) - r,
        }
    }

    /// 穿插时沿径向法线推出（写入并返回是否推动）。退化
    /// `d ≈ 0` 沿 +Y 顶起（demo pushOut 的退化臂）。
    pub fn push_out(&self, p: &mut [f32; 3], r: f32) -> bool {
        match self {
            WorldCollider::Plane { origin, normal } => {
                let d = math::dot(sub(*p, *origin), *normal);
                if d >= r {
                    return false;
                }
                *p = add(*p, mul(*normal, r - d));
                true
            }
            WorldCollider::Sphere { center, radius } => {
                let total = radius + r;
                let off = sub(*p, *center);
                let d = math::length(off);
                if d >= total {
                    return false;
                }
                if d <= 1e-9 {
                    *p = add(*center, [0.0, total, 0.0]);
                    return true;
                }
                *p = add(*center, mul(off, total / d));
                true
            }
            WorldCollider::Capsule { p0, p1, r0, r1 } => {
                let (cp, face) = closest_on_capsule(*p, *p0, *p1, *r0, *r1);
                let total = face + r;
                let off = sub(*p, cp);
                let d = math::length(off);
                if d >= total {
                    return false;
                }
                if d <= 1e-9 {
                    *p = add(cp, [0.0, total, 0.0]);
                    return true;
                }
                *p = add(cp, mul(off, total / d));
                true
            }
        }
    }
}

/// 胶囊最近点与该处的面半径（含 t∈[0,1] 端点夹取；
/// 轴长平方 ≤ 1e-18 视作点（demo 退化臂））。
fn closest_on_capsule(p: [f32; 3], p0: [f32; 3], p1: [f32; 3], r0: f32, r1: f32) -> ([f32; 3], f32) {
    let axis = sub(p1, p0);
    let seg2 = math::dot(axis, axis);
    let t = if seg2 <= 1e-18 {
        0.0
    } else {
        (math::dot(sub(p, p0), axis) / seg2).clamp(0.0, 1.0)
    };
    (add(p0, mul(axis, t)), r0 + (r1 - r0) * t)
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn mul(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 球：穿插粒子沿径向推到 `R + r` 表面；深度退化沿 +Y。
    #[test]
    fn sphere_push_out() {
        let s = WorldCollider::Sphere {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
        };
        let mut p = [0.5, 0.0, 0.0];
        assert!(s.push_out(&mut p, 0.1));
        assert!((p[0] - 1.1).abs() < 1e-6);
        // 在表面外不动
        let mut q = [1.5, 0.0, 0.0];
        assert!(!s.push_out(&mut q, 0.1));
        assert_eq!(q, [1.5, 0.0, 0.0]);
        // 正中心退化沿 +Y
        let mut c = [0.0, 0.0, 0.0];
        assert!(s.push_out(&mut c, 0.1));
        assert!((c[1] - 1.1).abs() < 1e-6);
        // 净空符号
        assert!((s.clearance([0.5, 0.0, 0.0], 0.1) + 0.6).abs() < 1e-6);
    }

    /// 胶囊：length=半长、startRadius 在 -axis 端；面半径按 t 插值。
    #[test]
    fn capsule_half_length_and_radius_lerp() {
        // 沿 Z：p0=(0,0,-1) r0=0.2，p1=(0,0,+1) r1=1.0
        let c = WorldCollider::Capsule {
            p0: [0.0, 0.0, -1.0],
            p1: [0.0, 0.0, 1.0],
            r0: 0.2,
            r1: 1.0,
        };
        // 粒子在 (0, 0.3, 0.5)：t=0.75、面半径 = 0.2+0.8·0.75 = 0.8；
        // 总距 = 0.9，径向 +Y → 推到 (0, 0.9, 0.5)
        let mut p = [0.0, 0.3, 0.5];
        assert!(c.push_out(&mut p, 0.1));
        assert!((p[1] - 0.9).abs() < 1e-5, "got {p:?}");
        assert!((p[2] - 0.5).abs() < 1e-6);
        // -axis 端用 startRadius：粒子在 (0, 0.1, -1)（t=0）
        let mut q = [0.0, 0.1, -1.0];
        assert!(c.push_out(&mut q, 0.0));
        assert!((q[1] - 0.2).abs() < 1e-6, "start radius, got {q:?}");
        // +axis 端用 endRadius：粒子在 (0, 0.5, 1)（t=1）
        let mut r = [0.0, 0.5, 1.0];
        assert!(c.push_out(&mut r, 0.0));
        assert!((r[1] - 1.0).abs() < 1e-6, "end radius, got {r:?}");
    }

    /// 平面：有符号推出到法线侧 `r` 净空。
    #[test]
    fn plane_push_out() {
        let pl = WorldCollider::Plane {
            origin: [0.0, 0.0, 0.0],
            normal: [0.0, 1.0, 0.0],
        };
        let mut p = [0.3, -0.2, -0.4];
        assert!(pl.push_out(&mut p, 0.1));
        assert!((p[1] - 0.1).abs() < 1e-6);
        assert!((p[0] - 0.3).abs() < 1e-6);
        // 法线侧够远不动
        let mut q = [0.0, 0.5, 0.0];
        assert!(!pl.push_out(&mut q, 0.1));
    }

    /// 局部定义 → 世界：轴旋转换向、缩放按三轴平均进半径。
    #[test]
    fn from_def_world_transform() {
        // 单位缩放、恒等旋转的球
        let def = ColliderDef::Sphere {
            center: [1.0, 0.0, 0.0],
            radius: 0.5,
        };
        let w = WorldCollider::from_def(&def, [10.0, 0.0, 0.0], math::quat_identity(), [1.0; 3]);
        assert_eq!(
            w,
            WorldCollider::Sphere {
                center: [11.0, 0.0, 0.0],
                radius: 0.5
            }
        );
        // 均匀缩放 2 → 半径乘 2
        let w2 = WorldCollider::from_def(&def, [0.0, 0.0, 0.0], math::quat_identity(), [2.0; 3]);
        assert_eq!(
            w2,
            WorldCollider::Sphere {
                center: [2.0, 0.0, 0.0],
                radius: 1.0
            }
        );
    }
}
