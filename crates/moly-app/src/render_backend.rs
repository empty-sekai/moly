//! wasm 渲染后端选择：WebGPU 优先、WebGL2 兜底，运行时定。
//!
//! 0.18 的后端选择是编译期三选一（bevy_render `settings.rs`：`webgl` 且非
//! `webgpu` → GL；`webgpu` → 仅 BROWSER_WEBGPU），workspace 两个特性都开后
//! 默认值落在 BROWSER_WEBGPU，且 adapter 请求失败没有重试
//! （bevy_render `renderer/mod.rs` 直接 expect panic）——所以必须在起 app
//! 前探测、把后端钉死，否则无 WebGPU 的浏览器启动即炸。

use bevy::render::settings::{Backends, WgpuLimits, WgpuSettings};

/// 探测 `navigator.gpu`，按结果给 `RenderPlugin.render_creation` 用的设置。
///
/// 取 Rust 侧直查（读 `navigator.gpu` 属性、`is_undefined()` 判在不在）
/// 不取 web 壳注入全局标志：拒绝点与消费点同在入口 crate，壳保持
/// 只管 bootstrap；全局标志要跨语言传、且在页面脚本层可被改动，直查每次
/// 加载拿浏览器真值。代价只是 wasm 多编一组属性读。
///
/// 属性读取走 js-sys 的 `Reflect.get`，不用 web-sys 的 `Navigator::gpu()`
/// 绑定——后者还挂在 `web_sys_unstable_apis` cfg 门后，为它给树加
/// rustflags 会改共享构建缓存的指纹面（全体工作树连带复编）。Reflect
/// 与真实属性读同语义（走 getter 链；属性缺席返回 undefined），且
/// js-sys 已是 web-sys 的既有依赖，不新增锁内包。
///
/// 两臂 limits 都显式给：两特性同开时 `WgpuSettings::default()` 的 limits
/// 是全量默认值，GL 臂若靠默认值会丢掉 webgl2 的保守上限。WebGPU 臂吃全
/// 量默认（等价 webgpu 单特性时代的形状）。
///
/// 探测只认 `navigator.gpu` 的在场性：暴露了该属性但 `requestAdapter` 失败
/// 的环境仍会在 Bevy 的 adapter 请求上 panic——0.18 无重试，异步先探
/// adapter 要改导出面为 async，超出本单（见 REPORT 挂账）。
pub(crate) fn settings() -> WgpuSettings {
    let webgpu = web_sys::window()
        .map(|window| has_navigator_gpu(window.navigator().as_ref()))
        .unwrap_or(false);
    if webgpu {
        WgpuSettings {
            backends: Some(Backends::BROWSER_WEBGPU),
            ..Default::default()
        }
    } else {
        // 照 `webgl` 单特性时代 `WgpuSettings::default()` 给 GL 的形状。
        WgpuSettings {
            backends: Some(Backends::GL),
            limits: WgpuLimits::downlevel_webgl2_defaults(),
            ..Default::default()
        }
    }
}

/// `navigator.gpu` 在不在（读属性触发 getter 链；不存在/读失败都算不在）。
fn has_navigator_gpu(navigator: &wasm_bindgen::JsValue) -> bool {
    js_sys::Reflect::get(navigator, &"gpu".into())
        .map(|value| !value.is_undefined())
        .unwrap_or(false)
}
