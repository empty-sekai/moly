# moly

MYSEKAI的还原项目，使用 Rust 与 [Bevy](https://bevyengine.org/) 构建，
支持原生程序和浏览器（WebAssembly）。

目标是还原场景探索、角色行为、家具互动与天气表现。**项目仍在开发中**

![moly 中的角色、家具与头顶气泡](screenshots/readme.png)

## 资源

运行所需的模型、贴图、动作和音频由使用者自行提供，不随仓库分发。
项目代码与游戏资源的权利归属相互独立。

## 代码结构

| 目录 | 职责 |
|---|---|
| `crates/moly-law` | 不依赖引擎的行为与计算逻辑 |
| `crates/moly-assets` | 资源目录、资源包与数据加载 |
| `crates/moly-game` | 场景、交互、UI 与渲染 |
| `crates/moly-app` | 原生与浏览器入口 |
| `web` | 浏览器页面与构建工具 |
| `tools` | 仓库边界与渲染顺序检查 |
| `screenshots/readme.png` | README 展示图 |

## 许可

代码采用 [AGPL-3.0-only](LICENSE)。第三方内容按各自许可证分发，见
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。代码许可证不授予游戏资源的使用或分发权。
