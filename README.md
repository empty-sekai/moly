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
> **本仓库已归档。** moly 的开发随 0.0.7 结束：不再增加功能，也不再修复问题，Issues 与 Pull requests 均已关闭。
> 代码按许可证可自由 fork 与修改。

## 完成情况

按玩法域分组，括号内是实现它的 crate（各 crate 的职责见 [项目结构](docs/architecture.md)）。
行为与原版客户端兼容，结构按本仓需要组织；如何重新实现、兼容的范围与验证方式见 [方法](docs/METHOD.md)。

- **场景与移动**（`moly-world`、`moly-features`）：进入 MySekai 的入场流程与横幅；大炮与门在站点之间移动，门可交互并切换场景，地图选择界面；室内房间（室内壳与墙面剔除），多个站点常驻；玩家角色、脚步声与表情。
- **采集**（`moly-harvest`）：采集地图、障碍物、工具与体力、采集粒子、掉落与拾取、持有上限；采集 HUD（当前工具、动作图标、缺少工具时的提示气泡、体力条）；获得提示与获得物对话框；长按蓄力的大锤一键清图，见 [批量采集](docs/bulk-harvest.md)。
- **配送**（`moly-delivery`）：生日派对的配送场地与完整配送流程，掉落、荣誉奖励与总奖励，配送界面。
- **制作与转换**（`moly-features`、`moly-convert`）：制作界面与制作结果对话框，制作与图纸的服务端回复；转换机的槽位、转换界面与转换的服务端规则。
- **家具编辑**（`moly-fixture-edit`）：背包界面与从背包摆放家具；地面家具、道路／地砖与环境的布局编辑，库存与已放置对象随同一份布局事务更新。
- **任务与拍照**（`moly-mission`、`moly-photo`）：任务界面与奖励领取，拍照模式与保存照片。
- **内容库**（`moly-content-library`）：浏览对话、家具与 NPC 活动；独立模式下搭起临时布景，在场地上布置并播放所选内容，结束后复原站点。
- **NPC**（`moly-world`）：寻路、配对后的对话、头顶 tweet 气泡、待机动作决策、自主与家具互动；按原版可行走格选取目标位置；日服 7.0.0 的分层 NPC 状态机，以及不同团体角色之间的对话。
- **天气与粒子**（`moly-law`、`moly-render`）：天气现象的完整链路（宿主生命周期、子发射器、曲线、雨滴面片等）；天气页按已解锁内容选择环境；界面粒子效果。
- **画面**（`moly-render`）：主光源阴影的范围、分辨率、过滤方式（`hard`、`tent`、`pcf`、`pcss`）与强度由 `moly.toml` 的 `[graphics]` 设置，运行中可经 `moly.graphics.set` 修改，见 [配置](docs/configuration.md)。
- **音频与演出**（`moly-audio`、`moly-cutscene`）：BGM、SE、语音与环境音，唱片 BGM 与 BGM 选择界面；生日派对的过场动画、派对对话与生日语音、收场与站点复原。
- **序列播放**（`moly-sequence`、`moly-sequence-player`）：用 JSON 序列文档按时间轴驱动相机、角色走位与动作、表情、台词、字幕、声音与画面效果，可暂停、跳转、调速，并把暂停时的相机写回文档，见 [序列](docs/sequence.md)。
- **布娃娃**（`moly-ragdoll`）：角色脱离动画以刚体倒下，静止后起身并回到原来的行为，见 [布娃娃](docs/ragdoll.md)。
- **界面与设置**（`moly-ui`）：菜单、信息页与设置、学习现象对话框、界面切换管理。
- **导演脚本与插件**（`moly-act`、`moly-devtools`、`moly-plugin`）：用 JSON 脚本或 Luau 插件编排玩家、角色、相机、天气与服务端模拟面板，两者经同一份动作注册表执行；插件有清单、依赖与冲突声明、独立沙箱与每帧预算，默认不启用，见 [插件](docs/plugins.md)。
- **录制**（`moly-devtools`）：把窗口画面与游戏混音录成 MP4，录制时游戏时钟按固定帧率前进，见 [录制](docs/recording.md)。
- **MCP 服务器**（`tools/mcp`）：供 AI 客户端启动并驱动原生或浏览器实例，读取状态、运行脚本、截取画面，见 [MCP 服务器](docs/mcp.md)。
- **服务端与启动器**（`moly-server`、`moly-launcher`）：服务端作为独立进程保存玩家文档并应答游戏的请求；启动器选择、下载并校验资源版本，启动服务端与游戏，检查并更新程序版本，各目录可更改并迁移已有内容。
- **数据与文本**（`moly-masters`、`moly-text`）：master 以日服为主体，按区服合并其他区服的独占行，运行时从使用者配置的 master 仓库读取，镜像作为后备，见 [主表与区服](docs/masters.md)；网页版选择中文时可用匹配的国服文本替换日服显示文本；使用开源字体 Resource Han Rounded。
- **网页版**（`moly-stage`、`web/`）：嵌入舞台与完整游戏，各含 WebGPU 与 WebGL2 构建；网页版只运行原版，不加载插件。

## 已知限制

以下原版内容没有实现：

- 访客流程；采集工具选择器；墙面与叠放编辑；房间升级。
- 背包的排序、筛选、分类页签、分解与 3D 预览；商店与兑换；换装。
- 软深度、部分光效与轨道粒子等渲染细节；各界面的部分附属对话框。
- 只启用日服；其他区服的独占内容与界面语言切换没有实现。

平台：

- Android 不受支持：程序包在真机上显示最初几帧后崩溃。`crates/app/moly-android` 与 [构建说明](docs/building.md) 保留原样，仅供参考。
- Linux 与 macOS 程序包由发布工作流编译并核对版本，没有在真机上验证。

## 快速开始

### 资源

运行所需的模型、贴图、动作和音频由使用者自行提供，不随仓库分发。
项目代码与游戏资源的权利归属相互独立。

### Windows 发布包

1. 从 [Releases](https://github.com/empty-sekai/moly/releases) 下载 Windows 程序包并解压。
   包内有游戏 `moly-app.exe`、服务端 `moly-server.exe` 与启动器 `moly-launcher.exe`。
2. 运行启动器，配置**你自己的**资源地址，选择资源版本并下载。下载完成后逐个核对 SHA-256，中断后可以继续。
3. 点击「开始游戏」。启动器先启动服务端，再启动游戏。

启动器的数据目录、资源目录与迁移规则见 [启动器说明](crates/app/moly-launcher/README.md)。

### Linux 与 macOS 发布包

Releases 中另有 Linux x64（`x86_64-unknown-linux-gnu`）与 macOS Apple Silicon
（`aarch64-apple-darwin`）程序包，内容与用法同 Windows 包。这两个平台的程序包由发布工作流编译并核对版本，
没有在真机上验证。

- Linux：需要系统提供 ALSA、udev、Vulkan 驱动与 WebKitGTK 4.1（启动器窗口）。
- macOS：程序未签名，解压后先执行 `xattr -dr com.apple.quarantine <解压目录>` 再运行启动器。

### 在自己的另一台机器上运行服务端

每个平台另有一个只含服务端的程序包 `moly-server-<版本>-<平台>`，用于在自己的另一台机器上运行服务端，供自己的 moly 客户端连接。
它与游戏读取同一份资源，监听地址、资源目录与存档目录在配置文件 `moly.toml` 中设置，也可用对应的环境变量，见
[服务端](docs/server.md) 与 [配置](docs/configuration.md#server)。

## 文档

- [从源码构建](docs/building.md)：环境要求，原生程序与浏览器版的构建方法（Android 的构建方法仅供参考）。
- [服务端](docs/server.md)：服务端的协议、监听地址、控制端口，以及在自己的另一台机器上运行的方法。
- [配置](docs/configuration.md)：配置文件 `moly.toml`、覆盖其键的环境变量与浏览器版的页面参数。
- [玩家数据导入](docs/player-data.md)：开发窗口中导入玩家数据的步骤、备份与资源要求。
- [主表与区服](docs/masters.md)：主表的读取来源、合并规则与启用的区服。
- [MCP 服务器](docs/mcp.md)：供 AI 客户端启动并驱动游戏实例的 MCP 服务器。
- [插件](docs/plugins.md)：Luau 插件的接口、全部导演步骤、可观察的段落、事件与环境变量开关。
- [序列](docs/sequence.md)：序列文档的格式与播放、暂停、跳转和取景的方法。
- [布娃娃](docs/ragdoll.md)：让角色倒下与起身的方法、参数与动作步骤。
- [录制](docs/recording.md)：把窗口画面与游戏混音录成 MP4 的方法与参数。
- [项目结构](docs/architecture.md)：各目录与 crate 的职责。
- [方法](docs/METHOD.md)：如何在理解原版客户端后重新实现，哪些行为与原版逐值兼容、哪些由本仓自定，以及如何验证。
- [开发](docs/development.md)：测试与检查、提交规范、持续集成与版本发布。
- [发布包说明](docs/release-package.md)：下载发布包后的内容、运行与升级方法。

## 许可

代码采用 [AGPL-3.0-only](LICENSE)。第三方内容按各自许可证分发，见
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。代码许可证不授予游戏资源的使用或分发权。
