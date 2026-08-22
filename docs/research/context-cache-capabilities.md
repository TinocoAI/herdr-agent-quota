# Context 与 prompt cache 能力调研

研究日期：2026-08-22（Asia/Shanghai）
范围：本仓库当前的 Codex、Grok Build、Claude Code、Agy/Antigravity 四个 provider。
来源约束：优先使用供应商官方文档、官方 CLI/app-server 源码和本仓库 parser/fixture；没有把第三方状态栏项目当作能力契约。

## 先说结论

“额度窗口重置”与“会话 context 使用率”是两个不同指标；“缓存 token 数量”与“缓存过期时间”也不是同一个字段。四家都能在一定程度上提供 context 使用信息，但只有 Codex 的 app-server、Claude/Agy 的 statusLine、Grok 的 status-line payload 会提供 token 级数据。四家当前都没有一个可供本插件安全显示的“这条缓存还剩多久”动态字段。

| Provider | context 百分比 | cached token 数量 | 缓存实际过期时间/剩余 TTL | 本仓库当前来源 |
| --- | --- | --- | --- | --- |
| Codex | 支持：app-server 的 `last.totalTokens` + `modelContextWindow`，可计算 | 支持：`cachedInputTokens`；当前 schema 还包括 `cacheWriteInputTokens` | 不支持动态剩余 TTL。OpenAI API 文档有请求级 TTL，但 Codex app-server 的 token-usage 事件不带 entry expiry | 读取 `account/rateLimits/read`，另做有界 `thread/list` 会话预览；没有接入活动会话 token-usage 事件 |
| Claude Code | 支持：statusLine 的 `context_window.used_percentage`/`remaining_percentage` | 支持：`current_usage.cache_read_input_tokens`/`cache_creation_input_tokens` | 不支持实际过期时间。Claude API 有 5m/1h policy，但 Claude Code statusLine 不返回所选 TTL 或 expiry timestamp | 解析 `rate_limits` 和可选 `context_window`；compact/首响应缺失时保留上次 context |
| Agy/Antigravity | 支持：statusLine 的 `context_window.used_percentage`/`remaining_percentage` | 支持：`context_window.current_usage` 中的 cache read/create 数量 | 不支持；官方 statusLine 只有 quota reset 字段，没有 prompt-cache TTL | 解析 `quota` 和可选 `context_window`；compact/首响应缺失时保留上次 context |
| Grok Build | 支持：官方 status-line 的 `context_window.used_percentage`/`remaining_percentage` | 支持：`session_usage.cache_read_input_tokens`/`cache_creation_input_tokens` | 不支持；官方 status-line contract 没有 TTL/expiry 字段 | 只请求 billing credits，旧 Grok quota hook 已被统一 watcher 移除 |

因此建议：可以增加 **context 使用百分比**，也可以增加 **cached/read/write token 统计**；不要显示“cached 过期倒计时”。如果 UI 需要一个“数据新鲜度”，应显示本地快照的 `observed_at`，不能把它伪装成服务端缓存 expiry。

## Codex

### 支持

官方 app-server 文档说明，turn 期间会独立发送 `thread/tokenUsage/updated` 通知；`thread/resume` 在已有持久化 token usage 时也会立即重放该通知（[app-server README 的 thread/resume 与 turn events](https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md#turn-events)）。当前官方生成的 v2 schema 定义了：

- `tokenUsage.last.totalTokens`：最近一次上下文的 token 数；
- `tokenUsage.modelContextWindow`：模型上下文窗口上限；
- `tokenUsage.last.inputTokens`、`outputTokens`、`reasoningOutputTokens`；
- `tokenUsage.last.cachedInputTokens` 和 `cacheWriteInputTokens`；
- `tokenUsage.total`：会话累计 token usage。

字段是官方 app-server 协议中的稳定类型，见 [ThreadTokenUsageUpdatedNotification schema](https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/schema/json/v2/ThreadTokenUsageUpdatedNotification.json) 和 [Codex core token usage types](https://github.com/openai/codex/blob/main/codex-rs/protocol/src/protocol.rs#L1936-L2021)。官方 TUI 也明确把 `last_token_usage.total_tokens` 当作当前 context，并用 `model_context_window` 计算剩余百分比（[官方 TUI token usage 实现](https://github.com/openai/codex/blob/main/codex-rs/tui/src/token_usage.rs#L33-L48)）。

`account/usage/read` 还可以按 `threadId` 返回 thread usage 汇总，其中包括 `cachedInputTokens`，但它是计费/用量汇总，不是当前 context window；schema 见 [GetAccountTokenUsageResponse](https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/schema/json/v2/GetAccountTokenUsageResponse.json)。

### 当前仓库缺口

[`src/providers/codex.rs`](../../src/providers/codex.rs) 启动一个临时 `codex app-server --stdio`，只做 `account/read` 与 `account/rateLimits/read`，然后退出。这个连接没有活动 pane 的 `threadId`，也没有订阅现有会话的 `thread/tokenUsage/updated`；所以目前能读 quota reset，却读不到截图中用户所说的 Codex 会话 context summary。

如果要支持 Codex context，优先级应是：

1. 从活动 Codex 会话已经产生的 app-server token-usage 事件接入（需要可靠的 thread/session 关联）；或
2. 使用官方 app-server 的 thread read/resume 读取持久化 usage，并严格按当前 Codex 版本的 schema 适配。

不建议把 `~/.codex/sessions/**/*.jsonl` 的 `token_count` 当作长期公共 API。它是本地 rollout 记录，路径和事件形状可能变，且按文件轮询会产生不必要的磁盘 I/O。

### 不支持与风险

OpenAI API 当前文档有 `prompt_cache_options.ttl`（默认 `30m`，当前文档列出的唯一值）以及旧的 `prompt_cache_retention`（`24h` 语义），见 [Responses API create reference](https://developers.openai.com/api/reference/cli/resources/responses/methods/create)。这只是请求/模型策略，不是 Codex app-server token-usage 事件中的某条 cache entry 的创建时间或到期时间。缓存命中会刷新缓存，单凭 `cachedInputTokens` 无法反推出“还剩几分钟”。

结论：Codex 可以显示 context 使用率和 cached token 数，但不能诚实地显示动态 cached expiry。不要把 quota `resetsAt` 当成 cache expiry。

## Claude Code

### 支持

官方 [Claude Code statusLine 文档](https://code.claude.com/docs/en/statusline#available-data)把以下字段定义为可用数据：

- `context_window.context_window_size`；
- `context_window.used_percentage` 与 `remaining_percentage`；
- `context_window.total_input_tokens` 与 `total_output_tokens`；
- `context_window.current_usage.input_tokens`、`output_tokens`、`cache_creation_input_tokens`、`cache_read_input_tokens`。

官方还说明：`used_percentage` 只按 input 侧计算（包含 cache creation/read，不包含 output），`current_usage` 在首个 API 响应前以及 `/compact` 后短暂为 `null`。因此 parser 遇到缺失/null 时应保留上一个有效快照，不要把它写成 0。

### 当前仓库缺口

[`src/providers/claude.rs`](../../src/providers/claude.rs) 目前只解析 `rate_limits.five_hour` 与 `rate_limits.seven_day`；[`src/configure/claude.rs`](../../src/configure/claude.rs) 的 statusLine wrapper 只保存 quota snapshot。现有 fixture [`tests/fixtures/claude/statusline-both.json`](../../tests/fixtures/claude/statusline-both.json) 也只包含 rate limits。

因此 Claude 是最适合先加 context 的 provider：数据已经由用户正在运行的 Claude Code 通过 stdin 提供，不需要额外网络请求、重新登录或读取 pane。

### 不支持与风险

Anthropic API 的 [prompt caching 文档](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)定义了默认 5 分钟和可选 1 小时 TTL，并说明缓存被使用时会刷新；[API 类型文档](https://platform.claude.com/docs/en/api/typescript/messages)也只把 `ttl` 作为请求里的 `cache_control` 参数。Claude Code statusLine payload 没有返回所用 TTL、cache-created-at 或 expires-at。

结论：Claude 可以显示 context 百分比及 cache read/create 数量，但不显示 cached expiry。`rate_limits.*.resets_at` 仍然只是额度窗口 reset。

## Agy / Antigravity

### 支持

Google 官方 [Antigravity statusLine schema](https://antigravity.google/docs/cli/statusline/#available-json-fields)明确提供：

- `context_window.total_input_tokens`、`total_output_tokens`、`context_window_size`；
- `context_window.used_percentage`、`remaining_percentage`；
- `context_window.current_usage`（示例包含 input/output、`cache_creation_input_tokens`、`cache_read_input_tokens`）；
- `quota[*].reset_time` 与可选 `reset_in_seconds`。

官方示例同时展示了 `gemini-weekly` quota bucket 和 context/cache 数据。现有仓库的 [`src/providers/agy.rs`](../../src/providers/agy.rs) 只解析 quota bucket；fixture [`tests/fixtures/agy/statusline-both.json`](../../tests/fixtures/agy/statusline-both.json) 没有 `context_window`。

### 不支持与风险

官方 statusLine contract 没有 prompt-cache TTL、创建时间或 expiry timestamp。`quota[*].reset_time` 是 Gemini/第三方额度窗口重置，不是 prompt cache 的过期时间。Agy 的 `current_usage` 在首轮/compact 后可能缺失时，应保留上一份 context 快照。

结论：Agy 可以显示 context 百分比和 cache read/create 数量；不能显示 cached expiry，也不能从 quota reset 推导它。

## Grok Build

### 支持

xAI 官方 Grok Build status-line 文档的 [Available data](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/25-status-line.md#available-data)定义了：

- `context_window.context_window_size` 与 `context_tokens`（当前上下文占用）；
- `context_window.used_percentage` 与 `remaining_percentage`；
- `context_window.session_input_tokens`、`session_output_tokens`；
- `context_window.session_usage.input_tokens`、`output_tokens`、`cache_creation_input_tokens`、`cache_read_input_tokens`。

官方 payload 类型和构造逻辑见 [`StatusLineContextWindow`](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-status-line/src/context.rs#L155-L186) 与 [`build_context_window`](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-shell/src/session/acp_session_impl/status_line.rs#L50-L96)。其中 `context_tokens` 是当前 context，`session_usage` 是会话累计值，不能把二者混用。

### 当前仓库缺口

[`src/providers/grok.rs`](../../src/providers/grok.rs) 当前只读取官方 billing credits endpoint；Grok 旧的 quota hook 已经由统一 watcher 清理。要接入 Grok context，需要增加一个官方 statusLine command wrapper，或让用户已有的 statusLine command 经过本插件的链式 wrapper；从本地 unified log 推导虽然可行，但那不是本项目应承诺的稳定 provider contract。

### 不支持与风险

官方 status-line payload 类型中没有 TTL、cache-created-at、expires-at 或 cache policy 字段；cache read/create 只是累计计数。结论：Grok 可以显示 context 百分比和会话 cache 计数，但不能显示 cached expiry。

## “会话总结”应该如何定义

本分支已按这个边界实现：Codex 通过同一次 app-server 连接做一次有上限、
`useStateDbOnly` 的 `thread/list`，只保存每个 thread 的首行短预览；不 resume
thread、不扫 rollout JSONL。默认 prompt 为空时，这个预览作为 `$quota_topic`
的回退值，不新增 metadata token；其他 provider 没有会话摘要回退。

UI 把几个概念分开，避免截图里的 quota 行承担过多含义：

| 展示项 | 语义 | 是否适合放在 provider 行 |
| --- | --- | --- |
| `week 84% reset 6d16h` | 额度窗口剩余百分比与 quota reset | 是，现有字段 |
| `ctx 23%` / `ctx 77% left` | 当前会话 context window 使用率/剩余率 | 可以，来自 statusLine/app-server |
| `cache 12k/80k` | 已读/已写或 cached input token 统计 | 可选，建议窄侧边栏默认不显示 |
| `cache expires 3m` | 服务端某个 cache entry 的剩余 TTL | 四家当前都没有可可靠读取的通用字段，不应显示 |
| `observed 42s ago` | 本地快照新鲜度 | 如需诊断可显示，但不要叫 cache expiry |

context 百分比还要标注口径：Claude/Agy 官方 `used_percentage` 是 input-only；Codex 官方 TUI 的“剩余百分比”会扣除固定 baseline；Grok 的 status-line 已由 provider 计算。不要在共享 renderer 中用一个未经说明的 `total_tokens / context_window_size` 覆盖四家语义。

## 对实现的建议

1. 内部快照增加可选的 `ContextUsage.used_percent`；旧 quota-only cache 反序列化后为 `None`。
2. Claude/Agy 直接扩展现有 statusLine wrapper；输入已经是 provider 产生的 JSON，不增加 API 请求。
3. Codex 当前只接入本地 thread preview；官方 token-usage 事件仍需要活动 thread 关联，
   因此暂不把 context 百分比猜测性接入。
4. Grok 若要支持，新增 statusLine 链式 wrapper；不要恢复每次 tool call 的 quota hook，也不要默认轮询 unified log。
5. 只增加 context/cache token 的最小 metadata interface；不增加 `$cache_expiry` 之类无法保证正确的 token。
6. 加入官方形状 fixture：Codex `ThreadTokenUsageUpdatedNotification`、Claude/Agy statusLine context、Grok `StatusLineContextWindow`；覆盖首轮缺失、compact 后缺失、字段顺序变化和旧 quota-only cache。

## 证据索引

- Codex：[app-server README](https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md#turn-events)、[token-usage event schema](https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/schema/json/v2/ThreadTokenUsageUpdatedNotification.json)、[core token types](https://github.com/openai/codex/blob/main/codex-rs/protocol/src/protocol.rs#L1936-L2021)、[TUI percentage implementation](https://github.com/openai/codex/blob/main/codex-rs/tui/src/token_usage.rs#L33-L48)、[Responses API cache TTL](https://developers.openai.com/api/reference/cli/resources/responses/methods/create)。
- Claude：[statusLine available data](https://code.claude.com/docs/en/statusline#available-data)、[prompt caching TTL](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)、[cache control API type](https://platform.claude.com/docs/en/api/typescript/messages)。
- Agy：[official statusLine schema and payload](https://antigravity.google/docs/cli/statusline/#available-json-fields)。
- Grok：[official status-line contract](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/25-status-line.md#available-data)、[official payload type](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-status-line/src/context.rs#L155-L186)、[official builder](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-shell/src/session/acp_session_impl/status_line.rs#L50-L96)。
