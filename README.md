# moly

[![Static checks](https://github.com/empty-sekai/moly/actions/workflows/ci.yml/badge.svg)](https://github.com/empty-sekai/moly/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/empty-sekai/moly)](https://github.com/empty-sekai/moly/releases)
[![License: AGPL-3.0-only](https://img.shields.io/badge/license-AGPL--3.0--only-blue.svg)](LICENSE)

MYSEKAI 的还原项目，使用 Rust 与 [Bevy](https://bevyengine.org/) 构建，可作为原生程序运行，
也可在浏览器中运行（WebAssembly，WebGPU 或 WebGL2）。

项目还原场景探索、角色行为、家具互动与天气表现。客户端行为按原版还原；由服务端决定的数值
由独立的服务端进程提供，集中在一个可编辑的模拟面板中。

> **项目仍在开发中。** 当前版本的范围与之后的计划见 [路线图](ROADMAP.md)，
> 已知问题见 [Issues](https://github.com/empty-sekai/moly/issues)。

![moly 中的角色、家具与头顶气泡](screenshots/readme.png)

## 目录

- [功能](#功能)
- [快速开始](#快速开始)
- [从源码构建](#从源码构建)
- [配置](#配置)
- [玩家数据导入](#玩家数据导入)
- [主表与区服](#主表与区服)
- [MCP 服务器](#mcp-服务器)
- [项目结构](#项目结构)
- [开发](#开发)
- [许可](#许可)

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
3. 点击启动。启动器先启动服务端，再启动游戏。

启动器的数据目录、资源目录与迁移规则见 [启动器说明](crates/app/moly-launcher/README.md)。

### 浏览器

浏览器版部署在 <https://emptysekai.dev>，缺省连接公共服务端 `https://server.emptysekai.dev`。

## 从源码构建

### 环境要求

- Rust 稳定版。
- Node.js 22（浏览器版构建、开发服务器与 MCP 服务器）。
- 已提取的资源目录，由 `MOLY_ASSET_ROOT` 指定；未设置时使用启动器安装的资源。

构建默认写入仓库的 `target/`。需要共享缓存时设置 `CARGO_TARGET_DIR`。

### 原生程序

原生版由两个进程组成：服务端 `moly-server`（crate `moly-server-host`）保存玩家文档并应答请求，
游戏 `moly-app` 通过 HTTP 向它发送二进制编码的请求。先启动服务端，再启动游戏：

```sh
cargo run --release -p moly-server-host
cargo run --release -p moly-app
```

- 服务端缺省监听 `127.0.0.1:15800`，控制端口 `127.0.0.1:15801`，就绪时打印一行地址；
  端口被占用时以状态 1 退出。
- 游戏按 `MOLY_SERVER_URL` 连接服务端，连接不上时给出具名的提示，不会退回进程内的替代实现。
- 服务端与游戏读取同样的资源根与主表变量，两者读到的主表一致。

原生程序缺省运行与网页相同的游戏：从家园进入，家园等级取自服务端文档。
设置 `MOLY_DEV=1` 启动开发窗口：它读取 [配置](#配置) 中标为“开发窗口”的变量，
服务端文档只保存在内存中，并提供玩家数据导入等开发工具。

| 按键 | 作用 |
|---|---|
| **F10** | 画面与音量设置 |
| **F9** | 控制面板 |

### 浏览器版

需要 `wasm32-unknown-unknown` target 与和 `Cargo.lock` 一致的 wasm-bindgen CLI：

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.127 --locked
node web/build-wasm.mjs
node web/serve.mjs
```

构建脚本分别生成 `web/pkg/webgpu/` 与 `web/pkg/webgl2/`，页面只下载所选后端的模块；
WebGPU 不可用时选择 WebGL2。开发服务器只监听回环地址，并提供嵌入舞台
`/stage.html?assets=/assets/`。

- 构建产物、后端选择、存档写入权与资源加载限制：[web/RUNTIME.md](web/RUNTIME.md)
- 在已有站点中嵌入舞台：[web/EMBEDDING.md](web/EMBEDDING.md)
- 资源来源：[web/RESOURCE-SOURCES.md](web/RESOURCE-SOURCES.md)
- 浏览器检查：[web/TESTING.md](web/TESTING.md)

### Android

> Android 版目前在真机上开始出帧后不久崩溃，尚不可用，见
> [#2](https://github.com/empty-sekai/moly/issues/2)。

`android/` 是 Android 工程。原生库（crate `moly-android`，目标 `arm64-v8a`）由 `cargo ndk` 构建，
再由 Gradle 打包 APK。游戏连接独立运行的服务端，首次启动时从公开资源库把资源下载到应用的私有缓存。

```sh
cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build -p moly-android --release
cd android && ./gradlew :app:assembleRelease
```

SDK、NDK、JDK 版本、服务端地址与日志查看方式见 [android/README.md](android/README.md)。

## 配置

### 环境变量

| 变量 | 作用 | 缺省 |
|---|---|---|
| `MOLY_ASSET_ROOT` | 资源根目录 | 启动器选中的版本，未选中时取最近安装的版本 |
| `MOLY_SERVER_URL` | 游戏连接的服务端，`http://` 或 `https://` 地址 | `http://127.0.0.1:15800` |
| `MOLY_SERVER_ADDR` | 服务端监听地址 | `127.0.0.1:15800` |
| `MOLY_SERVER_CONTROL_PORT` | 服务端控制端口 | `15801` |
| `MOLY_SERVER_DATA` | 玩家文档的保存目录 | 不设置时只在内存中，每次启动从头开始 |
| `MOLY_DATA_DIR` | 数据目录（存档、设置、日志） | 见 [启动器说明](crates/app/moly-launcher/README.md) |
| `MOLY_SETTINGS_FILE` | 设置文件 | 数据目录下的 `settings.json` |
| `MOLY_DEV` | 为 `1` 时启动开发窗口 | 关闭 |
| `MOLY_MASTER_HARUKI_BASE` | 主表的第一个来源，含 `{repo}` 占位的 HTTPS 模板 | 见 [主表与区服](#主表与区服) |
| `MOLY_MASTER_BASE`、`MOLY_MASTER_MIRROR_BASE` | 主表的其后两个来源 | 见 [主表与区服](#主表与区服) |
| `MOLY_PLAYER_API` | 玩家数据的 URL 模板（开发窗口） | 见 [玩家数据导入](#玩家数据导入) |
| `MOLY_PLAYER_REGION` | 玩家数据的区服，`cn` 或 `jp`（开发窗口） | |
| `MOLY_PLAYER_UID` | 启动时获取并预览该 UID 的数据（开发窗口） | |
| `MOLY_PLAYER_DATA_FILE` | 启动时导入指定 JSON（开发窗口） | |

`MOLY_PLAYER_UID` 与 `MOLY_PLAYER_DATA_FILE` 只能设置一个。

设置、服务端文档与本地列表分别保存在设置文件及其旁边的 `settings.server.json`、
`settings.local.json` 中；同时运行的第二个进程只读这些文件。

### 页面参数（浏览器版）

| 参数 | 作用 |
|---|---|
| `server` | 连接的服务端地址，缺省为公共服务端 |
| `renderer=webgl2` | 使用 WebGL2 后端 |
| `player_uid`、`player_region`、`player_api` | 玩家数据的获取输入 |

每个浏览器在本地存储中保存一个随机的用户 id。

## 玩家数据导入

打开 **Settings（F10）→ Player data**，选择与资源一致的区服，输入 UID 并点击 **Fetch player**，
或用 **Choose JSON** 读取玩家数据文件；原生程序也支持拖入 JSON 文件。确认预览后点击
**Import preview**，即加载家具布局、配色及等级对应的场地大小，并进入对应场景。
重启时缺省打开已导入的主页；显式站点参数仍可覆盖启动位置。

- 支持原始 Home 响应及 `updatedResources` 包装，JSON 上限 32 MiB。
- 导入前保留一份原布局备份，**Restore backup** 可以恢复；音量、画质及其他设置保持原值。
- 有未保存的家具编辑、数据无效或资源不匹配时不会覆盖存档。超限、取消或读取失败均不修改
  当前布局和存档；同一进程同时只读取一份导入正文，迟到的结果不会进入预览。
- 自定义图片、饰件和墙纸地板记录会保留，但这些外观的渲染尚未接入。

资源根需要提供 `fixture-models/player-data.json` 及其引用的配色贴图。该目录包含区服及家具、
场地、等级解锁主表，不含玩家数据；各区服需使用对应的主表和模型资源。

原生程序缺省从 `https://haruki-api.menardi.top/api/{region}/mysekai/{target_user_id}` 获取数据，
`MOLY_PLAYER_API` 可替换该模板。浏览器版使用同源的 `/player-api/{region}/{target_user_id}`：
开发服务器 `web/serve.mjs` 已包含转发；使用 Cloudflare 静态资源绑定时可使用
`web/player-api-worker.mjs`。

## 主表与区服

主表（master）按表名从远端读取：先读 [Team-Haruki](https://github.com/Team-Haruki) 的各区服主表仓库
（`haruki-sekai-master`、`haruki-sekai-sc-master`、`haruki-sekai-tc-master`、`haruki-sekai-kr-master`、
`haruki-sekai-en-master`），再依次读镜像。`MOLY_MASTER_HARUKI_BASE` 替换第一个来源，例如
`https://raw.githubusercontent.com/Team-Haruki/{repo}/`；`MOLY_MASTER_BASE` 与
`MOLY_MASTER_MIRROR_BASE` 替换其后的两个来源。

游戏使用的每张表都是合并表：

- 日服的全部行在前，顺序不变；其后是每个已启用的其他区服里日服没有的主键所在的行，
  这些行带有字段 `"moly:region"` 标明来源区服。
- 主键缺省为 `id`，少数表使用组合主键（如 `resourceBoxes` 为 `resourceBoxPurpose` 与 `id`）；
  单对象表只取日服。
- 两个其他区服加入同一主键时，该表整表拒绝，并写明表名、主键与区服。某一行缺少主键，
  或其他区服的表读取失败时，该表只用日服的行，并按表名报告原因。

当前启用的区服只有日服；其他区服在资源库提供其独占行所引用的资源后加入。

## MCP 服务器

`tools/mcp` 是 MCP 服务器，供 AI 客户端启动游戏的原生程序或浏览器实例，通过 JSON-RPC 读取状态、
执行导演脚本与截取画面。它会先启动服务端、取得实际绑定的地址，再启动游戏。

```sh
npm ci --prefix tools/mcp
```

仓库根目录的 `.mcp.json` 已将它注册为 `moly`。工具、JSON-RPC 方法与安全说明见
[tools/mcp/README.md](tools/mcp/README.md)。

## 项目结构

| 目录 | 职责 |
|---|---|
| `crates/base/moly-law` | 不依赖引擎的行为与计算逻辑 |
| `crates/base/moly-store` | 本地资源库：数据目录、CDN 发布清单、资源下载与校验、安装记录 |
| `crates/engine/moly-assets` | 资源目录、资源包与数据加载 |
| `crates/engine/moly-perf` | 逐帧性能计数 |
| `crates/game/moly-core` | 游戏各层共用的类型、标记与系统排序 |
| `crates/game/moly-render` | 渲染：材质、着色器与绘制 |
| `crates/game/moly-audio` | 背景音乐与音效 |
| `crates/game/moly-masters` | 主表读取与类型化 |
| `crates/game/moly-world` | 场地、家具、步行面与角色行为 |
| `crates/game/moly-ui` | 预制体界面、界面管理与通用对话框 |
| `crates/game/moly-features` | 采集、制作、配送、转换、任务等玩法与界面 |
| `crates/game/moly-devtools` | 控制端口、导演脚本与开发面板 |
| `crates/game/moly-game` | 把以上各层组装成游戏 |
| `crates/net/moly-protocol` | 游戏与服务端之间的请求、应答与推送类型 |
| `crates/net/moly-link` | 游戏一侧的服务端连接 |
| `crates/net/moly-layout` | 家具布局文档 |
| `crates/server/moly-server` | 服务端：玩家文档、规则与请求处理 |
| `crates/app/moly-app` | 游戏程序：原生与浏览器入口 |
| `crates/app/moly-server-host` | 服务端程序 `moly-server` |
| `crates/app/moly-launcher` | 启动器：下载资源、设置、启动服务端与游戏（Tauri，页面在 `ui/`） |
| `crates/app/moly-android` | Android 原生库：游戏入口，连接独立运行的服务端 |
| `crates/vendor/cpal` | 带一处网页音频修正的 CPAL |
| `android` | Android 工程（Gradle） |
| `web` | 浏览器页面与构建工具 |
| `tools` | 版本、仓库边界与渲染顺序检查 |
| `tools/mcp` | MCP 服务器 |

## 开发

### 测试与检查

```sh
# 无 GPU 的核心与回归测试：网络和存档检查使用回环服务、合成输入及临时文件，不需要游戏资源
cargo test --release --workspace --lib -- --test-threads=1

# 版本一致性与开源边界
python tools/version/check.py
node tools/boundary/check.mjs
```

### 提交规范

提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)；拉取请求的标题和提交由
commitlint 按 `commitlint.config.mjs` 检查。

### 持续集成

GitHub Actions 在推送、拉取请求和手动触发时执行版本一致性（含 `CHANGELOG.md` 中有当前版本的段落）、
JavaScript 语法、开源边界及提交空白检查。这些检查不编译、不打包，也不需要游戏资源。

### 版本与发布

版本统一定义在 `Cargo.toml` 的 `[workspace.package]` 中，所有 crate 继承该版本。设置面板显示当前版本；
`moly-app --version` 无需加载资源即可查询。

提升版本的提交同时更新 [CHANGELOG.md](CHANGELOG.md)，由 [git-cliff](https://git-cliff.org/) 按
`cliff.toml` 从提交记录生成：

```sh
git cliff --unreleased --tag v<版本> --prepend CHANGELOG.md
```

推送 `v<版本>` 标签时，发布工作流先运行上述检查并核对标签与工作区版本一致，再编译 Windows x64、
Linux x64、macOS arm64 原生程序和 WebGPU、WebGL2 两个网页后端，把 Windows 程序包（游戏、服务端与
启动器）、网页包（`pkg/` 目录）和 `SHA256SUMS` 附加到一个草稿 Release。Linux 与 macOS 程序只编译并
核对版本，不附加。Release 说明是 git-cliff 按同一配置生成的该版本段落；仓库中有
`release-notes/v<版本>.md` 时改用该文件。手动触发只生成工作流产物，不创建 Release。
发布包不含任何游戏资源。

## 许可

代码采用 [AGPL-3.0-only](LICENSE)。第三方内容按各自许可证分发，见
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。代码许可证不授予游戏资源的使用或分发权。
