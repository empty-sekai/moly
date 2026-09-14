//! 入口库：native 与 wasm 共用一条启动路径。wasm 业务导出面为 0——
//! 仅 `start` 一个导出；native 走 `main.rs`。

pub mod asset_source;
pub mod site_request;

#[cfg(target_arch = "wasm32")]
mod render_backend;

#[cfg(target_arch = "wasm32")]
use bevy::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

/// 起一次 app。资产源与站点选择在这里解析、经 `moly_game::app` 装上——
/// 消费方读资源，不要再各自去读 env 或 URL（拒绝点全树只在 `asset_source`
/// 与 `site_request` 两处入口）。
pub fn run() {
    let source = match asset_source::resolve() {
        Ok(source) => source,
        Err(message) => fail_loud(&message),
    };
    let site = match site_request::resolve() {
        Ok(site) => site,
        Err(message) => fail_loud(&message),
    };
    // 渲染后端只在 wasm 分支选（取舍在 `render_backend`）；native 展开后
    // 与双后端升级前逐行一致。
    #[cfg(target_arch = "wasm32")]
    let mut app = moly_game::app(source, site, render_backend::settings());
    #[cfg(not(target_arch = "wasm32"))]
    let mut app = moly_game::app(source, site);
    #[cfg(target_arch = "wasm32")]
    attach_canvas(&mut app);
    app.run();
}

#[cfg(not(target_arch = "wasm32"))]
fn fail_loud(message: &str) -> ! {
    eprintln!("moly-app: {message}");
    std::process::exit(1);
}

#[cfg(target_arch = "wasm32")]
fn fail_loud(message: &str) -> ! {
    panic!("moly-app: {message}");
}

/// 认领 `index.html` 里的 `#app-canvas`。主窗实体由 DefaultPlugins 在
/// build 时生成，但裸窗到 runner 的 `resumed()` 才创建、届时才读 `canvas`
/// 选择器（找不到元素会 panic）——所以在 `run()` 之前改主窗仍然生效，
/// 不必为 canvas 给 moly-game 开配置口。
#[cfg(target_arch = "wasm32")]
fn attach_canvas(app: &mut App) {
    // 0.18 的 prelude 不含这个标记组件，真身在 bevy::window。
    use bevy::window::PrimaryWindow;
    let mut windows = app
        .world_mut()
        .query_filtered::<&mut Window, With<PrimaryWindow>>();
    // DefaultPlugins 只建一个主窗；多了少了都是组装层坏了，值得在这里炸。
    let mut window = windows
        .single_mut(app.world_mut())
        .expect("exactly one primary window");
    window.canvas = Some("#app-canvas".into());
    // 画布跟随父元素尺寸（body 定为视口高）；父元素不能反过来随子元素
    // 撑开，否则每次 resize 反馈放大一圈。
    window.fit_canvas_to_parent = true;
}

/// wasm 侧唯一导出：起一次 app。
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start() {
    console_error_panic_hook::set_once();
    run();
}
