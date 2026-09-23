//! 光照 pass 推全局量前的零值钳：`MysekaiCommandBufferExtension` 的
//! `SetGlobalFloatSafe` / `SetGlobalColorSafe` / `SetGlobalVectorSafe`。
//!
//! 逐分量：`|x|` **严格小于**阈值 ⇒ 写 `+0.0`，否则原值原样写出。比较是
//! 有序小于，NaN 不命中、原样通过；负零与绝对值不足阈值的负数都成 `+0.0`。
//! 光照 pass 的每一次调用都显式传同一个阈值（与签名默认值相同）。
//!
//! 钳的只是**推进全局量的那一份拷贝**：静态光照设置本身不被改写，CPU 侧
//! 直接读设置原值的消费者（影子相机的光向、太阳光晕参数）读到的是未钳值。

/// 光照 pass 传给三个钳函数的阈值：`0.001_f32`，位型 `0x3a83126f`。
pub const THRESHOLD: f32 = 0.001;

/// 单分量钳（float / color / vector 三个入口的共同分量律）。
pub fn clamp_zero(x: f32) -> f32 {
    if x.abs() < THRESHOLD { 0.0 } else { x }
}

/// 四分量钳：color 与 vector 两个入口（vector 的 w 由调用方给）。
pub fn clamp_zero4(v: [f32; 4]) -> [f32; 4] {
    v.map(clamp_zero)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Value by value against the native helper bodies, executed at the light
    /// pass threshold: each forwarded component equals `clamp_zero` bit for bit,
    /// and `clamp_zero4` reproduces the four-component entries whole.
    #[test]
    #[ignore = "MOLY_LIGHT_PASS_SAFE_NATIVE must name the executed Set*Safe helper receipt"]
    fn clamp_matches_the_native_safe_helpers() {
        let path = std::env::var("MOLY_LIGHT_PASS_SAFE_NATIVE")
            .expect("MOLY_LIGHT_PASS_SAFE_NATIVE must name the executed Set*Safe helper receipt");
        let receipt = super::super::json::parse(&std::fs::read(path).unwrap()).unwrap();
        let bits = |value: &super::super::json::Value| {
            u32::from_str_radix(value.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
        };
        assert_eq!(THRESHOLD.to_bits(), bits(receipt.get("thresholdBits").unwrap()));
        let helpers = receipt.get("helpers").unwrap();
        let (mut compared, mut flushed, mut nan_kept) = (0usize, 0usize, 0usize);
        for name in ["Float", "Color", "Vector"] {
            let helper = helpers.get(name).unwrap();
            let k = helper.get("components").unwrap().as_f64().unwrap() as usize;
            let rows = helper.get("rows").unwrap().as_array().unwrap();
            assert!(!rows.is_empty(), "{name}");
            for row in rows {
                let row = row.as_array().unwrap();
                assert_eq!(row.len(), 2 * k, "{name}");
                let input: Vec<u32> = row[..k].iter().map(bits).collect();
                let native: Vec<u32> = row[k..].iter().map(bits).collect();
                for (&x, &y) in input.iter().zip(&native) {
                    assert_eq!(clamp_zero(f32::from_bits(x)).to_bits(), y, "{name} input {x:#010x}");
                    compared += 1;
                    flushed += usize::from(x != y);
                    nan_kept += usize::from(f32::from_bits(x).is_nan() && x == y);
                }
                if k == 4 {
                    let v = [0, 1, 2, 3].map(|i| f32::from_bits(input[i]));
                    assert_eq!(clamp_zero4(v).map(f32::to_bits).to_vec(), native, "{name}");
                }
            }
        }
        // Positive arms: the receipt exercises the flush and the NaN pass-through,
        // not only values that are forwarded unchanged.
        assert!(flushed > 0 && flushed < compared, "{flushed} of {compared}");
        assert!(nan_kept > 0);
    }
}
