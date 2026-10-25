# moly

中文 | [English](README.en.md)

[![Static checks](https://github.com/empty-sekai/moly/actions/workflows/ci.yml/badge.svg)](https://github.com/empty-sekai/moly/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/empty-sekai/moly)](https://github.com/empty-sekai/moly/releases)
[![License: AGPL-3.0-only](https://img.shields.io/badge/license-AGPL--3.0--only-blue.svg)](LICENSE)

MYSEKAI 的还原项目，使用 Rust 与 [Bevy](https://bevyengine.org/) 构建，可作为原生程序运行，
也可在浏览器中运行（WebAssembly，WebGPU 或 WebGL2）。

项目还原场景探索、角色行为、家具互动与天气表现。客户端行为按原版还原；由服务端决定的数值
由独立的服务端进程提供，集中在一个可编辑的模拟面板中。

> **项目仍在开发中。** 当前版本的范围与之后的计划见 [路线图](ROADMAP.md)，
> 已知问题见 [Issues](https://github.com/empty-sekai/moly/issues)。

## 功能

- **场景与移动**：入场流程，大炮与门在站点之间移动，室内房间，玩家角色、脚步声与表情。
- **采集**：障碍物、采集粒子、持有上限、采集 HUD 与获得提示。
- **配送、制作与摆放**：配送流程，制作界面，背包，从背包摆放家具，地面家具的布局编辑。
- **NPC**：寻路、配对后的对话、头顶 tweet 气泡、待机动作决策、自主与家具互动。
- **天气**：天气现象的完整链路。
- **音频与演出**：BGM、SE、语音与环境音，唱片 BGM，生日派对。
- **导演脚本**：用 JSON 脚本编排玩家、角色、相机、天气与服务端模拟面板。
- **MCP 服务器**：供 AI 客户端启动并驱动原生或浏览器实例。

逐项范围见 [路线图](ROADMAP.md)。

## 快速开始

### 资源

运行所需的模型、贴图、动作和音频由使用者自行提供，不随仓库分发。
项目代码与游戏资源的权利归属相互独立。

### Windows 发布包

1. 从 [Releases](https://github.com/empty-sekai/moly/releases) 下载 Windows 程序包并解压。
   包内有游戏 `moly-app.exe`、服务端 `moly-server.exe` 与启动器 `moly-launcher.exe`。
2. 运行启动器，选择资源版本并下载。下载完成后逐个核对 SHA-256，中断后可以继续。
3. 点击「开始游戏」。启动器先启动服务端，再启动游戏。

启动器的数据目录、资源目录与迁移规则见 [启动器说明](crates/app/moly-launcher/README.md)。

### Linux 与 macOS 发布包

Releases 中另有 Linux x64（`x86_64-unknown-linux-gnu`）与 macOS Apple Silicon
（`aarch64-apple-darwin`）程序包，内容与用法同 Windows 包。这两个平台的程序包由发布工作流编译并核对版本，
尚未经过实机验证，使用中遇到的问题请提交到 [Issues](https://github.com/empty-sekai/moly/issues)。

- Linux：需要系统提供 ALSA、udev、Vulkan 驱动与 WebKitGTK 4.1（启动器窗口）。
- macOS：程序未签名，解压后先执行 `xattr -dr com.apple.quarantine <解压目录>` 再运行启动器。

### 单独运行服务端

每个平台另有一个只含服务端的程序包 `moly-server-<版本>-<平台>`，用于在另一台机器上单独运行服务端。
它与游戏读取同一份资源，监听地址、资源目录与存档目录由环境变量指定，见
[服务端](docs/server.md) 与 [环境变量](docs/configuration.md#environment-variables)。

## 文档

- [从源码构建](docs/building.md)：环境要求，原生程序、浏览器版与 Android 的构建方法。
- [服务端](docs/server.md)：服务端的监听地址、控制端口与单独运行方法。
- [配置](docs/configuration.md)：环境变量与浏览器版的页面参数。
- [玩家数据导入](docs/player-data.md)：开发窗口中导入玩家数据的步骤、备份与资源要求。
- [主表与区服](docs/masters.md)：主表的读取来源、合并规则与启用的区服。
- [MCP 服务器](docs/mcp.md)：供 AI 客户端启动并驱动游戏实例的 MCP 服务器。
- [项目结构](docs/architecture.md)：各目录与 crate 的职责。
- [开发](docs/development.md)：测试与检查、提交规范、持续集成与版本发布。
- [发布包说明](docs/release-package.md)：下载发布包后的内容、运行与升级方法。

## 参与贡献

贡献代码或文档前，请阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可

代码采用 [AGPL-3.0-only](LICENSE)。第三方内容按各自许可证分发，见
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。代码许可证不授予游戏资源的使用或分发权。
