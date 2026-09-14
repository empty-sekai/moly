//! 入口。native 开窗；wasm 是一次 `App::run()`，业务导出面为 0。
//!
//! wasm 下本 bin 不是入口：rustc 会把 bin 的 main 放进 wasm 的 start
//! section，wasm-bindgen 把它搬成 `__wbindgen_start` 在实例化时自动执行，
//! 页面再显式调用 lib 的 `start` 导出就成了双跑（冒烟实证：第二次
//! App 构建撞 winit 的全局限一次 event loop，启动即 panic）。所以 wasm 的
//! main 不执行 run，唯一入口是 lib 的 `start` 导出。
//!
//! 但也不能写成空 main：链接器会把「没有任何引用的」wasm-bindgen 运行时
//! 内建导出 GC 掉，wasm-bindgen 随即报 clone_ref intrinsic 缺失。这里引用
//! run 而不调用，整张依赖图保持可达。

#[cfg(target_arch = "wasm32")]
fn main() {
    let _run_referenced_not_called: fn() = moly_app::run;
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    moly_app::run();
}
