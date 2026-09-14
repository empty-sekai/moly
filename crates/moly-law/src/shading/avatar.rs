//! `Mysekai/Avatar` 程序的逐句翻译——观众 avatar（玩家）第三条管线。
//!
//! 与 `Mysekai/Character` 是两份不同的程序：Avatar 16 个属性、Character 20 个，
//! 只共有 `_DitherAlpha` 与 `_groupDither` 两个，pass 集也不同（Avatar 是
//! Base / StencilShadow / ShadowCaster，Character 是 Base / ShadowCaster /
//! Eyebrow）。**不照 Character 的 toon 抄**——那条的 face/body 遮罩、球面距离场、
//! 眉毛穿透这一族 Avatar 一概没有。
//!
//! Avatar 的 Base 片元是一条「part-index 槽分派」链：合并网格把多个 part
//! 合成一个网格一份材质，靠顶点 UV0 的 z 分量区分哪片属于哪个 part，
//! 片元按它选贴图与调色。槽分派见 [`slot_albedo`]。
//!
//! **顶点 UV0.z（part-index）是数据，不是绘制结构。** 真源合并网格
//! （`AvatarUtility.CombineMesh`）把每个 part 的 part-index 烘进它的 UV0.z，
//! 合并出的单网格单材质靠它分派。本仓的合成体 glb **没有 z 分量**——那是
//! 提取缺口，不是消费缺口：`CombineMesh` 那一步在提取侧没把 part-index 烘进
//! UV0.z。⇒ 这里**照真源按「UV0.z 会在」写**；喂数据那一侧（`moly-game` 的
//! avatar_material.rs）在缺 z 分量时响亮拒绝，**不拆绘制**（拆成多绘制是
//! 另一个结构，真源是单网格单材质）。
//!
//! 本程序不读顶点色：全部记录的顶点输入只有 POSITION/TEXCOORD/NORMAL。
//! `_USE_DITHER` 具名挂账：代码明确 `EnableKeyword`，而 4 个 program record
//! 里出现的 keyword 只有 `_USE_ALPHA_CLIP`——它要么不产生变体（非 shader_feature），
//! 要么 census 采集口径漏了；bayer 抖动那段在本程序无条件执行（无 `_UseDither`
//! 门，与 Character 的两支路不同）。

use crate::shading::{bayer_value_avatar, SECOND_COLOR_TARGET};

/// 该族在真源里的 shader 名。
pub const SHADER_NAME: &str = "Mysekai/Avatar";

/// part-index 槽的常量：身体基础图 `_SkinTex`。
pub const SLOT_SKIN: i32 = 0;
/// 饰品合批贴图 `_AccessoryTex`。
pub const SLOT_ACCESSORY: i32 = 1;
/// 荧光棒身体 `_PenlightBodyTex`（左 2、右 4 两支合并）。
pub const SLOT_PENLIGHT_BODY_L: i32 = 2;
pub const SLOT_PENLIGHT_BODY_R: i32 = 4;
/// 荧光棒发光 `_PenlightLightTex`（左 3、右 5）。
pub const SLOT_PENLIGHT_LIGHT_L: i32 = 3;
pub const SLOT_PENLIGHT_LIGHT_R: i32 = 5;

/// 观众 avatar 一帧的全局量，字段与源片元的全局块声明一一对应。现象值
/// 由调用方供给，这里不猜值。雾与屏幕参数/投影参数同 Character 一族共用
/// （bayer 表消费屏幕参数；本单只交付 Base 片元，雾变体不在此列）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvatarGlobals {
    /// `_GlobalCharacterDirectionalLightColor`：xyz 光色、w 混合因子。
    pub light_color: [f32; 4],
    /// `_MysekaiScreenParams`：bayer 表消费的屏幕像素尺寸（xy）。
    pub screen_params: [f32; 4],
    /// `_GlobalMipBias`：源只吃 `.x`（采样 mip 偏置）。
    pub mip_bias: [f32; 2],
}

/// Avatar 槽的材质值（16 属性里 Base 片元消费的子集）。默认形象：四列
/// null + penlight 不挂 ⇒ `_SkinColor=(1,1,1,1)`、`_AccessoryTex` 空、
/// penlight 七个属性停默认（`_EnablePenlightLighting` 默认 0 < 1 ⇒
/// penlight 叠加整支被门关掉）。
#[derive(Debug, Clone, PartialEq)]
pub struct AvatarMaterial {
    /// `_SkinColor`：身体调色（rgba）。默认 `(1,1,1,1)`。
    pub skin_color: [f32; 4],
    /// `_Alpha`：alpha 增益项的底（`(-_Alpha)*_Alpha + 1`）。默认 0 ⇒ 增益 1。
    pub alpha: f32,
    /// `_DitherAlpha`：bayer discard 的阈值减数。默认形象不抖 ⇒ 取 0。
    pub dither_alpha: f32,
    /// `_EnablePenlightLighting`：penlight 叠加的总门（< 1 关）。默认 0。
    pub enable_penlight_lighting: f32,
    // penlight 七属性停默认；默认形象 Mysekai 装配路径不挂荧光棒，
    // `_LeftPenlightActive`/`_RightPenlightActive` 停 0，叠加权重恒 0。
    pub left_penlight_active: f32,
    pub right_penlight_active: f32,
    pub left_penlight_color: [f32; 4],
    pub right_penlight_color: [f32; 4],
    /// `(xyz 位置, w 强度)`；默认位置原点、强度 0。
    pub left_penlight_param: [f32; 4],
    pub right_penlight_param: [f32; 4],
}

/// 默认形象（`AvatarData.CreateDefaultAvatar`：四列 null + penlight id 1，
/// 而 Mysekai 装配路径不挂荧光棒）的材质值。
pub fn default_material() -> AvatarMaterial {
    AvatarMaterial {
        skin_color: [1.0, 1.0, 1.0, 1.0],
        alpha: 0.0,
        dither_alpha: 0.0,
        enable_penlight_lighting: 0.0,
        left_penlight_active: 0.0,
        right_penlight_active: 0.0,
        left_penlight_color: [1.0, 1.0, 1.0, 1.0],
        right_penlight_color: [1.0, 1.0, 1.0, 1.0],
        left_penlight_param: [0.0, 0.0, 0.0, 0.0],
        right_penlight_param: [0.0, 0.0, 0.0, 0.0],
    }
}

/// 逐片元输入：顶点 varying 与各贴图采样器返回值。`part_index` 是顶点
/// UV0.z（见模块头注）；`skin_rgba`/`accessory_rgb`/`penlight_*` 是对应贴图
/// 在该 UV 的采样。`screen_pos`/`world_pos` 同 Character 一族的语义。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvatarFragmentInputs {
    /// 顶点 UV0.z 取整后的 part-index（源在顶点程序里 `int(in_TEXCOORD0.z)`）。
    pub part_index: i32,
    /// `_SkinTex` 采样（rgba；a 进 `_SkinColor` 混合）。
    pub skin_rgba: [f32; 4],
    /// `_AccessoryTex` 采样（只取 rgb）。
    pub accessory_rgb: [f32; 3],
    /// `_PenlightBodyTex` 采样（只取 rgb）。
    pub penlight_body_rgb: [f32; 3],
    /// `_PenlightLightTex` 采样（rgba；a 进 penlight 调色）。
    pub penlight_light_rgba: [f32; 4],
    /// 顶点 varying：世界坐标（penlight 距离场读它）。
    pub world_pos: [f32; 3],
    /// 顶点 varying：屏幕位置（xy 像素、w=clip.w），bayer 表消费。
    pub screen_pos: [f32; 4],
}

/// alpha 增益：源对 skin/accessory/penlight_body 三支同式——
/// `albedo = (1-albedo) * ((-A)*A + 1) + albedo`，A=`_Alpha`。
/// A=0 时增益 1、式退化为原值；A=1 时整片变白。
#[must_use]
pub fn alpha_gain(albedo: [f32; 3], alpha: f32) -> [f32; 3] {
    let gain = (-alpha) * alpha + 1.0;
    [
        (1.0 - albedo[0]) * gain + albedo[0],
        (1.0 - albedo[1]) * gain + albedo[1],
        (1.0 - albedo[2]) * gain + albedo[2],
    ]
}

/// 槽分派：按 part-index 选贴图与调色，产出 penlight 叠加前的基色。
/// 源是一条 if/else 链（0=skin / 1=accessory / 2,4=penlight_body /
/// 3,5=penlight_light / 其余=白），对未列出的 part-index 恒白。
#[must_use]
pub fn slot_albedo(input: &AvatarFragmentInputs, material: &AvatarMaterial) -> [f32; 3] {
    match input.part_index {
        SLOT_SKIN => {
            // skin：tex.a 把 tex.rgb 往 _SkinColor.rgb 混，再过 alpha 增益。
            let s = input.skin_rgba;
            let mut base = [0.0; 3];
            for c in 0..3 {
                // s.a * (skin_color - s.rgb) + s.rgb
                base[c] = s[3] * (material.skin_color[c] - s[c]) + s[c];
            }
            alpha_gain(base, material.alpha)
        }
        SLOT_ACCESSORY => alpha_gain(input.accessory_rgb, material.alpha),
        SLOT_PENLIGHT_BODY_L | SLOT_PENLIGHT_BODY_R => {
            alpha_gain(input.penlight_body_rgb, material.alpha)
        }
        SLOT_PENLIGHT_LIGHT_L | SLOT_PENLIGHT_LIGHT_R => {
            // penlight_light：tex.rgb * (tex.a * (pen_color - 1) + 1)。
            let t = input.penlight_light_rgba;
            let pen = if input.part_index == SLOT_PENLIGHT_LIGHT_L {
                material.left_penlight_color
            } else {
                material.right_penlight_color
            };
            let mut out = [0.0; 3];
            for c in 0..3 {
                let tint = t[3] * (pen[c] - 1.0) + 1.0;
                out[c] = t[c] * tint;
            }
            out
        }
        _ => [1.0, 1.0, 1.0],
    }
}

/// 一支 penlight 的距离衰减权重：`(1 - min(d * 2.85714293, 1))² * w * active`，
/// d 是世界位到 penlight 位置的距离，w 是 `_XxxPenlightParam.w`，active 是
/// `_XxxPenlightActive`（0/1）。左支半径 0.35（×2.85714=1/0.35），右支再乘 0.5。
#[must_use]
pub fn penlight_weight(world: [f32; 3], param: [f32; 4], active: f32) -> f32 {
    let dx = world[0] - param[0];
    let dy = world[1] - param[1];
    let dz = world[2] - param[2];
    let d = (dx * dx + dy * dy + dz * dz).sqrt();
    let t = (1.0 - (d * 2.85714293).min(1.0)).max(0.0);
    t * t * param[3] * active
}

/// penlight 叠加：左支加 `_LeftPenlightColor`、右支加 `_RightPenlightColor`
/// （右支权重再乘 0.5）。总门 `_EnablePenlightLighting < 1` 或 part 不属于
/// penlight（index ∉ {2,3,4,5}）时整支旁路，基色直通。
#[must_use]
pub fn penlight_overlay(
    base: [f32; 3],
    input: &AvatarFragmentInputs,
    material: &AvatarMaterial,
) -> [f32; 3] {
    let is_penlight_part = matches!(
        input.part_index,
        SLOT_PENLIGHT_BODY_L | SLOT_PENLIGHT_BODY_R | SLOT_PENLIGHT_LIGHT_L | SLOT_PENLIGHT_LIGHT_R
    );
    // 源：notEqual(index, 0) && notEqual(index, 1) ⇒ penlight 一族；
    // 或 _EnablePenlightLighting < 1 ⇒ 直通基色。
    if !is_penlight_part || material.enable_penlight_lighting < 1.0 {
        return base;
    }
    let lw = penlight_weight(
        input.world_pos,
        material.left_penlight_param,
        material.left_penlight_active,
    );
    let rw = penlight_weight(
        input.world_pos,
        material.right_penlight_param,
        material.right_penlight_active,
    ) * 0.5;
    let mut out = base;
    for c in 0..3 {
        out[c] += lw * material.left_penlight_color[c] + rw * material.right_penlight_color[c];
    }
    out
}

/// 全色片元：slot 分派 → penlight 叠加 → 光混，两个声明目标。
/// bayer discard 无法表达成值，是否丢弃由调用方另问 [`dither_discards`]；
/// 被丢弃的片元两个目标全零。源对 penlight 一族与非 penlight 一族都执行
/// penlight 距离场计算，但非 penlight part 的输出在门后被丢——这里按门的
/// 语义先判门。
#[must_use]
pub fn avatar_fragment(
    input: &AvatarFragmentInputs,
    globals: &AvatarGlobals,
    material: &AvatarMaterial,
) -> ([f32; 4], [f32; 4]) {
    let base = slot_albedo(input, material);
    let shaded = penlight_overlay(base, input, material);
    // 光混：light.w * (albedo * light.rgb - albedo) + albedo。
    let lc = globals.light_color;
    let mut rgb = [0.0; 3];
    for c in 0..3 {
        rgb[c] = lc[3] * (shaded[c] * lc[c] - shaded[c]) + shaded[c];
    }
    ([rgb[0], rgb[1], rgb[2], 1.0], SECOND_COLOR_TARGET)
}

/// 源的 discard 判据：`_DitherAlpha` 减 bayer 阈值低于零丢弃。
/// 本程序无 `_UseDither` 门（与 Character 不同），整支无条件执行。
#[must_use]
pub fn dither_discards(
    dither_alpha: f32,
    screen_pos: [f32; 4],
    screen_params: [f32; 4],
) -> bool {
    dither_alpha - bayer_value_avatar(screen_pos, screen_params) < 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shading::close;

    fn globals() -> AvatarGlobals {
        AvatarGlobals {
            light_color: [1.0, 1.0, 1.0, 1.0],
            screen_params: [4.0, 4.0, 1.25, 1.25],
            mip_bias: [0.0, 0.0],
        }
    }

    fn skin_input(part: i32, rgba: [f32; 4]) -> AvatarFragmentInputs {
        AvatarFragmentInputs {
            part_index: part,
            skin_rgba: rgba,
            accessory_rgb: [rgba[0], rgba[1], rgba[2]],
            penlight_body_rgb: [rgba[0], rgba[1], rgba[2]],
            penlight_light_rgba: rgba,
            world_pos: [0.0, 0.0, 0.0],
            screen_pos: [0.25, 0.25, 0.0, 1.0],
        }
    }

    #[test]
    fn alpha_gain_soft_light_by_one_minus_alpha_sq() {
        // 式：out = (1-a)*gain + a，gain = 1 - A²。与 A 的符号无关（A²）。
        // A=0：gain=1 ⇒ out = (1-a)+a = 1，对任意 a 恒 1——不是恒等，是
        // 「增益 1 时全提亮到白」；恒等只在 a=1（albedo 满白）。
        let g = alpha_gain([0.2, 0.5, 0.8], 0.0);
        for c in 0..3 {
            close(g[c], 1.0);
        }
        // a=1：恒等（out = 0*gain + 1 = 1）。
        let g = alpha_gain([1.0, 1.0, 1.0], 0.0);
        for c in 0..3 {
            close(g[c], 1.0);
        }
        // A=0.5：gain = 1-0.25 = 0.75。a=0.2 ⇒ 0.8*0.75+0.2 = 0.8。
        close(alpha_gain([0.2, 0.0, 0.0], 0.5)[0], 0.8);
        // A=1：gain=0 ⇒ out = a（原值直通）。
        close(alpha_gain([0.2, 0.0, 0.0], 1.0)[0], 0.2);
    }

    #[test]
    fn skin_slot_blends_tex_toward_skin_color_by_alpha() {
        // skin 支：base = tex.a*(skin_color-tex.rgb)+tex.rgb，再过 alpha 增益
        // （默认 _Alpha=0 ⇒ 增益 1 ⇒ 整支白）。tex.a=1 ⇒ base=skin_color=白，
        // 增益后仍白——默认形象（skin_color=白、_Alpha=0）下整片恒白，是
        // 真源这支的固有形态（身体基础色由 _SkinTex 供给，_SkinColor=白调色
        // 不改动）。这里钉调色混合的中间值：令 _Alpha=1（增益 0）看清 base。
        let mut m = default_material();
        m.alpha = 1.0; // 增益 0 ⇒ 输出 = base（看清混合）
        let input = skin_input(SLOT_SKIN, [0.4, 0.2, 0.0, 0.5]);
        let out = slot_albedo(&input, &m);
        // base = 0.5*(1-tex)+tex。
        close(out[0], 0.7); // 0.5*0.6+0.4 = 0.7
        close(out[1], 0.6); // 0.5*0.8+0.2 = 0.6
        close(out[2], 0.5); // 0.5*1.0+0.0 = 0.5
    }

    #[test]
    fn slot_switch_dispatches_each_part_and_defaults_white() {
        let mut m = default_material();
        m.alpha = 1.0; // 增益 0 ⇒ skin/accessory/penlight_body 直通原采样
        // accessory 直通。
        let acc = slot_albedo(&skin_input(SLOT_ACCESSORY, [0.3, 0.6, 0.9, 0.0]), &m);
        for (c, want) in [0.3, 0.6, 0.9].iter().enumerate() {
            close(acc[c], *want);
        }
        // 未列出的 part-index 恒白（与增益无关）。
        let unknown = slot_albedo(&skin_input(9, [0.1, 0.2, 0.3, 1.0]), &m);
        for c in 0..3 {
            close(unknown[c], 1.0);
        }
        // penlight_light：tex.rgb * (tex.a*(pen-1)+1)；pen=白 ⇒ 直通 tex.rgb。
        let pl = slot_albedo(&skin_input(SLOT_PENLIGHT_LIGHT_L, [0.5, 0.5, 0.5, 0.0]), &m);
        for c in 0..3 {
            close(pl[c], 0.5);
        }
    }

    #[test]
    fn penlight_weight_falls_off_with_distance() {
        // 在 penlight 位置：d=0 ⇒ (1-0)²*w*active = w。
        let w = penlight_weight([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], 1.0);
        close(w, 1.0);
        // 距离 0.35（半径）：d*2.85714 = 1 ⇒ 权重 0。
        let w = penlight_weight([0.35, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], 1.0);
        close(w, 0.0);
        // active=0 ⇒ 恒 0。
        let w = penlight_weight([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], 0.0);
        close(w, 0.0);
    }

    #[test]
    fn penlight_overlay_bypasses_for_non_penlight_part_or_gate_closed() {
        let m = default_material(); // enable_penlight_lighting = 0
        // skin part：非 penlight ⇒ 直通。
        let base = [0.4, 0.5, 0.6];
        let out = penlight_overlay(base, &skin_input(SLOT_SKIN, [0.0; 4]), &m);
        for c in 0..3 {
            close(out[c], base[c]);
        }
        // penlight part 但门关（默认 enable=0）⇒ 直通。
        let out = penlight_overlay(base, &skin_input(SLOT_PENLIGHT_LIGHT_L, [0.0; 4]), &m);
        for c in 0..3 {
            close(out[c], base[c]);
        }
    }

    #[test]
    fn fragment_light_blend_uses_light_w_as_factor() {
        // _Alpha=1（增益 0）让 albedo 直通 base，专注看光混。
        let mut m = default_material();
        m.alpha = 1.0;
        let mut g = globals();
        // light w=0 ⇒ lit = 0*(base*l - base) + base = base。
        g.light_color = [0.5, 0.5, 0.5, 0.0];
        let input = skin_input(SLOT_SKIN, [0.4, 0.4, 0.4, 1.0]); // a=1 ⇒ base=skin_color=白
        let (target0, target1) = avatar_fragment(&input, &g, &m);
        close(target0[0], 1.0); // base=白，w=0 不暗化
        close(target0[3], 1.0);
        assert_eq!(target1, [0.0, 0.0, 0.0, 0.0]);
        // w=1、光色 0.5：lit = 1*(1*0.5 - 1) + 1 = 0.5。
        g.light_color = [0.5, 0.5, 0.5, 1.0];
        let (target0, _) = avatar_fragment(&input, &g, &m);
        close(target0[0], 0.5);
    }

    #[test]
    fn dither_discards_below_the_bayer_threshold() {
        // 屏幕参数 4x4、pos (0.25,0.25,·,1)：qx=0.25 ⇒ ix=1，iy=1 ⇒ 表元 4，
        // 阈值 4*0.061875+0.01 = 0.2575。
        let pos = [0.25, 0.25, 0.0, 1.0];
        let params = [4.0, 4.0, 0.0, 0.0];
        assert!(dither_discards(0.25, pos, params));
        assert!(!dither_discards(0.26, pos, params));
        // 判别臂（真源那一行改值会红）：阈值随 dither_alpha 单调——alpha 越小
        // 越容易丢，恰等于阈值不丢（< 0 才丢）。
        assert!(dither_discards(0.2574, pos, params));
        assert!(!dither_discards(0.2575, pos, params));
        // 另一个 bayer 格：pos (0.5,0.25) ⇒ ix=2、iy=1 ⇒ 表元 11，
        // 阈值 11*0.061875+0.01 ≈ 0.6906。
        let pos2 = [0.5, 0.25, 0.0, 1.0];
        assert!(dither_discards(0.69, pos2, params));
        assert!(!dither_discards(0.70, pos2, params));
    }
}
