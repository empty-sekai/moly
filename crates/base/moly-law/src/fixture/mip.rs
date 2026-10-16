//! 家具主贴图 mip 全链的级数律。
//!
//! 源侧（贴图资产序列化的 mip 计数字段）对家具 Basic 主贴图全体是
//! 「按最长边的完整链」：级数 = log2(最长边)+1，逐级两边各减半、短边
//! 到 1 之后保持 1（2048×1024 的蛋嘴图 12 级、2048² 眼图 12 级、
//! 512² 身图 10 级、256² 件图 9 级）。级数只由形状决定，与内容无关；
//! 摆放面 29 包 45 条「材质 → 主贴图」行逐值核过，无一例外
//! （见 [`mip_levels`] 与文末 corpus 测试的期望表）。
//!
//! 非全链的形状存在于资产里（128² 只有 1 级的通用图案图），但只挂在
//! Basic 主贴图路径之外（动作盒材质的底图），不进本律的消费面。
//! 真正无链的形状（任一边为 0、任一边非 2 的幂）在家具贴图全体里
//! 不出现，按无链拒绝。

/// 形状门 + 级数：两边都为非零 2 的幂时，返回完整链的级数（按最长
/// 边减半到 1×1 的步数 + 1，短边先到 1 就保持 1）；否则 `None`——
/// 这种形状源侧不出链，消费方按具名跳过处理，不静默近似。
#[must_use]
pub fn mip_levels(width: u32, height: u32) -> Option<u32> {
    if width == 0 || height == 0 || !width.is_power_of_two() || !height.is_power_of_two() {
        return None;
    }
    let mut levels = 1;
    let (mut w, mut h) = (width, height);
    while w > 1 || h > 1 {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
        levels += 1;
    }
    Some(levels)
}

#[cfg(test)]
#[path = "glb_json.rs"]
mod glb_json;

#[cfg(test)]
mod tests {
    use super::mip_levels;

    /// 摆放面四种活形状 → 源序列化 mip 计数（player-data 侧逐值核得：
    /// 256² 件图 9、512² 身图 10、2048² 眼图 12、2048×1024 嘴图 12）。
    /// 级数换任何算法（按短边 / 按两边取小 / 方形限死）这四行都会红。
    #[test]
    fn live_shapes_match_source_mip_counts() {
        assert_eq!(mip_levels(256, 256), Some(9));
        assert_eq!(mip_levels(512, 512), Some(10));
        assert_eq!(mip_levels(2048, 2048), Some(12));
        // 非方形：级数按最长边——嘴部图 2048×1024 是 12 级（最末 1×1），
        // 不是按短边的 11，也不是「无链」。
        assert_eq!(mip_levels(2048, 1024), Some(12));
        assert_eq!(mip_levels(1024, 2048), Some(12));
    }

    /// 无链形状：任一边为 0、任一边非 2 的幂。阳性对照同行放一个
    /// 合法形状，证明 `None` 不是「全部都 None」。
    #[test]
    fn illegal_shapes_have_no_chain() {
        assert_eq!(mip_levels(0, 256), None);
        assert_eq!(mip_levels(256, 0), None);
        assert_eq!(mip_levels(640, 480), None);
        assert_eq!(mip_levels(2048, 1000), None);
        assert_eq!(mip_levels(3, 4), None);
        // 阳性对照：形状门两侧只差一个数。
        assert_eq!(mip_levels(640, 512), None);
        assert_eq!(mip_levels(512, 512), Some(10));
    }

    /// 短边先到 1 的形状：链继续按长边走（1 是 2 的幂，形状合法）。
    /// 这是「按最长边」的另一半——只测 2048×1024 抓不住退化边。
    #[test]
    fn degenerate_short_edge_keeps_halving_the_long_edge() {
        assert_eq!(mip_levels(1, 1), Some(1));
        assert_eq!(mip_levels(2, 2), Some(2));
        assert_eq!(mip_levels(1, 2048), Some(12));
        assert_eq!(mip_levels(2048, 1), Some(12));
        assert_eq!(mip_levels(2, 8), Some(4));
    }

    // ---- corpus：摆放面的真 glb 逐包核对 ------------------------------
    //
    // `#[ignore]` + env 门（`MOLY_ASSET_ROOT`）：默认跳过时是红的
    // skip（`cargo test -- --ignored` 列表可见），env 在时真断言。

    /// 摆放面包名（产品侧摆放表的包名列；扩员时两侧同补——这张表是
    /// 判据的分母，漏一行 = 少核一张）。
    const PLACED_PACKAGES: [&str; 29] = [
        "mysekai__fixture__mdl_bir1103_fixture_chair1",
        "mysekai__fixture__mdl_bir1103_fixture_cake1",
        "mysekai__fixture__mdl_bir1103_fixture_balloon1",
        "mysekai__fixture__mdl_bir1103_fixture_flower1",
        "mysekai__fixture__mdl_con0004_fixture_wallshelf1",
        "mysekai__fixture__mdl_env0002_window_window1",
        "mysekai__fixture__mdl_env0002_fixture_byoubu1",
        "mysekai__fixture__mdl_ext0019_fixture_sofa1",
        "mysekai__fixture__mdl_clb1102_fixture_egg1",
        "mysekai__fixture__mdl_clb1102_fixture_egg2",
        "mysekai__fixture__mdl_clb1102_fixture_egg3",
        "mysekai__fixture__mdl_clb1102_fixture_egg4",
        "mysekai__fixture__mdl_ext0001_fixture_lamp1",
        "mysekai__fixture__mdl_ext0009_fixture_lamp1",
        "mysekai__fixture__mdl_ext0010_fixture_lamp1",
        "mysekai__fixture__mdl_ext0008_fixture_lamp1",
        "mysekai__fixture__mdl_ext0019_fixture_lamp1",
        "mysekai__fixture__mdl_env0002_fixture_lamp1",
        "mysekai__fixture__mdl_env0012_fixture_lamp1",
        "mysekai__fixture__mdl_mis0001_fixture_lamp1",
        "mysekai__fixture__mdl_env0004_fixture_lamp1",
        "mysekai__fixture__mdl_non9001_fixture_groundlight1",
        "mysekai__fixture__mdl_twcollect_fixture_table1",
        "mysekai__fixture__mdl_non0005_system_desktop1",
        "mysekai__fixture__mdl_non0005_system_laptop1",
        "mysekai__fixture__mdl_twcollect_fixture_planter1",
        "mysekai__fixture__mdl_cncollect_fixture_tea3",
        "mysekai__fixture__mdl_con0002_fixture_coral1",
        "mysekai__fixture__mdl_chr0004_fixture_nenerobo1",
    ];

    /// 摆放面 Basic 主贴图的期望表：(贴图名, 宽, 高, 源序列化 mip 计数)。
    /// mip 计数逐张贴图从资产包里核得——它们不是「log2(最长边)+1」的
    /// 推导值，是独立量出来的源事实，这行表就是判据的锚：律或贴图任
    /// 一边变了，corpus 测试红。闭集：表外贴图名出现即红（新摆入的包
    /// 必须先来这行表里补上它的源 mip 计数）。
    #[rustfmt::skip]
    const EXPECTED: [(&str, u32, u32, u32); 36] = [
        ("tex_bir1103_fixture_chair1_1",        256,  256,  9),
        ("tex_bir1103_fixture_cake1_1",         256,  256,  9),
        ("tex_bir1103_fixture_balloon1_1",      256,  256,  9),
        ("tex_bir1103_fixture_flower1_1",       256,  256,  9),
        ("tex_con0004_fixture_wallshelf1_1",    256,  256,  9),
        ("tex_env0002_window_window1_1",        256,  256,  9),
        ("tex_env0002_fixture_byoubu1_1",       256,  256,  9),
        ("tex_ext0019_fixture_sofa1_1",         256,  256,  9),
        ("tex_clb1102_fixture_egg1_body_1",     512,  512, 10),
        ("tex_clb1102_fixture_egg1_eye_1",     2048, 2048, 12),
        ("tex_clb1102_fixture_egg1_mouth_1",   2048, 1024, 12),
        ("tex_clb1102_fixture_egg2_body_1",     512,  512, 10),
        ("tex_clb1102_fixture_egg2_eye_1",     2048, 2048, 12),
        ("tex_clb1102_fixture_egg3_body_1",     512,  512, 10),
        ("tex_clb1102_fixture_egg3_eye_1",     2048, 2048, 12),
        ("tex_clb1102_fixture_egg3_mouth_1",   2048, 1024, 12),
        ("tex_clb1102_fixture_egg4_body_1",     512,  512, 10),
        ("tex_clb1102_fixture_egg4_eye_1",     2048, 2048, 12),
        ("tex_clb1102_fixture_egg4_mouth_1",   2048, 1024, 12),
        ("tex_ext0001_fixture_lamp1_1",         256,  256,  9),
        ("tex_ext0009_fixture_lamp1_1",         256,  256,  9),
        ("tex_ext0010_fixture_lamp1_1",         256,  256,  9),
        ("tex_ext0008_fixture_lamp1_1",         256,  256,  9),
        ("tex_ext0019_fixture_lamp1_1",         256,  256,  9),
        ("tex_env0002_fixture_lamp1_1",         256,  256,  9),
        ("tex_env0012_fixture_lamp1_1",         256,  256,  9),
        ("tex_mis0001_fixture_lamp1_1",         256,  256,  9),
        ("tex_env0004_fixture_lamp1_1",         256,  256,  9),
        ("tex_non9001_fixture_groundlight1_1",  256,  256,  9),
        ("tex_mdl_twcollect_fixture_table1_1",  256,  256,  9),
        ("tex_non0005_system_desktop1_1",       256,  256,  9),
        ("tex_non0005_system_laptop1_1",        256,  256,  9),
        // 这两张贴图名自带 mat_/无 mdl 前缀，是资产侧的命名实况，不是笔误。
        ("mat_twcollect_fixture_planter1_1",    256,  256,  9),
        ("tex_cncollect_fixture_tea3_1",        256,  256,  9),
        ("tex_con0002_fixture_coral1_3",        256,  256,  9),
        ("tex_chr0004_fixture_nenerobo1_1",     512,  512, 10),
    ];

    /// 摆放面「Basic 材质 → 主贴图」行数（多材质共享同一张贴图的行
    /// 各自计）——数量漂移（包里少了/多了材质）先在这里红。
    const EXPECTED_ROWS: usize = 45;

    /// 读摆放面的全部 glb：返回 Basic 材质的主贴图 (包名, 贴图名, 宽, 高)。
    /// JSON 块用本文件旁的私有最小读取器（`glb_json.rs`，零依赖姿势与
    /// crate 里另两份同构件同一约定）；贴图尺寸从内嵌 PNG 的 IHDR 直读
    /// （宽高在 8 字节签名 + 4 长度 + 4 类型之后的大端 u32）。
    fn placed_basic_main_texes() -> Option<Vec<(String, String, u32, u32)>> {
        use super::glb_json::{parse, Value};

        let root = std::env::var("MOLY_ASSET_ROOT").ok()?;
        let mut rows = Vec::new();
        for package in PLACED_PACKAGES {
            let path = format!("{root}/fixture-models/{package}.glb");
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|err| panic!("摆放面包 {package} 读不到：{err}"));
            assert_eq!(&bytes[..4], b"glTF", "{package} 不是 glb");
            let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
            assert_eq!(&bytes[16..20], b"JSON", "{package} 首块不是 JSON");
            let doc = parse(&bytes[20..20 + json_len])
                .unwrap_or_else(|err| panic!("{package} 的 JSON 块解析失败：{err}"));
            let bin_head = (20 + json_len + 3) & !3;
            let bin_len =
                u32::from_le_bytes(bytes[bin_head..bin_head + 4].try_into().unwrap()) as usize;
            assert_eq!(
                &bytes[bin_head + 4..bin_head + 8],
                b"BIN\0",
                "{package} 第二块不是 BIN"
            );
            let bin = &bytes[bin_head + 8..bin_head + 8 + bin_len];
            let views = doc
                .get("bufferViews")
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("{package} 缺 bufferViews"));
            let images = doc
                .get("images")
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("{package} 缺 images"));
            let textures = doc
                .get("textures")
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("{package} 缺 textures"));
            let materials = doc
                .get("materials")
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("{package} 缺 materials"));
            for material in materials {
                let Some(extras) = material.get("extras") else {
                    continue;
                };
                if extras.get("shader").and_then(Value::as_str) != Some("Mysekai/Fixture/Basic") {
                    continue;
                }
                let Some(index) = extras
                    .get("textures")
                    .and_then(|textures| textures.get("_MainTex"))
                    .and_then(Value::as_f64)
                else {
                    continue;
                };
                let image_index = textures[index as usize]
                    .get("source")
                    .and_then(Value::as_f64)
                    .unwrap_or_else(|| panic!("{package} 的纹理 {index} 缺 source"))
                    as usize;
                let image = &images[image_index];
                let name = image
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_owned();
                let view_index = image
                    .get("bufferView")
                    .and_then(Value::as_f64)
                    .unwrap_or_else(|| panic!("{package} 的图 {name} 缺 bufferView"))
                    as usize;
                let view = &views[view_index];
                let offset = view
                    .get("byteOffset")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as usize;
                let head = &bin[offset..offset + 24];
                assert_eq!(&head[..8], b"\x89PNG\r\n\x1a\n", "{package} 的 {name} 不是 PNG");
                let width = u32::from_be_bytes(head[16..20].try_into().unwrap());
                let height = u32::from_be_bytes(head[20..24].try_into().unwrap());
                rows.push((package.to_owned(), name, width, height));
            }
        }
        Some(rows)
    }

    #[test]
    #[ignore = "MOLY_ASSET_ROOT not set: 0 of the 29 placed packages were checked"]
    fn placed_basic_main_texes_match_source_mip_counts() {
        let Some(rows) = placed_basic_main_texes() else {
            panic!("MOLY_ASSET_ROOT 未设置——corpus 判据需要它指向 local-data");
        };
        assert_eq!(rows.len(), EXPECTED_ROWS, "摆放面行数漂移");
        for (package, name, width, height) in &rows {
            let Some(&(_, want_w, want_h, want_mips)) =
                EXPECTED.iter().find(|(want, _, _, _)| want == name)
            else {
                panic!("{package} 的主贴图 {name} 不在期望表里——先去资产包核它的源 mip 计数再补表");
            };
            assert_eq!((*width, *height), (want_w, want_h), "{package}/{name} 尺寸漂移");
            let levels = mip_levels(*width, *height)
                .unwrap_or_else(|| panic!("{package}/{name} 形状无链（{}×{}）", width, height));
            assert_eq!(
                levels, want_mips,
                "{package}/{name}：链级数 {} != 源 mip 计数 {}",
                levels, want_mips
            );
        }
    }
}
