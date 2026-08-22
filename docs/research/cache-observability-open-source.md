# Cache 命中率、TTL 与 context 可观测性调研

研究日期：2026-08-22（Asia/Shanghai）
范围：本仓库的 Codex、Claude Code、Agy/Antigravity、Grok Build；同时检查了几个开源 statusline/HUD 实现。
目标：找出可以在不发送模型请求、不重新登录、不 resume 活动会话的前提下显示的指标，并区分“可观测事实”和“本地估算”。

## 结论先行

1. **context 百分比可以按 provider 原生字段显示，并建议按 0–50 / 50–80 / 80+ 分成正常、警告、危险三色。** Claude/Agy 的 `used_percentage` 由 CLI 计算，Codex 的活动会话 token-usage 事件带 `last.totalTokens/modelContextWindow`，Grok 的 status/headless 数据也有当前 context token。不要在共享渲染器里重新推导四家百分比，否则会混用不同口径。
2. **缓存命中率可以安全显示“最近一次 API 调用”的比例。** Claude/Agy 直接从 statusLine 的 `current_usage` 读取；Grok 的官方 headless 结果有 `cache_read_input_tokens`；Codex app-server 的官方 token-usage schema 有 `cachedInputTokens` 和 `cacheWriteInputTokens`。推荐公式：

   ```text
   hit_percent = cache_read / (fresh_input + cache_creation + cache_read) * 100
   ```

   `cache_creation` 是写入，不算命中；把它放进分母比 `read / (fresh + read)` 更诚实。分母为零时隐藏该字段。

3. **“缓存还剩几分钟”只有 Claude 可以做一个近似显示，而且不应作为准确 expiry。** Claude API 返回 5m/1h 写入桶，官方说明缓存使用时会刷新，且 TTL 从请求开始计时；statusLine 本身没有 entry 的 `created_at`/`expires_at`。开源项目通过读取当前会话 transcript 的尾部推断 TTL 桶和最近 timestamp，但这仍是近似值，不能代表多个 cache breakpoint 的统一到期时间。
4. **Codex/OpenAI、Agy、Grok 当前没有可供本插件诚实显示的动态 cache-entry expiry。** OpenAI 的 `prompt_cache_options.ttl` 是请求策略（不是活动 cache entry 的剩余时间）；Agy/Grok status payload 只有 token 计数；Codex token-usage event 也没有 expiry timestamp。
5. **实现把 cache read/write/fresh 计数和命中率合并进现有 `$quota_context`，不增加 `$cache_expiry`。** 计数来自正在运行的 CLI/statusLine 输入，不联网、不登录、不消耗模型 token。Claude 在 statusLine 同时提供 transcript 路径和明确 5m/1h bucket 时，默认显示带 `≈` 的有界 TTL 估算；证据不足就隐藏。

## 四个 provider 的可获取字段

| Provider | context | cache read/write | TTL/过期时间 | 安全来源 |
| --- | --- | --- | --- | --- |
| Claude Code | `context_window.used_percentage`；`context_window.context_window_size` | `context_window.current_usage.input_tokens`、`cache_creation_input_tokens`、`cache_read_input_tokens` | API usage 有 `cache_creation.ephemeral_5m_input_tokens` / `ephemeral_1h_input_tokens`，但 statusLine 没有 entry expiry；可由 transcript 做近似 countdown | 当前 statusLine 的 stdin JSON |
| Agy/Antigravity | `context_window.used_percentage`、`context_window.context_window_size` | 同名 `current_usage` 字段 | 官方 statusLine 没有 TTL/expiry 字段 | 当前 statusLine 的 stdin JSON |
| Codex | app-server `thread/tokenUsage/updated` 的 `last.totalTokens` / `modelContextWindow` 可计算；必须关联活动 `threadId` | app-server schema 的 `cachedInputTokens`、`cacheWriteInputTokens` | event/schema 无 expiry；OpenAI API TTL 是请求级策略 | 活动 app-server 事件；临时独立 app-server 无法读取活动 thread 的实时事件 |
| Grok Build | 官方 status/headless payload 的 context token/percentage | headless `usage.cache_read_input_tokens`、`cache_creation_input_tokens`；统一 TokenUsage 也保留 cache-read | 当前 xAI status/headless contract 无 cache-entry TTL | 当前会话 event/status 输出；不要为了 HUD 额外启动模型 turn |

官方字段依据：

- [Claude statusLine 可用字段与 context 口径](https://code.claude.com/docs/en/statusline#available-data) 明确给出 `current_usage` 四类 token，并说明 `used_percentage` 只计算 input + cache creation + cache read；statusLine 命令在本地运行且不消耗 API token。
- [Anthropic prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching) 说明默认 5 分钟、可选 1 小时、命中会刷新 TTL、TTL 从请求开始计时，并给出 `cache_creation` 的 5m/1h 分桶。
- [Antigravity statusLine schema](https://www.agy.dev/docs/cli/statusline/#available-json-fields) 的示例包含 context window、cache read/create 和 quota reset；没有 cache expiry 字段。
- [Codex `ThreadTokenUsageUpdatedNotification` schema](https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/schema/json/v2/ThreadTokenUsageUpdatedNotification.json) 包含 `last`、`total`、`modelContextWindow` 及 `cachedInputTokens`、`cacheWriteInputTokens`；没有 TTL。
- [Grok headless usage contract](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/14-headless-mode.md) 明确区分 uncached `input_tokens`、`cache_read_input_tokens` 和 `cache_creation_input_tokens`；[Grok normalized TokenUsage](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-sampling-types/src/conversation.rs) 也保留 cache-read/write 语义。

## 可借鉴的开源实现

### 1. Claude cache hit：只读 statusLine，零网络

[`vfmatzkin/claude-statusline`](https://github.com/vfmatzkin/claude-statusline) 直接读取 stdin 的 `current_usage`，把 `cache_read_input_tokens` 与输入总量换算成 `cache 96%`；不启动后台进程，也不调用 API。适合作为本项目默认方案：在现有 Claude/Agy wrapper 中解析字段即可。

[`waelmas/claude-stat`](https://github.com/waelmas/claude-stat) 也从 statusLine 读取 context，并通过 transcript 累加 `cache_read_input_tokens`、`cache_creation_input_tokens` 与 fresh input，计算跨 turn 的命中比例。它用文件 size 做缓存，典型 repaint 开销约 5–20ms，而且没有后台 daemon 或网络请求。这里的“跨 turn”是本地统计，不是 provider 返回的单个 session 命中率；要避免重复计数，必须按 transcript offset/size 或 `prompt_id` 去重。

推荐借鉴：

- 默认只显示最新调用的 raw read/write/fresh 与 hit%；无需读取 transcript。
- 需要 session 累计时，使用 `session_id + transcript_path` 做 key，并缓存文件 size/offset；每次只读取新增尾部，不扫完整历史。
- `current_usage` 在首个响应及 `/compact` 后可能为 `null`；保持上次有效值，不能写成 0。

### 2. Claude TTL：可做近似，但不要伪装成 expiry

[`ilia-pluzhnikov/claude-code-statusline`](https://github.com/ilia-pluzhnikov/claude-code-statusline) 是目前看到的最完整方案：

- 计数直接来自 statusLine `current_usage`；
- 只读取 transcript 尾部约 16 KiB；
- 从最近的 assistant usage 记录中识别 `cache_creation.ephemeral_5m_input_tokens` 或 `ephemeral_1h_input_tokens`；
- 用记录 timestamp + 5m/1h 渲染一个倒计时；
- 配合 `refreshInterval: 60`，避免 idle 时倒计时冻结。

其文档同时承认 stdin 没有 TTL bucket 和 per-message timestamp，因此这是估算，不是服务端 expiry。Anthropic 官方还说明 TTL 从请求开始计时，并且每次命中都会刷新；长响应、多个 breakpoint、compact、模型/工具切换都会让“最后一条 assistant timestamp + TTL”偏离真实条目状态。本项目只在证据齐全时默认显示这个带 `≈` 的提示，其他情况隐藏。

### 3. Codex 本地 usage：可统计命中率，但当前不适合实时 watcher

[`razzededge/codex-usage-audit`](https://github.com/razzededge/codex-usage-audit) 只读本地 Codex rollout/session JSONL，统计 input、cached input、cache write、output 及 context pressure；它使用私有 aggregate cache，未变化的 rollout 不重复解析，并且运行时不联网。

[`harveyxiacn/codex-usage-monitor`](https://github.com/harveyxiacn/codex-usage-monitor) 采用类似策略，读取 `~/.codex/sessions` 的 `token_count` 事件，计算 `cached_input_tokens / input_tokens` 和最新请求 context fill。

这些项目证明“本地 JSONL 能做历史命中率”，但不应直接复制到本插件的每分钟全局 watcher：

- rollout 文件含会话内容，扫描/解析有隐私和 I/O 成本；
- 每个 pane 对应的活动 thread/session 关联不稳定；
- token_count 是 turn 事件，和 app-server `thread/tokenUsage/updated` 的实时事件不是同一订阅；
- 用户明确要求不 resume、不扫描 rollout 时，应保持当前 Codex `thread/list` 仅用于会话预览，不新增 rollout 扫描。

长期可选方向是：由 Codex 官方 hook/长连接提供 event-driven token usage，再把事件按 `threadId` 写入本地 cache；没有可靠关联前不要猜 context 或命中率。

### 4. Grok/Agy HUD：本地读取可用，但能力边界不同

[`xiyouMc/grok-hud`](https://github.com/xiyouMc/grok-hud) 说明 Grok 没有 Claude 风格的原生 statusLine API，因此外部 HUD 读取 `~/.grok` 的 `signals.json`、`summary.json`、`updates.jsonl`；context 来自 signals，billing API 结果约 60 秒缓存。它没有通用 cache TTL，也不建议为了显示数据启动新的模型请求。

[`weby-homelab/antigravity-cli-statusline`](https://github.com/weby-homelab/antigravity-cli-statusline) 直接消费 Antigravity stdin payload，显示 context/session/current-turn tokens 和 quota；没有额外网络调用。Agy 的官方 stdin 已经给出 cache read/create，因此本项目无需仿照外部 server polling。

## 推荐实现分层

### 默认层（建议现在实现）

1. `ContextUsage` 保持 provider 原生百分比；用独立的紫色强调色与额度健康色区分，
   不把四家不同口径的 context 百分比重新套成统一危险阈值。
2. `CacheUsage` 使用可选字段：`fresh_input_tokens`、`read_tokens`、`creation_tokens`、`hit_percent`。
3. Claude/Agy 从当前 statusLine payload 解析；Grok/Codex 只有在当前安全链路拿到官方字段时才显示，缺失就隐藏。
4. Herdr metadata report 有 16-token 上限；不要另加 `$quota_cache`，应把 `cache 87%`（或 R/W）并入现有 context token，避免触发报告失败。
5. 不把 quota reset 当作 cache expiry。

### 有界 TTL 诊断层

本项目采用 ilia 项目的边界：只读取 `transcript_path` 尾部 16 KiB，并显示
`ttl≈54m` 这类带剩余时间的本地估算（5m/1h bucket），不显示“expires in”。任何解析失败、compact、字段缺失
都不生成新估算，并保留上次合法值；不会把估算放进独立 metadata token 或全局 watcher。

### 明确不做

- 不为查询 cache 而调用 API、重新登录、发起 warm-up/model 请求；
- 不 resume 活动 Codex thread；
- 不把 quota `resets_at/reset_time` 伪装为 cache expiry；
- 不对每个 pane 启动独立 watcher 或重复读取 pane；
- 不默认扫描完整 transcript/rollout JSONL。

## 风险与验证重点

- **重复统计**：statusLine 可能在同一 turn 重绘多次；累计值必须用 `prompt_id`、transcript offset 或内容指纹去重，不能每次调用都加总。
- **口径漂移**：Claude 官方 `used_percentage` 是 input-only；Codex TUI 还有 baseline；Grok/Agy 可能由自身 CLI 计算。保存原始 provider 字段和 `source`，不要在 renderer 中二次“统一计算”。
- **缓存不是单一条目**：一个请求可有多个 breakpoint 和不同 TTL；任何倒计时都只能是近似提示。
- **compact/首轮空值**：`current_usage=null` 或字段缺失时保留旧值，避免侧栏闪烁和 metadata repaint。
- **性能**：cache 计数解析是 O(1) JSON；Claude TTL 每次 statusLine 最多读取 transcript 最后 16 KiB，不能从头扫文件，也不启动 daemon；全局 watcher 和 pane 访问路径不变。

## 参考来源

- [Claude statusLine data and local execution](https://code.claude.com/docs/en/statusline#available-data)
- [Anthropic prompt caching: TTL, refresh, usage fields](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
- [Antigravity statusLine schema](https://www.agy.dev/docs/cli/statusline/#available-json-fields)
- [Codex app-server token usage schema](https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/schema/json/v2/ThreadTokenUsageUpdatedNotification.json)
- [Grok headless usage fields](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/14-headless-mode.md)
- [Grok normalized token usage](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-sampling-types/src/conversation.rs)
- [waelmas/claude-stat](https://github.com/waelmas/claude-stat)
- [ilia-pluzhnikov/claude-code-statusline](https://github.com/ilia-pluzhnikov/claude-code-statusline)
- [vfmatzkin/claude-statusline](https://github.com/vfmatzkin/claude-statusline)
- [razzededge/codex-usage-audit](https://github.com/razzededge/codex-usage-audit)
- [harveyxiacn/codex-usage-monitor](https://github.com/harveyxiacn/codex-usage-monitor)
- [xiyouMc/grok-hud](https://github.com/xiyouMc/grok-hud)
- [weby-homelab/antigravity-cli-statusline](https://github.com/weby-homelab/antigravity-cli-statusline)
