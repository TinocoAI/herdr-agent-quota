# herdr-agent-quota

**别再做到一半才发现额度用完。** 在 Herdr 左侧 Agents 列表中实时显示
Claude Code、Codex、Grok 和 Agy/Antigravity 的订阅额度。

[![CI](https://github.com/levi-qiao/herdr-agent-quota/actions/workflows/ci.yml/badge.svg)](https://github.com/levi-qiao/herdr-agent-quota/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/built%20with-Rust-dea584?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Herdr plugin](https://img.shields.io/badge/Herdr-plugin-0.8%2B-5b6ee1)](https://herdr.dev/docs/plugins/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![GitHub stars](https://img.shields.io/github/stars/levi-qiao/herdr-agent-quota?style=social)](https://github.com/levi-qiao/herdr-agent-quota)

[English README](README.md)

```text
● Owner · Claude
  hi                     ← 这个 pane 当前在做什么
  5h 100% reset 3h07m
  week 31% reset 2d3h
```

![Herdr 左侧额度截图](docs/screenshots/herdr-sidebar-live.png)

*真实的 Herdr 工作区：Claude 分行显示 5 小时和周额度及其重置倒计时，
Codex、Grok 显示周额度；每张 agent 卡片的话题来自用户最后一次输入，
不会使用 AI 生成的状态标题。*

- **四个 CLI，一个侧栏** —— Claude Code、Codex、Grok、Agy/Antigravity。
- **每个 pane 三或四行** —— provider、每个额度窗口各一行、最后一条用户输入。
- **全本地** —— 不上传任何用量数据，不读浏览器 cookie 和系统钥匙串，
  也不会写入或刷新凭证。
- **不会给你错的数** —— 刷新失败时保留上一次的有效数值，而不是闪成
  `unavailable`；API key 登录也不会被当成订阅额度显示。
- **完全可回滚** —— 一个 action 装好，一个 action 原样还原你的配置。

下载仓库后执行一个命令即可批量安装所有可恢复的集成（[快速开始](#快速开始)）：

```sh
./install.sh
```

截图是真实的 Herdr 本地会话。其中的额度和话题来自当时的会话，
不是插件写死的示例数据。

### 按时间进度计算额度健康度

颜色回答的是“当前额度能否撑到重置”，而不是简单套用固定百分比阈值。
对于每个可用的 5 小时或 7 天窗口，插件统一计算：

```text
剩余时间比例 = (重置时间 - 当前时间) / 窗口总时长
剩余额度比例 = remaining_percent / 100
健康度       = 剩余额度比例 / 剩余时间比例
```

- **绿色** —— 健康度 `>= 1`：额度消耗速度不快于时间进度。
- **琥珀色** —— 健康度 `< 1`：当前消耗已经快于可持续速度。
- **红色** —— 健康度 `< 1` 且剩余额度低于 20%：额度不足且预计无法撑到重置。
- **琥珀色兜底** —— reset 缺失或过期，不把无法判断的数据错误标成安全。

截图中的 Claude 5 小时额度剩 89%，而窗口剩余时间略高于 89%，所以显示
琥珀色；周额度虽然只剩 24%，但本周时间只剩约 13%，所以仍为绿色。Grok
周额度只剩 17%，窗口时间却还剩约 69%，已经同时满足“续航不足且额度低于
20%”，所以显示红色。所有 provider 共用同一套计算，适配器只负责提供窗口数据。

## 快速开始

要求：Herdr `0.8.0+`、Rust `1.95+`、macOS 或 Linux，以及至少一个支持的
CLI。下载仓库后，在目录中执行下面的一键安装命令即可完成构建、链接、启用和
可恢复配置：

```sh
./install.sh
```

恢复原来的 sidebar/statusLine 配置并解除插件链接：

```sh
./uninstall.sh
```

两个脚本都可以重复执行。卸载会保留 Herdr 插件 state 中的本地额度快照（不含
凭证）；如需释放磁盘空间可再手动删除。

等价的 Herdr 命令是：

```sh
herdr plugin link . --enabled
herdr plugin action invoke herdr-agent-quota.configure
```

configure action 会统一使用 Herdr 的插件 state 目录，批量写入 sidebar 行、安装
或修复可恢复的 Claude/Agy statusLine 采集器，并自动 reload 配置。之后可随时在
Herdr action 菜单执行 **Install / repair agent quota**，重复
执行也是安全的。需要手动刷新时执行 **Refresh agent quota**。

选中一个 pane 时也会触发该 provider 的额度刷新，60 秒内自动合并重复请求。
进入 working 后，插件只启动一个全局短生命周期 watcher：每轮只调用一次
`herdr agent list`，统一找出所有 working provider，再批量发布缓存并按相同去抖规则
查询 Codex/Grok；每个 provider 结束时再按去抖规则补一次。
这条路径不会读取终端内容；如果 pane 正在查看 scrollback，则暂缓 metadata 写入，
回到底部后再补上，避免刷新把 viewport 拉走。默认轮询间隔为 60 秒，可在安装时自定义
为 30 秒到 1 小时：

```sh
./install.sh --watch-interval-seconds 300
```

已有安装也可以这样更新间隔：

```sh
HERDR_AGENT_QUOTA_WATCH_INTERVAL_SECONDS=300 \
  herdr plugin action invoke herdr-agent-quota.configure
```

每轮只使用一个全局协调锁和一次 `herdr agent list`。各 provider 的网络查询仍由
原有刷新标记独立限制为每 60 秒最多一次，即使用户把轮询间隔设得更短也不会突破。
watcher 不发送 prompt、不重新登录、不刷新凭证，也不会消耗模型/对话 token；只有用户
明确执行手动 `--force` 刷新时才绕过这层去抖。

只查看配置变更、不写入文件：

```sh
./target/release/herdr-agent-quota configure --check
```

要一次撤销所有插件配置并恢复原来的 Claude/Agy statusLine，可在 action 菜单
执行 **Uninstall agent quota configuration**，或运行：

```sh
herdr plugin action invoke herdr-agent-quota.uninstall
```

action 完成后，如需连插件注册也删除，再执行
`herdr plugin unlink herdr-agent-quota`。写配置刻意只通过插件 action 进行，
避免采集器写到另一份缓存目录。

插件会保留 Herdr 原生的状态圆点和 plane/tab 提示，只追加 provider、
usage、topic 三类 token，不会覆盖官方 agent 指示。卸载 action 会删除插件
添加的行，并恢复原来的 Claude/Agy `statusLine`。

## 支持的 CLI

| CLI | 侧栏显示 | 本地数据来源 | 额外配置 |
| --- | --- | --- | --- |
| Claude Code `2.1.233` | `5h` + `week` | 官方 `statusLine` JSON：`rate_limits.five_hour`、`seven_day` | 配置 action 自动安装并串联原命令；turn 中由统一 watcher 发布缓存 |
| OpenAI Codex `0.147.0` | `week` | 一次性的 `codex app-server --stdio`，调用 `account/rateLimits/read` | 使用 ChatGPT 订阅登录；API key 模式显示为不可用；turn 中由统一 watcher 去抖查询 |
| Grok CLI / Grok Build `1.0.4` | `week` | `~/.grok/auth.json` 和官方 CLI 使用的额度接口 | 由统一 watcher 处理，不再安装回复 hook |
| Agy / Antigravity CLI `1.1.13` | `5h` + `week` | 官方 `statusLine` JSON 的 `quota`（`gemini-*`、`3p-*`） | 配置 action 自动安装并串联原命令；turn 中由统一 watcher 发布缓存 |

上面的版本是 2026-08-15 在开发机上实际检查的版本。解析器按照供应商的
字段工作，不会把这些版本号写死；兼容的新版本可以继续使用。

侧栏显示的是**剩余百分比**和距离下次重置的时间，不是 token 数量：

```text
● Owner · Claude
  hi
  5h 100% reset 3h07m
  week 31% reset 2d3h
```

Codex 和 Grok 提供周额度；Claude Code 和 Agy 提供 5 小时额度与周额度。
不到一小时显示分钟，不到一天显示小时和分钟，超过一天显示天和小时。
侧栏数值在 agent 事件、working turn 的短生命周期刷新脉冲或手动刷新时重新计算，
不是常驻的逐分钟跳动倒计时。
刷新失败时，插件会保留上一次成功的缓存值，不会把旧值清空为
`unavailable`。从未成功采集过的 provider 才会显示 `N/A`。

## Agy / Antigravity 采集

Agy 通过原生的一次性 `statusLine` hook 把额度 JSON 传给插件。配置 action
会自动安装它；如果用户原来已有 statusLine 命令，会先备份并串联，卸载时恢复。
插件自己的采集器不输出任何 status-line 文本，只从 stdin 读取 JSON，把脱敏后的
百分比写入本地插件缓存，然后退出。它不是常驻进程，也不使用浏览器 Cookie 或
私有 API。

## 侧栏布局

默认配置保持紧凑，并且每个 provider 名称只显示一次：

```toml
[ui.sidebar.agents]
row_gap = 1 # herdr-agent-quota
rows = [
  ["state_icon", "tab", { token = "$quota_provider", bold = true, dim = false }],
  [{ token = "$quota_topic", dim = false }],
  [{ token = "$quota_5h_normal", fg = "#84b084", bold = true, dim = false }],
  [{ token = "$quota_5h_warning", fg = "#cdaa65", bold = true, dim = false }],
  [{ token = "$quota_5h_danger", fg = "#ca6470", bold = true, dim = false }],
  [{ token = "$quota_week_normal", fg = "#84b084", bold = true, dim = false }],
  [{ token = "$quota_week_warning", fg = "#cdaa65", bold = true, dim = false }],
  [{ token = "$quota_week_danger", fg = "#ca6470", bold = true, dim = false }],
]
```

- `state_icon`、`tab` 是 Herdr 内置的状态和 plane 标签。
- `$quota_provider` 是 `Claude`、`Codex`、`Grok` 或 `Agy`。
- 默认 provider 名称使用易辨识的柔和品牌色，并且不影响额度健康色：
  Claude 柔橘、Codex 粉彩蓝、Grok 柔白、Agy 使用 Antigravity 风格薄荷绿。
- `$quota_topic` 放在额度上方，阅读顺序是 agent、当前任务、资源状态。
- 每个窗口只发布一个样式 token。颜色按额度续航动态判断，不再使用固定
  余额阈值：将剩余额度比例与窗口剩余时间比例比较，额度消耗不快于时间
  进度时为绿色；落后于时间进度时为琥珀色；落后且额度低于 20% 时为
  红色。reset 缺失或过期时使用告警色。Codex/Grok 不支持的 5h 行
  会自动隐藏。
- `row_gap = 1` 在 agent 卡片之间留一行空白；已有的显式 `row_gap`
  配置会原样保留。
- `$quota_5h`、`$quota_week`、`$quota_summary` 仍保留给需要无样式或
  紧凑布局的自定义配置。

Herdr 0.8 的样式只接受固定十六进制颜色，不支持跟随主题的语义色。
默认的绿色、琥珀色和红色采用高明度柔和色阶并加粗，降低 Herdr 深色侧栏
长时间阅读的视觉疲劳，同时保持状态辨识度。

Provider 品牌样式通过 Herdr 静态 `rows_by_agent` 投影实现，额度健康色继续
使用动态 metadata。两者相互独立，也不会为静态名称额外占用 metadata token
容量。

Herdr plugin v1 只支持文本 token，不能由插件向原生 Agent renderer 注入
品牌 SVG/PNG。因此默认使用清晰的 provider 名称和 Herdr 原生圆点，不再
添加辨识度不高的 Unicode 图标。仓库中的 [`docs/icons/`](docs/icons/) 只
作为可选视觉参考，不会被注入左侧原生 sidebar。

话题读取由事件触发：插件扫描 pane 最近输出，只提取最后一次用户输入。
如果没有找到 prompt，话题保持空白，不会回退为 AI 生成的 `Thinking`、
`Executing` 等终端标题，也不会显示工作目录。

## 数据来源与隐私

- **Codex：** 使用本地官方
  [app-server JSON-RPC](https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md)
  的 rate-limit 响应，按窗口时长识别周额度。API key 登录不会被误标记为
  ChatGPT 订阅额度。
- **Grok：** 在内存中读取本地 `~/.grok/auth.json` 登录 key，访问 Grok CLI
  使用的周额度接口。只有明确标记为 weekly 的响应才会接受。这是
  SuperGrok 订阅额度，不是 xAI 开发者/API team 账单。统一 watcher 配合原有
  60 秒去抖查询额度，不会反复登录、刷新登录 key，也不会消耗对话 token。
- **Claude Code：** 使用官方
  [`statusLine` JSON hook](https://code.claude.com/docs/en/statusline) 提供
  5 小时和 7 天额度。原有 statusLine 会被备份、串联，并可由
  卸载 action 恢复；working turn 中由统一 watcher 发布新缓存。
- **Agy/Antigravity：** 使用官方
  [`/usage` 和 statusline 文档](https://antigravity.google/docs/cli/commands/usage?app=antigravity-ide)
  中的 Gemini 和第三方额度池。两个额度池同时存在时，取较低的剩余百分比，
  让单个 Agy 行保持保守，working turn 中同样由统一 watcher 发布缓存。

快照和刷新标记保存在 Herdr 插件状态目录中。插件不会上传使用数据，不读取
浏览器 Cookie/Keychain，不刷新或写入 provider 凭据。provider 失败时保留
上一次成功的本地值。

Grok CLI 的额度接口属于 CLI 内部契约，不是 xAI 面向开发者的公开稳定 API。
如果接口变化，插件会保留上一周额度，而不是清空侧栏。

## 常见问题

| 现象 | 处理方式 |
| --- | --- |
| 侧栏没有新增行 | 执行 `herdr server reload-config`，再运行 **Refresh agent quota**。 |
| Claude 或 Agy 显示 `N/A` | 发起一次对话，让原生 `statusLine` 产生 JSON，然后刷新。 |
| 切换 pane 时 Claude 短暂变化 | 已有缓存会保留；如果还没有快照，发送一次 prompt 或手动刷新。 |
| Agy 没有额度 | 执行 **Install / repair agent quota**，完成一次 Agy 对话后再手动刷新。 |
| 任一运行中的 turn 额度不更新 | 执行 **Install / repair agent quota**，下一次 working turn 会自动启动统一 watcher，回合结束时还会按去抖规则补一次。 |
| 话题为空或没有更新 | 在该 pane 发送 prompt；话题提取依赖 agent 事件和最近输出。 |
| 原有 Claude statusLine 没有被修改 | 执行 `configure --check`；对于不能安全串联的配置，插件会拒绝覆盖。 |

## 开发检查

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features --locked
cargo build --release --locked
```

每个 PR 都会在 Linux 和 macOS 上跑这几条命令。

[`CONTRIBUTING.md`](CONTRIBUTING.md) 说明了所有 parser 遵循的设计约束，
以及如何新增一个 provider。安全问题反馈见 [`SECURITY.md`](SECURITY.md)，
版本变更见 [`CHANGELOG.md`](CHANGELOG.md)。

Grok 调研记录见
[`docs/research/codexbar-grok-usage.md`](docs/research/codexbar-grok-usage.md)，
实现约定见
[`docs/plans/herdr-agent-quota-implementation.md`](docs/plans/herdr-agent-quota-implementation.md)。

## 参与贡献

新增一个 CLI 的成本很低：一个纯函数 `parse_*`、一份脱敏 fixture、一个
测试。具体约束见 [`CONTRIBUTING.md`](CONTRIBUTING.md)。

如果这个插件帮你少切了几次 pane，点个 ⭐ 能让更多 Herdr 用户找到它。
提一个带 CLI 版本号的 issue 更有帮助 —— 它决定了下一个修哪个 provider。

## 许可证

MIT。本项目与 Herdr、OpenAI、Anthropic、xAI 或 Google 没有隶属关系。
