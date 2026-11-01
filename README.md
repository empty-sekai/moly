# moly

中文 | [English](README.en.md)

[![Static checks](https://github.com/empty-sekai/moly/actions/workflows/ci.yml/badge.svg)](https://github.com/empty-sekai/moly/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/empty-sekai/moly)](https://github.com/empty-sekai/moly/releases)
[![License: AGPL-3.0-only](https://img.shields.io/badge/license-AGPL--3.0--only-blue.svg)](LICENSE)

与 MYSEKAI 兼容的开源游戏运行时，使用 Rust 与 [Bevy](https://bevyengine.org/) 构建，
可作为原生程序运行，也可在浏览器中运行（WebAssembly，WebGPU 或 WebGL2）。

它加载使用者自行提供的 MYSEKAI 资源，以与原版客户端兼容的行为运行场景探索、角色行为、
家具互动与天气表现。由服务端决定的数值由随附的服务端进程提供，集中在一个可编辑的模拟面板中。

> [!IMPORTANT]
> **使用前必读：用途与范围**
>
> **moly 仅供学习研究与本地单机使用，目标不是替代原版游戏的客户端或服务器。**
>
> 本仓库只包含 moly 自己的程序代码。对游戏内容，它只做一件事：加载使用者自己提供的资源，并以与原版客户端兼容的方式运行。
>
> - **没有游戏资源**：不含模型、贴图、动作、音频、主表等任何游戏资源，也不含由它们派生的数据；不内置任何资源、主表或玩家数据的地址，这些地址都由使用者自行配置。
> - **没有提取工具**：程序只读取已经整理好的资源目录，不解包、不解密游戏文件。
> - **没有原版客户端代码**：不含任何形式的原版客户端源代码或反编译代码。行为在理解原版客户端之后重新实现，见 [方法](docs/METHOD.md)。
> - **服务端是另一套实现**：随附的服务端是 moly 自己的实现，与原版游戏服务端完全不同，没有任何兼容的接口或实现。它只有一个端点，使用 moly 自己的二进制协议，每个请求都先声明 moly 的协议版本；原版游戏客户端无法与它建立任何实质性的连接，它只服务于 moly 的客户端。
> - **只供本地单机**：不连接原版游戏的服务器；服务端由使用者自己运行，只服务于自己的游玩，没有账号、登录或支付，也没有多人联机。

> [!NOTE]
> 项目仍在开发中。当前版本的范围与之后的计划见 [路线图](ROADMAP.md)，已知问题见 [Issues](https://github.com/empty-sekai/moly/issues)。

## 功能

按玩法域分组，括号内是实现它的 crate（各 crate 的职责见 [项目结构](docs/architecture.md)）：

- **场景与移动**（`moly-world`、`moly-features`）：入场流程，大炮与门在站点之间移动，室内房间，玩家角色、脚步声与表情。
- **采集**（`moly-harvest`）：采集地图、障碍物、工具与体力、采集粒子、掉落与拾取、持有上限、采集 HUD、批量采集与获得提示。
- **配送**（`moly-delivery`）：生日派对的配送场地、配送流程、掉落、荣誉奖励与配送界面。
- **制作与转换**（`moly-features`、`moly-convert`）：制作界面与背包，转换机的槽位与转换界面。
- **家具编辑**（`moly-fixture-edit`）：从背包摆放家具，地面家具、道路与环境的布局编辑。
- **任务与拍照**（`moly-mission`、`moly-photo`）：任务界面与奖励领取，拍照模式与保存照片。
- **内容库**（`moly-content-library`）：浏览对话、家具与 NPC 活动，在场地上布置并播放所选内容。
- **NPC**（`moly-world`）：寻路、配对后的对话、头顶 tweet 气泡、待机动作决策、自主与家具互动。
- **天气与粒子**（`moly-law`、`moly-render`）：天气现象的完整链路。
- **音频与演出**（`moly-audio`、`moly-cutscene`）：BGM、SE、语音与环境音，唱片 BGM，生日派对与过场演出。
- **导演脚本与插件**（`moly-devtools`、`moly-plugin`）：用 JSON 脚本或 Luau 插件编排玩家、角色、相机、天气与服务端模拟面板。
- **MCP 服务器**（`tools/mcp`）：供 AI 客户端启动并驱动原生或浏览器实例。

行为与原版客户端兼容，结构按本仓需要组织：如何重新实现、兼容的范围与验证方式见 [方法](docs/METHOD.md)，逐项范围见 [路线图](ROADMAP.md)。

## 快速开始

### 资源

运行所需的模型、贴图、动作和音频由使用者自行提供，不随仓库分发。
项目代码与游戏资源的权利归属相互独立。

### Windows 发布包

1. 从 [Releases](https://github.com/empty-sekai/moly/releases) 下载 Windows 程序包并解压。
   包内有游戏 `moly-app.exe`、服务端 `moly-server.exe` 与启动器 `moly-launcher.exe`。
2. 运行启动器，提供自己的资源：在设置中把资源目录指向自己准备好的资源，或填写自己的资源地址后从中下载一个版本（下载后逐个核对 SHA-256，中断后可以继续）。本项目不提供任何资源，也不提供资源地址。
3. 点击「开始游戏」。启动器先启动服务端，再启动游戏。

启动器的数据目录、资源目录与迁移规则见 [启动器说明](crates/app/moly-launcher/README.md)。

### Linux 与 macOS 发布包

Releases 中另有 Linux x64（`x86_64-unknown-linux-gnu`）与 macOS Apple Silicon
（`aarch64-apple-darwin`）程序包，内容与用法同 Windows 包。这两个平台的程序包由发布工作流编译并核对版本，
尚未经过实机验证，使用中遇到的问题请提交到 [Issues](https://github.com/empty-sekai/moly/issues)。

- Linux：需要系统提供 ALSA、udev、Vulkan 驱动与 WebKitGTK 4.1（启动器窗口）。
- macOS：程序未签名，解压后先执行 `xattr -dr com.apple.quarantine <解压目录>` 再运行启动器。

### 在自己的另一台机器上运行服务端

每个平台另有一个只含服务端的程序包 `moly-server-<版本>-<平台>`，用于在自己的另一台机器上运行服务端，供自己的 moly 客户端连接。
它与游戏读取同一份资源，监听地址、资源目录与存档目录在配置文件 `moly.toml` 中设置，也可用对应的环境变量，见
[服务端](docs/server.md) 与 [配置](docs/configuration.md#server)。

## 文档

- [从源码构建](docs/building.md)：环境要求，原生程序、浏览器版与 Android 的构建方法。
- [服务端](docs/server.md)：服务端的协议、监听地址、控制端口，以及在自己的另一台机器上运行的方法。
- [配置](docs/configuration.md)：配置文件 `moly.toml`、覆盖其键的环境变量与浏览器版的页面参数。
- [玩家数据导入](docs/player-data.md)：开发窗口中导入玩家数据的步骤、备份与资源要求。
- [主表与区服](docs/masters.md)：主表的读取来源、合并规则与启用的区服。
- [MCP 服务器](docs/mcp.md)：供 AI 客户端启动并驱动游戏实例的 MCP 服务器。
- [插件](docs/plugins.md)：Luau 插件的接口、全部导演步骤、可观察的段落、事件与环境变量开关。
- [项目结构](docs/architecture.md)：各目录与 crate 的职责。
- [方法](docs/METHOD.md)：如何在理解原版客户端后重新实现，哪些行为与原版逐值兼容、哪些由本仓自定，以及如何验证。
- [开发](docs/development.md)：测试与检查、提交规范、持续集成与版本发布。
- [发布包说明](docs/release-package.md)：下载发布包后的内容、运行与升级方法。

## 参与贡献

贡献代码或文档前，请阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可

代码采用 [AGPL-3.0-only](LICENSE)。第三方内容按各自许可证分发，见
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。代码许可证不授予游戏资源的使用或分发权。
