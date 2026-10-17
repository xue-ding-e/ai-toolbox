# 网关 Session 统计对齐 tokei 补齐实现计划

> 状态：**阶段一至阶段五已实施**（2026-10-17）；阶段五（项目维度）曾在 `worktree-tokei-parity` 中经用户决策取消，后于同日按新需求重新实施（schema v29 + 采集层 cwd + 统计页项目 tab）。
> 参照物：cclank/tokei（`D:\GitHub\tokei`，macOS/Windows 用量监控应用，采集器为单文件 Python `usage.30s.py`，17,947 行）。

## 交付状态（2026-10-17）

| 阶段 | 内容 | 状态 | 提交 |
|---|---|---|---|
| 一 | P0 采集修复 ×5（ZCode、Hermes、Grok、Antigravity、Codex） | 已完成 | `9a275894` `09f5e6cd` `f03cde18` `7e3a1373` `11215c5d` |
| 二/三 | 10 个 usage-only 新工具 | 已完成 | `96a8ef11` |
| 四 | 时间区间自然边界 + 模型行展开单价 | 已完成 | `98567a97` |
| 五 | 项目维度（schema v29 + cwd 采集 + 项目视图） | 已完成（2026-10-17 重新实施） | 本次提交 |

门禁：`cargo test` 2863 + 229 + 4 + 29 + 11 通过；`pnpm test` 974 通过；`tsc --noEmit` 与 `pnpm i18n:check` 通过。

**相对原计划的简化与偏离**（均已在模块注释/AGENTS.md 记录）：

- Codex 双写去重不做跨扫描持久化：文件变化必然整文件重解析，`pending_responses`/`response_ids`/`legacy_before_response` 只在单次解析内有效，`SourceState` 未新增 serde 字段。
- WorkBuddy/CodeBuddy 的 `providerData.messageId` 去重按文件进行（tokei 为全局）；CodeBuddy Credit 不折算美元、未接运行日志 TTFT；MiniMax 模型名只取库内列 + 会话回退，未实现日志增量状态机。
- Qwen Code 的 `sync_root` 读的是合并视图，文件遍历仍会单独解析同一批文件；request id 相同、合并视图后写收敛，结果正确但有冗余读取。
- 新工具验证以合成 fixture + tokei 源码口径对齐为准（本机无真实用量），未声称实测账号数据。

## Context（背景）

以 tokei 为参照，对 ai-toolbox 网关「本地 session 用量统计」做了逐工具对比。结论：

- **工具覆盖缺口**：tokei 有真实 token 采集而 ai-toolbox 完全没有的工具共 12 个（其中 macOS-only 的 Grok Bot、Qoder Desktop/QoderWork 不纳入；Qoder CLI/CN token 恒 0 不纳入）。
- **已支持工具的采集缺陷（5 处硬伤）**：
  1. **Antigravity token 全 0**：`session_import/antigravity.rs:206-210` 硬编码 `input_tokens: Some(0)`，只数 MODEL step；真实 token 在 `conversations/<uuid>.db` 的 `gen_metadata` 表（protobuf），tokei 已逆向字段号。
  2. **Grok 采集路径漂移**：`grok.rs` 只读 `sessions/**/updates.jsonl`；本机 Grok 0.2.106 实测该文件不存在，真实 token 在 `logs/unified.jsonl` 的 `shell.turn.inference_done`（349 条，含 prompt/cached/completion/reasoning tokens + ttft）。
  3. **Codex 未解析新版 `token_usage_record` 事件**：本机最新 rollout 实测 160 条（带 response_id + 逐响应 usage），现靠 `event_msg/token_count` 双写兜底（162 条）；上游停双写后 ai-toolbox 会静默丢全部 Codex session 用量。
  4. **ZCode 重复计费**：`zcode.rs` 把 `input_tokens` 列直接当 fresh input；本机实测该列**含缓存**（input=95855 > cache_read=83840），tokei 的口径是 `fresh = input - cache_read - cache_write`。当前实现把缓存同时计入 input 和 cache_read，总量与成本双高估。另漏读 `reasoning_tokens` 列与 `completed_at`。
  5. **Hermes 单库**：只读根 `state.db`；tokei 还扫 `profiles/*/state.db` 并合并 `session_model_usage_v21`（0.19 升级中断表）。

- **范围决策**：
  - ① 修 5 处采集 bug + ② 低成本新工具（Prime Agent、WorkBuddy×2、MiMoCode）+ ③ 全新解析器工具（Qwen Code、Muse Code、Command Code、CodeBuddy、Devin CLI、MiniMax Code）**全部纳入**。
  - 新工具**仅做网关统计采集**（usage-only，照 `kimi_cli`/`zcode` 先例），不建 tab、不做 provider 管理、不走完整 CLI 接入 SOP。
  - **OMP（oh_my_pi）保持独立工具，不并入 pi**（与 tokei 的合并做法相反，维持 ai-toolbox 现状）。
  - 展示层纳入：时间区间补自然边界（yesterday/本周/上周/本月/本年）、模型行展开单价；项目维度统计同日重新实施（统计页「项目」tab）。
  - 不纳入：额度/配额 API 类（Cursor/Zed/Sub2API/z.ai/QwenWork/Kimi 额度等，非本地日志）、Grok Bot（本地快照无 token，仅会话活动）、Qoder 系 4 个变体（CLI/CN token 恒 0 只有时长与上下文比；Desktop/QoderWork 为 IDE 型 SQLite；注：tokei Windows 版经 `%APPDATA%` 路径同样支持它们，非平台限制）、多设备同步、年度回顾、推理 token 独立桶（schema 级变更，单独立项）。
  - 路径解析参考：tokei 的 `usage.30s.py` 用 `HOME`/`APPDATA`/`LOCALAPPDATA` 跨平台解析（51-53 行），移植时用 ai-toolbox 现有的 `dirs` crate 与 runtime_location 机制等价实现。

## 现有架构事实（已核实）

- 采集入口：`tauri/src/coding/proxy_gateway/session_import.rs`（1,351 行），60s 调度 + 手动命令，`GatewaySessionImportCli` 枚举选择工具。
- 16 个 `GatewayUsageTool`（`types.rs:48-69`），usage-only 先例：`kimi_cli`、`zcode`、`hermes`、`dsh`、`pi`、`oh_my_pi`、`omo_native`、`openclaw`。
- SQLite 采集器模式：`hermes.rs`/`open_code.rs`/`open_claw.rs` 各有 `sync_database`，在 `session_import.rs:865-908` 的 sources 循环后独立调用；`open_read_only` + `columns()` 探测可复用。
- 四桶语义：`TokenUsage { input_tokens(fresh), output_tokens, cache_read_tokens, cache_creation_tokens }`（`usage_parser.rs:8-16`）；新解析器必须遵守「input 是非缓存输入」。
- 文件状态账本：`SourceState { mtime/size, parser_revision, records: BTreeMap<id, ImportedRecord> }`；`parsers::revision(cli)` 变更触发旧文件重扫。
- 成本复查：`cost_reconciliation.rs` 的 `supports(tool)` 决定哪些工具参与缺价补算。
- 前端注册面：`GATEWAY_USAGE_TOOLS`（`web/services/proxyGatewayApi.ts:885-888`）+ `settings.gateway.cli.*` i18n key + `GatewayStatisticsView.tsx` cliOptions。
- 时间区间：`GATEWAY_USAGE_RANGE_PRESETS = ['today','1d','7d','14d','30d','custom']`（`web/features/coding/gateway/utils/gatewayFormatters.ts:494`），`resolveGatewayUsageRange` 纯前端解析，后端只收 start/end 时间戳。

## P0 采集缺陷修复

### P0-1 Antigravity：从 gen_metadata 读真实 token

- 数据源：`<antigravity_root>/conversations/<uuid>.db` 的 `gen_metadata` 表（`idx, data`），`data` 为 protobuf。tokei 逆向的字段号（`usage.30s.py:5106-5179`）：
  - 字段 1 = 单次生成记录（嵌套）：19=模型名；4=token 桶 {2:输入（不含缓存），3:输出（含思考），5:缓存读，9:思考，10:回复}；11/12=首字延迟/流式时长（google.protobuf.Duration {1:秒，2:纳秒}）；9→4→1=生成开始时间（秒）。
  - 防错位守卫：token 上界 1e8、时间戳下界 2020-01-01；`out = response(字段10) ?? max(out - thoughts, 0)`。
  - 新版兼容：`steps` 表的 `metadata` 字段 1 是 google.protobuf.Timestamp，gen_metadata 字段 2 是 packed varint 引用的 step idx，时间戳缺失时用它兜底。
- ai-toolbox 现状：`antigravity.rs` 的 `load_db_models` 已打开同一个表但只 ASCII 扫描模型名；`parse()` 对 transcript_full.jsonl 产出全 0 记录。
- 方案：新增 `antigravity::sync_databases`（照 hermes/openclaw 的 SQLite 模式），把 conversations/*.db 作为主源产出真实 token 记录；transcript 解析保留（模型名/会话边界），全 0 记录停发。protobuf 解析手写 varint/wire-type walker（tokei 的 `_parse_proto_fields` 语义），不引依赖。
- 身份键：`SESSION:antigravity:<uuid>:gen:<idx>:<record_index>`（同 idx 可有多条生成记录）。
- parser_revision bump 触发重扫。

### P0-2 Grok：补 unified.jsonl 主源

- 数据源：`<grok_root>/logs/unified.jsonl`，事件 `msg == "shell.turn.inference_done"`，字段（`ctx`）：`prompt_tokens`、`cached_prompt_tokens`、`completion_tokens`、`reasoning_tokens`、`loop_index`、`attempts`、`model_elapsed_ms`；顶层 `ts`、`sid`。
- 口径（tokei `_grok_usage_record` @5634-5636，已核实）：`in = prompt - cached`、`cr = cached`、`out = completion - reasoning`、reasoning 并入 output（四桶语义）；clamp：`cached = min(cached, prompt)`、`reasoning = min(reasoning, completion)`、负数归 0。
- **旧版降级**：ctx 中 4 个 token 键全缺 → 不产生 usage 记录（tokei @5615-5617 同判），只留上下文快照——ai-toolbox 无此卡片维度，直接跳过即可。
- 模型归因：`sid` → `sessions/*/*/summary.json` 的 `info.id` + `current_model_id`（tokei @5592-5593）；cwd 取会话 group 目录的 `.cwd` 文件内容，否则 unquote 目录名（@5578-5594）——ai-toolbox 用 `state.json`/`.project_root`/session header 等价实现，不额外解析 `.cwd` 文件。
- 现有 `updates.jsonl` 解析保留（旧版会话兼容），两源 request_id 键不同天然不冲突。
- root：`default_session_roots` 的 Grok 分支追加 `<root>/logs`；`session_files` 匹配 `unified.jsonl`。
- 身份键：`SESSION:grok:<sid>:<ts>:<loop_index>:<attempts>`（对齐 tokei 的复合 id 语义，同键幂等）。

### P0-3 Codex：解析 token_usage_record

- 事件结构（本机实测）：顶层 `type == "token_usage_record"`，`payload { thread_id, turn_id, session_id, root_turn_id, response_id, usage{input_tokens(含缓存), cached_input_tokens, cache_write_input_tokens, output_tokens(含推理), reasoning_output_tokens, total_tokens}, turn_token_usage, thread_token_usage }`。
- 方案：`parse_codex_lines` 增加该分支；`usage` 字段按 Codex 口径（input 含缓存 → fresh = input - cached - cache_write，复用 `Counters::into_usage` 语义；**保留 cache_write 读取**——tokei 只读 4 字段 cw 恒 0，ai-toolbox 的 AGENTS.md 规则要求 cache_write_input_tokens 解析为 cache_creation_tokens，更完整）；身份键 `SESSION:codex:<thread>:<response_id>`（envelope_id = response_id，可参与跨源匹配）。
- **双写去重状态机**（照 tokei @4565-4624 的抵扣语义，适配到流式解析）：
  - 新式先到：把 `usage` 4 项（input/cached/output/reasoning）缓存进 `pending_responses`；fork 场景（`payload.thread_id != session_id`）不计数但同样入 pending（@4598-4601）。
  - legacy（token_count）后到：`mirror = last_token_usage` 4 项；若 mirror 命中 pending_responses → 该 token_count 不产出 usage 记录（只喂 CodexSnapshot），并从 pending 移除（@4619-4624）。
  - legacy 先到：记 `last_legacy_snapshot = total_key + mirror`；新式后到且 thread_token_usage 快照匹配 → 把 response_id 回填到已计数的 legacy 记录身份上（升级其 request_id 为 response_id 键），不再重复计数（@4584-4597）。
  - `task_started` 事件清空 pending 与 last_legacy_snapshot（@4559-4561）。
  - 跨扫描状态：pending_responses / response_ids / last_legacy_snapshot 存入 `SourceState`（serde 字段，照 codex_snapshots 先例）。
- parser_revision bump（4→5）触发全量重扫，旧记录身份经 `adopt_known_records` 迁移。

### P0-4 ZCode：修正 input 口径 + 补列

- `USAGE_QUERY` 增加 `completed_at`（reasoning_tokens 列本机全 0 且四桶无第五桶，不读；output 保持含 reasoning 的总量语义，与 tokei 成本口径一致——tokei 计价也用 output_total）。
- `into_record` 改为 `fresh = max(input - cache_read - cache_creation, 0)`（tokei @12433 实证口径：ZCode 的 input_tokens 列含缓存）；时间戳优先 `completed_at`（@12421）。
- parser_revision bump（zcode 当前为 2，→3）触发重扫修正历史（总量与成本回落到正确值）。

### P0-5 Hermes：多 profile + v21 表

- `default_session_roots` Hermes 分支：除根 `state.db` 外追加 `profiles/*/state.db`（目录枚举）；`session_import.rs` 的 Hermes sync 调用改为对每个 db 文件执行。
- `hermes.rs`：`read_usage` 对 `session_model_usage_v21` 与 `session_model_usage` **两表都读**（tokei @9635：v21 先、新表后；0.19/schema v22 迁移被外键中断时两表并存），去重键 = `(session_id, model, billing_provider, billing_base_url, billing_mode, task)` 6 元组（@9661-9663），冲突保留 token 总量大的一行（@9664-9679）。
- `sessions` 表回退条件收紧（tokei @9682-9695）：仅对**没有 task=="" 主循环行**的会话才用 sessions 汇总补记录，避免 v22 主循环行与 sessions 汇总重复计算。
- v21 表的累计差额身份与主表区分（表名进身份键），避免互相覆盖；ai-toolbox 现有的 cumulative high-water 机制保留（tokei 是整库重读，我们的增量机制更优，不动）。

## 新工具接入（仅 usage 采集）

### 注册面清单（每个新工具都走这套；★ = 缺一不可，已逐行核对）

**后端类型**：

1. ★ `types.rs:46-69` `GatewayUsageTool` 枚举加 variant（带「Usage-only」doc 注释，照 Zcode 66-68）；★ `all()`（71-91）；★ `as_str()`（93-112，落库 `app_type` 值 + 账本前缀）；`gateway_cli()` **不加**（usage-only 走 `_ => None`）；★ `GatewaySessionImportCli`（1451-1475）加同名 variant。
2. ★ `session_import.rs:20-32` `mod` 声明；★ `import_cli_keys`（379-399）；★ `default_session_roots`（401-524，三种模板：home 元组臂 / 自定义早返回臂 / runtime location 臂）；★ `source_identity`（526-581，通用文件名的工具加进 hash 列表）；★ `session_files`（583-656）。
3. ★ `parsers.rs:110-133` `parse_file` 分派（专用解析器加臂；**注意：usage-only 工具 `gateway_cli()` 为 None，`parse_value` 通用分支的 `from_response_body(cli_key.gateway_cli()?, …)` 会提前返回 None——所有新工具必须写专用解析器或专用分支，不能依赖 generic 路径**）。
4. `parsers.rs:30-43` `revision()` 默认 2，解析语义变更时 bump。
5. SQLite 型工具（Devin/MiniMax/MiMoCode）：在 `session_import.rs:865-908` sources 循环后加独立 `sync_database` 调用臂（照 Hermes/OpenCode）；`cost_reconciliation.rs:156-175` 文件清单加 match 臂 + `:251-256` `cost_records` 分派。`supports()` 默认参与（= tool != Hermes），无需改。
6. `usage_stats.rs:2009-2045` `load_provider_names`：**仅当新工具有自己的 provider 表**才加行（zcode 先例 2019-2021；漏了 = 已落库行在列表/统计里静默丢弃）。`cli_key_from_app_type` 经 `all()` 自动覆盖。
7. **前端**：★ `proxyGatewayApi.ts:883` `GatewayUsageTool` 类型 + ★ `:885-888` `GATEWAY_USAGE_TOOLS`；★ `zh-CN.json`/`en-US.json` `settings.gateway.cli.<key>`（833-856 块）；★ `gatewayFormatters.ts:147` `getGatewayRequestsPerMinute` 的 usage-only 排除列表**必须加新工具**（漏了会显示 0 而非 `-`）。`GatewayStatisticsView`/`GatewayRequestsView` 的 cliOptions 列表驱动自动扩展；`ModelPricingModal`/`GatewaySettingsPanel` 是接管 CLI 列表，不注册。
8. 命令面/调度**无需改**：`proxy_gateway_import_session_usage`/`rebuild_session_usage` 经 `GatewaySessionImportCli` 自动覆盖；60s 调度固定 `All`。
9. 测试：照 `zcode.rs:141-293` 模板（tempfile + rusqlite 建库 + `parse()` 断言 + 负例）+ 端到端 `sync_sources` 幂等 + `usage_summary` 往返；JSONL 工具照 `session_import/tests.rs` 的 `write_jsonl`/`run_sync` harness。

### 工具清单与接入规格（格式细节已从 tokei 源码提取完毕）

> 通用约束：所有新解析器必须写**专用解析器**（`parse_value` 通用分支经 `gateway_cli()?` 对 usage-only 工具直接返回 None）；input 语义按各工具实测判定是否含缓存；复用 `read_jsonl`/`number`/`string`/`timestamp`/`native_record`/`reported_cost` 与 `hermes.rs` 的 `open_read_only`/`columns` SQLite 套件。`cost_reconciliation::supports()` 默认参与；无公开价的模型不计价（ai-toolbox 现有哲学，不落 Opus 兜底）。

| # | 工具 | key | 难度 | 数据源 | 复用 |
|---|---|---|---|---|---|
| 1 | MiMoCode | `mimocode` | 低 | XDG 路径序（`%LOCALAPPDATA%`/`%APPDATA%`/`~/.local/share`/`$MIMOCODE_HOME/data`）下 `mimocode.db` 或 mtime 最新的 `mimocode-*.db` 渠道库 | **直接复用 `open_code.rs`** 的 session_v2/message 双 schema 扫描，只加路径发现 |
| 2 | Prime Agent | `prime_agent` | 低 | `~/.prime/agent/sessions/*.jsonl` + `~/.prime/agent/session-artifacts/**/*.jsonl`（RLM 子代理独立会话） | **复用 `pi.rs::parse_entry`**；补 `model_change` 事件（`{provider, modelId}`）、canonical 副本择优 + 全局事件键去重（`{session_id}:{event_id}`，event_id = obj.id/message.id/message.responseId）；`child_usage_attributed` 靠现有未知 type 过滤天然剔除 |
| 3 | Devin CLI | `devin` | 低 | `~/.local/share/devin/cli/sessions.db`（候选 `$TOKEI_DEVIN_CLI_DB` 等同 tokei 218-223 的多平台序） | 新 SQLite 解析器，复用 `hermes.rs` 套件；`message_nodes.chat_message` JSON 的 `metadata.metrics{input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens}`（四桶独立、input 不含缓存）；模型 `metadata.generation_model` 回退 `sessions.model`；去重键 `(session_id, metadata.created_at 原文, model, 四桶)`（森林兄弟节点双写）；时间优先 `metadata.created_at`（µs）兜底行 `created_at` |
| 4 | Muse Code | `musecode` | 低-中 | `~/.local/share/muse/sessions/`（5 层或 1 层双模式） | 新写；行过滤 `runtime.session.metadata`/`run.model.configured`/`model_completed`；三层嵌套 `payload.event.usage{input_tokens(含缓存), cached_tokens(兜底 cache_read_tokens, 钳≤input), output_tokens, cache_write_tokens, reasoning_tokens}`；去重 `record.source_run_record_id`（**无 id 不去重照常计入**）；模型 `same-as-main` → `run.model.configured` 的 run_id 映射两级回退；时刻 `record.recorded_at`（µs/ms/s 三态判定）；项目 `payload.record.workspace_root` |
| 5 | Command Code | `cmdcode` | 低-中 | `~/.commandcode/projects/*/*.jsonl` | 新写；**排除**文件名 `hooks-audit-*` 与 stem 含 `.`（checkpoints `<session>.<n>.jsonl`）；usage 双取 `record.usage` → `nested.usage`，四桶 `inputTokens/outputTokens/cacheReadTokens/cacheWriteTokens`（彼此独立、input 不含缓存）；`costUsd` 真实成本（钳 0..1e6 有限值）；`session.cwd` 项目、`model_change` 换模型；去重 sha256（消息 id/messageId + timestamp），跨文件全局 `seen_events` |
| 6 | Qwen Code | `qwencode` | 中 | 逐请求 `~/.qwen/usage/token-usage-*.jsonl`（runtime dir = `$QWEN_RUNTIME_DIR` ∥ settings 的 `advanced.runtimeOutputDir` ∥ `~/.qwen`）+ 会话汇总 `~/.qwen/usage_record.jsonl` | 新写；逐请求字段 `inputTokens(含缓存)/cachedTokens/outputTokens/thoughtsTokens`，`cached=min(cached,input)`，`inp=input-cached`，thoughts 并入 output；按记录 `id` 后写覆盖去重。**双源合并**：保留全部逐请求；汇总按 sessionId 减去同会话逐请求已覆盖模型的同字段（负值钳 0、cr/cw/in 重排、hour=None 只补差额）；未被覆盖会话的汇总原样使用 |
| 7 | WorkBuddy | `workbuddy` | 中 | `~/.workbuddy/projects/**/*.jsonl` | 与 CodeBuddy 共用一套新解析器（`workbuddy_family.rs`，按 tool 参数化） |
| 8 | WorkBuddy Intl. | `workbuddy_ai` | 中 | `~/.workbuddy-ai/projects/**/*.jsonl` | 同上；缓存/账本/统计各自独立 key |
| 9 | CodeBuddy Code | `codebuddy` | 中-高 | `~/.codebuddy/projects/**/*.jsonl`（含 subagents，递归自然覆盖） | 与 WorkBuddy 共用解析器 + 独有增量：**三来源 usage 合并**（`message.usage` → `providerData.usage` → `providerData.rawUsage`，后者只补全不重复累计）、cache 键别名族（`cache_read_input_tokens|cacheReadInputTokens|cache_read|cacheRead|cached_tokens|…|prompt_cache_hit_tokens` 及 inputTokensDetails 嵌套）、inclusive/exclusive total 判别（total==in+out 则 cr/cw/in 按差减）、**Credit 累计**（`providerData.rawUsage.credit`，仅精确价才计美元）、全局去重 `sha256(providerData.messageId)`、运行日志 TTFT 关联（`logs/**/*.log` 的 `First meaningful token received: … requestId=…, ttft=` / `Stream completed: … elapsed=` 正则） |
| 10 | MiniMax Code | `minimax` | 高 | `~/.minimax/v2/sqlite/runtime-state.sqlite` + `~/.minimax/v2/observability/logs/runtime-*.log` | 新 SQLite 解析器（状态最重）；`local_runtime_token_usage` 四桶并列，**reasoning 仅当 `raw.totalTokens == inp+out+cr+cw+reason` 才计**（对不上宁可少算）；模型名按 `turn_id` 关联日志 `llm_response_identifiers` JSON（增量字节偏移、半行缓冲、文件重写检测、行号区间 runs 缓存 + pending TTL 2 天、库重建清零 runs）；项目 `local_runtime_sessions` 的 workspace 列（默认 workspace 排除） |

**各工具身份键（request_id）**：`SESSION:{tool}:{session}:{identity}`（`native_record` 规范）；identity 按上表去重键语义选取。`source_identity` 判定：WorkBuddy 系/CodeBuddy/Command/Muse 的会话文件名为 UUID，走 stem 分支；Qwen 的 `token-usage-*.jsonl` 文件名含日期，走 stem 分支。

**P0-3 Codex 补充细节**：fork 场景 `payload.thread_id != session_id` 的记录跳过计数但镜像入 pending；`task_started` 清空镜像状态；tokei 不读 `cache_write_input_tokens`（cw 恒 0），ai-toolbox 保留读取（AGENTS.md 既有规则，更完整）。**本轮不做**：Codex Reserve 通道（gpt-reserve/Luna 独立计价）与高上下文加价（input>272K ×2/×1.5）——涉及计价体系变更，留待后续；`proxy_gateway_rebuild_session_usage` 命令已存在（备份→按 app_type 清→重同步），P0 修完后用它重建历史。

## 展示层增强（落点已核对）

1. **时间区间自然边界**：`gatewayFormatters.ts:494` `GATEWAY_USAGE_RANGE_PRESETS` 追加 `yesterday`/`week`/`last_week`/`month`/`year`；`resolveGatewayUsageRange`（:524-553）补分支——注意现有契约 `endDate` 恒为 now，`yesterday` 需返回**过去的 end**（本地今天零点），后端只做 BETWEEN 过滤可直接接受；`resolveGatewayRequestRange`（:555-571）自动继承；i18n `gateway.page.statistics.range.*` 两语言各加 5 键（zh-CN.json:91-98 / en-US.json:91-98）。后端**无预设概念无需改**；趋势桶阈值 `usage_stats.rs:894`（≤24h 小时桶）恰好让 yesterday 落在小时桶，week/month/year 走日桶，`formatTrendDateLabel` 两种桶已兼容。测试：`web/test/features/coding/gateway/utils/gatewayFormatters.test.ts:53-79` 补新预设断言。
2. **模型行展开单价**：统计页 models 表行加 expandable，展开显示四桶单价（input/output/cache_read/cache_creation per million，读现有 model_pricing 查询命令）+ `has_pricing` 状态；未定价显示现有 `unpricedModelIds` 提示语义。新工具的模型要能定价须在 `tauri/resources/model_pricing.json` 补价（守卫 `every_published_preset_model_resolves_to_model_pricing`，豁免集仅图片模型）。
3. **项目维度（已实施）**：采集层解析各 CLI 记录的工作目录（Claude 每行 `cwd`、Codex `session_meta.payload.cwd`、Pi/OMP/Prime/DSH/ZCode/Cmdcode 会话头、OpenCode/ZCode `session.directory`、Grok `unified.jsonl` 的 `session created` ctx、WorkBuddy 系 `cwd`、Kimi `state.json`/`kimi.json`、Gemini `.project_root`），`proxy_request_logs.project_path`（v29）保存；`usage_daily_rollups` 重建主键加入 `project_path`，日志裁剪后仍保留项目拆分；统计页新增「项目」tab（按项目路径合并、跨 CLI 汇总、会话数/请求数/Token/成本/主模型/最近活跃）。代理转发流量没有工作目录，不进入项目维度。

## SQLite 型新工具的挂载方式

MiMoCode/Devin/MiniMax 三个 SQLite 工具照 Hermes/OpenCode 模式挂载：

- `session_import.rs:865-908` sources 循环后加 `sync_database` 调用臂（MiMoCode 复用 `open_code::sync_database` 逻辑但 source_id 前缀独立；Devin/MiniMax 新写，水位：MiniMax 用库文件 mtime/size，Devin 同）。
- `cost_reconciliation.rs:156-175` 文件清单加 match 臂 + `:251-256` `cost_records` 分派（照 `open_code::cost_records` 343-352 先例）。
- MiniMax 的日志关联状态（runs/through/pending）存进 `SourceState`（serde 字段，照 `cumulative_usage` 先例）；**口径升级时 runs 保留**（是从已删日志攒下的证据）、days 作废重算。

## 实施阶段（增量交付，每阶段独立提交、独立可验证）

1. **阶段一：P0 采集修复**（5 个独立提交）
   - 1a. ZCode input 口径 + completed_at（revision 2→3，改动最小先行）
   - 1b. Hermes 多 profile + v21 表合并
   - 1c. Grok unified.jsonl 主源（新增 `session_files` 匹配 + 解析分支）
   - 1d. Antigravity gen_metadata protobuf 真实 token（`antigravity::sync_databases` + 手写 protobuf walker）
   - 1e. Codex token_usage_record + 双写抵扣状态机（revision 4→5，`SourceState` 加 pending/snapshot serde 字段）
   - 验证：本机真实数据（Antigravity/Grok/Codex/ZCode）+ 合成 fixture；完成后用 `rebuild_session_usage` 重建历史（该命令已存在：备份→按 app_type 清 session 明细/rollup/账本→重同步，proxy 行不动）。
2. **阶段二：低成本新工具**（每个一个提交，按注册面清单逐项过）
   - 顺序：MiMoCode（几乎纯复用）→ Prime Agent（pi.rs 扩展）→ Devin（新 SQLite，量小）→ Command Code → Muse Code。
3. **阶段三：中高成本新工具**
   - 顺序：workbuddy_family.rs 共用解析器（WorkBuddy + WorkBuddy Intl 一次交付）→ CodeBuddy（在 family 解析器上加三来源/credits/全局去重/日志 TTFT）→ Qwen Code（双源合并）→ MiniMax（日志关联状态机）。
4. **阶段四：展示层**
   - 4a. 时间区间自然边界（纯前端 + i18n + 测试）
   - 4b. 模型行展开单价
5. **阶段五：项目维度**（schema v29 + 采集层 cwd 解析 + 统计页项目视图）——2026-10-17 重新实施。

每阶段收尾跑对应测试；跨阶段交付前跑全量门禁（`cargo test` + `pnpm test` + `tsc --noEmit`）。新工具一律 usage-only 注册面，不动 `GatewayCliKey`/接管/托盘/备份。

## 验证

- 每个解析器：合成 fixture 单测（照 `zcode.rs:141-293` 模板：tempfile/rusqlite 建库或 `tests.rs` 的 `write_jsonl`/`run_sync` harness）+ 负例（缺表/缺文件/坏行）+ `sync_sources` 幂等 + `usage_summary` 往返。
- P0 修复：本机真实数据验证（Antigravity db / Grok unified.jsonl 349 条 / Codex rollout 160 条 / ZCode db 5,364 行；Hermes 本机无数据用合成）。
- **本机数据盘点（2026-10-16 实测）**：新工具中本机仅 Qwen Code（`~/.qwen` 无 usage 目录）与 WorkBuddy（`~/.workbuddy` 无 projects）有安装痕迹但**无用量数据**；Muse/Command/CodeBuddy/Prime/Devin/MiniMax/MiMoCode 本机均未安装。因此新工具的验证依据是「合成 fixture 回归 + tokei 源码口径对齐」，不声称已实测真实账号会话（与 `docs/gateway-native-session-usage-research.md` 的既有先例一致）。Qoder 本机有真实会话 JSONL，但 token 恒 0，已定不纳入。
- 前端：统计页筛选器出现新工具、时间区间新预设切换、模型行展开、i18n 两语言对齐（`pnpm i18n:check`）。
- 全量门禁：`cd tauri && cargo test`、`pnpm test`、`pnpm exec tsc --noEmit`。

## 文档与记忆更新（实施时同步）

- 本文件即计划落盘版，含对比报告结论与全部实施规格。
- `proxy_gateway/AGENTS.md`：P0 修复涉及的关键语义（Codex 双写抵扣、Grok 双源、Antigravity protobuf 字段号、ZCode input 口径、Hermes v21）按模块文档纪律写入对应小节。
- 记忆库 `projects/ai-toolbox/gateway-session-usage-coverage.md`：实施完成后更新覆盖矩阵（16→26 工具）与 P0 修复状态。
