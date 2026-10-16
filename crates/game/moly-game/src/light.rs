//! 晴天光照：平行光加全局环境光。

use bevy::prelude::*;

/// 晴天现象配置的全局太阳角（角度制）；该站点不在站点覆盖表里，全局值即生效值。
pub(crate) const ANGLE_XZ: f32 = 80.0;
pub(crate) const ANGLE_Y: f32 = 57.9;

/// 晴天现象的阴面色，照抄源值。
pub(crate) const SHADE_COLOR: [f32; 3] = [0.8248537182807922, 0.8232467174530029, 0.9433962106704712];

/// 真源没有环境光的可读量（穷举：ClientConfig 光照键 0 处、站点 glb 光实体 0 处、
/// URP/RenderSettings 序列化资产 0 份——等价组件 SekaiAmbientLight 的值在运行时
/// prefab 里，语料读不出）。
/// 按 demo（viewer 沙盒＝用户验收过的观感基线）定标：demo 光九项同样没有环境光
/// 亮度，但其站点着色式给出 暗带 = 反照率 × 阴面色、亮带 = 反照率 × 光色（白、
/// 权重 1），暗带/亮带线性比 = 阴面色均值 0.8638。本仓管线侧反解：
///   环境光出射 = (0.4524×反照率 − 0.0024) × 亮度（bevy_pbr pbr_ambient.wgsl，
///   EnvBRDFApprox×F_AB(1.0,·)，与材质粗糙度无关）；相机为 bevy 默认曝光
///   Exposure::BLENDER（EV100 9.7，×2^−9.7/1.2）+ TonyMcMapface（1.0 → 0.80）。
///   白反照率亮带锚定 0.80 ⇒ 亮带出射 = 1.0/曝光 = 998.7；
///   暗带出射 = 0.8638 × 998.7 = 862.7 ⇒ 亮度 = 862.7 / 0.4500 = 1917。
const AMBIENT_BRIGHTNESS: f32 = 1917.0;

/// 真源没有照度字段；demo 的着色式同样无光强量（方向光贡献只有
/// 亮带 − 暗带 = 0.1362 × 1.0/曝光 这一线性量）。按 Lambert 反解：
/// 直射出射 = 照度 × N·L × 1/π（bevy_pbr pbr_lighting.wgsl Fd_Burley，
/// 光色已按 illuminance 预乘——render/light.rs），N·L=1 处
/// 照度 = 0.1362 × 998.7 × π = 427。
const SUN_ILLUMINANCE: f32 = 427.0;

/// 真源式：仰角转 y 分量，方位角转 XZ 平面分量；返回朝向光源的单位向量。
pub(crate) fn dir_toward_light(angle_xz: f32, angle_y: f32) -> Vec3 {
    let (a, b) = (angle_xz.to_radians(), angle_y.to_radians());
    Vec3::new(a.cos() * b.cos(), b.sin(), a.sin() * b.cos())
}

/// Startup：太阳平行光 + 全局环境光，两者缺一画面就会黑或平。
pub fn spawn(mut commands: Commands, mut ambient: ResMut<GlobalAmbientLight>) {
    let toward = dir_toward_light(ANGLE_XZ, ANGLE_Y);
    // DirectionalLight 沿实体前向（-Z）照射：灯放在太阳一侧、看向场地，光即反向铺满。
    commands.spawn((
        DirectionalLight {
            // 晴天现象的平行光色，源值恰为纯白。
            color: Color::WHITE,
            // demo 验收观感链定标值，推导见上（SUN_ILLUMINANCE）。
            illuminance: SUN_ILLUMINANCE,
            ..default()
        },
        Transform::from_translation(toward * 100.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    ambient.color = Color::srgb(SHADE_COLOR[0], SHADE_COLOR[1], SHADE_COLOR[2]);
    ambient.brightness = AMBIENT_BRIGHTNESS;
}

/// Spherical interpolation of the two direction vectors, not of independently
/// constructed look rotations. For non-antipodal unit inputs this is the unique
/// shortest great-circle path used by EnvironmentShaderView's Vector3.Slerp.
/// The source angle producer returns unit vectors; no light magnitude is blended.
pub(crate) fn slerp_direction(from: Vec3, to: Vec3, progress: f32) -> Vec3 {
    let t = progress.clamp(0.0, 1.0);
    if t == 0.0 || from == to { return from; }
    if t == 1.0 { return to; }
    let a = from.normalize();
    let b = to.normalize();
    let cross = a.cross(b);
    let sine = cross.length();
    let cosine = a.dot(b).clamp(-1.0, 1.0);
    if sine == 0.0 {
        // Antipodal unit vectors have no unique geodesic. No current source
        // pair reaches this branch; do not invent Unity's native fallback axis.
        assert!(cosine >= 0.0, "antipodal source light directions require native axis evidence");
        return a;
    }
    let axis = cross / sine;
    let angle = sine.atan2(cosine) * t;
    let (s,c) = angle.sin_cos();
    a*c + axis.cross(a)*s
}

#[cfg(test)]
mod direction_transition_tests {
    use super::*;
    #[test]
    fn direction_slerp_stays_on_the_source_great_circle() {
        let midpoint = slerp_direction(Vec3::X,Vec3::Z,0.5);
        let expected=Vec3::new(std::f32::consts::FRAC_1_SQRT_2,0.0,std::f32::consts::FRAC_1_SQRT_2);
        assert!(midpoint.distance(expected)<1e-6);
        assert_eq!(slerp_direction(Vec3::Y,Vec3::Y,0.5),Vec3::Y);
    }
    #[test]
    fn oblique_light_vectors_do_not_blend_independent_look_rotations() {
        let a=dir_toward_light(80.0,57.9);
        let b=dir_toward_light(240.0,70.0);
        let midpoint=slerp_direction(a,b,0.5);
        // For unit endpoints the midpoint lies along their normalized sum.
        assert!(midpoint.distance((a+b).normalize())<1e-6);
        assert!(midpoint.dot(a.cross(b)).abs()<1e-6);
    }
}
