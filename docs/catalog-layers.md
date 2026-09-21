# 目录消费分层

## 决定

目录**不拆**。moly 继续发布完整的 `catalog/index.json` 与快照身份——它有多个消费者，独立页面不能假设对方有 masterdata 管线。

同一次导出**增发**一份紧凑产物 `catalog/support.json`，供已经持有 masterdata 的消费者使用。加一个产物，不减任何东西。

| 层 | 内容 | 消费者 |
| --- | --- | --- |
| A `catalog/index.json` | 现有全量投影 | 独立页面、离线、无 masterdata 管线的消费者 |
| B `catalog/support.json` | `{region, version, snapshotId, supported[], reasons{}}` | 已有 masterdata 的消费者 |

真源按消费者成立：**身份与结构**取消费者自己已经信任的那份（A 层，或对方的 masterdata）；**可播性**永远且只有 moly。

## 为什么

当前 CN 6.0.0 目录与 masterdata 行数精确相等：

```
mysekaiFixtures.json        952 行  ←→  catalog fixture 条目    952
mysekaiCharacterTalks.json 6,180 行 ←→  catalog talk 条目     6,180
```

身份键本来就是 masterdata id（`npc_objective.rs` 的 `TalkContent.master_id`、`content_library.rs` 的 `EntryKey::Talk(backend, i32)` / `Fixture(i32)`），运行时读的也是 masterdata 形态的表（`moly://mysekai-fixtures.json`、`moly://mysekai-character-talks.json`）。所以对已有 masterdata 的消费者，A 层的 talk+fixture 部分（9,137 条中的 7,132 条）是纯重复。

3.6 MB 条目载荷的归属：masterdata 可推导约 30%（characters / title / fixtureIds / unitIds），呈现标签约 57%（presentation / image / subtitle / detail），**真正不可替代的可播性判断约 2%**（available / reason / reasonCode）。

实测瘦身：

```
A  index.json      6.05 MB raw   0.73 MB gz
B  support.json     189 KB raw    26 KB gz      33x / 28x
   supported 8,926 条 · reasons 211 条
```

## 可行性：已验证

B 层能在发布期算全，无需活世界。

`context.rs` 的 `selected_reason` 一进来就分叉，独立模式直接走 `independent_reason(key, catalog)`——**签名里没有 world**，只查 catalog：家具存不存在、`available_for_preview()`、activity 的 `unavailable` 字段、talk 的 `units.is_empty()`。而 `bridge_export::publish_if_requested` 正是以 `mode: Independent` 对空世界导出的。

依赖活世界的理由（`selected_instance`、`activities::current_reason`、`talk_here`、实例 ready）**全部只在 current 模式路径上**。那是玩家自己的世界，本来就不可发布，宿主也已按这条线分开取值。**架构天然沿这条缝切开。**

## 前置条件：reasonCode 必须先真码化

`bridge_presentation.rs` 现在这样发码：

```rust
row["reasonCode"] = if !available {
    if mode == Independent { "source_unavailable" } else { "scene_unavailable" }
} else { Null };
```

`independent_reason` 能返回 5 种不同理由，全部塌缩成一个码。CN 6.0.0 的 211 条不可用**全是** `source_unavailable`——不是只有一种失败，是码丢掉了区别。区别只活在中文散文 `reason` 里，而那是内部文案，不能当多语言站点的用户可见串。

不先修这个，B 层里那 211 条等于没信息。按 `independent_reason` 的分支发码：

| 分支 | 建议码 |
| --- | --- |
| 家具不在当前来源图鉴中 | `fixture_not_in_source` |
| 家具原始模型尚未完整导出 | `fixture_assets_missing` |
| activity 原始数据不可用 | `activity_data_missing` |
| activity 自带 `unavailable` | 沿用该字段的具体原因 |
| talk 没有有效登场角色 | `talk_no_cast` |

散文 `reason` 保留在 A 层供内部与独立页面使用；B 层只发码，由消费者本地化。

## 边界：不变的部分

播放路径完全不动。`mountMoly` 本来就收内容键，`fixture_scene_inputs.rs` 的 `validate_admission` 仍在播放时判准入。moly 继续发布对白正文、3D 资源、控制器、头像。

快照保留。masterdata 在 `/latest` 是可变的，快照 id 提供不可变内容身份，独立页面需要它。

## 接受的代价

**activity 没有 masterdata 对应行**：2,005 条（22%）。B 层只给"支持哪些键"，不给 activity 的标题与内容，消费者要么对 activity 回落到 A 层，要么 B 层为 activity 多带字段。设计时需明确取舍。

**呈现标签归属**：`presentation` / `image` / `subtitle` / `detail` 占全量 57%，现在由 moly 决定。只取 B 层的消费者需自行渲染，风格会与 moly 独立页面不一致。

**未提取上线的内容直接不支持**：masterdata 里有、published set 里没有的，渲染为 unsupported，不做任何推断或替代。

## 未做

本文件只是设计记录，不是进度。B 层尚未实现，reasonCode 尚未真码化，消费者尚未接入。按本仓尺子衡量，呈现层与模拟层的计数均未因此改变。
