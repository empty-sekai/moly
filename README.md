# moly

MYSEKAI的还原项目，使用 Rust 与 [Bevy](https://bevyengine.org/) 构建，
支持原生程序和浏览器（WebAssembly）。

目标是还原场景探索、角色行为、家具互动与天气表现。**项目仍在开发中**

当前版本的范围与之后的计划见 [路线图](ROADMAP.md)。

![moly 中的角色、家具与头顶气泡](screenshots/readme.png)

## 资源

运行所需的模型、贴图、动作和音频由使用者自行提供，不随仓库分发。
项目代码与游戏资源的权利归属相互独立。

## 浏览器入口

全屏游戏由独立的 `moly-web` 站点加载 `game` 发布配置，通过 `game-boot.mjs`
进入游戏。已有站点可通过 [嵌入接口](web/EMBEDDING.md) 使用 `stage.html`，由宿主
提供内容目录和页面导航。网页检查说明见 [浏览器检查](web/TESTING.md)。

嵌入舞台的“对话与互动”内容中，默认的 **独立体验** 会前往一处
临时空场景，只布置所选内容确实需要的角色与源资源家具；结束、关闭或失败后
返回原场景并恢复布局、名册、玩家与相机。**当前场景** 则只使用眼前已经存在
的角色和家具，不会自动补齐。没有角色动作的陈设家具在独立体验中用于查看，
不会被描述或派发成互动。

资源根是不可混合的区服快照。界面显示当前 `region` 与 `gameVersion`；CN、JP
相同数字 ID 不会跨资源根合并。家具只有在主表的资源叶名、导出包和 fixture
视图相互一致时才进入临时布置。墙纸、地板沿用原始房间材质和 UV 选择规则
呈现，自定义表面使用原始配置；缺少源资源的条目会明确说明原因。

## 本地运行

安装稳定版 Rust 和 Node.js 22，将 `MOLY_ASSET_ROOT` 指向已提取的资源目录，
运行 `cargo run --release -p moly-app`。构建默认写入仓库的 `target/`；需要共享缓存时，
自行设置 `CARGO_TARGET_DIR`，增量编译可通过 `CARGO_INCREMENTAL` 配置。

原生程序默认运行与网页相同的游戏：从家园进入，家园等级随服务端文档中的等级，
不读取其他环境开关。设置、服务端文档和本地列表分别保存在设置文件
（`MOLY_SETTINGS_FILE`，缺省为数据目录下的 `settings.json`，数据目录见下文“启动器”）及其旁边的
`settings.server.json`、`settings.local.json` 中；同时运行的第二个进程只读这些文件。
设置 `MOLY_DEV=1` 则启动开发窗口：它读取各项原生环境开关，服务端文档只保存在内存中，
并提供玩家数据导入等开发工具。两种模式下按 **F10** 打开画面与音量设置，按 **F9** 打开控制面板。

浏览器版需要 `wasm32-unknown-unknown` target 和与 Cargo.lock 一致的 wasm-bindgen CLI：

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.127 --locked
node web/build-wasm.mjs
node web/serve.mjs
```

开发服务器提供 `/stage.html?assets=/assets/` 嵌入舞台。全屏游戏使用 `moly-web` 的
站点服务器及 `game` 发布目录；其进入按钮负责在用户手势中启用音频。
构建脚本分别生成 `web/pkg/webgpu/` 和 `web/pkg/webgl2/`，页面只下载所选后端的模块。
字体单独写入 `web/pkg/fonts/<sha256>.ttf`，两个后端的 `build.json` 记录同一份字体的长度与散列。
部署时保留完整 `pkg/` 目录；页面在启动前验证并安装字体，缺失或不匹配时会明确拒绝启动。
WebGPU 不可用时会选择 WebGL2；启动失败页可重新加载，或用 `?renderer=webgl2` 重试。
每次重试都会重新加载页面。浏览器同时只允许一个页面写入存档，其余页面可浏览；
关闭可写页面后，重新加载其他页面可申请写入权限。不支持 Web Locks 的环境保持只读。

开发服务器只监听回环地址。资源目录应可信且只读，目录内链接的实际目标必须仍在同一挂载根内。
分包模式（`?packs=1`）限制目录/清单为 16 MiB、单 blob 为 128 MiB、单个解码资产为
256 MiB（分包载荷），解压大小最多为 blob 大小的 256 倍加 64 KiB。最多同时读取 8 份资源，传输与解码
缓冲合计预算为 512 MiB。浏览器分包响应逐块读取，超限或超时即中止；这些预算不包含
加载器已经接收的数据、解析后的目录、资源及场景对象、GPU 资源和浏览器内部缓存。
暂时的预算不足会按请求顺序等待，取消读取会释放排队和已占用的预算；单个请求超过上限时
直接拒绝。identity 数据只预留传输缓冲，gzip 同时预留传输与解码缓冲。分包读取器按顺序
消费，读完或丢弃时释放读取预算；需要 seek 的自定义加载器可使用 Bevy 的 VecReader 回退。

## 玩家数据导入

打开 **Settings（F10）→ Player data**，选择与资源一致的区服，输入 UID 并点击
**Fetch player**，或使用 **Choose JSON** 读取玩家数据。确认预览后点击
**Import preview**，即可加载家具布局、配色及等级对应的场地大小。
原生程序也支持拖入 JSON 文件。支持原始 Home 响应及 `updatedResources` 包装。
导入后会进入对应场景，重启时默认打开已导入的主页；显式站点参数仍可覆盖启动位置。

导入保留一份原布局备份，**Restore backup** 可以恢复；音量、画质及其他设置保持原值。
有未保存的家具编辑、数据无效或资源不匹配时不会覆盖存档。自定义图片、饰件和墙纸地板
记录会保留，但这些外观的渲染尚未接入。

JSON 输入上限为 32 MiB。UID 请求和文件选择使用受限流式读取；浏览器先检查文件大小，
再打开读取流。同一应用进程同时只读取一份导入正文，取消会中止正在读取的请求或文件，
迟到结果不会进入预览。超限、取消或读取失败均不修改当前布局和存档。

资源根需要提供 `fixture-models/player-data.json` 和其中引用的配色贴图。该目录包含
区服及家具、场地、等级解锁主表，不含玩家数据。各区服需使用对应的主表和模型资源。

原生默认从 `https://haruki-api.menardi.top/api/{region}/mysekai/{target_user_id}` 获取数据。
可通过 `MOLY_PLAYER_API` 覆盖 URL 模板，`MOLY_PLAYER_REGION` 选择 `cn` 或 `jp`。
`MOLY_PLAYER_UID` 在启动时获取并预览数据；`MOLY_PLAYER_DATA_FILE` 在启动时导入指定 JSON。
UID 与文件启动参数只能选择一种。

浏览器版使用同源 `/player-api/{region}/{target_user_id}`，开发服务器 `web/serve.mjs`
已包含转发。使用 Cloudflare 静态资源绑定时，可使用 `web/player-api-worker.mjs`。
页面参数 `player_uid`、`player_region`、`player_api` 可覆盖获取输入。

## 启动器

`moly-launcher` 是原生版的入口程序。它从 CDN（默认 `https://assets.pjsk.moe/sekai-extra-assets/`）
读取发布清单 `game/manifest.json`，列出可下载的资源版本，把选中版本的目录、包清单与资源文件
下载到资源目录并逐个核对 SHA-256，然后启动同目录下的 `moly-app`。下载可随时取消，下次从断点
继续；对已安装的版本再执行一次即为校验与修复。多个版本共用内容相同的文件。

CDN 地址、资源目录、并行下载数与开发者窗口在启动器的设置中修改，保存在数据目录的
`launcher.json`。数据目录在 Windows 上是程序所在文件夹里的 `data\`，启动器与游戏在同一个
文件夹中共用它；其他系统是 `$XDG_DATA_HOME/moly`（或 `~/.local/share/moly`）。设置环境变量
`MOLY_DATA_DIR` 可改用别的目录，启动器启动游戏时会把自己的数据目录传给游戏。游戏的存档与
设置文件、启动器写的游戏日志（`logs/`）也在数据目录里。

资源目录首次打开时确定并写入 `launcher.json`：默认是数据目录里的 `resources/`；若它位于
Windows 系统盘，则改用剩余空间最大的其他固定磁盘根下的 `moly\`。在设置中选择文件夹时，
选中的文件夹为空、已含资源或名为 `moly` 则直接使用，否则使用其中的 `moly\`（选 `E:\` 得到
`E:\moly`）。更换资源目录时可把已安装的资源迁移过去：同一磁盘内直接移动文件夹；跨磁盘时
逐个复制并按文件名中的 SHA-256 校验，切换到新目录后才删除旧文件，中途取消或失败时原目录
保持可用。

不经启动器直接运行 `moly-app` 且未设置 `MOLY_ASSET_ROOT` 时，游戏读取启动器资源目录中
已安装的版本：`launcher.json` 选中的版本，没有选中时取最近安装的版本。

## 版本与 CI

版本统一定义在 `Cargo.toml` 的 `[workspace.package]` 中，所有 crate 继承该版本。
设置面板显示当前版本；原生程序可通过 `moly-app --version` 查询，无需加载资源。

GitHub Actions 在推送、拉取请求和手动触发时执行版本一致性、JavaScript 语法、
开源边界及提交空白检查，不编译、不打包，也不需要游戏资源。

推送 `v<版本>` 标签时，发布工作流先运行上述检查并核对标签与工作区版本一致，再编译
Windows x64、Linux x64、macOS arm64 原生程序和 WebGPU、WebGL2 两个网页后端，
把 Windows 程序包（游戏与启动器）、网页包（`pkg/` 目录）和 `SHA256SUMS` 附加到一个草稿 Release。
Linux 与 macOS 程序只编译并核对版本，不附加。手动触发只生成工作流产物，不创建 Release。
发布包不含任何游戏资源。
版本和边界检查可在本地运行 `python tools/version/check.py`、`node tools/boundary/check.mjs`。
本地无 GPU 的核心与回归测试可运行 `cargo test --release --workspace --lib -- --test-threads=1`，
其中的网络和存档检查使用回环服务、合成输入及临时文件，不需要游戏资源。

## 代码结构

| 目录 | 职责 |
|---|---|
| `crates/base/moly-law` | 不依赖引擎的行为与计算逻辑 |
| `crates/base/moly-store` | 本地资源库：数据目录、CDN 发布清单、资源下载与校验、安装记录 |
| `crates/engine/moly-assets` | 资源目录、资源包与数据加载 |
| `crates/engine/moly-perf` | 逐帧性能计数 |
| `crates/game/moly-game` | 场景、交互、UI 与渲染 |
| `crates/app/moly-app` | 游戏程序：原生与浏览器入口 |
| `crates/app/moly-launcher` | 启动器：下载资源、设置、启动游戏（Tauri，页面在 `ui/`） |
| `crates/vendor/cpal` | 带一处网页音频修正的 CPAL |
| `web` | 浏览器页面与构建工具 |
| `tools` | 仓库边界与渲染顺序检查 |
| `tools/mcp` | MCP 服务器，启动并驱动原生与浏览器实例 |
| `screenshots/readme.png` | README 展示图 |

## MCP 服务器

`tools/mcp` 是 MCP 服务器，供 AI 客户端启动游戏的原生程序或浏览器实例，并通过 JSON-RPC 读取状态、执行导演脚本与截取画面。依赖安装命令：

```sh
npm ci --prefix tools/mcp
```

仓库根目录的 `.mcp.json` 已将它注册为 `moly`。工具、JSON-RPC 方法与安全说明见 [tools/mcp/README.md](tools/mcp/README.md)。

## 许可

代码采用 [AGPL-3.0-only](LICENSE)。第三方内容按各自许可证分发，见
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。代码许可证不授予游戏资源的使用或分发权。
