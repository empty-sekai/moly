//! The joystick's pure laws and its camera bake: the handle geometry,
//! `HandleInput`'s clamp and `OnTouchJoyStick`'s bake of one direction on one
//! camera. The touch surface itself is `crate::joystick`.

use bevy::prelude::*;

/// 手柄 RectTransform 边长（canvas 单位；序列化 sizeDelta 128×128）。
/// 也是方向的归一分母（OnDrag 式的分母 = 底盘半边 × handleRange ×
/// canvas 缩放 = 128 × 1.0 × 缩放）。
pub(crate) const HANDLE_SIZE: f32 = 128.0;

/// 死区（序列化 deadZone 0——只捕精确 0）。
pub(crate) const DEAD_ZONE: f32 = 0.0;

/// 方向上限（序列化 Limit 1.0：超限归一 × 1，中段保持原始差值）。
pub(crate) const LIMIT: f32 = 1.0;

/// HandleInput's clamp of a raw stick ratio: the dead zone 0 keeps only an
/// exact zero at zero, above the limit the ratio is normalised times the
/// limit, in between it is kept.
pub(crate) fn handle_input(raw: Vec2) -> Vec2 {
    let magnitude = raw.length();
    if magnitude <= DEAD_ZONE {
        Vec2::ZERO
    } else if magnitude > LIMIT {
        raw / magnitude * LIMIT
    } else {
        raw
    }
}

/// presenter 的 UPDATE 烘基式（OnTouchJoyStick）：移动向量 =
/// 相机前（去 y 归一；水平模长近零时**该项**回零，右项保留）×
/// 方向.y + 相机右 × 方向.x，和向量不归一。相机不在场 → None
/// （真源整式短路，不置活跃）。
pub(crate) fn bake_move_vector(
    direction: Vec2,
    cameras: &Query<&GlobalTransform, With<Camera3d>>,
) -> Option<Vec3> {
    Some(bake(direction, cameras.single().ok()?))
}

/// OnTouchJoyStick's bake of one direction on one camera: camera forward
/// with y removed and normalised (that term zero when its horizontal length
/// is not above 1e-5) times direction.y, plus camera right times
/// direction.x, the sum not normalised.
pub(crate) fn bake(direction: Vec2, global: &GlobalTransform) -> Vec3 {
    let forward = global.forward();
    let horizontal = (forward.x * forward.x + forward.z * forward.z).sqrt();
    let forward_flat = if horizontal <= 0.00001 {
        Vec3::ZERO
    } else {
        Vec3::new(forward.x / horizontal, 0.0, forward.z / horizontal)
    };
    forward_flat * direction.y + global.right().as_vec3() * direction.x
}
