# Third-party notices

本仓自身代码按 [AGPL-3.0-only](LICENSE) 授权。第三方内容保留各自的版权与
许可证，不因随本项目分发而改用 AGPL。

## Resource Han Rounded（字体子集）

`crates/moly-game/assets/font/ResourceHanRoundedSC-Medium.subset.ttf` 是
Resource Han Rounded SC Medium 的一个字形子集，运行时用于烘焙文字图集。
按 SIL Open Font License 1.1 授权，许可全文及版权声明见
[OFL-1.1.txt](crates/moly-game/assets/font/OFL-1.1.txt)
（© 2018–2022 Cyano Hao；部分 © 2014–2021 Adobe）。

子集化是 OFL 意义上的修改版本，文件名以 `.subset` 后缀标注。字体名不含
Adobe 的保留字体名（Reserved Font Name "Source"）。

## Arm Optimized Routines（单精度 powf 移植）

`crates/moly-law/src/powf.rs` 是 Arm Optimized Routines 中单精度 `powf`
（`math/powf.c`，及其数据表 `math/powf_log2_data.c`、`math/exp2f_data.c`）
的 Rust 移植，逐位复现 Android 10 起系统 libm 所用的那份实现。上游按
MIT 许可证授权，许可全文如下：

```text
MIT License

Copyright (c) 1999-2018, Arm Limited.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Rust 直接依赖

下表覆盖工作区直接使用的第三方 Rust 依赖，包括仅浏览器目标使用的依赖。
确切版本、来源与传递依赖由 [Cargo.lock](Cargo.lock) 固定。

| 依赖 | 许可证 | 用途 |
|---|---|---|
| async-lock | MIT OR Apache-2.0 | 异步加载中的同步与互斥 |
| bevy | MIT OR Apache-2.0 | 引擎、场景、渲染与输入 |
| console_error_panic_hook | MIT OR Apache-2.0 | 将 wasm panic 信息写入浏览器控制台 |
| flate2 | MIT OR Apache-2.0 | 压缩资源解码 |
| futures-lite | MIT OR Apache-2.0 | 异步任务与 I/O 辅助 |
| gltf | MIT OR Apache-2.0 | glTF 模型数据解析 |
| js-sys | MIT OR Apache-2.0 | JavaScript 内建对象绑定 |
| parry3d | Apache-2.0 | 源凸碰撞网格的三维凸包构建 |
| serde | MIT OR Apache-2.0 | 数据序列化与反序列化 |
| serde_json | MIT OR Apache-2.0 | JSON 清单与资源数据解析 |
| sha2 | MIT OR Apache-2.0 | 资源包 SHA-256 完整性校验 |
| swash | MIT OR Apache-2.0 | 字形光栅化、塑形与度量 |
| wasm-bindgen | MIT OR Apache-2.0 | Rust 与 JavaScript 互操作 |
| web-sys | MIT OR Apache-2.0 | 浏览器 Web API 绑定 |

`MIT OR Apache-2.0` 表示可依相应许可条款选择其一。传递依赖各按自身许可证
分发，不应从本表推定为同一许可。

本文件是第三方内容索引，不替代各依赖的版权声明和许可全文；这些文件随对应
crates.io 源码包提供。分发构建产物时，应一并保留适用的第三方声明和许可文件。

## Arm Optimized Routines（logf、exp、cosf 移植）

`crates/moly-law/src/particle/device_libm.rs` 中的 `logf`、`exp`、`cosf`
是 Arm Optimized Routines（`math/logf.c`、`math/logf_data.c`、`math/exp.c`、
`math/exp_data.c`、`math/cosf.c`、`math/sincosf.h`、`math/sincosf_data.c`）
的 Rust 移植，逐位复现 Android 10 起系统 libm 所用的那份实现（加权曲线段的
Bezier 时间解调用它们）。上游按 MIT 许可证授权，许可全文如下：

```text
MIT License

Copyright (c) 1999-2019, Arm Limited.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## FreeBSD msun（atan2f、atanf 移植）

`crates/moly-law/src/particle/device_libm.rs` 中的 `atan2f` 与 `atanf` 是
FreeBSD msun（`e_atan2f.c`、`s_atanf.c`，bionic libm 沿用）的 Rust 移植。
上游声明如下：

```text
Conversion to float by Ian Lance Taylor, Cygnus Support, ian@cygnus.com.

====================================================
Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.

Developed at SunPro, a Sun Microsystems, Inc. business.
Permission to use, copy, modify, and distribute this
software is freely granted, provided that this notice
is preserved.
====================================================
```
